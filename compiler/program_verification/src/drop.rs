//! Select the language-bound Drop protocol in an immutable implementation world.
//! This fixes a method instance and its program contracts, not permission to run
//! it. Ownership, access, effects, runtime environment acquisition and post-Drop
//! field state remain obligations of the resource verifier, outside PTC.
use crate::{
    capabilities::{Qualification, Witness, World},
    implementation::{Selection, TypeAssessment, TypeResolution},
    ptc::{Budget, Formula, Outcome},
};
use nir::{
    DeclarationId, Parameter, Primitive, Sort, Term,
    contracts::ProgramContract,
    program::{FunctionPhase, LocalId, MethodContract, MethodInstance, Package, TraitRequirement},
};
use std::fmt;

mod evidence;

pub use evidence::SelectionEvidence;

#[derive(Clone, Debug, PartialEq)]
pub struct DropEnvRequirement {
    pub declaration: DeclarationId,
    pub identity: Term,
    pub binding: LocalId,
    pub ty: Term,
}

impl fmt::Display for DropEnvRequirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(drop-env (declaration {})", self.declaration)?;
        write!(f, " (identity {})", self.identity)?;
        write!(f, " (binding {}) (type {}))", self.binding, self.ty)
    }
}

#[derive(Clone, Debug)]
pub struct SelectedDrop {
    pub protocol: DeclarationId,
    pub subject: Term,
    pub implementation: DeclarationId,
    pub implementation_arguments: Vec<Term>,
    pub method: MethodInstance,
    pub contract: MethodContract,
    pub parameters: Vec<LocalId>,
    pub env_requirements: Vec<DropEnvRequirement>,
    pub program: ProgramContract,
    /// Concrete type capabilities, not captured runtime handler/allocator values.
    capabilities: Vec<TraitRequirement>,
    /// Private authority: descriptions may be inspected, but query premises and
    /// applicability evidence cannot be replaced independently by consumers.
    evidence: SelectionEvidence,
}

#[derive(Clone, Debug)]
pub enum Resolution {
    /// No user destructor in this query world. Automatic field cleanup remains.
    NoCustomDrop(SelectionEvidence),
    Assumed {
        requirement: TraitRequirement,
        contract: MethodContract,
    },
    Selected(SelectedDrop),
    Pending(String),
}

impl SelectedDrop {
    pub fn selection(&self) -> &TypeResolution {
        self.evidence.type_selection()
    }

    pub fn qualification(&self) -> &[Witness] {
        self.evidence.qualification()
    }

    pub fn capabilities(&self) -> &[TraitRequirement] {
        &self.capabilities
    }
}

impl fmt::Display for SelectedDrop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(selected-drop (protocol {}) (subject {}) (implementation {}) (implementation-arguments",
            self.protocol, self.subject, self.implementation
        )?;
        for argument in &self.implementation_arguments {
            write!(f, " {argument}")?;
        }
        write!(
            f,
            ") {} (contract {}) (parameters",
            self.method, self.contract
        )?;
        for parameter in &self.parameters {
            write!(f, " {parameter}")?;
        }
        if !self.env_requirements.is_empty() {
            f.write_str(") (env-requirements")?;
            for env in &self.env_requirements {
                write!(f, " {env}")?;
            }
        }
        write!(f, ") {} (capabilities", self.program)?;
        for capability in &self.capabilities {
            write!(f, " {capability}")?;
        }
        f.write_str(") (qualification")?;
        for witness in self.qualification() {
            write!(f, " {witness}")?;
        }
        write!(f, ") {})", self.evidence)
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCustomDrop(evidence) => {
                write!(f, "(drop-resolution (no-custom-drop {evidence}))")
            }
            Self::Assumed {
                requirement,
                contract,
            } => write!(f, "(drop-resolution (assumed {requirement} {contract}))"),
            Self::Selected(selected) => write!(f, "(drop-resolution {selected})"),
            Self::Pending(reason) => write!(
                f,
                "(drop-resolution (pending {}))",
                Term::String(reason.clone().into())
            ),
        }
    }
}

fn signature(subject: Term) -> Term {
    Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: Term::Pointer(Box::new(subject)),
        }],
        result: Box::new(Term::Primitive(Primitive::Void)),
    }
}

