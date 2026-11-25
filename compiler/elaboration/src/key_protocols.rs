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

mod work;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamingHasher {
    pub hasher_type: Term,
    pub init_body: DeclarationId,
    pub finish_body: DeclarationId,
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
    pub streaming_hasher: Option<StreamingHasher>,
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
    select_metered(package, world, protocol, subject, operation, &mut {
        budget
    })
}

/// Carry remaining work across lookup, specialization, type proofs and candidate
/// qualification. Selection never executes a program contract or key method.
pub fn select_metered(
    package: &Package,
    world: &World<'_>,
    protocol: DeclarationId,
    subject: &Term,
    operation: Operation,
    budget: &mut Budget,
) -> Result<Selected, String> {
    if budget.steps == 0
        || budget.max_depth == 0
        || crate::implementation::Key::from_type_metered(subject, budget).is_err()
    {
        return Err("comptime key selection budget exhausted".into());
    }
    work::spend(
        budget,
        package
            .traits
            .len()
            .saturating_add(world.traits.len().saturating_mul(2)),
    )?;
    let mut traits = package.traits.iter().filter(|t| t.declaration == protocol);
    let interface = traits
        .next()
        .ok_or("comptime key protocol is not a declared trait")?;
    let counterpart = world.traits.iter().find(|t| t.declaration == protocol);
    if let Some(counterpart) = counterpart {
        if !std::ptr::eq(interface, counterpart) {
            nir::copy_budget::charge_trait_metadata(interface, &mut budget.steps)
                .and_then(|_| {
                    nir::copy_budget::charge_trait_metadata(counterpart, &mut budget.steps)
                })
                .map_err(|error| format!("key interface comparison budget exhausted: {error:?}"))?;
        }
    }
    if traits.next().is_some()
        || world
            .traits
            .iter()
            .filter(|t| t.declaration == protocol)
            .count()
            != 1
        || counterpart.is_none_or(|other| !std::ptr::eq(interface, other) && other != interface)
    {
        return Err("comptime key protocol package and world disagree".into());
    }
    let has_rhs_type_param = operation == Operation::Eq
        && interface.type_parameters.len() == 1
        && (interface.parameter_types.is_empty()
            || matches!(
                interface.parameter_types.as_slice(),
                [Term::Primitive(Primitive::Type)]
            ));
    if !interface.type_parameters.is_empty() && !has_rhs_type_param {
        return Err("comptime key protocol has explicit type parameters".into());
    }
    // The selected trait identity supplies the phase-specific protocol. Method
    // spelling is shared with runtime interfaces; prefixed names are ordinary
    // unrelated members and cannot override or repair this contract.
    work::spend(
        budget,
        interface
            .methods
            .len()
            .saturating_mul(operation.name().len() + 1),
    )?;
    let mut methods = interface
        .methods
        .iter()
        .filter(|m| m.name == operation.name());
    let member = methods
        .next()
        .ok_or("comptime key protocol method is missing")?;
    let streaming_h_param = match (operation, member.type_parameters.as_slice()) {
        (Operation::Hash, [h_param]) if member.type_arguments.is_empty() => Some(*h_param),
        _ => None,
    };
    if methods.next().is_some()
        || (!member.type_parameters.is_empty() && streaming_h_param.is_none())
        || !member.type_arguments.is_empty()
    {
        return Err("comptime key protocol method is duplicated or generic".into());
    }
    if member.phase != nir::program::TraitPhase::Comptime {
        return Err("key protocol method requires comptime trait membership".into());
    }
    let ordinary_signature = member
        .signature
        .ordinary()
        .map_err(|_| "key protocol requires a comptime method")?;
    let Term::ComptimeFunction(signature) = (match ordinary_signature {
        Term::PureFunction(inner) => inner.as_ref(),
        other => other,
    }) else {
        return Err("key protocol requires a comptime method".into());
    };
    let Term::Function { parameters, result } = signature.as_ref() else {
        return Err("invalid comptime key signature".into());
    };
    work::charge_term(signature, budget)?;
    let self_type = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let expected_arity = if streaming_h_param.is_some() {
        2
    } else {
        operation.arity()
    };
    let expected = match operation {
        Operation::Eq => {
            let rhs_type = if let [rhs_param] = interface.type_parameters.as_slice() {
                Term::Rigid {
                    declaration: *rhs_param,
                    sort: Sort::Type,
                }
            } else {
                self_type.clone()
            };
            vec![
                Parameter {
                    sort: Sort::Runtime,
                    ty: self_type,
                },
                Parameter {
                    sort: Sort::Runtime,
                    ty: rhs_type,
                },
            ]
        }
        Operation::Hash => {
            if let Some(h_param) = streaming_h_param {
                let h_type = Term::Rigid {
                    declaration: h_param,
                    sort: Sort::Type,
                };
                vec![
                    Parameter {
                        sort: Sort::Runtime,
                        ty: self_type,
                    },
                    Parameter {
                        sort: Sort::Runtime,
                        ty: h_type,
                    },
                ]
            } else {
                vec![Parameter {
                    sort: Sort::Runtime,
                    ty: self_type,
                }]
            }
        }
    };
    let valid_result = match operation {
        Operation::Eq => result.as_ref() == &Term::Primitive(Primitive::Bool),
        Operation::Hash => {
            if let Some(h_param) = streaming_h_param {
                result.as_ref()
                    == &Term::Rigid {
                        declaration: h_param,
                        sort: Sort::Type,
                    }
            } else {
                matches!(result.as_ref(), Term::Primitive(p) if *p == Primitive::Integer || p.fixed_integer_layout().is_some())
            }
        }
    };
    if parameters != &expected
        || !valid_result
        || !member.by_value_receiver
        || member.by_reference_receiver
        || member.parameter_bindings.len() != expected_arity
    {
        return Err("invalid comptime key protocol receiver, parameters or result".into());
    }
    let streaming_hasher =
        if let Some(h_param) = streaming_h_param {
            let h_type = Term::Rigid {
                declaration: h_param,
                sort: Sort::Type,
            };
            let hasher_protocol = member
                .capabilities
                .iter()
                .filter_map(|req| {
                    (req.subject == h_type && req.interface_arguments.is_empty())
                        .then_some(req.interface)
                })
                .chain(member.requirements.iter().filter_map(|pred| match pred {
                    nir::program::TypePredicate::Trait(req)
                        if req.subject == h_type && req.interface_arguments.is_empty() =>
                    {
                        Some(req.interface)
                    }
                    _ => None,
                }))
                .next()
                .ok_or("streaming comptime hash requires ComptimeHasher constraint")?;
            work::spend(
                budget,
                package
                    .implementations
                    .len()
                    .saturating_add(world.implementations.len()),
            )?;
            let hasher_impl = package
                .implementations
                .iter()
                .filter(|i| {
                    i.interface == Some(hasher_protocol)
                        && i.type_parameters.is_empty()
                        && i.requirements.is_empty()
                        && i.phases.comptime
                        && world.implementations.iter().any(|w| {
                            w.declaration == i.declaration && (std::ptr::eq(*i, w) || *i == w)
                        })
                })
                .min_by_key(|i| {
                    if i.declaration.package == hasher_protocol.package {
                        0u8
                    } else {
                        1
                    }
                })
                .ok_or("streaming comptime hash requires concrete ComptimeHasher implementation")?;
            let hasher_type = hasher_impl.subject.clone();
            work::charge_term(&hasher_type, budget)?;
            let finish_body = hasher_impl
                .methods
                .iter()
                .find(|(name, _)| name == "finish")
                .ok_or("ComptimeHasher implementation is missing finish method")?
                .1;
            let finish_fn = package
                .functions
                .iter()
                .find(|f| f.declaration == finish_body)
                .ok_or("ComptimeHasher finish function is missing")?;
            if finish_fn.phase != FunctionPhase::Comptime
                || !finish_fn.type_parameters.is_empty()
                || finish_fn.parameters.len() != 1
                || !finish_fn.defaults.is_empty()
            {
                return Err("invalid ComptimeHasher finish signature".into());
            }
            let init_body = package
                .implementations
                .iter()
                .filter(|i| {
                    i.interface.is_none()
                        && i.subject == hasher_type
                        && i.type_parameters.is_empty()
                        && world.implementations.iter().any(|w| {
                            w.declaration == i.declaration && (std::ptr::eq(*i, w) || *i == w)
                        })
                })
                .find_map(|i| {
                    i.methods
                        .iter()
                        .find(|(name, _)| name == "new")
                        .map(|(_, id)| *id)
                })
                .ok_or("ComptimeHasher type is missing comptime new() constructor")?;
            let init_fn = package
                .functions
                .iter()
                .find(|f| f.declaration == init_body)
                .ok_or("ComptimeHasher new() function is missing")?;
            if init_fn.phase != FunctionPhase::Comptime
                || !init_fn.type_parameters.is_empty()
                || !init_fn.parameters.is_empty()
                || !init_fn.defaults.is_empty()
            {
                return Err("invalid ComptimeHasher new() signature".into());
            }
            Some((
                h_param,
                hasher_protocol,
                StreamingHasher {
                    hasher_type,
                    init_body,
                    finish_body,
                },
            ))
        } else {
            None
        };
    let (binders, inputs) = if let [rhs_param] = interface.type_parameters.as_slice() {
        (
            vec![
                nir::DeclarationParameter {
                    declaration: interface.self_parameter,
                    sort: Sort::Type,
                },
                nir::DeclarationParameter {
                    declaration: *rhs_param,
                    sort: Sort::Type,
                },
            ],
            vec![subject.clone(), subject.clone()],
        )
    } else if let Some((h_param, _, hasher)) = &streaming_hasher {
        (
            vec![
                nir::DeclarationParameter {
                    declaration: interface.self_parameter,
                    sort: Sort::Type,
                },
                nir::DeclarationParameter {
                    declaration: *h_param,
                    sort: Sort::Type,
                },
            ],
            vec![subject.clone(), hasher.hasher_type.clone()],
        )
    } else {
        (
            vec![nir::DeclarationParameter {
                declaration: interface.self_parameter,
                sort: Sort::Type,
            }],
            vec![subject.clone()],
        )
    };
    work::charge_term(subject, budget)?;
    work::spend(
        budget,
        member
            .name
            .len()
            .saturating_add(member.parameter_bindings.len())
            .saturating_add(member.requirements.len())
            .saturating_add(member.capabilities.len()),
    )?;
    let contract = MethodContract {
        source: member.source,
        phase: member.phase,
        is_quote: member.is_quote,
        pure: member.pure,
        is_unsafe: member.is_unsafe,
        type_arguments: vec![],
        type_parameters: vec![],
        capabilities: member
            .capabilities
            .iter()
            .filter(|r| {
                !matches!(
                    streaming_h_param,
                    Some(h_param)
                        if r.subject
                            == Term::Rigid {
                                declaration: h_param,
                                sort: Sort::Type,
                            }
                            && r.interface_arguments.is_empty()
                )
            })
            .map(|r| work::requirement(r, &binders, &inputs, budget))
            .collect::<Result<_, _>>()?,
        program: work::program(
            &member.program.preconditions,
            &member.program.postconditions,
            member.program.result_binding,
            &member.program.entry_snapshots,
            &binders,
            &inputs,
            budget,
        )?,
        parameter_bindings: member.parameter_bindings.clone(),
        by_value_receiver: member.by_value_receiver,
        by_reference_receiver: member.by_reference_receiver,
        declaration: member.declaration,
        name: member.name.clone(),
        signature: nir::code_interfaces::MethodSignature::Callable(work::term(
            member
                .signature
                .ordinary()
                .map_err(|error| error.to_string())?,
            &binders,
            &inputs,
            budget,
        )?),
        requirements: member
            .requirements
            .iter()
            .map(|p| work::predicate(p, &binders, &inputs, budget))
            .collect::<Result<_, _>>()?,
        default: member.default,
    };
    for requirement in interface.requirements.iter().chain(&member.requirements) {
        let predicate = work::predicate(requirement, &binders, &inputs, budget)?;
        if !work::prove(world, &predicate, budget)? {
            return Err("comptime key protocol type requirement is not established".into());
        }
    }
    let interface_instance = nir::program::InterfaceInstance {
        declaration: protocol,
        arguments: if interface.type_parameters.is_empty() {
            vec![]
        } else {
            vec![subject.clone()]
        },
    };
    let selection = world.resolve_instance_metered(&interface_instance, subject, budget)?;
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
    work::spend(
        budget,
        package
            .implementations
            .len()
            .saturating_add(world.implementations.len().saturating_mul(2)),
    )?;
    let mut definitions = package
        .implementations
        .iter()
        .filter(|i| i.declaration == implementation);
    let definition = definitions
        .next()
        .ok_or("comptime key implementation is missing")?;
    let counterpart = world
        .implementations
        .iter()
        .find(|i| i.declaration == implementation);
    if let Some(counterpart) = counterpart {
        if !std::ptr::eq(definition, counterpart) {
            nir::copy_budget::charge_implementation_metadata(definition, &mut budget.steps)
                .and_then(|_| {
                    nir::copy_budget::charge_implementation_metadata(counterpart, &mut budget.steps)
                })
                .map_err(|error| {
                    format!("key implementation comparison budget exhausted: {error:?}")
                })?;
        }
    }
    if definitions.next().is_some()
        || world
            .implementations
            .iter()
            .filter(|i| i.declaration == implementation)
            .count()
            != 1
        || counterpart.is_none_or(|other| !std::ptr::eq(definition, other) && other != definition)
    {
        return Err("comptime key implementation package and world disagree".into());
    }
    if !definition.phases.comptime {
        return Err("selected key implementation lacks comptime trait phase".into());
    }
    work::spend(
        budget,
        definition
            .methods
            .len()
            .saturating_mul(member.name.len() + 1),
    )?;
    let mut bodies = definition
        .methods
        .iter()
        .filter(|(body_name, _)| body_name == &member.name);
    let body = bodies
        .next()
        .ok_or("comptime key method body is missing")?
        .1;
    if bodies.next().is_some() {
        return Err("comptime key method body is duplicated".into());
    }
    let (arguments, method_arguments) = if member.default == Some(body) {
        let mut args = inputs
            .iter()
            .map(|ty| work::term(ty, &[], &[], budget))
            .collect::<Result<Vec<_>, _>>()?;
        let method_args = if let Some((_, _, hasher)) = &streaming_hasher {
            vec![work::term(&hasher.hasher_type, &[], &[], budget)?]
        } else {
            vec![]
        };
        if member.default == Some(body) && args.len() > 1 && streaming_hasher.is_some() {
            // Default body expects [impl_args..., method_arg] matching binders/inputs.
            let _ = &mut args;
        }
        (args, method_args)
    } else {
        work::spend(budget, evidence.type_arguments.len())?;
        let mut args = evidence
            .type_arguments
            .iter()
            .map(|ty| work::term(ty, &[], &[], budget))
            .collect::<Result<Vec<_>, _>>()?;
        let method_args = if let Some((_, _, hasher)) = &streaming_hasher {
            let h_ty = work::term(&hasher.hasher_type, &[], &[], budget)?;
            args.push(h_ty.clone());
            vec![h_ty]
        } else {
            vec![]
        };
        (args, method_args)
    };
    work::spend(budget, package.functions.len())?;
    let mut functions = package.functions.iter().filter(|f| f.declaration == body);
    let function = functions.next().ok_or("comptime key function is missing")?;
    if functions.next().is_some() {
        return Err("comptime key function identity is duplicated".into());
    }
    let method_binders =
        package.declaration_binders(&function.type_parameters, &mut budget.steps)?;
    let signature = work::term(&function.signature, &method_binders, &arguments, budget)?;
    let expected_ordinary = contract.signature.ordinary().unwrap();
    let Term::ComptimeFunction(expected) = (match expected_ordinary {
        Term::PureFunction(inner) => inner.as_ref(),
        other => other,
    }) else {
        unreachable!()
    };
    if function.phase != FunctionPhase::Comptime
        || &signature != expected.as_ref()
        || function.parameters.len() != expected_arity
        || !function.defaults.is_empty()
    {
        return Err("selected key body does not implement the comptime protocol signature".into());
    }
    for requirement in &function.requirements {
        let predicate = work::predicate(requirement, &method_binders, &arguments, budget)?;
        if !work::prove(world, &predicate, budget)? {
            return Err("comptime key body type requirement is not established".into());
        }
    }
    let program = work::program(
        &function.preconditions,
        &function.postconditions,
        function.result_binding,
        &function.entry_snapshots,
        &method_binders,
        &arguments,
        budget,
    )?;
    work::spend(
        budget,
        contract
            .capabilities
            .len()
            .saturating_add(function.capabilities.len())
            .saturating_add(interface.capabilities.len())
            .saturating_add(interface.parents.len()),
    )?;
    let mut capabilities = contract
        .capabilities
        .iter()
        .map(|r| work::requirement(r, &[], &[], budget))
        .collect::<Result<Vec<_>, _>>()?;
    for capability in &function.capabilities {
        capabilities.push(work::requirement(
            capability,
            &method_binders,
            &arguments,
            budget,
        )?);
    }
    if let Some((_, hasher_protocol, hasher)) = &streaming_hasher {
        capabilities.retain(|cap| {
            !(cap.interface == *hasher_protocol
                && cap.subject == hasher.hasher_type
                && cap.interface_arguments.is_empty())
        });
    }
    for capability in &interface.capabilities {
        capabilities.push(work::requirement(capability, &binders, &inputs, budget)?);
    }
    for parent in &interface.parents {
        work::spend(budget, parent.arguments.len())?;
        capabilities.push(TraitRequirement {
            interface: parent.declaration,
            interface_arguments: parent
                .arguments
                .iter()
                .map(|ty| work::term(ty, &binders, &inputs, budget))
                .collect::<Result<_, _>>()?,
            subject: work::term(subject, &[], &[], budget)?,
        });
    }
    work::spend(budget, function.parameters.len())?;
    work::charge_term(subject, budget)?;
    Ok(Selected {
        operation,
        protocol,
        subject: subject.clone(),
        implementation,
        method: MethodInstance {
            member: member.declaration,
            body,
            arguments,
            method_arguments,
        },
        contract,
        parameters: function.parameters.clone(),
        program,
        capabilities,
        selection,
        streaming_hasher: streaming_hasher.map(|(_, _, hasher)| hasher),
    })
}
