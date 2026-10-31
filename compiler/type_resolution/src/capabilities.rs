//! Candidate qualification against an immutable implementation world. Type
//! matching happens first; nested capability obligations settle before ranking.
//! A recursive goal is pending, never evidence for itself.
use crate::implementation::{
    Applicability, Index, InherentIndex, Selection, TypeEvidence, TypeResolution,
};
use crate::ptc::{Budget, Context, Formula, Outcome};
use nir::program::{Implementation, InterfaceInstance, Trait, TraitRequirement};
use nir::{DeclarationId as Id, Term};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;

mod closed_inputs;
mod conditions;
mod propositions;
pub use conditions::ConditionSupport;
pub use propositions::{TraitDecision, TraitTruth, WorldProposition};

#[derive(Clone, Debug)]
pub enum Qualification {
    Proved(Vec<Witness>),
    Excluded(Vec<Witness>),
    Pending(String),
}
#[derive(Clone, Debug)]
pub struct Witness {
    pub requirement: TraitRequirement,
    pub origin: Origin,
}
#[derive(Clone, Debug)]
pub enum Origin {
    Given(TraitRequirement),
    Implementation {
        declaration: Id,
        arguments: Vec<Term>,
        premises: Vec<Witness>,
    },
    /// All candidates excluded in this query's immutable world. This is not a
    /// global PTC theorem and must not survive implementation-world changes.
    Absent,
}
#[derive(Debug)]
pub struct Resolution {
    // Shared by the entire recursive query, never reconstructed from a public
    // assessment table. A proof cannot escape its caller's premise scope.
    scope: Arc<ResolutionScope>,
    pub types: TypeResolution,
    pub qualifications: BTreeMap<Id, Qualification>,
    pub condition_support: BTreeMap<Id, Vec<ConditionSupport>>,
}
#[derive(Debug)]
struct ResolutionScope {
    context: Context,
    assumptions: Vec<TraitRequirement>,
}
impl ResolutionScope {
    fn capture(world: &World<'_>) -> Arc<Self> {
        Arc::new(Self {
            context: world.context.clone(),
            assumptions: world.assumptions.to_vec(),
        })
    }
    fn matches(&self, world: &World<'_>) -> bool {
        self.context == *world.context && self.assumptions == world.assumptions
    }
}
pub struct World<'a> {
    pub index: &'a Index,
    pub implementations: &'a [Implementation],
    pub traits: &'a [Trait],
    pub assumptions: &'a [TraitRequirement],
    pub context: &'a Context,
}

pub(crate) use crate::implementation::equivalent_domains;

/// Member lookup roots implied by capabilities. Parents are traversed for
/// their parameter capabilities, but remain inside their child's member lookup
/// so a child's own member continues to shadow a same-named parent member.
pub fn capability_sources(
    traits: &[Trait],
    assumptions: &[TraitRequirement],
    budget: Budget,
) -> Result<Vec<TraitRequirement>, String> {
    capability_sources_in_scope(traits, assumptions, budget, &[])
}

/// Collect member lookup roots without losing caller-bound subjects. Parents
/// are traversed for their contracts, but do not replace child lookup roots.
pub fn capability_sources_in_scope(
    traits: &[Trait],
    assumptions: &[TraitRequirement],
    budget: Budget,
    frames: &[Vec<nir::Sort>],
) -> Result<Vec<TraitRequirement>, String> {
    capability_sources_with_domains(traits, assumptions, budget, frames, &BTreeMap::new())
}

pub fn capability_sources_with_domains(
    traits: &[Trait],
    assumptions: &[TraitRequirement],
    budget: Budget,
    frames: &[Vec<nir::Sort>],
    domains: &BTreeMap<Id, Term>,
) -> Result<Vec<TraitRequirement>, String> {
    capability_sources_with_layouts(traits, assumptions, budget, frames, domains, None)
}

