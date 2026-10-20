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

#[derive(Clone, Debug)]
pub struct SelectedDrop {
    pub protocol: DeclarationId,
    pub subject: Term,
    pub implementation: DeclarationId,
    pub implementation_arguments: Vec<Term>,
    pub method: MethodInstance,
    pub contract: MethodContract,
    pub parameters: Vec<LocalId>,
    pub program: ProgramContract,
    /// Concrete type capabilities, not captured runtime handler/allocator values.
    pub capabilities: Vec<TraitRequirement>,
    pub qualification: Vec<Witness>,
    /// Retains the index identity/revision and type applicability evidence.
    pub selection: TypeResolution,
}
#[derive(Clone, Debug)]
pub enum Resolution {
    /// No user destructor in this query world. Automatic field cleanup remains.
    NoCustomDrop(TypeResolution),
    Assumed {
        requirement: TraitRequirement,
        contract: MethodContract,
    },
    Selected(SelectedDrop),
    Pending(String),
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
        write!(f, ") {} (capabilities", self.program)?;
        for capability in &self.capabilities {
            write!(f, " {capability}")?;
        }
        f.write_str(") (qualification")?;
        for witness in &self.qualification {
            write!(f, " {witness}")?;
        }
        write!(f, ") {})", self.selection)
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
) -> Result<MethodContract, String> {
    let mut contract = member.clone();
    let arguments = [subject.clone()];
    contract.signature = member
        .signature
        .instantiate_types(&[self_parameter], &arguments)
        .map_err(|e| e.to_string())?;
    contract.program = member
        .program
        .instantiate_types(&[self_parameter], &arguments)
        .map_err(|e| e.to_string())?;
    contract.requirements = member
        .requirements
        .iter()
        .map(|r| r.instantiate_types(&[self_parameter], &arguments))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    Ok(contract)
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
    let contract = instantiate_contract(member, trait_contract.self_parameter, subject)?;
    for requirement in &trait_contract.requirements {
        let requirement = requirement
            .instantiate_types(&[trait_contract.self_parameter], &[subject.clone()])
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
    if let Some(requirement) = world.given(&requirement, budget) {
        return Ok(Resolution::Assumed {
            requirement,
            contract,
        });
    }
    let resolution = world.resolve(protocol, subject, budget)?;
    let implementation = match resolution.types.selection {
        Selection::Selected(id) => id,
        Selection::NoApplicableCandidate if world.assumptions.is_empty() => {
            return Ok(Resolution::NoCustomDrop(resolution.types));
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
    let Some(Qualification::Proved(qualification)) = resolution.qualifications.get(&implementation)
    else {
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
    let actual = function
        .signature
        .instantiate_types(&function.type_parameters, &arguments)
        .map_err(|e| e.to_string())?;
    if function.phase != FunctionPhase::Runtime
        || actual != signature(subject.clone())
        || function.parameters.len() != 1
        || !function.defaults.is_empty()
    {
        return Err("Drop selected body does not implement runtime fn drop(*self) -> void".into());
    }
    for requirement in &function.requirements {
        let requirement = requirement
            .instantiate_types(&function.type_parameters, &arguments)
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
    let program = ProgramContract {
        preconditions: function.preconditions.clone(),
        postconditions: function.postconditions.clone(),
        result_binding: function.result_binding,
        entry_snapshots: function.entry_snapshots.clone(),
    }
    .instantiate_types(&function.type_parameters, &arguments)
    .map_err(|e| e.to_string())?;
    let mut capabilities: Vec<_> = function
        .capabilities
        .iter()
        .map(|c| c.instantiate_types(&function.type_parameters, &arguments))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for capability in &trait_contract.capabilities {
        capabilities.push(
            capability
                .instantiate_types(&[trait_contract.self_parameter], &[subject.clone()])
                .map_err(|e| e.to_string())?,
        );
    }
    for parent in &trait_contract.parents {
        let parent = parent
            .instantiate_types(&[trait_contract.self_parameter], &[subject.clone()])
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
        program,
        capabilities,
        qualification: qualification.clone(),
        selection: resolution.types,
    }))
}

mod access;
pub use access::{AccessCheck, PointerMemory, check_access, check_access_with_memory};

mod plan;
pub use plan::{
    AcquiredCleanup, CleanupPlan, CleanupSession, CleanupStep, VariantCleanup, acquire_cleanup,
    plan_cleanup, plan_cleanup_with_memory, plan_cleanup_with_variants,
};
