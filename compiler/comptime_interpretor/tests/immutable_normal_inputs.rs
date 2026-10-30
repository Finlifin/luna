use comptime_interpretor::{Budget, ErrorKind, Event, Interpreter, Value};
use nir::program::{
    Expr, ExprKind as K, Function, FunctionPhase, Literal, LocalId, Package, SourceRange,
};
use nir::{DeclarationId, Primitive, Term};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 5, local }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn function(local: u32, parameters: Vec<LocalId>, body: Expr) -> Function {
    Function {
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
        declaration: id(local),
        name: format!("f{local}"),
        signature: Term::Function {
            parameters: parameters
                .iter()
                .map(|_| nir::Parameter {
                    sort: nir::Sort::Runtime,
                    ty: body.ty.clone(),
                })
                .collect(),
            result: Box::new(body.ty.clone()),
        },
        parameters,
        body,
        defaults: vec![],
    }
}
fn callable() -> Term {
    Term::Function {
        parameters: vec![],
        result: Box::new(boolean()),
    }
    .with_comptime_phase(true)
}
fn world() -> Package {
    let source = SourceRange { start: 1, end: 2 };
    let binding = LocalId { owner: 2, index: 0 };
    Package {
        functions: vec![
            function(
                1,
                vec![],
                Expr {
                    source,
                    ty: callable(),
                    kind: K::Function(id(3)),
                },
            ),
            function(
                2,
                vec![binding],
                Expr {
                    source,
                    ty: callable(),
                    kind: K::Local(binding),
                },
            ),
            function(
                3,
                vec![],
                Expr {
                    source,
                    ty: boolean(),
                    kind: K::Literal(Literal::Bool(true)),
                },
            ),
        ],
        ..Default::default()
    }
}
#[test]
fn immutable_decode_never_runs_a_factory_even_when_nested() {
    let world = world();
    let mut vm = Interpreter::for_comptime(&world, Budget::default()).unwrap();
    let snapshot = Term::ClosureSnapshot {
        factory: id(1),
        arguments: vec![],
    };
    let optional = Term::Optional(Box::new(callable()));
    for (term, ty) in [
        (snapshot.clone(), callable()),
        (
            Term::FunctionValue {
                declaration: id(3),
                arguments: vec![],
            },
            callable(),
        ),
        (
            nir::objects::value(vec![nir::objects::entry(
                Some("callback"),
                callable(),
                snapshot.clone(),
            )]),
            Term::Primitive(Primitive::Object),
        ),
        (
            Term::TupleValue(vec![snapshot.clone()]),
            Term::Tuple(vec![callable()]),
        ),
        (
            Term::OptionalValue {
                ty: Box::new(optional.clone()),
                value: Some(Box::new(snapshot.clone())),
            },
            optional,
        ),
    ] {
        assert_eq!(
            vm.decode_immutable_input(&term, &ty).unwrap_err().kind,
            ErrorKind::Type
        );
        assert!(vm.events().is_empty());
    }
    // The ordinary invocation transport still reconstructs explicit snapshots.
    let value = vm
        .invoke_with_normal_arguments(id(2), vec![(callable(), snapshot)])
        .unwrap();
    assert!(matches!(value, Value::Function(_) | Value::Closure(_)));
    assert!(
        vm.events()
            .iter()
            .any(|event| matches!(event,Event::EnterFunction(declaration) if *declaration==id(1)))
    );
}
#[test]
fn immutable_decode_checks_scalar_domain_and_recovers_after_rejection() {
    let world = Package::default();
    let mut vm = Interpreter::for_comptime(&world, Budget::default()).unwrap();
    let int = |value| Term::Integer(comptime_runtime::Integer::from_u128(value));
    assert!(
        vm.decode_immutable_input(&int(256), &Term::Primitive(Primitive::U8))
            .is_err()
    );
    assert!(
        vm.decode_immutable_input(&Term::Symbol("wrong".into()), &boolean())
            .is_err()
    );
    assert_eq!(
        vm.decode_immutable_input(&Term::Bool(true), &boolean())
            .unwrap(),
        Value::Bool(true)
    );
    assert!(vm.events().is_empty());
}
#[test]
fn immutable_decode_requires_comptime_and_consumes_budget() {
    let world = Package::default();
    let mut runtime = Interpreter::new(&world, Budget::default()).unwrap();
    assert_eq!(
        runtime
            .decode_immutable_input(&Term::Bool(true), &boolean())
            .unwrap_err()
            .kind,
        ErrorKind::PhaseMismatch
    );
    let mut vm = Interpreter::for_comptime(&world, Budget::default()).unwrap();
    let before = vm.remaining_steps();
    vm.decode_immutable_input(&Term::Bool(true), &boolean())
        .unwrap();
    assert!(vm.remaining_steps() < before);
    let mut vm = Interpreter::for_comptime(&world, Budget::default()).unwrap();
    vm.charge_preparation(vm.remaining_steps() as usize)
        .unwrap();
    assert_eq!(
        vm.decode_immutable_input(&Term::Bool(true), &boolean())
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
}

#[test]
fn metadata_lookup_evaluates_selector_once_and_admits_only_selected_input() {
    let source = SourceRange { start: 40, end: 50 };
    let string = Term::Primitive(Primitive::Str);
    let selector = function(
        10,
        vec![],
        Expr {
            source,
            ty: string.clone(),
            kind: K::Literal(Literal::String("dep".into())),
        },
    );
    let lookup = function(
        11,
        vec![],
        Expr {
            source,
            ty: boolean(),
            kind: K::BuildMetadata {
                owner: 5,
                selector: Box::new(Expr {
                    source,
                    ty: string.clone(),
                    kind: K::Call {
                        parameter_bindings: vec![],
                        arguments: vec![],
                        callee: Box::new(Expr {
                            source,
                            ty: Term::Function {
                                parameters: vec![],
                                result: Box::new(string),
                            }
                            .with_comptime_phase(true),
                            kind: K::Function(id(10)),
                        }),
                    },
                }),
                entries: vec![
                    (
                        "unused".into(),
                        8,
                        Some(Term::ClosureSnapshot {
                            factory: id(1),
                            arguments: vec![],
                        }),
                    ),
                    ("dep".into(), 9, Some(Term::Bool(true))),
                ],
            },
        },
    );
    let mut package = world();
    package.functions.extend([selector, lookup]);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(11), vec![]).unwrap(), Value::Bool(true));
    assert_eq!(
        vm.events()
            .iter()
            .filter(
                |event| matches!(event, Event::EnterFunction(declaration) if *declaration == id(10))
            )
            .count(),
        1
    );
    assert!(
        !vm.events().iter().any(
            |event| matches!(event, Event::EnterFunction(declaration) if *declaration == id(1))
        )
    );
}