pub fn capability_sources_with_layouts(
    traits: &[Trait],
    assumptions: &[TraitRequirement],
    budget: Budget,
    frames: &[Vec<nir::Sort>],
    domains: &BTreeMap<Id, Term>,
    layouts: Option<&nir::program::Package>,
) -> Result<Vec<TraitRequirement>, String> {
    for assumption in assumptions {
        assumption
            .instantiate_parameters_in_scope(&[], &[], frames)
            .map_err(|error| format!("invalid capability lookup scope: {error}"))?;
    }
    let mut output = assumptions.to_vec();
    let mut pending: Vec<_> = assumptions.iter().cloned().map(|a| (a, 0)).collect();
    let mut seen = HashSet::new();
    let mut remaining = budget.steps;
    while let Some((current, depth)) = pending.pop() {
        if remaining == 0 {
            return Err("capability member lookup budget exhausted".into());
        }
        remaining -= 1;
        if !seen.insert((current.interface_instance(), current.subject.clone())) {
            continue;
        }
        if depth >= budget.max_depth.min(128) {
            return Err("capability member lookup depth exhausted".into());
        }
        let Some(contract) = traits.iter().find(|t| t.declaration == current.interface) else {
            continue;
        };
        if contract.type_parameters.len() != current.interface_arguments.len() {
            continue;
        }
        contract.validate_arguments_with_layouts(
            &current.subject,
            &current.interface_arguments,
            frames,
            traits,
            domains,
            &mut remaining,
            |_, _, _| false,
            layouts,
        )?;
        let arguments: Vec<_> = std::iter::once(current.subject.clone())
            .chain(current.interface_arguments.iter().cloned())
            .collect();
        let substitution = contract
            .input_substitution(&arguments, frames, &mut remaining)
            .map_err(|error| error.to_string())?;
        for capability in &contract.capabilities {
            let capability = substitution
                .instantiate_requirement_bounded(capability, &mut remaining)
                .map_err(|error| format!("invalid capability member edge: {error}"))?;
            output.push(capability.clone());
            pending.push((capability, depth + 1));
        }
        for parent in &contract.parents {
            let parent = substitution
                .instantiate_interface_bounded(parent, &mut remaining)
                .map_err(|error| format!("invalid parent member edge: {error}"))?;
            pending.push((
                TraitRequirement {
                    subject: current.subject.clone(),
                    interface: parent.declaration,
                    interface_arguments: parent.arguments,
                },
                depth + 1,
            ));
        }
    }
    Ok(output)
}

/// Type guarantees exported by already-assumed interface capabilities.
/// Parameter-only requirements remain interface-formation obligations: they
/// must not bootstrap their own formation. Only predicates mentioning Self
/// are discharged when an implementation subject is selected.
///
/// This is bounded forward implication, not implementation search. Exhaustion
/// can omit facts but cannot manufacture a proof or assume a candidate.
pub fn subject_requirements(
    traits: &[Trait],
    assumptions: &[TraitRequirement],
    budget: Budget,
) -> Vec<nir::program::TypePredicate> {
    subject_requirements_with_domains(traits, assumptions, budget, &BTreeMap::new())
}

/// Propagate subject guarantees with immutable declaration-domain evidence.
/// Domain evidence admits inputs; it does not establish parameter-only requirements.
pub fn subject_requirements_with_domains(
    traits: &[Trait],
    assumptions: &[TraitRequirement],
    budget: Budget,
    domains: &BTreeMap<Id, Term>,
) -> Vec<nir::program::TypePredicate> {
    subject_requirements_with_layouts(traits, assumptions, budget, domains, None)
}

