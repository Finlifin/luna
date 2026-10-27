use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use nir::program::{
    Expr, ExprKind as K, Function, FunctionPhase, Package, RuntimeFunctionIdentity, SourceRange,
};
use nir::{DeclarationId, Primitive, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn signature(result: Term) -> Term {
    Term::Function {
        parameters: vec![],
        result: Box::new(result),
    }
}
fn package() -> Package {
    let callable = signature(Term::Primitive(Primitive::Bool));
    Package {
        runtime_function_identities: vec![RuntimeFunctionIdentity {
            declaration: id(2),
            type_parameters: vec![id(3)],
            signature: signature(Term::Rigid {
                declaration: id(3),
                sort: Sort::Type,
            }),
        }],
        functions: vec![Function {
            declaration: id(1),
            name: "obtain".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: false,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            signature: signature(callable.clone()),
            parameters: vec![],
            defaults: vec![],
            body: Expr {
                source: SourceRange { start: 1, end: 2 },
                ty: callable,
                kind: K::FunctionInstance {
                    declaration: id(2),
                    arguments: vec![Term::Primitive(Primitive::Bool)],
                },
            },
        }],
        ..Default::default()
    }
}

#[test]
fn runtime_identity_is_typed_transport_without_a_body() {
    let package = package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let value = vm.invoke(id(1), vec![]).unwrap();
    assert_eq!(
        value,
        Value::RuntimeFunction {
            declaration: id(2),
            arguments: vec![Term::Primitive(Primitive::Bool)]
        }
    );
    assert!(format!("{package}").contains("(runtime-function-identity"));
    assert!(format!("{value}").starts_with("(runtime-function "));
    assert_eq!(
        vm.invoke(id(2), vec![]).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
}

#[test]
fn runtime_identity_snapshots_validate_scope_and_recover_after_budget_failure() {
    let value = Value::RuntimeFunction {
        declaration: id(2),
        arguments: vec![Term::Rigid {
            declaration: id(3),
            sort: Sort::Type,
        }],
    };
    let scope = std::collections::BTreeSet::from([id(3)]);
    let original = value.clone();
    assert_eq!(
        value.validate_runtime_identity_snapshot(
            &mut comptime_runtime::Budget::default(),
            &Default::default()
        ),
        Err(ErrorKind::Type)
    );
    assert_eq!(
        value.validate_runtime_identity_snapshot(
            &mut comptime_runtime::Budget {
                steps: 0,
                max_limbs: 4096
            },
            &scope
        ),
        Err(ErrorKind::StepsExhausted)
    );
    value
        .validate_runtime_identity_snapshot(&mut comptime_runtime::Budget::default(), &scope)
        .unwrap();
    assert_eq!(value, original);
    for argument in [
        Term::Rigid {
            declaration: id(3),
            sort: Sort::ComptimeValue,
        },
        Term::TypeListConcat(vec![Term::TypeListValue(vec![])]),
    ] {
        let unresolved = Value::RuntimeFunction {
            declaration: id(2),
            arguments: vec![argument],
        };
        assert_eq!(
            unresolved.validate_runtime_identity_snapshot(
                &mut comptime_runtime::Budget::default(),
                &scope
            ),
            Err(ErrorKind::Type)
        );
    }
}

#[test]
fn malformed_identity_catalogs_and_instances_are_rejected() {
    let mut duplicate = package();
    duplicate
        .runtime_function_identities
        .push(duplicate.runtime_function_identities[0].clone());
    assert!(nir::phases::validate(&duplicate).is_err());
    assert!(Interpreter::new(&duplicate, Budget::default()).is_err());
    let mut collision = package();
    collision.runtime_function_identities[0].declaration = id(1);
    assert!(Interpreter::for_comptime(&collision, Budget::default()).is_err());
    let mut wrong_arity = package();
    if let K::FunctionInstance { arguments, .. } = &mut wrong_arity.functions[0].body.kind {
        arguments.clear();
    }
    let mut vm = Interpreter::for_comptime(&wrong_arity, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(1), vec![]).unwrap_err().kind, ErrorKind::Type);
}

#[test]
fn an_identity_cannot_be_called_even_without_the_comptime_entry_gate() {
    let mut package = package();
    let callee = package.functions[0].body.clone();
    package.functions[0].signature = signature(Term::Primitive(Primitive::Bool));
    package.functions[0].body = Expr {
        source: callee.source,
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Call {
            callee: Box::new(callee),
            arguments: vec![],
            parameter_bindings: vec![],
        },
    };
    assert!(Interpreter::for_comptime(&package, Budget::default()).is_err());
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
}

fn trait_package() -> Package {
    let mut package = package();
    package.traits.push(nir::program::Trait {
        parameter_types: Vec::new(),
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        type_parameters: vec![],
        declaration: id(4),
        parents: vec![],
        self_parameter: id(4),
        name: "Factory".into(),
        methods: vec![nir::program::MethodContract {
            phase: Default::default(),
            is_quote: false,
            pure: false,
            is_unsafe: false,
            type_arguments: vec![],
            type_parameters: vec![id(3)],
            capabilities: vec![],
            program: Default::default(),
            parameter_bindings: vec![],
            by_value_receiver: false,
            by_reference_receiver: false,
            declaration: id(5),
            name: "make".into(),
            signature: package.runtime_function_identities[0]
                .signature
                .clone()
                .into(),
            requirements: vec![],
            default: None,
        }],
    });
    package.implementations.push(nir::program::Implementation {
        phases: Default::default(),
        associated_initializers: vec![],
        parameter_types: vec![],
        associated: vec![],
        interface_arguments: vec![],
        capabilities: vec![],
        type_parameters: vec![],
        requirements: vec![],
        declaration: id(6),
        subject: Term::Primitive(Primitive::Bool),
        interface: Some(id(4)),
        methods: vec![("make".into(), id(2))],
    });
    package.functions[0].body.kind = K::TraitMethod {
        interface: id(4),
        interface_arguments: vec![],
        subject: Term::Primitive(Primitive::Bool),
        member: id(5),
        method_arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    package
}

#[test]
fn body_free_trait_method_retains_the_interface_origin() {
    let package = trait_package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let value = vm.invoke(id(1), vec![]).unwrap();
    assert_eq!(
        value,
        Value::RuntimeTraitMethod(Box::new(comptime_interpretor::TraitMethodIdentity {
            interface: id(4).into(),
            subject: Term::Primitive(Primitive::Bool),
            member: id(5),
            method_arguments: vec![Term::Primitive(Primitive::Bool)],
        }))
    );
    assert!(format!("{value}").starts_with("(runtime-trait-method "));
}

#[test]
fn body_free_trait_method_cannot_execute_or_erase_a_precondition() {
    let mut package = trait_package();
    let callee = package.functions[0].body.clone();
    package.functions[0].signature = signature(Term::Primitive(Primitive::Bool));
    package.functions[0].body = Expr {
        source: callee.source,
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Call {
            callee: Box::new(callee),
            arguments: vec![],
            parameter_bindings: vec![],
        },
    };
    assert!(Interpreter::for_comptime(&package, Budget::default()).is_err());
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
    let mut package = trait_package();
    package.traits[0].methods[0]
        .program
        .preconditions
        .push(Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(nir::program::Literal::Bool(true)),
        });
    assert!(Interpreter::for_comptime(&package, Budget::default()).is_err());
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    // Reject storing the contracted interface value before attempting to
    // resolve an executable body; its precondition cannot be erased.
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap_err().kind,
        ErrorKind::Unsupported
    );
}

