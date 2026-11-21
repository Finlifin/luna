//! Evidence-driven associated Type queries in an immutable, admitted world.
//! One-step queries retain residuals; recursive normalization closes projections
//! transactionally with cycle detection and a query-local completed cache.
//! Substitution never invokes normal-form heads or substitutes trait defaults.
use crate::capabilities::{Qualification, World};
use crate::implementation::{Selection, TypeEvidence};
use crate::ptc::{Budget, Formula, Outcome};
use nir::program::{ExprKind, InterfaceInstance};
use nir::{NominalAdmissionError as QueryError, Primitive, Term};
use std::fmt;

/// The selected result and its exact implementation substitution travel
/// together. Evidence is valid only in the immutable world used for this query.
#[derive(Clone, Debug)]
pub struct AssociatedTypeResolution {
    pub value: Term,
    pub evidence: TypeEvidence,
    pub qualification: Qualification,
}

/// One step in a selected projection. Only a requesting comptime context
/// can complete an initializer; passive consumers require a ready identity.
pub enum TypeProjection {
    Ready(AssociatedTypeResolution),
    Initializer(SelectedTypeInitializer),
}

pub struct SelectedTypeInitializer {
    open_inputs: bool,
    evidence: TypeEvidence,
    qualification: Qualification,
}

impl SelectedTypeInitializer {
    pub fn needs_inputs(&self) -> bool {
        self.open_inputs
    }
    pub fn complete(
        self,
        value: Term,
        fuel: &mut usize,
    ) -> Result<AssociatedTypeResolution, QueryError> {
        if self.open_inputs {
            return Err(QueryError::Unresolved(
                "associated Type initializer requires concrete inputs".into(),
            ));
        }
        charge_projection_copy(&value, fuel)?;
        value
            .check_scope(&[])
            .map_err(|failure| failure.to_string())?;
        if value.sort() != nir::Sort::Type {
            return Err("associated Type initializer returned the wrong sort".into());
        }
        Ok(AssociatedTypeResolution {
            value,
            evidence: self.evidence,
            qualification: self.qualification,
        })
    }
}

impl TypeProjection {
    fn ready(self) -> Result<AssociatedTypeResolution, QueryError> {
        match self {
            Self::Ready(value) => Ok(value),
            Self::Initializer(_) => {
                Err("selected associated value is not a completed Type template".into())
            }
        }
    }
}

impl World<'_> {
    fn check_associated_formation(
        &self,
        contract: &nir::program::Trait,
        substitution: &nir::DeclarationSubstitution<'_>,
        budget: &mut Budget,
        resolve: &mut dyn FnMut(
            &InterfaceInstance,
            &Term,
            &mut Budget,
        ) -> Result<crate::capabilities::Resolution, QueryError>,
    ) -> Result<(), QueryError> {
        for requirement in &contract.requirements {
            let mut pending = vec![(requirement, 0usize)];
            while let Some((predicate, depth)) = pending.pop() {
                if depth >= budget.max_depth.min(64) {
                    return Err(QueryError::Budget);
                }
                budget.steps = budget.steps.checked_sub(1).ok_or(QueryError::Budget)?;
                match predicate {
                    nir::program::TypePredicate::And(items)
                    | nir::program::TypePredicate::Or(items) => {
                        if items.len() > budget.steps {
                            return Err(QueryError::Budget);
                        }
                        pending.extend(items.iter().map(|item| (item, depth + 1)));
                    }
                    nir::program::TypePredicate::Not(inner) => pending.push((inner, depth + 1)),
                    _ => {}
                }
            }
            let requirement =
                substitution.instantiate_predicate_bounded(requirement, &mut budget.steps)?;
            match self
                .context
                .prove_metered(Formula::from_predicate(&requirement), budget)
            {
                Outcome::Entailed(_) => {}
                Outcome::Unknown {
                    reason: crate::ptc::UnknownReason::BudgetExhausted,
                    ..
                } => return Err(QueryError::Budget),
                Outcome::Unknown { reason, .. } => {
                    return Err(QueryError::Unresolved(format!(
                        "associated interface requirement: {reason:?}"
                    )));
                }
                Outcome::NotEntailed(_) => {
                    return Err(QueryError::Unresolved(
                        "associated interface requirement is not established".into(),
                    ));
                }
                Outcome::IllFormed(reason) => return Err(QueryError::Invalid(reason)),
            }
        }
        for capability in &contract.capabilities {
            let capability =
                substitution.instantiate_requirement_bounded(capability, &mut budget.steps)?;
            if self
                .given_in_context_metered(&capability, budget)?
                .is_some()
            {
                continue;
            }
            let resolved = resolve(
                &capability.interface_instance(),
                &capability.subject,
                budget,
            )?;
            if !matches!(resolved.types.selection, Selection::Selected(_)) {
                return Err(QueryError::Unresolved(
                    "associated interface capability is not established".into(),
                ));
            }
        }
        Ok(())
    }

    /// Resolve a well-scoped Type projection after its input substitutions.
    /// Member lookup has already chosen an origin (including inherited origins).
    /// The caller must establish any requested child view separately.
    pub fn resolve_associated_type(
        &self,
        projection: &Term,
        budget: Budget,
    ) -> Result<AssociatedTypeResolution, QueryError> {
        self.resolve_associated_type_budgeted(projection, &mut { budget })
            .map(|(result, _)| result)
    }

    fn resolve_associated_type_budgeted(
        &self,
        projection: &Term,
        budget: &mut Budget,
    ) -> Result<(AssociatedTypeResolution, usize), QueryError> {
        let value = self
            .prepare_associated_type_metered(projection, budget)?
            .ready()?;
        Ok((value, budget.steps))
    }

    pub fn prepare_associated_type_metered(
        &self,
        projection: &Term,
        budget: &mut Budget,
    ) -> Result<TypeProjection, QueryError> {
        self.prepare_associated_type_using(
            projection,
            budget,
            &mut |interface, subject, allowance| {
                self.resolve_projected_instance_metered(interface, subject, allowance)
            },
        )
    }

    fn prepare_associated_type_using(
        &self,
        projection: &Term,
        budget: &mut Budget,
        resolve: &mut dyn FnMut(
            &InterfaceInstance,
            &Term,
            &mut Budget,
        ) -> Result<crate::capabilities::Resolution, QueryError>,
    ) -> Result<TypeProjection, QueryError> {
        charge_projection_copy(projection, &mut budget.steps)?;
        projection.check_scope(&[]).map_err(|e| e.to_string())?;
        let Term::AssociatedType {
            origin,
            subject,
            arguments,
        } = projection
        else {
            return Err("associated Type query requires a projection".into());
        };
        let interface = InterfaceInstance {
            declaration: origin.interface,
            arguments: arguments.clone(),
        };
        let selected = self.select_scoped_member(
            &interface,
            origin.member,
            subject,
            values::Access::TypeProjection,
            budget,
            resolve,
        )?;
        selected.prepare_type(projection, budget)
    }
}