#[test]
fn metadata_lookup_distinguishes_missing_alias_input_and_bad_value() {
    let source = SourceRange { start: 40, end: 50 };
    let mut package = Package::default();
    for (index, alias) in ["unknown", "missing", "invalid", "valid"]
        .into_iter()
        .enumerate()
    {
        package.functions.push(function(
            20 + index as u32,
            vec![],
            Expr {
                source,
                ty: boolean(),
                kind: K::BuildMetadata {
                    owner: 5,
                    selector: Box::new(Expr {
                        source,
                        ty: Term::Primitive(Primitive::Str),
                        kind: K::Literal(Literal::String(alias.into())),
                    }),
                    entries: vec![
                        ("missing".into(), 8, None),
                        ("invalid".into(), 9, Some(Term::Symbol("wrong".into()))),
                        ("valid".into(), 10, Some(Term::Bool(false))),
                    ],
                },
            },
        ));
    }
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for (index, provider, alias) in [(20, None, "unknown"), (21, Some(8), "missing")] {
        let error = vm.invoke(id(index), vec![]).unwrap_err();
        assert_eq!(error.source, Some(source));
        assert_eq!(
            error.kind,
            ErrorKind::BuildMetadataUnavailable {
                owner: 5,
                provider,
                alias: alias.into()
            }
        );
    }
    assert_eq!(vm.invoke(id(22), vec![]).unwrap_err().kind, ErrorKind::Type);
    assert_eq!(vm.invoke(id(23), vec![]).unwrap(), Value::Bool(false));
}