#[test]
fn generator_method_contract_cannot_enter_ordinary_interpreter_dispatch() {
    let mut package = package();
    Interpreter::new(&package, Budget::default()).unwrap();
    package.traits.push(nir::program::Trait {
        parameter_types: vec![],
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        type_parameters: vec![],
        declaration: id(40),
        parents: vec![],
        self_parameter: id(40),
        name: "Generator".into(),
        methods: vec![nir::program::MethodContract {
            phase: Default::default(),
            is_quote: false,
            pure: false,
            is_unsafe: false,
            type_arguments: vec![],
            type_parameters: vec![],
            capabilities: vec![],
            program: Default::default(),
            parameter_bindings: vec![],
            by_value_receiver: false,
            by_reference_receiver: false,
            declaration: id(41),
            name: "build".into(),
            signature: nir::code_interfaces::MethodSignature::Generator(
                nir::code_interfaces::CodeFunctionSignature {
                    clause_domains: vec![],
                    inputs: vec![],
                    output: nir::code_interfaces::ExpressionCodeInterface {
                        infer_context: false,
                        context: Default::default(),
                        ports: Default::default(),
                        defaults: Default::default(),
                        result: Term::Primitive(Primitive::Bool),
                    },
                },
            ),
            requirements: vec![],
            default: None,
        }],
    });
    assert!(
        matches!(Interpreter::new(&package,Budget::default()),Err(error) if error.kind == ErrorKind::InvalidCallPlan)
    );
    package.traits.clear();
    Interpreter::new(&package, Budget::default()).unwrap();
}

#[test]
fn body_free_runtime_identity_checks_unused_value_parameter_domains() {
    let mut package = package();
    package.structs.push(nir::program::Struct {
        declaration: id(90),
        name: "Holder".into(),
        type_parameters: vec![id(3)],
        parameter_types: vec![Term::Primitive(Primitive::Bool)],
        fields: vec![],
        requirements: vec![],
        capabilities: vec![],
    });
    package.runtime_function_identities[0].signature = signature(Term::Primitive(Primitive::Bool));
    for value in [Term::Char('x'), Term::Bool(true), Term::Bool(false)] {
        let K::FunctionInstance { arguments, .. } = &mut package.functions[0].body.kind else {
            unreachable!()
        };
        *arguments = vec![value.clone()];
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let result = vm.invoke(id(1), vec![]);
        if matches!(value, Term::Bool(_)) {
            assert_eq!(
                result.unwrap(),
                Value::RuntimeFunction {
                    declaration: id(2),
                    arguments: vec![value]
                }
            );
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
        }
    }
}