impl fmt::Display for AssociatedTypeResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-type-resolution (value {}) (implementation {}) (arguments",
            self.value, self.evidence.implementation
        )?;
        for argument in &self.evidence.type_arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") (qualification {}))", self.qualification)
    }
}

/// Completed projection closure plus its deterministic, dependency-ordered
/// selection trace. No partial result or cross-world cache escapes on failure.
#[derive(Clone, Debug)]
pub struct AssociatedTypeNormalization {
    pub value: Term,
    pub steps: Vec<AssociatedTypeStep>,
    /// Closed program reads retained for source dispatch in this traversal.
    pub initializer_dependencies: Vec<Term>,
}
#[derive(Clone, Debug)]
pub struct AssociatedTypeStep {
    pub projection: Term,
    pub resolution: AssociatedTypeResolution,
}

#[derive(Clone, Copy)]
enum ProjectionView {
    ConcreteType,
    DispatchType,
    DispatchScopedType,
    ScopedType,
    ScopedOperand,
}

impl World<'_> {
    /// Resolve projections throughout type structure after caller substitution.
    /// Opaque normal-form heads are retained; only their inputs are traversed.
    /// A projection still depending on a function-local binder cannot be chosen
    /// here and must wait for that binder's instantiation.
    pub fn normalize_associated_types(
        &self,
        ty: &Term,
        budget: Budget,
    ) -> Result<AssociatedTypeNormalization, QueryError> {
        self.normalize_associated_types_budgeted(ty, budget)
            .map(|(result, _)| result)
    }

    pub(crate) fn normalize_associated_types_budgeted(
        &self,
        ty: &Term,
        budget: Budget,
    ) -> Result<(AssociatedTypeNormalization, usize), QueryError> {
        self.normalize_associated_types_mode(
            ty,
            &mut { budget },
            ProjectionView::ConcreteType,
            None,
            None,
        )
        .map(|(result, remaining, _)| (result, remaining))
    }

    /// The caller owns this cache for one immutable implementation world.
    /// Publish newly completed projections only after the whole query succeeds.
    pub fn normalize_associated_types_cached(
        &self,
        ty: &Term,
        budget: Budget,
        cache: &mut std::collections::HashMap<Term, Term>,
    ) -> Result<(AssociatedTypeNormalization, usize), QueryError> {
        let mut budget = budget;
        let (result, completed) = self.prepare_normalized_types(ty, &mut budget, cache)?;
        cache.extend(completed);
        Ok((result, budget.steps))
    }

    /// Compute in query-local storage. The enclosing operation publishes these
    /// completed entries only after its own retention checks have succeeded.
    pub(crate) fn prepare_normalized_types(
        &self,
        ty: &Term,
        budget: &mut Budget,
        cache: &std::collections::HashMap<Term, Term>,
    ) -> Result<
        (
            AssociatedTypeNormalization,
            std::collections::HashMap<Term, Term>,
        ),
        QueryError,
    > {
        self.normalize_associated_types_mode(
            ty,
            budget,
            ProjectionView::ConcreteType,
            Some(cache),
            None,
        )
        .map(|(result, _, completed)| (result, completed))
    }

    /// Definition-side type view: given interfaces with open inputs remain
    /// abstract; completed inputs still select their associated value. IIR uses
    /// the strict normalization entry and cannot retain abstract results.
    pub fn associated_type_view(
        &self,
        ty: &Term,
        budget: Budget,
    ) -> Result<AssociatedTypeNormalization, QueryError> {
        self.associated_type_view_metered(ty, &mut { budget })
    }

    pub fn associated_type_view_metered(
        &self,
        ty: &Term,
        budget: &mut Budget,
    ) -> Result<AssociatedTypeNormalization, QueryError> {
        self.normalize_associated_types_mode(ty, budget, ProjectionView::ScopedType, None, None)
            .map(|(value, _, _)| value)
    }

    /// Collect declaration dependencies for source invocation. A selected
    /// initializer remains a projection for the VM to observe with its inputs.
    pub fn dispatch_type_view_metered(
        &self,
        ty: &Term,
        budget: &mut Budget,
    ) -> Result<AssociatedTypeNormalization, QueryError> {
        self.normalize_associated_types_mode(ty, budget, ProjectionView::DispatchType, None, None)
            .map(|(view, _, _)| view)
    }

    pub fn staged_dispatch_type_view_cached_metered(
        &self,
        ty: &Term,
        budget: &mut Budget,
        cache: &mut std::collections::HashMap<Term, Term>,
    ) -> Result<AssociatedTypeNormalization, QueryError> {
        let (value, _, completed) = self.normalize_associated_types_mode(
            ty,
            budget,
            ProjectionView::DispatchScopedType,
            Some(cache),
            None,
        )?;
        cache.extend(completed);
        Ok(value)
    }

    /// Reuse completed views in one immutable world. Failed queries return
    /// their remaining work but publish none of their newly completed children.
    pub fn associated_type_view_cached_metered(
        &self,
        ty: &Term,
        budget: &mut Budget,
        cache: &mut std::collections::HashMap<Term, Term>,
    ) -> Result<AssociatedTypeNormalization, QueryError> {
        let (value, _, completed) = self.normalize_associated_types_mode(
            ty,
            budget,
            ProjectionView::ScopedType,
            Some(cache),
            None,
        )?;
        cache.extend(completed);
        Ok(value)
    }

    pub(crate) fn projection_operand_view_cached(
        &self,
        operand: &Term,
        budget: &mut Budget,
        completed: &mut std::collections::HashMap<Term, Term>,
    ) -> Result<AssociatedTypeNormalization, QueryError> {
        let (view, _, new) = self.normalize_associated_types_mode(
            operand,
            budget,
            ProjectionView::ScopedOperand,
            Some(completed),
            None,
        )?;
        completed.extend(new);
        Ok(view)
    }

    pub(crate) fn associated_type_view_for_selection(
        &self,
        ty: &Term,
        budget: &mut Budget,
        resolve: &mut dyn FnMut(
            &InterfaceInstance,
            &Term,
            &mut Budget,
        ) -> Result<crate::capabilities::Resolution, QueryError>,
    ) -> Result<(Term, usize), QueryError> {
        self.normalize_associated_types_mode(
            ty,
            budget,
            ProjectionView::ScopedOperand,
            None,
            Some(resolve),
        )
        .map(|(result, remaining, _)| (result.value, remaining))
    }

    fn normalize_associated_types_mode(
        &self,
        ty: &Term,
        budget: &mut Budget,
        mode: ProjectionView,
        shared: Option<&std::collections::HashMap<Term, Term>>,
        resolve: Option<
            &mut dyn FnMut(
                &InterfaceInstance,
                &Term,
                &mut Budget,
            ) -> Result<crate::capabilities::Resolution, QueryError>,
        >,
    ) -> Result<
        (
            AssociatedTypeNormalization,
            usize,
            std::collections::HashMap<Term, Term>,
        ),
        QueryError,
    > {
        normalize_type_in(
            &mut PassiveProjectionWorld {
                world: *self,
                resolve,
            },
            ty,
            budget,
            mode,
            shared,
        )
    }
}