pub fn subject_requirements_with_layouts(
    traits: &[Trait],
    assumptions: &[TraitRequirement],
    budget: Budget,
    domains: &BTreeMap<Id, Term>,
    layouts: Option<&nir::program::Package>,
) -> Vec<nir::program::TypePredicate> {
    let mut pending: Vec<_> = assumptions.iter().cloned().map(|a| (a, 0)).collect();
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    let mut remaining = budget.steps;
    while let Some((assumption, depth)) = pending.pop() {
        if remaining == 0 {
            break;
        }
        remaining -= 1;
        if depth >= budget.max_depth.min(128)
            || !seen.insert((assumption.interface_instance(), assumption.subject.clone()))
        {
            continue;
        }
        let Some(contract) = traits
            .iter()
            .find(|t| t.declaration == assumption.interface)
        else {
            continue;
        };
        if contract.type_parameters.len() != assumption.interface_arguments.len() {
            continue;
        }
        if contract
            .validate_arguments_with_layouts(
                &assumption.subject,
                &assumption.interface_arguments,
                &[],
                traits,
                domains,
                &mut remaining,
                |_, _, _| false,
                layouts,
            )
            .is_err()
        {
            continue;
        }
        let arguments: Vec<_> = std::iter::once(assumption.subject.clone())
            .chain(assumption.interface_arguments.iter().cloned())
            .collect();
        let Ok(substitution) = contract.input_substitution(&arguments, &[], &mut remaining) else {
            continue;
        };
        for requirement in &contract.requirements {
            if requirement.mentions_rigid(contract.self_parameter) {
                if let Ok(requirement) =
                    substitution.instantiate_predicate_bounded(requirement, &mut remaining)
                {
                    output.push(requirement);
                }
            }
        }
        for parent in &contract.parents {
            if let Ok(parent) = substitution.instantiate_interface_bounded(parent, &mut remaining) {
                pending.push((
                    TraitRequirement {
                        subject: assumption.subject.clone(),
                        interface: parent.declaration,
                        interface_arguments: parent.arguments,
                    },
                    depth + 1,
                ));
            }
        }
        for capability in &contract.capabilities {
            if let Ok(capability) =
                substitution.instantiate_requirement_bounded(capability, &mut remaining)
            {
                pending.push((capability, depth + 1));
            }
        }
    }
    output
}
struct Solver<'a, 'w> {
    world: &'a World<'w>,
    scope: Arc<ResolutionScope>,
    active: HashSet<(InterfaceInstance, Term)>,
    remaining: usize,
}
impl World<'_> {
    pub fn given(
        &self,
        requirement: &TraitRequirement,
        budget: Budget,
    ) -> Option<TraitRequirement> {
        self.given_metered(requirement, &mut { budget })
    }
    /// Return the solver's remaining allowance on success and failure alike.
    pub fn given_metered(
        &self,
        requirement: &TraitRequirement,
        budget: &mut Budget,
    ) -> Option<TraitRequirement> {
        let mut solver = Solver {
            world: self,
            scope: ResolutionScope::capture(self),
            active: HashSet::new(),
            remaining: budget.steps,
        };
        let result = solver.given(requirement, *budget);
        budget.steps = budget.steps.min(solver.remaining);
        result
    }
    /// Search existing evidence in an ordinary caller telescope. This does not
    /// select implementations or turn an unresolved query into negative proof.
    pub fn given_in_scope(
        &self,
        requirement: &TraitRequirement,
        budget: Budget,
        parameters: &[nir::Parameter],
    ) -> Option<TraitRequirement> {
        let signature = Term::Function {
            parameters: parameters.to_vec(),
            result: Box::new(Term::Primitive(nir::Primitive::Void)),
        };
        signature.check_scope(&[]).ok()?;
        Solver {
            world: self,
            scope: ResolutionScope::capture(self),
            active: HashSet::new(),
            remaining: budget.steps,
        }
        .given_scoped(requirement, budget, parameters)
    }
    pub fn given_in_context(
        &self,
        requirement: &TraitRequirement,
        budget: Budget,
    ) -> Option<TraitRequirement> {
        self.given_in_context_metered(requirement, &mut { budget })
    }

    pub(crate) fn given_in_context_metered(
        &self,
        requirement: &TraitRequirement,
        budget: &mut Budget,
    ) -> Option<TraitRequirement> {
        let mut solver = Solver {
            world: self,
            scope: ResolutionScope::capture(self),
            active: HashSet::new(),
            remaining: budget.steps,
        };
        let result = solver.given_frames(requirement, *budget, &[], self.context.scope());
        budget.steps = budget.steps.min(solver.remaining);
        result
    }

    /// Search caller-owned premises while some runtime domains are unavailable.
    /// All subjects and interface arguments are validated in the supplied sort
    /// frame. Equality needs the world's own context; no type wrapper is invented.
    pub fn given_in_input_scope(
        &self,
        requirement: &TraitRequirement,
        budget: Budget,
        frames: &[Vec<nir::Sort>],
    ) -> Option<TraitRequirement> {
        Solver {
            world: self,
            scope: ResolutionScope::capture(self),
            active: HashSet::new(),
            remaining: budget.steps,
        }
        .given_frames(requirement, budget, &[], frames)
    }
    pub fn resolve(
        &self,
        interface: Id,
        subject: &Term,
        budget: Budget,
    ) -> Result<Resolution, String> {
        self.resolve_instance(&interface.into(), subject, budget)
    }
    pub fn resolve_instance(
        &self,
        interface: &InterfaceInstance,
        subject: &Term,
        budget: Budget,
    ) -> Result<Resolution, String> {
        self.resolve_instance_metered(interface, subject, &mut { budget })
    }
    /// Report work tracked by the capability solver, including failed queries.
    /// Candidate matching retains its own bounded subquery allowances.
    pub fn resolve_instance_metered(
        &self,
        interface: &InterfaceInstance,
        subject: &Term,
        budget: &mut Budget,
    ) -> Result<Resolution, String> {
        let mut solver = Solver {
            world: self,
            scope: ResolutionScope::capture(self),
            active: HashSet::new(),
            remaining: budget.steps,
        };
        let result = solver.resolve(interface, subject, *budget, 0);
        budget.steps = budget.steps.min(solver.remaining);
        result
    }
    pub(crate) fn resolve_projected_instance(
        &self,
        interface: &InterfaceInstance,
        subject: &Term,
        budget: Budget,
    ) -> Result<Resolution, String> {
        self.resolve_projected_instance_metered(interface, subject, &mut { budget })
    }

    pub(crate) fn resolve_projected_instance_metered(
        &self,
        interface: &InterfaceInstance,
        subject: &Term,
        budget: &mut Budget,
    ) -> Result<Resolution, String> {
        let mut solver = Solver {
            world: self,
            scope: ResolutionScope::capture(self),
            active: HashSet::new(),
            remaining: budget.steps,
        };
        let result = (|| {
            let subject = solver.prerequisite_type(subject, *budget, 0)?;
            let arguments = interface
                .arguments
                .iter()
                .map(|ty| solver.prerequisite_type(ty, *budget, 0))
                .collect::<Result<_, _>>()?;
            solver.resolve(
                &InterfaceInstance {
                    declaration: interface.declaration,
                    arguments,
                },
                &subject,
                *budget,
                0,
            )
        })();
        budget.steps = budget.steps.min(solver.remaining);
        result
    }

    pub fn resolve_inherent(
        &self,
        index: &InherentIndex,
        name: &str,
        subject: &Term,
        budget: Budget,
    ) -> Result<Resolution, String> {
        let mut solver = Solver {
            world: self,
            scope: ResolutionScope::capture(self),
            active: HashSet::new(),
            remaining: budget.steps,
        };
        let mut qualifications = BTreeMap::new();
        let mut condition_support = BTreeMap::new();
        let types = index
            .resolve_matched_with(
                name,
                subject,
                self.context,
                budget,
                |declaration, matched, allowance| {
                    let assessment = match matched {
                        Ok(matched) => {
                            let mut support = Vec::new();
                            let assessment = matched.discharge_metered_with(|goal, allowance| {
                                solver.prove_condition_metered(goal, allowance, 0, &mut support)
                            });
                            if !support.is_empty() {
                                condition_support.insert(declaration, support);
                            }
                            assessment
                        }
                        Err(assessment) => assessment,
                    };
                    let state = match &assessment {
                        crate::implementation::TypeAssessment::Applicable(evidence) => {
                            let qualification = solver.qualify(evidence, allowance, 0);
                            let state = qualification.state();
                            qualifications.insert(evidence.implementation, qualification);
                            state
                        }
                        _ => assessment.applicability(),
                    };
                    (assessment, state)
                },
            )
            .map_err(|e| format!("inherent capability query: {e:?}"))?;
        Ok(Resolution {
            scope: solver.scope.clone(),
            types,
            qualifications,
            condition_support,
        })
    }
}
impl Qualification {
    fn state(&self) -> Applicability {
        match self {
            Self::Proved(_) => Applicability::Applicable,
            Self::Excluded(_) => Applicability::Excluded,
            Self::Pending(_) => Applicability::Pending,
        }
    }
}
impl Solver<'_, '_> {
    fn resolve(
        &mut self,
        interface: &InterfaceInstance,
        subject: &Term,
        budget: Budget,
        depth: usize,
    ) -> Result<Resolution, String> {
        if depth >= budget.max_depth.min(128) || self.remaining == 0 {
            return Err("capability resolution budget exhausted".into());
        }
        self.remaining -= 1;
        if let Some(contract) = self
            .world
            .traits
            .iter()
            .find(|t| t.declaration == interface.declaration)
        {
            contract.validate_arguments_with_layouts(
                subject,
                &interface.arguments,
                self.world.context.scope(),
                self.world.traits,
                &self.world.index.parameter_domains,
                &mut self.remaining,
                |left, right, fuel| {
                    equivalent_domains(self.world.context, left, right, budget.max_depth, fuel)
                },
                Some(&self.world.index.nominal_layouts),
            )?;
        }
        let key = (interface.clone(), subject.clone());
        if !self.active.insert(key.clone()) {
            return Err("recursive capability obligation remains pending".into());
        }
        let mut qualifications = BTreeMap::new();
        let mut condition_support = BTreeMap::new();
        let index = self.world.index;
        let context = self.world.context;
        let mut search_budget = Budget {
            steps: budget.steps.min(self.remaining),
            ..budget
        };
        let before_search = search_budget.steps;
        let types = index
            .resolve_assessed_with_search_budget(
                interface.declaration,
                &interface.arguments,
                subject,
                &mut search_budget,
                |head, allowance| {
                    let assessment = match head.match_types_with_layouts(
                        subject,
                        &interface.arguments,
                        context,
                        allowance,
                        &index.parameter_domains,
                        Some(&index.nominal_layouts),
                    ) {
                        Ok(matched) => {
                            let mut support = Vec::new();
                            let assessment = matched.discharge_metered_with(|goal, allowance| {
                                self.prove_condition_metered(
                                    goal,
                                    allowance,
                                    depth + 1,
                                    &mut support,
                                )
                            });
                            if !support.is_empty() {
                                condition_support.insert(head.declaration, support);
                            }
                            assessment
                        }
                        Err(assessment) => assessment,
                    };
                    let state = match &assessment {
                        crate::implementation::TypeAssessment::Applicable(evidence) => {
                            let qualification = self.qualify(evidence, allowance, depth + 1);
                            let state = qualification.state();
                            qualifications.insert(evidence.implementation, qualification);
                            state
                        }
                        _ => assessment.applicability(),
                    };
                    (assessment, state)
                },
            )
            .map_err(|e| format!("capability query: {e:?}"));
        self.remaining = self
            .remaining
            .saturating_sub(before_search - search_budget.steps);
        // Phase availability is checked after selecting the implementation
        // relationship. Filtering candidates by phase could silently choose a
        // less specific implementation instead of reporting a missing phase.
        let result = (|| {
            let types = types?;
            if let Selection::Selected(selected) = types.selection {
                self.check_comptime_parent_phases(interface, subject, selected, budget, depth)?;
            }
            Ok(Resolution {
                scope: self.scope.clone(),
                types,
                qualifications,
                condition_support,
            })
        })();
        self.active.remove(&key);
        result
    }