#[test]
fn runtime_identity_admission_checks_structured_value_layouts() {
    let mut package = package();
    let config = Term::Nominal {
        declaration: id(90),
        arguments: vec![],
    };
    package.structs.push(nir::program::Struct {
        declaration: id(90),
        name: "Config".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        fields: vec![("enabled".into(), Term::Primitive(Primitive::Bool))],
        requirements: vec![],
        capabilities: vec![],
    });
    package
        .function_parameters
        .push(nir::function_parameters::Declaration {
            declaration: id(2),
            parameters: vec![id(3)],
            domains: vec![config.clone()],
        });
    package.runtime_function_identities[0].signature = signature(Term::Primitive(Primitive::Bool));
    for fields in [
        vec![Term::Bool(true)],
        vec![Term::Char('x')],
        vec![],
        vec![Term::Bool(true), Term::Bool(false)],
        vec![Term::Bool(false)],
    ] {
        let valid = matches!(fields.as_slice(), [Term::Bool(_)]);
        let argument = Term::NominalValue {
            ty: Box::new(config.clone()),
            variant: None,
            fields,
        };
        let K::FunctionInstance { arguments, .. } = &mut package.functions[0].body.kind else {
            unreachable!()
        };
        *arguments = vec![argument.clone()];
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let result = vm.invoke(id(1), vec![]);
        if valid {
            assert_eq!(
                result.unwrap(),
                Value::RuntimeFunction {
                    declaration: id(2),
                    arguments: vec![argument],
                }
            );
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
        }
    }
}

#[test]
fn host_runtime_identity_cannot_hide_an_invalid_unused_value_argument() {
    let mut package = package();
    package.runtime_function_identities[0].signature = signature(Term::Primitive(Primitive::Bool));
    package
        .function_parameters
        .push(nir::function_parameters::Declaration {
            declaration: id(2),
            parameters: vec![id(3)],
            domains: vec![Term::Primitive(Primitive::Bool)],
        });
    package.functions[0].signature = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Runtime,
            ty: signature(Term::Primitive(Primitive::Bool)),
        }],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    package.functions[0].parameters = vec![nir::program::LocalId { owner: 1, index: 0 }];
    package.functions[0].body = Expr {
        ty: Term::Primitive(Primitive::Bool),
        source: SourceRange { start: 1, end: 2 },
        kind: K::Literal(nir::program::Literal::Bool(true)),
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for argument in [Term::Char('x'), Term::Bool(true), Term::Bool(false)] {
        let valid = matches!(argument, Term::Bool(_));
        let result = vm.invoke(
            id(1),
            vec![Value::RuntimeFunction {
                declaration: id(2),
                arguments: vec![argument],
            }],
        );
        if valid {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
        }
    }
    drop(vm);
    package.function_parameters.clear();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let opaque = Value::RuntimeFunction {
        declaration: id(2),
        arguments: vec![Term::Rigid {
            declaration: id(99),
            sort: Sort::Type,
        }],
    };
    assert!(vm.invoke(id(1), vec![opaque.clone()]).is_err());
    vm.authorize_abstract_type_parameters(&[id(99)]).unwrap();
    assert_eq!(vm.invoke(id(1), vec![opaque]).unwrap(), Value::Bool(true));
}

#[test]
fn host_trait_method_identity_checks_unused_value_parameters() {
    let mut package = trait_package();
    let callable = signature(Term::Primitive(Primitive::Bool));
    package.runtime_function_identities[0].signature = callable.clone();
    package.traits[0].methods[0].signature = callable.clone().into();
    package
        .function_parameters
        .push(nir::function_parameters::Declaration {
            declaration: id(2),
            parameters: vec![id(3)],
            domains: vec![Term::Primitive(Primitive::Bool)],
        });
    package.functions[0].signature = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Runtime,
            ty: callable,
        }],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    package.functions[0].parameters = vec![nir::program::LocalId { owner: 1, index: 0 }];
    package.functions[0].body = Expr {
        ty: Term::Primitive(Primitive::Bool),
        source: SourceRange { start: 1, end: 2 },
        kind: K::Literal(nir::program::Literal::Bool(true)),
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for arguments in [vec![Term::Char('x')], vec![], vec![Term::Bool(true)]] {
        let valid = matches!(arguments.as_slice(), [Term::Bool(_)]);
        let identity =
            Value::RuntimeTraitMethod(Box::new(comptime_interpretor::TraitMethodIdentity {
                interface: id(4).into(),
                subject: Term::Primitive(Primitive::Bool),
                member: id(5),
                method_arguments: arguments,
            }));
        let result = vm.invoke(id(1), vec![identity]);
        if valid {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
        }
    }
}