fn instantiate_contract(
    member: &MethodContract,
    self_parameter: DeclarationId,
    subject: &Term,
    fuel: &mut usize,
) -> Result<MethodContract, String> {
    let parameters = [self_parameter];
    let domains = [Term::Primitive(Primitive::Type)];
    let arguments = std::slice::from_ref(subject);
    let closed = nir::DeclarationSubstitution::bounded(&parameters, &domains, arguments, &[], fuel)
        .map_err(|e| e.to_string())?;
    let signature = member.signature.ordinary().map_err(|e| e.to_string())?;
    let completed_signature = closed
        .instantiate_bounded(signature, fuel)
        .map_err(|e| e.to_string())?;
    let Term::Function {
        parameters: inputs, ..
    } = signature.callable_signature()
    else {
        return Err("Drop contract lacks an ordinary signature".into());
    };
    *fuel = fuel
        .checked_sub(
            inputs
                .len()
                .saturating_add(member.name.len())
                .saturating_add(member.type_parameters.len())
                .saturating_add(member.parameter_bindings.len()),
        )
        .ok_or("Drop contract copy budget exhausted")?;
    let frames = [inputs.iter().map(|p| p.sort).collect()];
    let scoped =
        nir::DeclarationSubstitution::bounded(&parameters, &domains, arguments, &frames, fuel)
            .map_err(|e| e.to_string())?;
    Ok(MethodContract {
        source: member.source,
        declaration: member.declaration,
        name: member.name.clone(),
        phase: member.phase,
        is_quote: member.is_quote,
        pure: member.pure,
        is_unsafe: member.is_unsafe,
        by_value_receiver: member.by_value_receiver,
        by_reference_receiver: member.by_reference_receiver,
        default: member.default,
        type_parameters: member.type_parameters.clone(),
        parameter_bindings: member.parameter_bindings.clone(),
        type_arguments: member
            .type_arguments
            .iter()
            .map(|ty| closed.instantiate_bounded(ty, fuel))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?,
        signature: completed_signature.into(),
        program: nir::contracts::CallableProgram::method(member)
            .map_err(|e| e.to_string())?
            .instantiate_contract(&scoped, fuel)
            .map_err(|e| e.to_string())?,
        requirements: member
            .requirements
            .iter()
            .map(|r| scoped.instantiate_predicate_bounded(r, fuel))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?,
        capabilities: member
            .capabilities
            .iter()
            .map(|r| scoped.instantiate_requirement_bounded(r, fuel))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?,
    })
}