/// Supplies the declaration world for each query. The traversal owns its
/// frames and completed projections while the requesting context services a read.
pub trait TypeProjectionEnvironment {
    fn world(&self) -> World<'_>;
    fn resolve_type(
        &mut self,
        projection: &Term,
        budget: &mut Budget,
    ) -> Result<TypeProjection, QueryError>;
}

type InstanceResolver<'a> = dyn FnMut(
        &InterfaceInstance,
        &Term,
        &mut Budget,
    ) -> Result<crate::capabilities::Resolution, QueryError>
    + 'a;

struct PassiveProjectionWorld<'w, 'r> {
    world: World<'w>,
    resolve: Option<&'r mut InstanceResolver<'r>>,
}

impl TypeProjectionEnvironment for PassiveProjectionWorld<'_, '_> {
    fn world(&self) -> World<'_> {
        self.world
    }
    fn resolve_type(
        &mut self,
        projection: &Term,
        budget: &mut Budget,
    ) -> Result<TypeProjection, QueryError> {
        if let Some(resolve) = self.resolve.as_deref_mut() {
            self.world
                .prepare_associated_type_using(projection, budget, resolve)
        } else {
            self.world
                .prepare_associated_type_metered(projection, budget)
        }
    }
}

pub fn scoped_type_view_in(
    environment: &mut impl TypeProjectionEnvironment,
    ty: &Term,
    budget: &mut Budget,
) -> Result<AssociatedTypeNormalization, QueryError> {
    normalize_type_in(environment, ty, budget, ProjectionView::ScopedType, None)
        .map(|(view, _, _)| view)
}

fn normalize_type_in(
    environment: &mut impl TypeProjectionEnvironment,
    ty: &Term,
    budget: &mut Budget,
    mode: ProjectionView,
    shared: Option<&std::collections::HashMap<Term, Term>>,
) -> Result<
    (
        AssociatedTypeNormalization,
        usize,
        std::collections::HashMap<Term, Term>,
    ),
    QueryError,
> {
    let preserve_given = !matches!(
        mode,
        ProjectionView::ConcreteType | ProjectionView::DispatchType
    );
    let environment_scope = environment.world().context.scope();
    let frames = if preserve_given {
        environment_scope.to_vec()
    } else {
        Vec::new()
    };
    let mut normalizer = Normalizer {
        environment,
        retain_programs: matches!(
            mode,
            ProjectionView::DispatchType | ProjectionView::DispatchScopedType
        ),
        preserve_given,
        budget: *budget,
        remaining: budget.steps,
        active: std::collections::HashSet::new(),
        completed: std::collections::HashMap::new(),
        shared,
        steps: Vec::new(),
        initializer_dependencies: Vec::new(),
        frames,
    };
    let result = (|| {
        normalizer.charge_copy(ty)?;
        ty.check_scope(&normalizer.frames)
            .map_err(|e| e.to_string())?;
        if !matches!(mode, ProjectionView::ScopedOperand) && ty.sort() != nir::Sort::Type {
            return Err("associated normalization requires a Type".into());
        }
        let value = normalizer.normalize(ty, 0)?;
        value
            .check_scope(&normalizer.frames)
            .map_err(|e| e.to_string())?;
        Ok((
            AssociatedTypeNormalization {
                value,
                steps: std::mem::take(&mut normalizer.steps),
                initializer_dependencies: std::mem::take(&mut normalizer.initializer_dependencies),
            },
            normalizer.remaining,
            std::mem::take(&mut normalizer.completed),
        ))
    })();
    budget.steps = normalizer.remaining;
    result
}