#[test]
fn host_runtime_identity_checks_dependent_domains_inside_aggregates() {
    let mut package = package();
    let callable = signature(Term::Primitive(Primitive::Bool));
    package.runtime_function_identities[0].signature = callable.clone();
    package.runtime_function_identities[0].type_parameters = vec![id(3), id(7)];
    package
        .function_parameters
        .push(nir::function_parameters::Declaration {
            declaration: id(2),
            parameters: vec![id(3), id(7)],
            domains: vec![
                Term::Primitive(Primitive::Type),
                Term::Rigid {
                    declaration: id(3),
                    sort: Sort::Type,
                },
            ],
        });
    package.functions[0].signature = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Runtime,
            ty: Term::Tuple(vec![callable]),
        }],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    package.functions[0].parameters = vec![nir::program::LocalId { owner: 1, index: 0 }];
    package.functions[0].body = Expr {
        ty: Term::Primitive(Primitive::Bool),
        source: SourceRange { start: 1, end: 2 },
        kind: K::Literal(nir::program::Literal::Bool(true)),
    };
    let config = Term::Nominal {
        declaration: id(90),
        arguments: vec![],
    };
    package.structs.push(nir::program::Struct {
        declaration: id(90),
        name: "Config".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        fields: vec![("enabled".into(), Term::Primitive(Primitive::Bool))],
        requirements: vec![],
        capabilities: vec![],
    });
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for (domain, value, valid) in [
        (Term::Primitive(Primitive::Bool), Term::Char('x'), false),
        (Term::Primitive(Primitive::Bool), Term::Bool(true), true),
        (
            config.clone(),
            Term::NominalValue {
                ty: Box::new(config.clone()),
                variant: None,
                fields: vec![Term::Char('x')],
            },
            false,
        ),
        (
            config.clone(),
            Term::NominalValue {
                ty: Box::new(config),
                variant: None,
                fields: vec![Term::Bool(false)],
            },
            true,
        ),
    ] {
        let identity = Value::RuntimeFunction {
            declaration: id(2),
            arguments: vec![domain, value],
        };
        let result = vm.invoke(id(1), vec![Value::Tuple(vec![identity])]);
        if valid {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
        }
    }
}

#[test]
fn available_runtime_body_does_not_erase_transported_trait_origin() {
    let mut package = trait_package();
    package.runtime_function_identities.clear();
    let method = &mut package.traits[0].methods[0];
    method.type_parameters.clear();
    method.signature = signature(Term::Primitive(Primitive::Bool)).into();
    let K::TraitMethod {
        method_arguments, ..
    } = &mut package.functions[0].body.kind
    else {
        unreachable!()
    };
    method_arguments.clear();
    let mut body = package.functions[0].clone();
    body.declaration = id(2);
    body.name = "runtime_body".into();
    body.phase = FunctionPhase::Runtime;
    body.signature = signature(Term::Primitive(Primitive::Bool));
    body.body = Expr {
        ty: Term::Primitive(Primitive::Bool),
        source: SourceRange { start: 3, end: 4 },
        kind: K::Literal(nir::program::Literal::Bool(true)),
    };
    package.functions.push(body);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap(),
        Value::RuntimeTraitMethod(Box::new(comptime_interpretor::TraitMethodIdentity {
            interface: id(4).into(),
            subject: Term::Primitive(Primitive::Bool),
            member: id(5),
            method_arguments: vec![],
        }))
    );
    let predicate = package.functions[1].body.clone();
    package.traits[0].methods[0]
        .program
        .preconditions
        .push(predicate);
    assert!(Interpreter::for_comptime(&package, Budget::default()).is_err());
}