/// Source spelling never designates the protocol. The caller must supply its
/// trusted language-item identity and the immutable package/world that owns it.
/// No result is cached across implementation worlds or assumption changes.
pub fn select(
    package: &Package,
    world: &World<'_>,
    protocol: DeclarationId,
    subject: &Term,
    budget: Budget,
) -> Result<Resolution, String> {
    let trait_contract = world
        .traits
        .iter()
        .find(|t| t.declaration == protocol)
        .ok_or("Drop protocol identity is not a declared trait")?;
    if package.traits.iter().find(|t| t.declaration == protocol) != Some(trait_contract) {
        return Err("Drop protocol package and query world disagree".into());
    }
    if !trait_contract.type_parameters.is_empty() {
        return Err("Drop protocol must have no explicit type parameters".into());
    }
    let mut members = trait_contract.methods.iter().filter(|m| m.name == "drop");
    let member = members
        .next()
        .ok_or("Drop protocol lacks its drop method")?;
    if members.next().is_some() {
        return Err("Drop protocol has duplicate drop methods".into());
    }
    let expected = signature(Term::Rigid {
        declaration: trait_contract.self_parameter,
        sort: Sort::Type,
    });
    if !member.by_reference_receiver
        || member.by_value_receiver
        || member.signature != expected
        || member.parameter_bindings.len() != 1
    {
        return Err("Drop protocol requires runtime fn drop(*self) -> void".into());
    }
    if budget.steps == 0 || budget.max_depth == 0 {
        return Ok(Resolution::Pending("Drop query budget exhausted".into()));
    }
    // Bound subject structure before recursive substitution/cloning.
    if crate::implementation::Key::from_type(subject, budget).is_err() {
        return Ok(Resolution::Pending(
            "Drop subject query budget exhausted".into(),
        ));
    }
    let mut preparation_fuel = budget.steps;
    let contract = instantiate_contract(
        member,
        trait_contract.self_parameter,
        subject,
        &mut preparation_fuel,
    )?;
    let self_parameters = [trait_contract.self_parameter];
    let self_domains = [Term::Primitive(Primitive::Type)];
    let self_substitution = nir::DeclarationSubstitution::bounded(
        &self_parameters,
        &self_domains,
        std::slice::from_ref(subject),
        &[],
        &mut preparation_fuel,
    )
    .map_err(|e| e.to_string())?;
    for requirement in &trait_contract.requirements {
        let requirement = self_substitution
            .instantiate_predicate_bounded(requirement, &mut preparation_fuel)
            .map_err(|e| e.to_string())?;
        if !matches!(
            world
                .context
                .prove(Formula::from_predicate(&requirement), budget),
            Outcome::Entailed(_)
        ) {
            return Ok(Resolution::Pending(
                "Drop interface type requirement is not established".into(),
            ));
        }
    }
    // Only type predicates enter PTC. Program pre/postconditions are retained
    // verbatim (with type substitution) for the independent verifier.
    for requirement in &contract.requirements {
        if !matches!(
            world
                .context
                .prove(Formula::from_predicate(requirement), budget),
            Outcome::Entailed(_)
        ) {
            return Ok(Resolution::Pending(
                "Drop method type requirement is not established".into(),
            ));
        }
    }
    let requirement = TraitRequirement {
        interface: protocol,
        interface_arguments: vec![],
        subject: subject.clone(),
    };
    if let Some(requirement) = world.given(&requirement, budget)? {
        return Ok(Resolution::Assumed {
            requirement,
            contract,
        });
    }
    let resolution = world.resolve(protocol, subject, budget)?;
    let implementation = match resolution.types.selection {
        Selection::Selected(id) => id,
        Selection::NoApplicableCandidate if world.assumptions.is_empty() => {
            return Ok(Resolution::NoCustomDrop(SelectionEvidence::capture(
                protocol,
                subject,
                resolution,
                &mut preparation_fuel,
            )?));
        }
        Selection::NoApplicableCandidate => {
            return Ok(Resolution::Pending(
                "Drop assumptions were not established".into(),
            ));
        }
        ref unresolved => return Ok(Resolution::Pending(unresolved.to_string())),
    };
    let Some(TypeAssessment::Applicable(evidence)) =
        resolution.types.assessments.get(&implementation)
    else {
        return Err("Drop selection lacks instantiated type evidence".into());
    };
    if !matches!(
        resolution.qualifications.get(&implementation),
        Some(Qualification::Proved(_))
    ) {
        return Err("Drop selection lacks capability evidence".into());
    };
    let definition = world
        .implementations
        .iter()
        .find(|i| i.declaration == implementation)
        .ok_or("Drop selected implementation is missing")?;
    if package
        .implementations
        .iter()
        .find(|i| i.declaration == implementation)
        != Some(definition)
    {
        return Err("Drop implementation package and query world disagree".into());
    }
    let mut bodies = definition.methods.iter().filter(|(name, _)| name == "drop");
    let body = bodies
        .next()
        .ok_or("Drop selected implementation has no drop body")?
        .1;
    if bodies.next().is_some() {
        return Err("Drop selected implementation has duplicate drop bodies".into());
    }
    let arguments = if member.default == Some(body) {
        vec![subject.clone()]
    } else {
        evidence.type_arguments.clone()
    };
    let function = package
        .functions
        .iter()
        .find(|f| f.declaration == body)
        .ok_or("Drop selected method body is missing")?;
    let domains =
        super::memory::memory_domains(package, &function.type_parameters, &mut preparation_fuel)?;
    let substitution = nir::DeclarationSubstitution::bounded(
        &function.type_parameters,
        &domains,
        &arguments,
        &[],
        &mut preparation_fuel,
    )
    .map_err(|e| e.to_string())?;
    let actual = substitution
        .instantiate_bounded(&function.signature, &mut preparation_fuel)
        .map_err(|e| e.to_string())?;
    if function.phase != FunctionPhase::Runtime
        || actual != signature(subject.clone())
        || function.parameters.len() != 1
        || !function.defaults.is_empty()
    {
        return Err("Drop selected body does not implement runtime fn drop(*self) -> void".into());
    }
    // The ABI check above fixes one ordinary runtime receiver frame. Program
    // predicates remain independent obligations; only type requirements enter PTC.
    let frames = [vec![Sort::Runtime]];
    let scoped = nir::DeclarationSubstitution::bounded(
        &function.type_parameters,
        &domains,
        &arguments,
        &frames,
        &mut preparation_fuel,
    )
    .map_err(|e| e.to_string())?;
    let env_requirements = drop_environment(
        package,
        function,
        &scoped,
        &arguments,
        &mut preparation_fuel,
    )?;
    for requirement in &function.requirements {
        let requirement = scoped
            .instantiate_predicate_bounded(requirement, &mut preparation_fuel)
            .map_err(|e| e.to_string())?;
        if !matches!(
            world
                .context
                .prove(Formula::from_predicate(&requirement), budget),
            Outcome::Entailed(_)
        ) {
            return Ok(Resolution::Pending(
                "Drop body type requirement is not established".into(),
            ));
        }
    }
    let program = nir::contracts::CallableProgram::function(function)
        .instantiate_contract(&scoped, &mut preparation_fuel)
        .map_err(|e| e.to_string())?;
    let mut capabilities: Vec<_> = function
        .capabilities
        .iter()
        .map(|c| scoped.instantiate_requirement_bounded(c, &mut preparation_fuel))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for capability in &trait_contract.capabilities {
        capabilities.push(
            self_substitution
                .instantiate_requirement_bounded(capability, &mut preparation_fuel)
                .map_err(|e| e.to_string())?,
        );
    }
    for parent in &trait_contract.parents {
        let parent = self_substitution
            .instantiate_interface_bounded(parent, &mut preparation_fuel)
            .map_err(|e| e.to_string())?;
        capabilities.push(TraitRequirement {
            interface: parent.declaration,
            interface_arguments: parent.arguments,
            subject: subject.clone(),
        });
    }
    Ok(Resolution::Selected(SelectedDrop {
        protocol,
        subject: subject.clone(),
        implementation,
        implementation_arguments: evidence.type_arguments.clone(),
        method: MethodInstance {
            method_arguments: vec![],
            member: member.declaration,
            body,
            arguments,
        },
        contract,
        parameters: function.parameters.clone(),
        env_requirements,
        program,
        capabilities,
        evidence: SelectionEvidence::capture(protocol, subject, resolution, &mut preparation_fuel)?,
    }))
}