struct Normalizer<'a, E> {
    environment: &'a mut E,
    retain_programs: bool,
    preserve_given: bool,
    budget: Budget,
    remaining: usize,
    active: std::collections::HashSet<Term>,
    completed: std::collections::HashMap<Term, Term>,
    shared: Option<&'a std::collections::HashMap<Term, Term>>,
    steps: Vec<AssociatedTypeStep>,
    initializer_dependencies: Vec<Term>,
    frames: Vec<Vec<nir::Sort>>,
}
impl<E: TypeProjectionEnvironment> Normalizer<'_, E> {
    fn spend(&mut self, steps: usize) -> Result<(), QueryError> {
        self.remaining = self
            .remaining
            .checked_sub(steps)
            .ok_or(QueryError::Budget)?;
        Ok(())
    }
    fn charge_copy(&mut self, term: &Term) -> Result<(), QueryError> {
        charge_projection_copy(term, &mut self.remaining)
    }
    fn many(&mut self, terms: &[Term], depth: usize) -> Result<Vec<Term>, QueryError> {
        if terms.len() > self.remaining {
            return Err(QueryError::Budget);
        }
        terms
            .iter()
            .map(|term| self.normalize(term, depth))
            .collect()
    }
    fn normalize(&mut self, term: &Term, depth: usize) -> Result<Term, QueryError> {
        if depth >= self.budget.max_depth.min(64) {
            return Err(QueryError::Budget);
        }
        self.spend(1)?;
        Ok(match term {
            Term::ErrorResult { errors, result } => Term::error_result(
                self.many(errors, depth + 1)?,
                self.normalize(result, depth + 1)?,
            ),
            Term::Computation { effects, result } => {
                self.spend(effects.len())?;
                let operations = effects
                    .iter()
                    .map(|operation| {
                        Ok(nir::effects::Operation {
                            declaration: operation.declaration,
                            arguments: self.many(&operation.arguments, depth + 1)?,
                        })
                    })
                    .collect::<Result<Vec<_>, QueryError>>()?;
                Term::computation(operations, self.normalize(result, depth + 1)?)
            }
            Term::AssociatedType {
                origin,
                arguments,
                subject,
            } => {
                let subject = self.normalize(subject, depth + 1)?;
                let arguments = self.many(arguments, depth + 1)?;
                let key = Term::AssociatedType {
                    origin: origin.clone(),
                    arguments,
                    subject: Box::new(subject),
                };
                if self.preserve_given && key.check_scope(&[]).is_err() {
                    // A projection can capture any enclosing callable frame,
                    // not just its immediate depth-zero frame. Validate it in
                    // the complete telescope before retaining it uncached.
                    key.check_scope(&self.frames).map_err(|e| e.to_string())?;
                    // A callable's ordinary inputs have not been supplied yet.
                    // Retain the projection without choosing an implementation
                    // or manufacturing an equality. Formation evidence is
                    // checked by elaboration; strict IIR normalization still
                    // requires a selected concrete projection.
                    let Term::AssociatedType {
                        origin,
                        subject,
                        arguments,
                    } = &key
                    else {
                        unreachable!()
                    };
                    let contract = self
                        .environment
                        .world()
                        .traits
                        .iter()
                        .find(|t| t.declaration == origin.interface)
                        .ok_or("associated Type query has no originating interface")?;
                    if arguments.len() != contract.type_parameters.len() {
                        return Err(
                            "associated Type query interface argument count mismatch".into()
                        );
                    }
                    let member = contract
                        .associated
                        .iter()
                        .find(|m| m.declaration == origin.member)
                        .ok_or("associated Type query member does not belong to its interface")?;
                    charge_projection_copy(&member.ty, &mut self.remaining)?;
                    let inputs = std::iter::once(subject.as_ref().clone())
                        .chain(arguments.iter().cloned())
                        .collect::<Vec<_>>();
                    let binders = contract.binders_with_self_checked(&mut self.remaining)?;
                    charge_projection_substitution(
                        &member.ty,
                        &binders,
                        &inputs,
                        &mut self.remaining,
                    )?;
                    let ty = contract
                        .input_substitution(&inputs, &self.frames, &mut self.remaining)
                        .and_then(|substitution| {
                            substitution.instantiate_bounded(&member.ty, &mut self.remaining)
                        })?;
                    if ty != Term::Primitive(Primitive::Type) {
                        return Err("associated projection member is not Type-valued".into());
                    }
                    return Ok(key);
                }
                if let Some(value) = self
                    .completed
                    .get(&key)
                    .or_else(|| self.shared.and_then(|cache| cache.get(&key)))
                {
                    // Use a temporary borrowed walk before cloning the cached
                    // result; the cache is local to this immutable-world call.
                    charge_projection_copy(value, &mut self.remaining)?;
                    return Ok(value.clone());
                }
                self.charge_copy(&key)?;
                let mut incomplete_given: Option<String> = None;
                if self.preserve_given && !self.environment.world().assumptions.is_empty() {
                    let Term::AssociatedType {
                        origin,
                        subject,
                        arguments,
                    } = &key
                    else {
                        unreachable!()
                    };
                    let mut closure_budget = Budget {
                        steps: self.remaining,
                        ..self.budget
                    };
                    if closure_budget.steps == 0 {
                        return Err(QueryError::Budget);
                    }
                    let closed = self.environment.world().closed_inputs_metered(
                        std::iter::once(subject.as_ref()).chain(arguments),
                        &mut closure_budget,
                    );
                    self.remaining = closure_budget.steps;
                    let may_be_open = match closed {
                        Ok(closed) => !closed,
                        // An unfinished closure proof leaves an established given view
                        // abstract rather than forcing a default or concrete selection.
                        Err(QueryError::Unresolved(_)) => true,
                        Err(failure) => return Err(failure),
                    };
                    let preserve = if may_be_open {
                        let query_steps = self.remaining / 2;
                        if query_steps == 0 {
                            return Err(QueryError::Budget);
                        }
                        let given = nir::program::TraitRequirement {
                            subject: subject.as_ref().clone(),
                            interface: origin.interface,
                            interface_arguments: arguments.clone(),
                        };
                        let mut given_budget = Budget {
                            steps: query_steps,
                            ..self.budget
                        };
                        // The projection is closed, but its evidence may still
                        // refer to the world's enclosing input frame. Nested
                        // callable frames belong to the observed type, not to
                        // those premises, so use the world's original context.
                        let found = self
                            .environment
                            .world()
                            .given_in_context_metered(&given, &mut given_budget);
                        // A failed premise lookup may consume very little work.
                        // Charging its whole allowance starves nested projections.
                        self.spend(query_steps - given_budget.steps)?;
                        match found {
                            Ok(Some(_)) => true,
                            Ok(None) => false,
                            Err(nir::NominalAdmissionError::Budget) => {
                                return Err(QueryError::Budget);
                            }
                            Err(nir::NominalAdmissionError::Invalid(reason)) => {
                                return Err(QueryError::Invalid(reason));
                            }
                            Err(nir::NominalAdmissionError::Unresolved(reason)) => {
                                incomplete_given = Some(reason);
                                false
                            }
                        }
                    } else {
                        false
                    };
                    if preserve {
                        let contract = self
                            .environment
                            .world()
                            .traits
                            .iter()
                            .find(|t| t.declaration == origin.interface)
                            .ok_or("associated Type query has no originating interface")?;
                        if arguments.len() != contract.type_parameters.len() {
                            return Err(
                                "associated Type query interface argument count mismatch".into()
                            );
                        }
                        let member = contract
                            .associated
                            .iter()
                            .find(|m| m.declaration == origin.member)
                            .ok_or(
                                "associated Type query member does not belong to its interface",
                            )?;
                        charge_projection_copy(&member.ty, &mut self.remaining)?;
                        let inputs: Vec<_> = std::iter::once(subject.as_ref().clone())
                            .chain(arguments.iter().cloned())
                            .collect();
                        let binders = contract.binders_with_self_checked(&mut self.remaining)?;
                        charge_projection_substitution(
                            &member.ty,
                            &binders,
                            &inputs,
                            &mut self.remaining,
                        )?;
                        let ty = contract
                            .input_substitution(&inputs, &self.frames, &mut self.remaining)
                            .and_then(|substitution| {
                                substitution.instantiate_bounded(&member.ty, &mut self.remaining)
                            })?;
                        if ty != Term::Primitive(Primitive::Type) {
                            return Err("associated projection member is not Type-valued".into());
                        }
                        self.charge_copy(&key)?;
                        self.charge_copy(&key)?;
                        self.completed.insert(key.clone(), key.clone());
                        return Ok(key);
                    }
                }
                if !self.active.insert(key.clone()) {
                    return Err(QueryError::Unresolved(
                        "cyclic associated Type projection".into(),
                    ));
                }
                let result = (|| {
                    let mut allowance = Budget {
                        steps: self.remaining,
                        ..self.budget
                    };
                    let resolved = self.environment.resolve_type(&key, &mut allowance);
                    self.remaining = allowance.steps;
                    let resolution = match resolved {
                        Ok(TypeProjection::Initializer(work)) if self.retain_programs || (self.preserve_given && work.needs_inputs()) => {
                            if self.retain_programs && !work.needs_inputs() {
                                self.charge_copy(&key)?;
                                self.initializer_dependencies.push(key.clone());
                            }
                            self.charge_copy(&key)?;
                            return Ok(key.clone());
                        }
                        other => other.and_then(TypeProjection::ready),
                    }.map_err(|error| match error {
                        QueryError::Budget => QueryError::Budget,
                        QueryError::Invalid(message) => QueryError::Invalid(format!(
                            "associated Type normalization of {key}: {message}"
                        )),
                        QueryError::Unresolved(message) => {
                            if let Some(reason) = &incomplete_given {
                                QueryError::Unresolved(format!(
                                    "associated Type normalization of {key}: {message} (given lookup unresolved: {reason})"
                                ))
                            } else {
                                QueryError::Unresolved(format!(
                                    "associated Type normalization of {key}: {message}"
                                ))
                            }
                        }
                    })?;
                    let value = self.normalize(&resolution.value, depth + 1)?;
                    self.charge_copy(&value)?;
                    self.charge_copy(&key)?;
                    self.charge_copy(&key)?;
                    self.completed.insert(key.clone(), value.clone());
                    self.steps.push(AssociatedTypeStep {
                        projection: key.clone(),
                        resolution,
                    });
                    Ok(value)
                })();
                self.active.remove(&key);
                return result;
            }
            Term::Nominal {
                declaration,
                arguments,
            } => Term::Nominal {
                declaration: *declaration,
                arguments: self.many(arguments, depth + 1)?,
            },
            Term::NormalForm {
                head,
                sort,
                arguments,
            } => Term::NormalForm {
                head: *head,
                sort: *sort,
                arguments: self.many(arguments, depth + 1)?,
            },
            Term::FunctionValue {
                declaration,
                arguments,
            } => Term::FunctionValue {
                declaration: *declaration,
                arguments: self.many(arguments, depth + 1)?,
            },
            Term::ClosureSnapshot { factory, arguments } => Term::ClosureSnapshot {
                factory: *factory,
                arguments: self.many(arguments, depth + 1)?,
            },
            Term::ClosureContextValue(items) => {
                Term::ClosureContextValue(self.many(items, depth + 1)?)
            }
            Term::ContextualClosure { context, signature } => Term::ContextualClosure {
                context: Box::new(self.normalize(context, depth + 1)?),
                signature: Box::new(self.normalize(signature, depth + 1)?),
            },
            Term::MethodValue(method) => Term::MethodValue(Box::new(nir::TraitMethodIdentity {
                interface: InterfaceInstance {
                    declaration: method.interface.declaration,
                    arguments: self.many(&method.interface.arguments, depth + 1)?,
                },
                member: method.member,
                method_arguments: self.many(&method.method_arguments, depth + 1)?,
                subject: self.normalize(&method.subject, depth + 1)?,
            })),
            Term::Tuple(items) => Term::Tuple(self.many(items, depth + 1)?),
            Term::TupleValue(items) => Term::TupleValue(self.many(items, depth + 1)?),
            Term::ListConcat { ty, parts } => {
                Term::list_concat(self.normalize(ty, depth + 1)?, self.many(parts, depth + 1)?)
                    .map_err(|error| error.to_string())?
            }
            Term::OptionalValue { ty, value } => Term::OptionalValue {
                ty: Box::new(self.normalize(ty, depth + 1)?),
                value: value
                    .as_ref()
                    .map(|value| self.normalize(value, depth + 1).map(Box::new))
                    .transpose()?,
            },
            Term::ErrorValue { ty, label, value } => {
                let Term::ErrorResult { errors, result } = ty.as_ref() else {
                    return Err("invalid frozen error result domain".into());
                };
                Term::error_value(
                    self.many(errors, depth + 1)?,
                    self.normalize(result, depth + 1)?,
                    match label {
                        Some(label) => Some(Box::new(self.normalize(label, depth + 1)?)),
                        None => None,
                    },
                    self.normalize(value, depth + 1)?,
                )
                .map_err(|error| error.to_string())?
            }
            Term::NominalValue {
                ty,
                variant,
                fields,
            } => Term::NominalValue {
                ty: Box::new(self.normalize(ty, depth + 1)?),
                variant: *variant,
                fields: self.many(fields, depth + 1)?,
            },
            Term::Record(fields) | Term::RecordValue(fields) => {
                if fields.len() > self.remaining {
                    return Err(QueryError::Budget);
                }
                let mut output = Vec::with_capacity(fields.len());
                for (name, ty) in fields {
                    self.spend(name.len())?;
                    output.push((name.clone(), self.normalize(ty, depth + 1)?));
                }
                if matches!(term, Term::RecordValue(_)) {
                    Term::RecordValue(output)
                } else {
                    Term::Record(output)
                }
            }
            Term::Optional(inner) => Term::Optional(Box::new(self.normalize(inner, depth + 1)?)),
            Term::Pointer(inner) => Term::Pointer(Box::new(self.normalize(inner, depth + 1)?)),
            Term::Lift(inner) => Term::Lift(Box::new(self.normalize(inner, depth + 1)?)),
            Term::NormalFormFunction(inner, collector) => {
                Term::NormalFormFunction(Box::new(self.normalize(inner, depth + 1)?), *collector)
            }
            Term::ComptimeFunction(inner) => {
                Term::ComptimeFunction(Box::new(self.normalize(inner, depth + 1)?))
            }
            Term::UnsafeFunction(inner) => {
                Term::UnsafeFunction(Box::new(self.normalize(inner, depth + 1)?))
            }
            Term::Function { parameters, result } => {
                if parameters.len() > self.remaining {
                    return Err(QueryError::Budget);
                }
                self.frames
                    .push(parameters.iter().map(|p| p.sort).collect());
                let normalized = (|| {
                    let parameters = parameters
                        .iter()
                        .map(|p| {
                            Ok(nir::Parameter {
                                sort: p.sort,
                                ty: self.normalize(&p.ty, depth + 1)?,
                            })
                        })
                        .collect::<Result<_, QueryError>>()?;
                    Ok::<_, QueryError>(Term::Function {
                        parameters,
                        result: Box::new(self.normalize(result, depth + 1)?),
                    })
                })();
                self.frames.pop();
                normalized?
            }
            Term::Closure {
                environment,
                signature,
            } => Term::Closure {
                environment: self.many(environment, depth + 1)?,
                signature: Box::new(self.normalize(signature, depth + 1)?),
            },
            Term::Float(value) => {
                self.spend(
                    value
                        .digits()
                        .len()
                        .saturating_add(value.exponent().bit_len().div_ceil(32)),
                )?;
                term.clone()
            }
            Term::Integer(value) => {
                self.spend(value.bit_len().div_ceil(32))?;
                term.clone()
            }
            Term::BitVec(value) => {
                self.spend(value.words().len())?;
                term.clone()
            }
            Term::String(value) | Term::Symbol(value) => {
                self.spend(value.len())?;
                term.clone()
            }
            _ => term.clone(),
        })
    }
}
impl fmt::Display for AssociatedTypeStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-type-step (projection {}) (resolution {}))",
            self.projection, self.resolution
        )
    }
}
impl fmt::Display for AssociatedTypeNormalization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-type-normalization (value {}) (steps",
            self.value
        )?;
        for step in &self.steps {
            write!(f, " {step}")?;
        }
        f.write_str(")")?;
        if !self.initializer_dependencies.is_empty() {
            f.write_str(" (initializer-dependencies")?;
            for key in &self.initializer_dependencies {
                write!(f, " {key}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

// Cache hits must pay for payload copying too, not only the node count.
// Shared preflight bounds recursive depth and the complete substituted payload.
fn charge_projection_substitution(
    term: &Term,
    parameters: &[nir::DeclarationParameter],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), QueryError> {
    nir::type_computations::charge_parameter_substitution(term, parameters, arguments, fuel)
        .map_err(QueryError::from)
}

pub fn charge_projection_copy(term: &Term, fuel: &mut usize) -> Result<(), QueryError> {
    nir::type_budget::charge_structure(term, fuel).map_err(|_| QueryError::Budget)
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use nir::program::{AssociatedContract, Trait, TraitRequirement};
    use nir::{AssociatedTypeOrigin, DeclarationId, Sort};

    #[test]
    fn given_views_preserve_valid_projections_without_weakening_strict_normalization() {
        let id = |local| DeclarationId { package: 1, local };
        let subject = Term::Rigid {
            declaration: id(3),
            sort: Sort::Type,
        };
        let projection = Term::AssociatedType {
            origin: Box::new(AssociatedTypeOrigin {
                interface: id(1),
                member: id(2),
            }),
            arguments: vec![],
            subject: Box::new(subject.clone()),
        };
        let mut traits = vec![Trait {
            comptime_only: false,
            parameter_types: Vec::new(),
            declaration: id(1),
            self_parameter: id(1),
            name: "Source".into(),
            type_parameters: vec![],
            parents: vec![],
            methods: vec![],
            requirements: vec![],
            capabilities: vec![],
            associated: vec![AssociatedContract {
                phase: Default::default(),
                declaration: id(2),
                name: "Element".into(),
                ty: Term::Primitive(Primitive::Type),
                default: None,
            }],
        }];
        let assumptions = vec![TraitRequirement {
            subject,
            interface: id(1),
            interface_arguments: vec![],
        }];
        let index = crate::implementation::Index::default();
        let context = crate::ptc::Context::default();
        let world = World {
            index: &index,
            implementations: &[],
            traits: &traits,
            assumptions: &assumptions,
            context: &context,
        };
        let repeated = Term::Tuple(vec![projection.clone(), projection.clone()]);
        let view = world
            .associated_type_view(&repeated, Budget::default())
            .unwrap();
        assert_eq!(view.value, repeated);
        assert!(view.steps.is_empty());
        assert!(
            world
                .normalize_associated_types(&projection, Budget::default())
                .is_err()
        );
        assert!(
            world
                .associated_type_view(
                    &projection,
                    Budget {
                        steps: 0,
                        ..Budget::default()
                    }
                )
                .is_err()
        );
        let mut invalid = projection.clone();
        let Term::AssociatedType { origin, .. } = &mut invalid else {
            unreachable!()
        };
        origin.member = id(99);
        assert!(
            world
                .associated_type_view(&invalid, Budget::default())
                .unwrap_err()
                .to_string()
                .contains("member does not belong")
        );
        let without_givens = World {
            assumptions: &[],
            ..world
        };
        assert!(
            without_givens
                .associated_type_view(&projection, Budget::default())
                .is_err()
        );
        let mut scoped_projection = projection.clone();
        if let Term::AssociatedType { subject, .. } = &mut scoped_projection {
            *subject = Box::new(Term::Bound {
                depth: 0,
                index: 0,
                sort: nir::Sort::Type,
            });
        }
        let callable = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(scoped_projection.clone()),
        };
        let view = without_givens
            .associated_type_view(&callable, Budget::default())
            .unwrap();
        assert_eq!(view.value, callable);
        assert!(view.steps.is_empty());
        let mut view_cache = std::collections::HashMap::new();
        let cached = without_givens
            .associated_type_view_cached_metered(&callable, &mut Budget::default(), &mut view_cache)
            .unwrap();
        assert_eq!(cached.value, callable);
        assert!(
            view_cache.is_empty(),
            "caller-bound projections must not enter a context-free cache"
        );
        let mut outer_projection = scoped_projection.clone();
        if let Term::AssociatedType { subject, .. } = &mut outer_projection {
            *subject = Box::new(Term::Bound {
                depth: 1,
                index: 0,
                sort: nir::Sort::Type,
            });
        }
        let nested = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(outer_projection),
            }),
        };
        let nested_view = without_givens
            .associated_type_view_cached_metered(&nested, &mut Budget::default(), &mut view_cache)
            .unwrap();
        assert_eq!(nested_view.value, nested);
        assert!(nested_view.steps.is_empty());
        assert!(
            view_cache.is_empty(),
            "outer-frame projections must not enter a context-free cache"
        );
        assert!(
            without_givens
                .normalize_associated_types(&nested, Budget::default())
                .is_err()
        );
        assert!(
            without_givens
                .normalize_associated_types(&callable, Budget::default())
                .is_err()
        );
        assert!(
            without_givens
                .associated_type_view(&scoped_projection, Budget::default())
                .is_err()
        );
        let mut bad_member = callable.clone();
        if let Term::Function { result, .. } = &mut bad_member {
            if let Term::AssociatedType { origin, .. } = result.as_mut() {
                origin.member = id(99);
            }
        }
        assert!(
            without_givens
                .associated_type_view(&bad_member, Budget::default())
                .unwrap_err()
                .to_string()
                .contains("member does not belong")
        );
        assert!(
            without_givens
                .associated_type_view_cached_metered(
                    &bad_member,
                    &mut Budget::default(),
                    &mut view_cache
                )
                .is_err()
        );
        assert!(view_cache.is_empty());
        traits[0].associated[0].ty = Term::Primitive(Primitive::Bool);
        let world = World {
            index: &index,
            implementations: &[],
            traits: &traits,
            assumptions: &assumptions,
            context: &context,
        };
        assert!(
            world
                .associated_type_view(&projection, Budget::default())
                .unwrap_err()
                .to_string()
                .contains("not Type-valued")
        );
        assert!(
            world
                .associated_type_view(&callable, Budget::default())
                .unwrap_err()
                .to_string()
                .contains("not Type-valued")
        );
    }
}