#[test]
fn frozen_runtime_identity_round_trips_without_changing_phase_or_responsibility() {
    use nir::{Parameter, program::LocalId};
    for unsafe_call in [false, true] {
        let mut package = package();
        let callable =
            signature(Term::Primitive(Primitive::Bool)).with_unsafe_call_requirement(unsafe_call);
        package.runtime_function_identities[0].signature = package.runtime_function_identities[0]
            .signature
            .clone()
            .with_unsafe_call_requirement(unsafe_call);
        package.functions[0].signature = signature(callable.clone());
        package.functions[0].body.ty = callable.clone();
        let binding = LocalId { owner: 4, index: 0 };
        let mut accept = package.functions[0].clone();
        accept.declaration = id(4);
        accept.name = "accept".into();
        accept.signature = Term::Function {
            parameters: vec![Parameter {
                sort: Sort::ComptimeValue,
                ty: callable.clone(),
            }],
            result: Box::new(callable.clone()),
        };
        accept.parameters = vec![binding];
        accept.body = Expr {
            source: SourceRange { start: 3, end: 4 },
            ty: callable.clone(),
            kind: K::Local(binding),
        };
        package.functions.push(accept);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let value = vm.invoke(id(1), vec![]).unwrap();
        assert_eq!(
            value.clone().into_normal_term(&mut Default::default()),
            Err(ErrorKind::Unsupported)
        );
        let frozen = value
            .clone()
            .into_scoped_invocation_term(&mut Default::default(), &Default::default())
            .unwrap();
        assert_eq!(
            frozen,
            Term::FunctionValue {
                declaration: id(2),
                arguments: vec![Term::Primitive(Primitive::Bool)]
            }
        );
        assert_eq!(
            vm.invoke_with_normal_arguments(id(4), vec![(callable.clone(), frozen.clone())])
                .unwrap(),
            value
        );
        for (domain, term) in [
            (callable.clone().with_comptime_phase(true), frozen.clone()),
            (
                signature(Term::Primitive(Primitive::Bool))
                    .with_unsafe_call_requirement(!unsafe_call),
                frozen.clone(),
            ),
            (
                callable.clone(),
                Term::FunctionValue {
                    declaration: id(99),
                    arguments: vec![],
                },
            ),
            (
                callable.clone(),
                Term::FunctionValue {
                    declaration: id(2),
                    arguments: vec![Term::Primitive(Primitive::Char)],
                },
            ),
            (
                callable.clone(),
                Term::FunctionValue {
                    declaration: id(2),
                    arguments: vec![],
                },
            ),
        ] {
            assert!(
                vm.invoke_with_normal_arguments(id(4), vec![(domain, term)])
                    .is_err()
            );
        }
        assert_eq!(
            vm.invoke(id(2), vec![]).unwrap_err().kind,
            ErrorKind::PhaseMismatch
        );
        assert_eq!(
            vm.invoke_with_normal_arguments(id(4), vec![(callable, frozen)])
                .unwrap(),
            value
        );
    }
}

