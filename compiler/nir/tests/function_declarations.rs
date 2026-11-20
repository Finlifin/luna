//! A callable declaration can be observed and substituted without any body.
use nir::program::{
    EntrySnapshot, Expr, ExprKind, FunctionDeclaration, FunctionPhase, LocalId, SourceRange,
    TraitRequirement, TypePredicate,
};
use nir::{DeclarationId, DeclarationParameter, Parameter, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 19, local }
}
fn binding(index: u32) -> LocalId {
    LocalId { owner: 1, index }
}
fn expression(ty: Term, kind: ExprKind) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 10, end: 20 },
    }
}
fn generic() -> Term {
    Term::Rigid {
        declaration: id(2),
        sort: Sort::Type,
    }
}
fn declaration() -> FunctionDeclaration {
    let type_value = expression(Term::Primitive(P::Type), ExprKind::TypeValue(generic()));
    let condition = expression(
        Term::Primitive(P::Bool),
        ExprKind::TypeEqual(Box::new(type_value.clone()), Box::new(type_value)),
    );
    FunctionDeclaration {
        source: nir::program::SourceRange { start: 0, end: 0 },
        declaration: id(1),
        name: "quoted\"name\nnext".into(),
        phase: FunctionPhase::Comptime,
        inline: true,
        pure: true,
        is_unsafe: true,
        preconditions: vec![condition.clone()],
        postconditions: vec![condition],
        result_binding: Some(binding(1)),
        result_type_binder: None,
        result_guarantees: vec![],
        entry_snapshots: vec![EntrySnapshot {
            parameter: binding(0),
            binding: binding(2),
        }],
        capabilities: vec![TraitRequirement {
            interface: id(3),
            interface_arguments: vec![generic()],
            subject: generic(),
        }],
        requirements: vec![TypePredicate::Equal(generic(), generic())],
        type_parameters: vec![id(2)],
        signature: Term::Function {
            parameters: vec![Parameter {
                sort: Sort::ComptimeValue,
                ty: generic(),
            }],
            result: Box::new(generic()),
        },
        parameters: vec![binding(0)],
        env_parameters: vec![],
        defaults: vec![(0, expression(generic(), ExprKind::Local(binding(3))))],
    }
}

#[test]
fn body_free_substitution_keeps_contracts_defaults_bindings_and_responsibility() {
    let original = declaration();
    let binders = [DeclarationParameter {
        declaration: id(2),
        sort: Sort::Type,
    }];
    assert!(original.instantiate_parameters(&binders, &[]).is_err());
    let concrete = Term::Primitive(P::Bool);
    let instantiated = original
        .instantiate_parameters(&binders, &[concrete.clone()])
        .unwrap();
    let callable = instantiated.callable_type();
    assert!(callable.is_comptime_callable() && callable.is_unsafe_callable());
    assert_eq!(
        callable.callable_signature(),
        &Term::Function {
            parameters: vec![Parameter {
                sort: Sort::ComptimeValue,
                ty: concrete.clone()
            }],
            result: Box::new(concrete.clone()),
        }
    );
    for condition in instantiated
        .preconditions
        .iter()
        .chain(&instantiated.postconditions)
    {
        let ExprKind::TypeEqual(left, right) = &condition.kind else {
            panic!("missing contract")
        };
        for operand in [left, right] {
            assert_eq!(operand.kind, ExprKind::TypeValue(concrete.clone()));
        }
    }
    assert_eq!(instantiated.defaults[0].1.ty, concrete);
    assert_eq!(instantiated.defaults[0].1.kind, ExprKind::Local(binding(3)));
    assert_eq!(
        instantiated.requirements,
        vec![TypePredicate::Equal(concrete.clone(), concrete.clone())]
    );
    assert_eq!(instantiated.capabilities[0].subject, concrete);
    assert_eq!(
        instantiated.capabilities[0].interface_arguments,
        vec![concrete]
    );
    assert_eq!(instantiated.parameters, original.parameters);
    assert_eq!(instantiated.entry_snapshots, original.entry_snapshots);
    assert_eq!(instantiated.result_binding, original.result_binding);
    assert_eq!(instantiated.type_parameters, original.type_parameters);
    assert_eq!(
        original,
        declaration(),
        "failed or successful substitution mutated its source"
    );
}

#[test]
fn declaration_display_preserves_contracts_and_escapes_names_without_a_body() {
    let header = declaration();
    let dump = header.to_string();
    assert!(dump.starts_with("(function-declaration "), "{dump}");
    assert!(dump.contains("(name \"quoted\\\"name\\nnext\")"), "{dump}");
    for field in [
        "(program-requires ",
        "(program-ensures ",
        "(defaults ",
        "(entry-snapshots ",
        "(capabilities ",
    ] {
        assert!(dump.contains(field), "{field}: {dump}");
    }
    assert!(!dump.contains("(body "), "{dump}");
    assert!(!dump.contains("(env-parameters "), "{dump}");
    assert_eq!(dump, header.to_string());
}