pub(crate) mod access;

pub use access::{
    AccessCheck, DropEnvContext, PointerMemory, check_access, check_access_with_memory,
};

mod plan;

pub use plan::{
    AcquiredCleanup, CleanupPlan, CleanupReadFootprint, CleanupReader, CleanupSession, CleanupStep,
    RequiredDropInstance, VariantCleanup, acquire_cleanup, plan_cleanup, plan_cleanup_with_memory,
    plan_cleanup_with_variants,
};

fn drop_environment(
    package: &Package,
    function: &nir::program::Function,
    substitution: &nir::DeclarationSubstitution<'_>,
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<Vec<DropEnvRequirement>, String> {
    let mut env_requirements = Vec::with_capacity(function.env_parameters.len());
    let mut existing_envs = Vec::with_capacity(function.env_parameters.len());
    for env in &function.env_parameters {
        let substituted = substitution
            .instantiate_bounded(
                &Term::Rigid {
                    declaration: env.declaration,
                    sort: Sort::Runtime,
                },
                fuel,
            )
            .map_err(|e| e.to_string())?;
        let (declaration, runtime_id) = match &substituted {
            Term::Runtime(id) => (env.declaration, Some(*id)),
            Term::Rigid {
                declaration: next,
                sort: Sort::Runtime,
            } => (*next, env.runtime_id),
            Term::RuntimeField { .. } => (env.declaration, None),
            _ => {
                return Err(
                    "Drop environment parameter did not instantiate to a runtime identity".into(),
                );
            }
        };
        let ty = substitution
            .instantiate_bounded(&env.ty, fuel)
            .map_err(|e| e.to_string())?;
        existing_envs.push(nir::program::RuntimeEnvParameter {
            declaration,
            runtime_id,
            binding: env.binding,
            ty: ty.clone(),
        });
        env_requirements.push(DropEnvRequirement {
            declaration,
            identity: runtime_id.map_or(substituted, Term::Runtime),
            binding: env.binding,
            ty,
        });
    }
    let implicit_envs = nir::program::implicit_runtime_env_parameters(
        &package.runtime_domains,
        &package.global_runtime_deliveries,
        arguments,
        &existing_envs,
    )?;
    for env in implicit_envs {
        env_requirements.push(DropEnvRequirement {
            declaration: env.declaration,
            identity: Term::Runtime(
                env.runtime_id
                    .ok_or("implicit environment requires a runtime identity")?,
            ),
            binding: env.binding,
            ty: env.ty,
        });
    }
    Ok(env_requirements)
}