#[test]
fn runtime_identity_freezing_is_bounded_and_keeps_aggregate_structure() {
    let identity = Value::RuntimeFunction {
        declaration: id(2),
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    let value = Value::Tuple(vec![Value::AnonymousRecord(vec![(
        "callback".into(),
        identity,
    )])]);
    assert_eq!(
        value.clone().into_scoped_invocation_term(
            &mut comptime_runtime::Budget {
                steps: 0,
                max_limbs: 4096
            },
            &Default::default()
        ),
        Err(ErrorKind::StepsExhausted)
    );
    let term = value
        .into_scoped_invocation_term(&mut Default::default(), &Default::default())
        .unwrap();
    assert_eq!(
        term,
        Term::TupleValue(vec![Term::RecordValue(vec![(
            "callback".into(),
            Term::FunctionValue {
                declaration: id(2),
                arguments: vec![Term::Primitive(Primitive::Bool)]
            }
        )])])
    );
    let unresolved = Value::RuntimeFunction {
        declaration: id(2),
        arguments: vec![Term::Rigid {
            declaration: id(8),
            sort: Sort::ComptimeValue,
        }],
    };
    assert_eq!(
        unresolved.into_scoped_invocation_term(&mut Default::default(), &Default::default()),
        Err(ErrorKind::Type)
    );
}

#[test]
fn indexed_world_lookup_preserves_late_function_collisions_and_duplicate_bodies() {
    let mut world = package();
    let prototype = world.functions[0].clone();
    world.functions.extend((100..612).map(|local| Function {
        declaration: id(local),
        ..prototype.clone()
    }));
    let mut vm = Interpreter::for_comptime(&world, Budget::default()).unwrap();
    assert!(
        matches!(vm.invoke(id(611), vec![]).unwrap(), Value::RuntimeFunction { declaration, .. } if declaration == id(2))
    );
    world.runtime_function_identities[0].declaration = id(611);
    assert!(
        matches!(Interpreter::new(&world, Budget::default()), Err(error) if error.kind == ErrorKind::InvalidCallPlan)
    );
    world.runtime_function_identities[0].declaration = id(2);
    world
        .functions
        .push(world.functions.last().unwrap().clone());
    assert!(
        matches!(Interpreter::new(&world, Budget::default()), Err(error) if error.kind == ErrorKind::InvalidCallPlan)
    );
    world.functions.pop();
    assert!(Interpreter::for_comptime(&world, Budget::default()).is_ok());
}

#[test]
fn dispatch_implementation_index_checks_missing_and_duplicate_identities() {
    let mut world = trait_package();
    let implementation = world.implementations.remove(0);
    world
        .implementations
        .extend((100..164).map(|local| nir::program::Implementation {
            declaration: id(local),
            interface: None,
            methods: vec![],
            ..implementation.clone()
        }));
    world.implementations.push(implementation.clone());
    world
        .dispatch_instances
        .push(nir::program::DispatchInstance {
            interface_arguments: vec![],
            interface: id(4),
            subject: Term::Primitive(Primitive::Bool),
            implementation: id(6),
            arguments: vec![],
            methods: vec![],
            associated: vec![],
            associated_initializers: vec![],
        });
    let mut omitted = Interpreter::for_comptime(&world, Budget::default()).unwrap();
    assert_eq!(
        omitted.invoke(id(1), vec![]).unwrap_err().kind,
        ErrorKind::UnmaterializedDispatch
    );
    world.dispatch_instances[0]
        .methods
        .push(nir::program::MethodInstance {
            method_arguments: vec![Term::Primitive(Primitive::Bool)],
            member: id(5),
            body: id(2),
            arguments: vec![Term::Primitive(Primitive::Bool)],
        });
    let mut vm = Interpreter::for_comptime(&world, Budget::default()).unwrap();
    assert!(matches!(
        vm.invoke(id(1), vec![]).unwrap(),
        Value::RuntimeTraitMethod(_)
    ));
    world.dispatch_instances[0].implementation = id(999);
    assert!(
        matches!(Interpreter::new(&world, Budget::default()), Err(error) if error.kind == ErrorKind::InvalidCallPlan)
    );
    world.dispatch_instances[0].implementation = id(6);
    world.implementations.push(implementation);
    assert!(
        matches!(Interpreter::new(&world, Budget::default()), Err(error) if error.kind == ErrorKind::InvalidCallPlan)
    );
    world.implementations.pop();
    assert!(Interpreter::for_comptime(&world, Budget::default()).is_ok());
}

#[test]
fn world_construction_reservation_retains_failure_and_reports_success_balance() {
    let mut invalid_phase = package();
    invalid_phase.runtime_function_identities[0].signature =
        Term::ComptimeFunction(Box::new(signature(Term::Primitive(Primitive::Bool))));
    let mut duplicate = package();
    duplicate.functions.push(duplicate.functions[0].clone());
    for invalid in [invalid_phase, duplicate] {
        let mut budget = Budget {
            steps: 321,
            depth: 128,
        };
        let expected = Interpreter::for_comptime_with_assumptions(&invalid, &[], budget)
            .err()
            .unwrap();
        let failure = Interpreter::for_comptime_in_budget(&invalid, &[], &mut budget)
            .err()
            .unwrap();
        assert_eq!(failure.kind, expected.kind);
        assert_eq!(failure.source, expected.source);
        assert_eq!(budget.steps, 0);
        assert_eq!(budget.depth, 128);
    }
    let world = package();
    let initial = Budget {
        steps: 321,
        depth: 128,
    };
    let expected = Interpreter::for_comptime(&world, initial)
        .unwrap()
        .remaining_steps();
    let mut budget = initial;
    let vm = Interpreter::for_comptime_in_budget(&world, &[], &mut budget).unwrap();
    assert_eq!(budget.steps, expected);
    assert_eq!(vm.remaining_steps(), expected);
    assert_eq!(budget.depth, initial.depth);
}

#[test]
fn rejected_world_premises_do_not_refund_reserved_allowance() {
    let world = Package::default();
    let assumptions = [nir::program::TraitRequirement {
        interface_arguments: vec![],
        subject: Term::Primitive(Primitive::Bool),
        interface: id(999),
    }];
    let mut budget = Budget {
        steps: 100,
        depth: 64,
    };
    assert!(Interpreter::for_comptime_in_budget(&world, &assumptions, &mut budget).is_err());
    assert_eq!(budget.steps, 0);
    // Removing the invalid premise does not silently restore the allowance.
    assert_eq!(
        Interpreter::for_comptime_in_budget(&world, &[], &mut budget)
            .err()
            .unwrap()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(budget.steps, 0);
    budget.steps = 100;
    let vm = Interpreter::for_comptime_in_budget(&world, &[], &mut budget).unwrap();
    assert_eq!(budget.steps, vm.remaining_steps());
    assert!(budget.steps < 100);
}

#[test]
fn trait_membership_phase_is_independent_of_function_execution_phase() {
    let mut package = trait_package();
    package.traits[0].methods[0].phase = nir::program::TraitPhase::Comptime;
    package.implementations[0].phases = nir::program::ImplementationPhases {
        runtime: false,
        comptime: true,
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(matches!(
        vm.invoke(id(1), vec![]).unwrap(),
        Value::RuntimeTraitMethod(_)
    ));
    assert!(
        package.traits[0].methods[0]
            .to_string()
            .contains("(phase comptime)")
    );
    assert!(
        package.implementations[0]
            .to_string()
            .contains("(phases (runtime false) (comptime true))")
    );
}

#[test]
fn unavailable_phase_cannot_supply_methods_or_defaults() {
    let mut package = trait_package();
    package.traits[0].methods[0].phase = nir::program::TraitPhase::Comptime;
    assert!(nir::phases::validate(&package).is_err());
    assert!(Interpreter::new(&package, Budget::default()).is_err());
    package.implementations[0].methods.clear();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap_err().kind,
        ErrorKind::UnmaterializedDispatch
    );
    package.traits[0].methods[0].default = Some(id(2));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap_err().kind,
        ErrorKind::UnmaterializedDispatch
    );
}

#[test]
fn available_phase_requires_all_its_methods_but_not_other_phase_methods() {
    let mut package = trait_package();
    let mut missing = package.traits[0].methods[0].clone();
    missing.declaration = id(7);
    missing.name = "other".into();
    missing.phase = nir::program::TraitPhase::Comptime;
    package.traits[0].methods.push(missing);
    assert!(Interpreter::for_comptime(&package, Budget::default()).is_ok());
    package.implementations[0].phases.comptime = true;
    assert!(Interpreter::for_comptime(&package, Budget::default()).is_err());
    package.implementations[0]
        .methods
        .push(("other".into(), id(2)));
    assert!(Interpreter::for_comptime(&package, Budget::default()).is_ok());
}

#[test]
fn empty_implementation_phase_set_is_invalid_in_nir_and_vm() {
    let mut package = trait_package();
    package.implementations[0].phases.runtime = false;
    assert!(nir::phases::validate(&package).is_err());
    assert!(Interpreter::new(&package, Budget::default()).is_err());
}

#[test]
fn associated_dispatch_requires_its_phase_and_retains_shared_runtime_values() {
    let mut package = trait_package();
    let value = Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Literal(nir::program::Literal::Bool(true)),
    };
    package.traits[0]
        .associated
        .push(nir::program::AssociatedContract {
            phase: nir::program::TraitPhase::Comptime,
            declaration: id(8),
            name: "flag".into(),
            ty: value.ty.clone(),
            default: None,
        });
    package.implementations[0]
        .associated
        .push((id(8), value.clone()));
    package
        .dispatch_instances
        .push(nir::program::DispatchInstance {
            interface: id(4),
            interface_arguments: vec![],
            subject: Term::Primitive(Primitive::Bool),
            implementation: id(6),
            arguments: vec![],
            methods: vec![],
            associated: vec![(id(8), value)],
            associated_initializers: vec![],
        });
    assert!(Interpreter::new(&package, Budget::default()).is_err());
    package.implementations[0].phases.comptime = true;
    assert!(Interpreter::new(&package, Budget::default()).is_ok());
    package.traits[0].associated[0].phase = nir::program::TraitPhase::Runtime;
    package.implementations[0].phases.runtime = false;
    package.implementations[0].methods.clear();
    assert!(Interpreter::new(&package, Budget::default()).is_ok());
    assert!(
        package.traits[0].associated[0]
            .to_string()
            .contains("(phase runtime)")
    );
}

