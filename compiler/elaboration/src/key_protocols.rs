//! Concrete comptime query-key method selection. Selection preserves obligations;
//! it does not authorize invocation, value freezing, or semantic query reuse.
use crate::{
    capabilities::{self, Qualification, World},
    implementation::{Selection, TypeAssessment},
    ptc::{Budget, Formula, Outcome},
};
use nir::contracts::ProgramContract;
use nir::program::{
    FunctionPhase, LocalId, MethodContract, MethodInstance, Package, TraitRequirement,
};
use nir::{DeclarationId, Parameter, Primitive, Sort, Term};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Eq,
    Hash,
}
impl Operation {
    fn name(self) -> &'static str {
        match self {
            Self::Eq => "eq",
            Self::Hash => "hash",
        }
    }
    fn arity(self) -> usize {
        match self {
            Self::Eq => 2,
            Self::Hash => 1,
        }
    }
}
impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug)]
pub struct Selected {
    pub operation: Operation,
    pub protocol: DeclarationId,
    pub subject: Term,
    pub implementation: DeclarationId,
    pub method: MethodInstance,
    pub contract: MethodContract,
    pub parameters: Vec<LocalId>,
    pub program: ProgramContract,
    pub capabilities: Vec<TraitRequirement>,
    /// Retain the complete resolution, including its captured assumption scope.
    pub selection: capabilities::Resolution,
}
impl fmt::Display for Selected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(comptime-key-method {} (protocol {}) (subject {}) (implementation {}) {} (contract {}) (parameters",
            self.operation,
            self.protocol,
            self.subject,
            self.implementation,
            self.method,
            self.contract
        )?;
        for p in &self.parameters {
            write!(f, " {p}")?;
        }
        write!(f, ") {} (capabilities", self.program)?;
        for c in &self.capabilities {
            write!(f, " {c}")?;
        }
        write!(f, ") {})", self.selection)
    }
}