    fn check_comptime_parent_phases(
        &mut self,
        interface: &InterfaceInstance,
        subject: &Term,
        selected: Id,
        budget: Budget,
        depth: usize,
    ) -> Result<(), String> {
        let Some(contract) = self
            .world
            .traits
            .iter()
            .find(|t| t.declaration == interface.declaration && t.comptime_only)
        else {
            return Ok(());
        };
        if !self.world.implementations.iter().any(|implementation| {
            implementation.declaration == selected && implementation.phases.comptime
        }) {
            return Err("comptime-only interface lacks comptime phase".into());
        }
        let arguments: Vec<_> = std::iter::once(subject.clone())
            .chain(interface.arguments.iter().cloned())
            .collect();
        let substitution = contract
            .input_substitution(&arguments, &[], &mut self.remaining)
            .map_err(|error| error.to_string())?;
        for parent in &contract.parents {
            let parent = substitution
                .instantiate_interface_bounded(parent, &mut self.remaining)
                .map_err(|error| error.to_string())?;
            // A comptime-only declaration makes the phase part of the given
            // interface's contract. Ordinary interface evidence cannot do so.
            if self
                .world
                .traits
                .iter()
                .any(|t| t.declaration == parent.declaration && t.comptime_only)
                && self
                    .given(
                        &TraitRequirement {
                            subject: subject.clone(),
                            interface: parent.declaration,
                            interface_arguments: parent.arguments.clone(),
                        },
                        budget,
                    )
                    .is_some()
            {
                continue;
            }
            // A bare ordinary Given capability says only that a relationship
            // exists; check the phase of its eventual concrete selection.
            let resolution = self.resolve(&parent, subject, budget, depth + 1)?;
            let Selection::Selected(selected) = resolution.types.selection else {
                return Err("parent interface comptime phase remains unresolved".into());
            };
            if !self.world.implementations.iter().any(|implementation| {
                implementation.declaration == selected && implementation.phases.comptime
            }) {
                return Err("parent interface lacks comptime phase".into());
            }
        }
        Ok(())
    }
    fn given(
        &mut self,
        requirement: &TraitRequirement,
        budget: Budget,
    ) -> Option<TraitRequirement> {
        self.given_scoped(requirement, budget, &[])
    }
    fn given_scoped(
        &mut self,
        requirement: &TraitRequirement,
        budget: Budget,
        scope: &[nir::Parameter],
    ) -> Option<TraitRequirement> {
        let frames = if scope.is_empty() {
            vec![]
        } else {
            vec![scope.iter().map(|parameter| parameter.sort).collect()]
        };
        self.given_frames(requirement, budget, scope, &frames)
    }
    fn given_frames(
        &mut self,
        requirement: &TraitRequirement,
        budget: Budget,
        scope: &[nir::Parameter],
        frames: &[Vec<nir::Sort>],
    ) -> Option<TraitRequirement> {
        let world = self.world;
        let valid = |requirement: &TraitRequirement, fuel: &mut usize| {
            for input in
                std::iter::once(&requirement.subject).chain(&requirement.interface_arguments)
            {
                if crate::associated::charge_projection_copy(input, fuel).is_err() {
                    return false;
                }
            }
            if requirement.subject.sort() != nir::Sort::Type
                || requirement.subject.check_scope(frames).is_err()
            {
                return false;
            }
            if let Some(contract) = world
                .traits
                .iter()
                .find(|t| t.declaration == requirement.interface)
            {
                contract
                    .validate_arguments_with_layouts(
                        &requirement.subject,
                        &requirement.interface_arguments,
                        frames,
                        world.traits,
                        &world.index.parameter_domains,
                        fuel,
                        |left, right, fuel| {
                            equivalent_domains(world.context, left, right, budget.max_depth, fuel)
                        },
                        Some(&world.index.nominal_layouts),
                    )
                    .is_ok()
            } else {
                requirement
                    .interface_arguments
                    .iter()
                    .all(|ty| ty.sort() == nir::Sort::Type && ty.check_scope(frames).is_ok())
            }
        };
        if !valid(requirement, &mut self.remaining) {
            return None;
        }
        let equal = |left: &Term, right: &Term, fuel: &mut usize| {
            for input in [left, right] {
                if crate::associated::charge_projection_copy(input, fuel).is_err() {
                    return false;
                }
            }
            if left == right {
                return true;
            }
            let closed = left.check_scope(&[]).is_ok() && right.check_scope(&[]).is_ok();
            if !scope.is_empty() && !closed && world.context.scope() != frames {
                for _ in 0..2 {
                    for parameter in scope {
                        if crate::associated::charge_projection_copy(&parameter.ty, fuel).is_err() {
                            return false;
                        }
                    }
                }
            }
            let scoped = |ty: &Term| {
                if scope.is_empty() || closed || world.context.scope() == frames {
                    ty.clone()
                } else {
                    Term::Function {
                        parameters: scope.to_vec(),
                        result: Box::new(ty.clone()),
                    }
                }
            };
            let goal = Formula::equal(scoped(left), scoped(right));
            let mut proof_budget = Budget {
                steps: budget.steps.min(*fuel),
                ..budget
            };
            let before = proof_budget.steps;
            let result = world.context.prove_metered(goal, &mut proof_budget);
            *fuel = fuel.saturating_sub(before - proof_budget.steps);
            matches!(result, Outcome::Entailed(_))
        };
        for assumption in world.assumptions {
            if self.remaining == 0 {
                return None;
            }
            if !valid(assumption, &mut self.remaining) {
                continue;
            }
            let mut pending = vec![assumption.clone()];
            let mut seen = HashSet::new();
            while let Some(current) = pending.pop() {
                if self.remaining == 0 {
                    return None;
                }
                if !valid(&current, &mut self.remaining) {
                    continue;
                }
                let interface = current.interface_instance();
                if self.remaining == 0 {
                    return None;
                }
                self.remaining -= 1;
                if !seen.insert((interface.clone(), current.subject.clone())) {
                    continue;
                }
                if interface.declaration == requirement.interface
                    && interface.arguments.len() == requirement.interface_arguments.len()
                    && interface
                        .arguments
                        .iter()
                        .zip(&requirement.interface_arguments)
                        .all(|(a, b)| equal(a, b, &mut self.remaining))
                    && equal(&current.subject, &requirement.subject, &mut self.remaining)
                {
                    return Some(assumption.clone());
                }
                if let Some(t) = self
                    .world
                    .traits
                    .iter()
                    .find(|t| t.declaration == interface.declaration)
                {
                    let arguments: Vec<_> = std::iter::once(current.subject.clone())
                        .chain(interface.arguments.iter().cloned())
                        .collect();
                    let substitution = t
                        .input_substitution(&arguments, &frames, &mut self.remaining)
                        .ok()?;
                    for capability in &t.capabilities {
                        if let Ok(capability) = substitution
                            .instantiate_requirement_bounded(capability, &mut self.remaining)
                        {
                            pending.push(capability);
                        }
                    }
                    for parent in &t.parents {
                        if let Ok(parent) =
                            substitution.instantiate_interface_bounded(parent, &mut self.remaining)
                        {
                            pending.push(TraitRequirement {
                                subject: current.subject.clone(),
                                interface: parent.declaration,
                                interface_arguments: parent.arguments,
                            });
                        }
                    }
                }
            }
        }
        None
    }
    fn prerequisite_type(
        &mut self,
        ty: &Term,
        budget: Budget,
        depth: usize,
    ) -> Result<Term, String> {
        // Reserve only part of the remaining search for one projection. A
        // pending projection must not hide a later definitely false premise.
        if depth >= budget.max_depth.min(64) {
            return Err("capability normalization depth exhausted".into());
        }
        if ty.is_structural_index_value() {
            crate::associated::charge_projection_copy(ty, &mut self.remaining)?;
            ty.check_scope(self.world.context.scope())
                .map_err(|e| e.to_string())?;
            return Ok(ty.clone());
        }
        let allowance = budget.steps.min(self.remaining / 2);
        if allowance == 0 {
            return Err("capability normalization budget exhausted".into());
        }
        let world = self.world;
        let normalized = world.associated_type_view_for_selection(
            ty,
            Budget {
                steps: allowance,
                ..budget
            },
            &mut |interface, subject, budget| {
                let before = self.remaining;
                let result = (|| {
                    let subject = self.prerequisite_type(subject, *budget, depth + 1)?;
                    let arguments = interface
                        .arguments
                        .iter()
                        .map(|ty| self.prerequisite_type(ty, *budget, depth + 1))
                        .collect::<Result<_, _>>()?;
                    self.resolve(
                        &InterfaceInstance {
                            declaration: interface.declaration,
                            arguments,
                        },
                        &subject,
                        *budget,
                        depth + 1,
                    )
                })();
                budget.steps = budget.steps.saturating_sub(before - self.remaining);
                result
            },
        );
        match normalized {
            Ok((ty, remaining)) => {
                self.remaining = self
                    .remaining
                    .checked_sub(allowance - remaining)
                    .ok_or("capability normalization budget exhausted")?;
                Ok(ty)
            }
            Err(error) => {
                // A failed nested normalization cannot return a fresh allowance.
                self.remaining = self.remaining.saturating_sub(allowance);
                Err(error)
            }
        }
    }