#[test]
fn metadata_ir_checks_phase_aliases_and_escaped_dump() {
    let source = SourceRange { start: 40, end: 50 };
    let mut package = Package {
        functions: vec![function(
            30,
            vec![],
            Expr {
                source,
                ty: boolean(),
                kind: K::BuildMetadata {
                    owner: 5,
                    selector: Box::new(Expr {
                        source,
                        ty: Term::Primitive(Primitive::Str),
                        kind: K::Literal(Literal::String("a\\b".into())),
                    }),
                    entries: vec![("a\\b".into(), 9, Some(Term::Bool(true)))],
                },
            },
        )],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    let dump = package.functions[0].body.to_string();
    assert!(dump.contains("(build-metadata (owner 5)"));
    assert!(dump.contains(r#"(alias "a\\b")"#));
    package.functions[0].phase = FunctionPhase::Runtime;
    assert!(nir::phases::validate(&package).is_err());
    package.functions[0].phase = FunctionPhase::Comptime;
    let K::BuildMetadata { entries, .. } = &mut package.functions[0].body.kind else {
        unreachable!()
    };
    entries.push(entries[0].clone());
    assert!(nir::phases::validate(&package).is_err());
}

fn callable_normal_form_world() -> Package {
    let mut package = world();
    package.normal_forms.push(nir::normal_forms::Head {
        declaration: id(20),
        collector: None,
        role: nir::normal_forms::Role::Opaque,
        parameters: vec![nir::normal_forms::Parameter {
            declaration: id(21),
            binding: LocalId {
                owner: 20,
                index: 0,
            },
            kind: nir::normal_forms::Kind::Value,
            ty: callable(),
        }],
        requirements: vec![],
    });
    package
}
fn callable_normal_form(declaration: DeclarationId, arguments: Vec<Term>) -> Term {
    Term::NormalForm {
        head: id(20),
        sort: nir::Sort::Type,
        arguments: vec![Term::FunctionValue {
            declaration,
            arguments,
        }],
    }
}

#[test]
fn frozen_callable_type_inputs_need_declaration_signature_evidence_without_execution() {
    let package = callable_normal_form_world();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let ty = Term::Primitive(Primitive::Type);
    let valid = callable_normal_form(id(3), vec![]);
    assert_eq!(
        vm.decode_immutable_input(&valid, &ty).unwrap(),
        Value::Type(valid.clone())
    );
    for invalid in [
        callable_normal_form(id(99), vec![]),
        callable_normal_form(id(1), vec![]),
        callable_normal_form(id(2), vec![]),
        callable_normal_form(id(3), vec![boolean()]),
    ] {
        assert!(
            vm.decode_immutable_input(&invalid, &ty).is_err(),
            "{invalid}"
        );
        assert!(vm.decode_immutable_input(&valid, &ty).is_ok());
    }
    assert!(vm.events().is_empty());
}

#[test]
fn frozen_callable_type_inputs_check_contracts_phase_and_safety() {
    for (phase, is_unsafe, requirement, expected) in [
        (FunctionPhase::Runtime, false, true, ErrorKind::Type),
        (FunctionPhase::Comptime, true, true, ErrorKind::Type),
        (
            FunctionPhase::Comptime,
            false,
            false,
            ErrorKind::TypeContractRejected,
        ),
    ] {
        let mut package = callable_normal_form_world();
        package.functions.drain(0..2);
        package.functions[0].phase = phase;
        package.functions[0].is_unsafe = is_unsafe;
        package.functions[0].requirements = vec![nir::program::TypePredicate::Bool(requirement)];
        if phase == FunctionPhase::Runtime {
            let runtime = package.functions.remove(0);
            package
                .runtime_function_identities
                .push(nir::program::RuntimeFunctionIdentity {
                    declaration: runtime.declaration,
                    type_parameters: runtime.type_parameters,
                    signature: runtime.signature,
                });
        }
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.decode_immutable_input(
                &callable_normal_form(id(3), vec![]),
                &Term::Primitive(Primitive::Type),
            )
            .unwrap_err()
            .kind,
            expected
        );
        assert!(vm.events().is_empty());
    }
}

#[test]
fn named_comptime_identity_resolves_in_receiving_vm_and_rechecks_contracts() {
    let mut package = world();
    let source = SourceRange { start: 1, end: 2 };
    let callback = LocalId { owner: 4, index: 0 };
    let mut consumer = function(
        4,
        vec![callback],
        Expr {
            source,
            ty: boolean(),
            kind: K::Call {
                callee: Box::new(Expr {
                    source,
                    ty: callable(),
                    kind: K::Local(callback),
                }),
                arguments: vec![],
                parameter_bindings: vec![],
            },
        },
    );
    consumer.signature = Term::Function {
        parameters: vec![nir::Parameter {
            sort: nir::Sort::Runtime,
            ty: callable(),
        }],
        result: Box::new(boolean()),
    };
    package.functions.push(consumer);
    package.functions[2].type_parameters = vec![id(30)];
    let descriptor = || Value::FunctionInstance {
        declaration: id(3),
        arguments: vec![boolean()],
    };
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        tree.invoke(id(4), vec![descriptor()]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(tree.bytecode_invocations(), 0);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(4), vec![descriptor()]).unwrap(),
        Value::Bool(true)
    );
    assert!(vm.bytecode_invocations() > 0);
    assert!(
        vm.events()
            .iter()
            .any(|event| matches!(event, Event::EnterFunction(d) if *d == id(3)))
    );
    for arguments in [vec![], vec![Term::Bool(true)]] {
        assert!(
            vm.invoke(
                id(4),
                vec![Value::FunctionInstance {
                    declaration: id(3),
                    arguments
                }]
            )
            .is_err()
        );
    }
    assert_eq!(
        vm.invoke(id(4), vec![descriptor()]).unwrap(),
        Value::Bool(true)
    );
    drop(vm);

    // The same transported identity resolves against the new world, not a
    // specialized body retained from its producer or a previous VM.
    package.functions[2].body.kind = K::Literal(Literal::Bool(false));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(4), vec![descriptor()]).unwrap(),
        Value::Bool(false)
    );
    drop(vm);
    package.functions[2]
        .requirements
        .push(nir::program::TypePredicate::Equal(
            boolean(),
            Term::Primitive(Primitive::Char),
        ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(vm.invoke(id(4), vec![descriptor()]).is_err());
    assert!(
        !vm.events()
            .iter()
            .any(|event| matches!(event, Event::EnterFunction(d) if *d == id(3)))
    );
}