/// The protocol identity comes from a trusted language-item binding, not its
/// source spelling. An assumed capability alone cannot supply executable code.
/// Hash currently supports primitive integer results; bucket encoding is a
/// separate operation and must not truncate an arbitrary-precision hash.
pub fn select(
    package: &Package,
    world: &World<'_>,
    protocol: DeclarationId,
    subject: &Term,
    operation: Operation,
    budget: Budget,
) -> Result<Selected, String> {
    if budget.steps == 0
        || budget.max_depth == 0
        || crate::implementation::Key::from_type(subject, budget).is_err()
    {
        return Err("comptime key selection budget exhausted".into());
    }
    let mut traits = package.traits.iter().filter(|t| t.declaration == protocol);
    let interface = traits
        .next()
        .ok_or("comptime key protocol is not a declared trait")?;
    if traits.next().is_some()
        || world
            .traits
            .iter()
            .filter(|t| t.declaration == protocol)
            .count()
            != 1
        || world.traits.iter().find(|t| t.declaration == protocol) != Some(interface)
    {
        return Err("comptime key protocol package and world disagree".into());
    }
    if !interface.type_parameters.is_empty() {
        return Err("comptime key protocol has explicit type parameters".into());
    }
    let mut methods = interface
        .methods
        .iter()
        .filter(|m| m.name == operation.name());
    let member = methods
        .next()
        .ok_or("comptime key protocol method is missing")?;
    if methods.next().is_some()
        || !member.type_parameters.is_empty()
        || !member.type_arguments.is_empty()
    {
        return Err("comptime key protocol method is duplicated or generic".into());
    }
    if member.phase != nir::program::TraitPhase::Comptime {
        return Err("key protocol method requires comptime trait membership".into());
    }
    let Ok(Term::ComptimeFunction(signature)) = member.signature.ordinary() else {
        return Err("key protocol requires a comptime method".into());
    };
    let Term::Function { parameters, result } = signature.as_ref() else {
        return Err("invalid comptime key signature".into());
    };
    let self_type = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let expected = vec![
        Parameter {
            sort: Sort::Runtime,
            ty: self_type
        };
        operation.arity()
    ];
    let valid_result = match operation {
        Operation::Eq => result.as_ref() == &Term::Primitive(Primitive::Bool),
        Operation::Hash => {
            matches!(result.as_ref(), Term::Primitive(p) if *p == Primitive::Integer || p.fixed_integer_layout().is_some())
        }
    };
    if parameters != &expected
        || !valid_result
        || !member.by_value_receiver
        || member.by_reference_receiver
        || member.parameter_bindings.len() != operation.arity()
    {
        return Err("invalid comptime key protocol receiver, parameters or result".into());
    }
    let binders = [interface.self_parameter];
    let arguments = [subject.clone()];
    let mut contract = member.clone();
    contract.signature = member
        .signature
        .instantiate_types(&binders, &arguments)
        .map_err(|e| e.to_string())?;
    contract.program = member
        .program
        .instantiate_types(&binders, &arguments)
        .map_err(|e| e.to_string())?;
    contract.requirements = member
        .requirements
        .iter()
        .map(|p| p.instantiate_types(&binders, &arguments))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    contract.capabilities = member
        .capabilities
        .iter()
        .map(|p| p.instantiate_types(&binders, &arguments))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for requirement in interface.requirements.iter().chain(&member.requirements) {
        let predicate = requirement
            .instantiate_types(&binders, &arguments)
            .map_err(|e| e.to_string())?;
        if !matches!(
            world
                .context
                .prove(Formula::from_predicate(&predicate), budget),
            Outcome::Entailed(_)
        ) {
            return Err("comptime key protocol type requirement is not established".into());
        }
    }
    let selection = world.resolve(protocol, subject, budget)?;
    let Selection::Selected(implementation) = selection.types.selection else {
        return Err(format!(
            "comptime key implementation is not concrete: {}",
            selection.types.selection
        ));
    };
    let Some(TypeAssessment::Applicable(evidence)) =
        selection.types.assessments.get(&implementation)
    else {
        return Err("comptime key selection lacks type evidence".into());
    };
    if !matches!(
        selection.qualifications.get(&implementation),
        Some(Qualification::Proved(_))
    ) {
        return Err("comptime key selection lacks capability evidence".into());
    }
    let mut definitions = package
        .implementations
        .iter()
        .filter(|i| i.declaration == implementation);
    let definition = definitions
        .next()
        .ok_or("comptime key implementation is missing")?;
    if definitions.next().is_some()
        || world
            .implementations
            .iter()
            .filter(|i| i.declaration == implementation)
            .count()
            != 1
        || world
            .implementations
            .iter()
            .find(|i| i.declaration == implementation)
            != Some(definition)
    {
        return Err("comptime key implementation package and world disagree".into());
    }
    if !definition.phases.comptime {
        return Err("selected key implementation lacks comptime trait phase".into());
    }
    let mut bodies = definition
        .methods
        .iter()
        .filter(|(name, _)| name == operation.name());
    let body = bodies
        .next()
        .ok_or("comptime key method body is missing")?
        .1;
    if bodies.next().is_some() {
        return Err("comptime key method body is duplicated".into());
    }
    let arguments = if member.default == Some(body) {
        vec![subject.clone()]
    } else {
        evidence.type_arguments.clone()
    };
    let mut functions = package.functions.iter().filter(|f| f.declaration == body);
    let function = functions.next().ok_or("comptime key function is missing")?;
    if functions.next().is_some() {
        return Err("comptime key function identity is duplicated".into());
    }
    let mut binder_fuel = budget.steps;
    let method_binders =
        package.declaration_binders(&function.type_parameters, &mut binder_fuel)?;
    let signature = function
        .signature
        .instantiate_parameters(&method_binders, &arguments)
        .map_err(|e| e.to_string())?;
    let Ok(Term::ComptimeFunction(expected)) = contract.signature.ordinary() else {
        unreachable!()
    };
    if function.phase != FunctionPhase::Comptime
        || &signature != expected.as_ref()
        || function.parameters.len() != operation.arity()
        || !function.defaults.is_empty()
    {
        return Err("selected key body does not implement the comptime protocol signature".into());
    }
    for requirement in &function.requirements {
        let predicate = requirement
            .instantiate_parameters(&method_binders, &arguments)
            .map_err(|e| e.to_string())?;
        if !matches!(
            world
                .context
                .prove(Formula::from_predicate(&predicate), budget),
            Outcome::Entailed(_)
        ) {
            return Err("comptime key body type requirement is not established".into());
        }
    }
    let program = ProgramContract {
        preconditions: function.preconditions.clone(),
        postconditions: function.postconditions.clone(),
        result_binding: function.result_binding,
        entry_snapshots: function.entry_snapshots.clone(),
    }
    .instantiate_parameters(&method_binders, &arguments)
    .map_err(|e| e.to_string())?;
    let mut capabilities = contract.capabilities.clone();
    for capability in &function.capabilities {
        capabilities.push(
            capability
                .instantiate_parameters(&method_binders, &arguments)
                .map_err(|e| e.to_string())?,
        );
    }
    for capability in &interface.capabilities {
        capabilities.push(
            capability
                .instantiate_types(&binders, &[subject.clone()])
                .map_err(|e| e.to_string())?,
        );
    }
    for parent in &interface.parents {
        let parent = parent
            .instantiate_types(&binders, &[subject.clone()])
            .map_err(|e| e.to_string())?;
        capabilities.push(TraitRequirement {
            interface: parent.declaration,
            interface_arguments: parent.arguments,
            subject: subject.clone(),
        });
    }
    Ok(Selected {
        operation,
        protocol,
        subject: subject.clone(),
        implementation,
        method: MethodInstance {
            member: member.declaration,
            body,
            arguments,
            method_arguments: vec![],
        },
        contract,
        parameters: function.parameters.clone(),
        program,
        capabilities,
        selection,
    })
}