    fn qualify(&mut self, evidence: &TypeEvidence, budget: Budget, depth: usize) -> Qualification {
        let Some(implementation) = self
            .world
            .implementations
            .iter()
            .find(|i| i.declaration == evidence.implementation)
        else {
            return Qualification::Pending("missing implementation contract".into());
        };
        let mut witnesses = Vec::new();
        let mut pending = None;
        let substitution = match nir::DeclarationSubstitution::bounded(
            &implementation.type_parameters,
            &implementation.parameter_types,
            &evidence.type_arguments,
            &[],
            &mut self.remaining,
        ) {
            Ok(substitution) => substitution,
            Err(error) => return Qualification::Pending(error.to_string()),
        };
        for capability in &implementation.capabilities {
            if self.remaining == 0 {
                return Qualification::Pending("capability resolution budget exhausted".into());
            }
            self.remaining -= 1;
            let requirement = match substitution
                .instantiate_requirement_bounded(capability, &mut self.remaining)
            {
                Ok(requirement) => requirement,
                Err(e) => return Qualification::Pending(e.to_string()),
            };
            if let Some(given) = self.given(&requirement, budget) {
                witnesses.push(Witness {
                    requirement,
                    origin: Origin::Given(given),
                });
                continue;
            }
            let requirement = match (|| {
                Ok::<_, String>(TraitRequirement {
                    interface: requirement.interface,
                    subject: self.prerequisite_type(&requirement.subject, budget, depth)?,
                    interface_arguments: requirement
                        .interface_arguments
                        .iter()
                        .map(|ty| self.prerequisite_type(ty, budget, depth))
                        .collect::<Result<_, _>>()?,
                })
            })() {
                Ok(requirement) => requirement,
                Err(error) => {
                    pending = Some(error);
                    continue;
                }
            };
            if let Some(given) = self.given(&requirement, budget) {
                witnesses.push(Witness {
                    requirement,
                    origin: Origin::Given(given),
                });
                continue;
            }
            let resolution = match self.resolve(
                &requirement.interface_instance(),
                &requirement.subject,
                budget,
                depth,
            ) {
                Ok(r) => r,
                Err(e) => {
                    pending = Some(e);
                    continue;
                }
            };
            match resolution.types.selection {
                Selection::Selected(id) => {
                    let Some(crate::implementation::TypeAssessment::Applicable(evidence)) =
                        resolution.types.assessments.get(&id)
                    else {
                        return Qualification::Pending("missing nested type evidence".into());
                    };
                    let Some(Qualification::Proved(premises)) = resolution.qualifications.get(&id)
                    else {
                        return Qualification::Pending("missing nested capability evidence".into());
                    };
                    witnesses.push(Witness {
                        requirement,
                        origin: Origin::Implementation {
                            declaration: id,
                            arguments: evidence.type_arguments.clone(),
                            premises: premises.clone(),
                        },
                    });
                }
                Selection::NoApplicableCandidate => {
                    if std::iter::once(&requirement.subject)
                        .chain(&requirement.interface_arguments)
                        .any(|term| {
                            crate::implementation::Key::from_type(term, budget)
                                .map_or(true, |key| key.is_open())
                        })
                    {
                        pending = Some(
                            "absence for an abstract capability subject is not established".into(),
                        );
                        continue;
                    }
                    witnesses.push(Witness {
                        requirement,
                        origin: Origin::Absent,
                    });
                    return Qualification::Excluded(witnesses);
                }
                other => {
                    let cause = resolution.qualifications.values().find_map(|q| match q {
                        Qualification::Pending(reason) => Some(reason.as_str()),
                        _ => None,
                    });
                    pending = Some(match cause {
                        Some(reason) => {
                            format!("nested capability is unsettled: {other}; {reason}")
                        }
                        None => format!("nested capability is unsettled: {other}"),
                    });
                }
            }
        }
        match pending {
            Some(reason) => Qualification::Pending(reason),
            None => Qualification::Proved(witnesses),
        }
    }
}
impl fmt::Display for Qualification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending(reason) => write!(f, "(capability-pending {reason:?})"),
            Self::Proved(witnesses) | Self::Excluded(witnesses) => {
                write!(
                    f,
                    "({}",
                    if matches!(self, Self::Proved(_)) {
                        "capability-proved"
                    } else {
                        "capability-excluded"
                    }
                )?;
                for witness in witnesses {
                    write!(f, " {witness}")?;
                }
                write!(f, ")")
            }
        }
    }
}
impl fmt::Display for Witness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(capability-witness {} ", self.requirement)?;
        match &self.origin {
            Origin::Given(given) => write!(f, "(given {given})")?,
            Origin::Absent => write!(f, "(absent-in-world)")?,
            Origin::Implementation {
                declaration,
                arguments,
                premises,
            } => {
                write!(f, "(implementation {declaration} (arguments")?;
                for a in arguments {
                    write!(f, " {a}")?;
                }
                write!(f, ") (premises")?;
                for p in premises {
                    write!(f, " {p}")?;
                }
                write!(f, "))")?;
            }
        }
        write!(f, ")")
    }
}
impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(capability-resolution {} (qualifications", self.types)?;
        for (id, qualification) in &self.qualifications {
            write!(f, "({id} {qualification})")?;
        }
        write!(f, ") (scope {} (capability-assumptions", self.scope.context)?;
        for assumption in &self.scope.assumptions {
            write!(f, " {assumption}")?;
        }
        f.write_str(")) (condition-support")?;
        for (id, supports) in &self.condition_support {
            write!(f, " ({id}")?;
            for support in supports {
                write!(f, " {support}")?;
            }
            f.write_str(")")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod projected_budget_tests {
    use super::*;

    #[test]
    fn failed_projected_queries_report_consumed_work() {
        let index = Index::default();
        let context = Context::default();
        let world = World {
            index: &index,
            implementations: &[],
            traits: &[],
            assumptions: &[],
            context: &context,
        };
        let id = nir::DeclarationId {
            package: 1,
            local: 1,
        };
        let interface = InterfaceInstance {
            declaration: id,
            arguments: vec![],
        };
        let subject = Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: id,
                member: id,
            }),
            subject: Box::new(Term::Primitive(nir::Primitive::Bool)),
            arguments: vec![],
        };
        for steps in [0, 1, 1000] {
            let mut budget = Budget {
                steps,
                ..Budget::default()
            };
            assert!(
                world
                    .resolve_projected_instance_metered(&interface, &subject, &mut budget)
                    .is_err()
            );
            assert!(budget.steps <= steps);
            // With fewer than two steps no normalization allowance can be
            // reserved, so rejection occurs before any work is performed.
            if steps > 1 {
                assert!(budget.steps < steps);
            }
        }
    }
}