#[test]
fn comptime_functions_execute_under_either_membership_phase() {
    for phase in [
        nir::program::TraitPhase::Runtime,
        nir::program::TraitPhase::Comptime,
    ] {
        let mut package = trait_package();
        package.runtime_function_identities.clear();
        let boolean = Term::Primitive(Primitive::Bool);
        let mut body = package.functions[0].clone();
        body.declaration = id(2);
        body.name = "make".into();
        body.signature = signature(boolean.clone());
        body.body = Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: boolean.clone(),
            kind: K::Literal(nir::program::Literal::Bool(true)),
        };
        package.functions.push(body);
        let callable = signature(boolean.clone()).with_comptime_phase(true);
        let member = &mut package.traits[0].methods[0];
        member.phase = phase;
        member.type_parameters.clear();
        member.signature = callable.clone().into();
        package.implementations[0].phases = nir::program::ImplementationPhases {
            runtime: phase == nir::program::TraitPhase::Runtime,
            comptime: phase == nir::program::TraitPhase::Comptime,
        };
        package.functions[0].signature = signature(boolean.clone());
        package.functions[0].body = Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: boolean,
            kind: K::Call {
                callee: Box::new(Expr {
                    source: SourceRange { start: 1, end: 2 },
                    ty: callable,
                    kind: K::TraitMethod {
                        interface: id(4),
                        interface_arguments: vec![],
                        subject: Term::Primitive(Primitive::Bool),
                        member: id(5),
                        method_arguments: vec![],
                    },
                }),
                arguments: vec![],
                parameter_bindings: vec![],
            },
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(vm.invoke(id(1), vec![]).unwrap(), Value::Bool(true));
        package.functions[1].phase = FunctionPhase::Runtime;
        assert!(Interpreter::for_comptime(&package, Budget::default()).is_err());
    }
}