#[test]
fn env_parameters_and_call_env_arguments_substitute_and_display() {
    use nir::program::RuntimeEnvParameter;
    let mut header = declaration();
    header.phase = FunctionPhase::Runtime;
    header.env_parameters.push(RuntimeEnvParameter {
        declaration: id(4),
        runtime_id: None,
        binding: binding(4),
        ty: Term::Pointer(Box::new(generic())),
    });
    let dump = header.to_string();
    assert!(dump.contains("(env-parameters (env-param "), "{dump}");

    let concrete = Term::Primitive(P::U32);
    let instantiated = header
        .instantiate_parameters(
            &[DeclarationParameter {
                declaration: id(2),
                sort: Sort::Type,
            }],
            std::slice::from_ref(&concrete),
        )
        .unwrap();
    assert_eq!(
        instantiated.env_parameters,
        vec![RuntimeEnvParameter {
            declaration: id(4),
            runtime_id: None,
            binding: binding(4),
            ty: Term::Pointer(Box::new(concrete.clone())),
        }]
    );

    let rt_id = nir::RuntimeId {
        instance: 2,
        local: 9,
    };
    let with_runtime_id = header
        .instantiate_parameters(
            &[
                DeclarationParameter {
                    declaration: id(2),
                    sort: Sort::Type,
                },
                DeclarationParameter {
                    declaration: id(4),
                    sort: Sort::Runtime,
                },
            ],
            &[concrete.clone(), Term::Runtime(rt_id)],
        )
        .unwrap();
    assert_eq!(
        with_runtime_id.env_parameters,
        vec![RuntimeEnvParameter {
            declaration: id(4),
            runtime_id: Some(rt_id),
            binding: binding(4),
            ty: Term::Pointer(Box::new(concrete.clone())),
        }]
    );
    assert!(
        with_runtime_id
            .to_string()
            .contains("(runtime-id (runtime (instance 2) (local 9)))"),
        "{with_runtime_id}"
    );

    let env_arg = expression(
        Term::Pointer(Box::new(concrete.clone())),
        ExprKind::Local(binding(5)),
    );
    let empty_call = expression(
        concrete.clone(),
        ExprKind::Call {
            callee: Box::new(expression(
                Term::Function {
                    parameters: vec![],
                    result: Box::new(concrete.clone()),
                },
                ExprKind::Function(id(10)),
            )),
            arguments: vec![],
            parameter_bindings: vec![],
            env_arguments: vec![],
        },
    );
    assert!(
        !empty_call.to_string().contains("(env-arguments "),
        "{empty_call}"
    );
    let with_env_call = expression(
        concrete.clone(),
        ExprKind::Call {
            callee: Box::new(expression(
                Term::Function {
                    parameters: vec![],
                    result: Box::new(concrete),
                },
                ExprKind::Function(id(10)),
            )),
            arguments: vec![],
            parameter_bindings: vec![],
            env_arguments: vec![env_arg],
        },
    );
    assert!(
        with_env_call.to_string().contains("(env-arguments "),
        "{with_env_call}"
    );
}

#[test]
fn implicit_runtime_env_parameters_filter_globals_existing_and_unregistered_deterministically() {
    let rt_global = nir::RuntimeId {
        instance: 0,
        local: 1,
    };
    let rt_explicit = nir::RuntimeId {
        instance: 0,
        local: 2,
    };
    let rt_first = nir::RuntimeId {
        instance: 0,
        local: 5,
    };
    let rt_second = nir::RuntimeId {
        instance: 0,
        local: 3,
    };
    let rt_unregistered = nir::RuntimeId {
        instance: 0,
        local: 9,
    };

    let alloc_a = Term::Primitive(P::U32);
    let alloc_b = Term::Primitive(P::U64);
    let ptr_a = Term::Pointer(Box::new(alloc_a));
    let ptr_b = Term::Pointer(Box::new(alloc_b));

    let runtime_domains = std::collections::BTreeMap::from([
        (rt_global, Term::Lift(Box::new(ptr_a.clone()))),
        (rt_explicit, Term::Lift(Box::new(ptr_a.clone()))),
        (rt_first, Term::Lift(Box::new(ptr_a.clone()))),
        (rt_second, Term::Lift(Box::new(ptr_b.clone()))),
    ]);
    let global_runtime_deliveries =
        std::collections::BTreeMap::from([(rt_global, expression(ptr_a.clone(), ExprKind::Unit))]);
    let existing = vec![nir::program::RuntimeEnvParameter {
        declaration: id(4),
        runtime_id: Some(rt_explicit),
        binding: binding(4),
        ty: ptr_a.clone(),
    }];
    let nested_arg = Term::Nominal {
        declaration: id(20),
        arguments: vec![
            Term::Runtime(rt_first),
            Term::Nominal {
                declaration: id(21),
                arguments: vec![
                    Term::Runtime(rt_global),
                    Term::Runtime(rt_explicit),
                    Term::Runtime(rt_second),
                    Term::Runtime(rt_unregistered),
                ],
            },
        ],
    };

    let implicit = nir::program::implicit_runtime_env_parameters(
        &runtime_domains,
        &global_runtime_deliveries,
        &[nested_arg],
        &existing,
    )
    .unwrap();

    assert_eq!(
        implicit,
        vec![
            nir::program::RuntimeEnvParameter {
                declaration: DeclarationId {
                    package: nir::program::IMPLICIT_RUNTIME_ENV_PACKAGE,
                    local: 3,
                },
                runtime_id: Some(rt_second),
                binding: LocalId {
                    owner: nir::program::IMPLICIT_RUNTIME_ENV_OWNER,
                    index: 3,
                },
                ty: ptr_b,
            },
            nir::program::RuntimeEnvParameter {
                declaration: DeclarationId {
                    package: nir::program::IMPLICIT_RUNTIME_ENV_PACKAGE,
                    local: 5,
                },
                runtime_id: Some(rt_first),
                binding: LocalId {
                    owner: nir::program::IMPLICIT_RUNTIME_ENV_OWNER,
                    index: 5,
                },
                ty: ptr_a,
            },
        ]
    );
}