#[cfg(test)]
mod shared_cache_tests {
    use super::*;
    use nir::program::{AssociatedContract, Expr, Implementation, SourceRange, Trait};
    use nir::{AssociatedTypeOrigin, DeclarationId};

    #[test]
    fn completed_projection_cache_reuses_nested_results_and_rolls_back_failed_queries() {
        let id = |local| DeclarationId { package: 1, local };
        let result = Term::Primitive(Primitive::Char);
        let projection = Term::AssociatedType {
            origin: Box::new(AssociatedTypeOrigin {
                interface: id(1),
                member: id(2),
            }),
            arguments: vec![],
            subject: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let traits = vec![Trait {
            comptime_only: false,
            parameter_types: Vec::new(),
            declaration: id(1),
            self_parameter: id(1),
            name: "Source".into(),
            type_parameters: vec![],
            parents: vec![],
            methods: vec![],
            requirements: vec![],
            capabilities: vec![],
            associated: vec![AssociatedContract {
                phase: Default::default(),
                declaration: id(2),
                name: "Element".into(),
                ty: Term::Primitive(Primitive::Type),
                default: None,
            }],
        }];
        let implementations = vec![Implementation {
            phases: Default::default(),
            associated_initializers: vec![],
            parameter_types: vec![],
            declaration: id(3),
            subject: Term::Primitive(Primitive::Bool),
            interface: Some(id(1)),
            interface_arguments: vec![],
            type_parameters: vec![],
            capabilities: vec![],
            requirements: vec![],
            methods: vec![],
            associated: vec![(
                id(2),
                Expr {
                    ty: Term::Primitive(Primitive::Type),
                    source: SourceRange { start: 0, end: 0 },
                    kind: ExprKind::TypeValue(result.clone()),
                },
            )],
        }];
        let mut index = crate::implementation::Index::default();
        index
            .insert(
                crate::implementation::ImplementationHead {
                    parameter_types: vec![],
                    declaration: id(3),
                    subject: Term::Primitive(Primitive::Bool),
                    interface: Some(id(1)),
                    interface_arguments: vec![],
                    type_parameters: vec![],
                    requirements: vec![],
                },
                Budget::default(),
            )
            .unwrap();
        let context = crate::ptc::Context::default();
        let world = World {
            index: &index,
            implementations: &implementations,
            traits: &traits,
            assumptions: &[],
            context: &context,
        };
        // Both normalization entry paths must debit recursive selection once.
        // The standalone view receives the same half-allowance as prerequisite_type.
        let mut view_budget = Budget {
            steps: 50_000,
            ..Budget::default()
        };
        let view = world
            .associated_type_view_metered(&projection, &mut view_budget)
            .unwrap();
        let mut sequential = Budget {
            steps: 100_000 - (50_000 - view_budget.steps),
            ..Budget::default()
        };
        let target: InterfaceInstance = id(90).into();
        let direct = world
            .resolve_instance_metered(&target, &view.value, &mut sequential)
            .unwrap();
        let mut nested = Budget::default();
        let projected = world
            .resolve_projected_instance_metered(&target, &projection, &mut nested)
            .unwrap();
        assert_eq!(projected.types.selection, direct.types.selection);
        assert_eq!(
            nested.steps, sequential.steps,
            "nested query work must be debited exactly once"
        );

        // Host query failures retain their category and actual consumption;
        // neither an error nor an unspent reservation can zero the caller.
        for failure in [
            QueryError::Budget,
            QueryError::Invalid("bad origin".into()),
            QueryError::Unresolved("pending origin".into()),
        ] {
            let mut consumption = Vec::new();
            for cost in [0, 17] {
                let mut budget = Budget::default();
                let mut invocations = 0;
                let result = world.associated_type_view_for_selection(
                    &projection,
                    &mut budget,
                    &mut |_, _, allowance| {
                        invocations += 1;
                        allowance.steps -= cost;
                        Err(failure.clone())
                    },
                );
                let error = result.unwrap_err();
                assert_eq!(
                    std::mem::discriminant(&error),
                    std::mem::discriminant(&failure)
                );
                assert_eq!(invocations, 1);
                assert!(budget.steps > 0 && budget.steps < 100_000);
                consumption.push(100_000 - budget.steps);
            }
            assert_eq!(consumption[1] - consumption[0], 17);
        }
        let mut invalid = projection.clone();
        let Term::AssociatedType { origin, .. } = &mut invalid else {
            unreachable!()
        };
        origin.member = id(99);
        let mut invalid_budget = Budget::default();
        assert!(matches!(
            world.associated_type_view_metered(&invalid, &mut invalid_budget),
            Err(QueryError::Invalid(_))
        ));
        assert!(invalid_budget.steps > 0 && invalid_budget.steps < 100_000);
        assert_eq!(
            world
                .associated_type_view_metered(&projection, &mut invalid_budget)
                .unwrap()
                .value,
            result
        );
        let mut cache = std::collections::HashMap::new();
        assert!(
            world
                .normalize_associated_types_cached(
                    &Term::Tuple(vec![projection.clone(), invalid]),
                    Budget::default(),
                    &mut cache
                )
                .is_err()
        );
        assert!(
            cache.is_empty(),
            "failed outer query must not publish its successful prefix"
        );
        let (first, _) = world
            .normalize_associated_types_cached(
                &Term::Tuple(vec![projection.clone()]),
                Budget::default(),
                &mut cache,
            )
            .unwrap();
        assert_eq!(first.steps.len(), 1);
        assert_eq!(cache.get(&projection), Some(&result));
        let (second, _) = world
            .normalize_associated_types_cached(
                &Term::Pointer(Box::new(projection.clone())),
                Budget {
                    steps: 100,
                    ..Budget::default()
                },
                &mut cache,
            )
            .unwrap();
        assert!(second.steps.is_empty());
        assert_eq!(second.value, Term::Pointer(Box::new(result.clone())));
        let (optional, _) = world
            .normalize_associated_types_cached(
                &Term::Optional(Box::new(projection.clone())),
                Budget::default(),
                &mut cache,
            )
            .unwrap();
        assert!(optional.steps.is_empty());
        assert_eq!(optional.value, Term::Optional(Box::new(result)));
        let before = cache.clone();
        assert!(
            world
                .normalize_associated_types_cached(
                    &projection,
                    Budget {
                        steps: 0,
                        ..Budget::default()
                    },
                    &mut cache
                )
                .is_err()
        );
        assert_eq!(cache, before);
    }
}

mod values;
pub use values::{
    AssociatedRead, AssociatedValueResolution, PendingAssociatedSource, PreparedAssociatedMember,
    SelectedAssociatedMember, SelectedMemberData,
};