fn dynamic_trait_package() -> Package {
    let mut package = trait_package();
    package.runtime_function_identities.clear();
    let boolean = Term::Primitive(Primitive::Bool);
    let callable = signature(boolean.clone()).with_comptime_phase(true);
    let source = SourceRange { start: 50, end: 60 };
    let binding = nir::program::LocalId { owner: 1, index: 0 };
    let mut body = package.functions[0].clone();
    body.declaration = id(2);
    body.name = "make".into();
    body.type_parameters = vec![id(3)];
    body.signature = signature(boolean.clone());
    body.body = Expr {
        source,
        ty: boolean.clone(),
        kind: K::Literal(nir::program::Literal::Bool(true)),
    };
    package.functions.push(body);
    package.traits[0].methods[0].signature = callable.clone().into();
    package.traits[0].methods[0].phase = nir::program::TraitPhase::Comptime;
    package.implementations[0].phases = nir::program::ImplementationPhases {
        runtime: false,
        comptime: true,
    };
    package.functions[0].signature = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Runtime,
            ty: Term::Primitive(Primitive::Type),
        }],
        result: Box::new(boolean.clone()),
    };
    package.functions[0].parameters = vec![binding];
    let packet = Expr {
        source,
        ty: Term::Tuple(vec![Term::Primitive(Primitive::Type); 2]),
        kind: K::Tuple(vec![
            Expr {
                source,
                ty: Term::Primitive(Primitive::Type),
                kind: K::TypeValue(boolean.clone()),
            },
            Expr {
                source,
                ty: Term::Primitive(Primitive::Type),
                kind: K::Local(binding),
            },
        ]),
    };
    package.functions[0].body = Expr {
        source,
        ty: boolean,
        kind: K::Call {
            callee: Box::new(Expr {
                source,
                ty: callable,
                kind: K::ComptimeFunctionInstance {
                    declaration: id(5),
                    subject: Box::new(packet),
                },
            }),
            arguments: vec![],
            parameter_bindings: vec![],
        },
    };
    package
}

#[test]
fn dynamic_trait_clauses_select_the_originating_contract_and_instance() {
    let package = dynamic_trait_package();
    assert!(format!("{package}").contains("comptime-function-instance"));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for ty in [Primitive::Bool, Primitive::U8] {
        assert_eq!(
            vm.invoke(id(1), vec![Value::Type(Term::Primitive(ty))])
                .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn dynamic_trait_clauses_do_not_erase_interface_program_checks() {
    for postcondition in [false, true] {
        let mut package = dynamic_trait_package();
        let false_value = Expr {
            source: SourceRange { start: 70, end: 80 },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(nir::program::Literal::Bool(false)),
        };
        let program = &mut package.traits[0].methods[0].program;
        if postcondition {
            program.postconditions.push(false_value);
        } else {
            program.preconditions.push(false_value);
        }
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert!(
            vm.invoke(id(1), vec![Value::Type(Term::Primitive(Primitive::Bool))])
                .is_err()
        );
    }
}

#[test]
fn dynamic_trait_clauses_reject_contract_erasure_and_missing_phases() {
    let mut package = dynamic_trait_package();
    package.traits[0].methods[0]
        .program
        .preconditions
        .push(Expr {
            source: SourceRange { start: 70, end: 80 },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(nir::program::Literal::Bool(true)),
        });
    let K::Call { callee, .. } = package.functions[0].body.kind.clone() else {
        unreachable!()
    };
    package.functions[0].body = *callee;
    let result = package.functions[0].body.ty.clone();
    if let Term::Function { result: output, .. } = &mut package.functions[0].signature {
        *output = Box::new(result);
    }
    assert!(Interpreter::for_comptime(&package, Budget::default()).is_err());
    let mut package = dynamic_trait_package();
    package.implementations[0].phases = nir::program::ImplementationPhases::default();
    package.implementations[0].methods.clear();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(
        vm.invoke(id(1), vec![Value::Type(Term::Primitive(Primitive::Bool))])
            .is_err()
    );
}

#[test]
fn dynamic_trait_clause_failure_does_not_poison_later_selection() {
    let mut package = dynamic_trait_package();
    package.traits[0].methods[0]
        .requirements
        .push(nir::program::TypePredicate::Equal(
            Term::Rigid {
                declaration: id(3),
                sort: Sort::Type,
            },
            Term::Primitive(Primitive::Bool),
        ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(
        vm.invoke(id(1), vec![Value::Type(Term::Primitive(Primitive::U8))])
            .is_err()
    );
    assert_eq!(
        vm.invoke(id(1), vec![Value::Type(Term::Primitive(Primitive::Bool))])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn dynamic_trait_clause_packet_requires_every_declared_input() {
    let mut package = dynamic_trait_package();
    let K::Call { callee, .. } = &mut package.functions[0].body.kind else {
        unreachable!()
    };
    let K::ComptimeFunctionInstance { subject, .. } = &mut callee.kind else {
        unreachable!()
    };
    let Term::Tuple(domains) = &mut subject.ty else {
        unreachable!()
    };
    domains.pop();
    let K::Tuple(values) = &mut subject.kind else {
        unreachable!()
    };
    values.pop();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(1), vec![Value::Type(Term::Primitive(Primitive::Bool))])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
}
