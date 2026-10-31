use comptime_interpretor::{Budget, ErrorKind, Event, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Literal, Package, SourceRange};
use nir::{DeclarationId, Primitive as P, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn boolean() -> Term {
    Term::Primitive(P::Bool)
}
fn optional(inner: Term) -> Term {
    Term::Optional(Box::new(inner))
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 1, end: 2 },
    }
}
fn absent(payload: Term) -> Expr {
    expr(optional(payload), K::OptionalAbsent)
}
fn present(value: Expr) -> Expr {
    expr(
        optional(value.ty.clone()),
        K::OptionalPresent(Box::new(value)),
    )
}
fn function(local: u32, body: Expr) -> Function {
    Function {
        declaration: id(local),
        name: format!("optional{local}"),
        phase: FunctionPhase::Comptime,
        inline: false,
        pure: false,
        is_unsafe: false,
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(body.ty.clone()),
        },
        parameters: vec![],
        defaults: vec![],
        body,
    }
}

#[test]
fn nested_optional_values_preserve_three_distinct_states_without_enum_declarations() {
    let inner_ty = optional(boolean());
    let ty = optional(inner_ty.clone());
    let package = Package {
        functions: vec![
            function(0, absent(inner_ty.clone())),
            function(1, present(absent(boolean()))),
            function(
                2,
                present(present(expr(boolean(), K::Literal(Literal::Bool(true))))),
            ),
        ],
        ..Default::default()
    };
    assert!(package.enums.is_empty());
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let absent = vm.invoke(id(0), vec![]).unwrap();
    let clear = vm.invoke(id(1), vec![]).unwrap();
    let set = vm.invoke(id(2), vec![]).unwrap();
    assert_eq!(
        absent,
        Value::Optional {
            ty: ty.clone(),
            value: None
        }
    );
    assert_eq!(
        clear,
        Value::Optional {
            ty: ty.clone(),
            value: Some(Box::new(Value::Optional {
                ty: inner_ty,
                value: None,
            }))
        }
    );
    assert_ne!(clear, set);
    assert_ne!(absent, clear);
    for value in [&absent, &clear, &set] {
        vm.validate_immutable_input(value, &ty).unwrap();
        assert!(
            value
                .to_string()
                .starts_with("(optional-value (type (optional")
        );
    }
}

#[test]
fn optional_injection_invokes_payload_once_and_host_admission_rejects_forged_values() {
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(boolean()),
    }
    .with_comptime_phase(true);
    let call = expr(
        boolean(),
        K::Call {
            parameter_bindings: vec![],
            callee: Box::new(expr(signature, K::Function(id(1)))),
            arguments: vec![],
        },
    );
    let package = Package {
        functions: vec![
            function(0, present(call)),
            function(1, expr(boolean(), K::Literal(Literal::Bool(true)))),
        ],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let actual = vm.invoke(id(0), vec![]).unwrap();
    let entries: Vec<_> = vm
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(entries, vec![id(0), id(1)]);
    let ty = optional(boolean());
    for forged in [
        Value::Optional {
            ty: boolean(),
            value: None,
        },
        Value::Optional {
            ty: ty.clone(),
            value: Some(Box::new(Value::Char('x'))),
        },
        Value::Optional {
            ty: optional(ty.clone()),
            value: None,
        },
    ] {
        assert_eq!(
            vm.validate_immutable_input(&forged, &ty).unwrap_err().kind,
            ErrorKind::Type
        );
        vm.validate_immutable_input(&actual, &ty).unwrap();
    }
}

#[test]
fn invalid_optional_construction_rejects_before_evaluating_payload() {
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(boolean()),
    }
    .with_comptime_phase(true);
    let call = expr(
        boolean(),
        K::Call {
            parameter_bindings: vec![],
            callee: Box::new(expr(signature, K::Function(id(1)))),
            arguments: vec![],
        },
    );
    let package = Package {
        functions: vec![
            function(
                0,
                expr(
                    optional(Term::Primitive(P::Char)),
                    K::OptionalPresent(Box::new(call)),
                ),
            ),
            function(1, expr(boolean(), K::Literal(Literal::Bool(true)))),
            function(2, expr(boolean(), K::OptionalAbsent)),
        ],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(0), vec![]).unwrap_err().kind, ErrorKind::Type);
    assert!(!vm.events().contains(&Event::EnterFunction(id(1))));
    assert_eq!(vm.invoke(id(2), vec![]).unwrap_err().kind, ErrorKind::Type);
    assert_eq!(vm.invoke(id(1), vec![]).unwrap(), Value::Bool(true));
}

#[test]
fn optional_copy_preflight_counts_payloads_and_rejected_deep_values_drop_iteratively() {
    let absent = Value::Optional {
        ty: optional(boolean()),
        value: None,
    };
    let present = Value::Optional {
        ty: optional(boolean()),
        value: Some(Box::new(Value::Bool(true))),
    };
    assert!(present.immutable_copy_work().unwrap() > absent.immutable_copy_work().unwrap());
    let mut deep = Value::Bool(true);
    for _ in 0..10_000 {
        deep = Value::Optional {
            ty: optional(boolean()),
            value: Some(Box::new(deep)),
        };
    }
    assert!(deep.immutable_copy_work().is_none());
    // Host representation is untrusted. Rejecting it must not recurse through
    // its unbounded tail while releasing the supplied input.
    assert!(
        deep.into_normal_term(&mut comptime_runtime::Budget {
            steps: 100,
            ..Default::default()
        })
        .is_err()
    );
}

#[test]
fn optional_freezing_round_trips_nested_states_through_comptime_value_inputs() {
    let ty = optional(optional(boolean()));
    let inner = optional(boolean());
    let values = vec![
        Value::Optional {
            ty: ty.clone(),
            value: None,
        },
        Value::Optional {
            ty: ty.clone(),
            value: Some(Box::new(Value::Optional {
                ty: inner.clone(),
                value: None,
            })),
        },
        Value::Optional {
            ty: ty.clone(),
            value: Some(Box::new(Value::Optional {
                ty: inner,
                value: Some(Box::new(Value::Bool(true))),
            })),
        },
    ];
    let frozen: Vec<_> = values
        .iter()
        .map(|value| {
            value
                .clone()
                .into_normal_term(&mut Default::default())
                .unwrap()
        })
        .collect();
    assert_ne!(frozen[0], frozen[1]);
    assert_ne!(frozen[1], frozen[2]);
    let package = Package {
        functions: frozen
            .iter()
            .enumerate()
            .map(|(i, value)| function(i as u32, expr(ty.clone(), K::ComptimeValue(value.clone()))))
            .collect(),
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for (i, expected) in values.into_iter().enumerate() {
        assert_eq!(vm.invoke(id(i as u32), vec![]).unwrap(), expected);
    }
}

#[test]
fn frozen_optional_decode_rejects_wrong_layer_and_payload_then_recovers() {
    let ty = optional(boolean());
    let valid = Term::OptionalValue {
        ty: Box::new(ty.clone()),
        value: Some(Box::new(Term::Bool(true))),
    };
    let package = Package {
        functions: vec![
            function(
                0,
                expr(
                    ty.clone(),
                    K::ComptimeValue(Term::OptionalValue {
                        ty: Box::new(ty.clone()),
                        value: Some(Box::new(Term::Char('x'))),
                    }),
                ),
            ),
            function(
                1,
                expr(optional(ty.clone()), K::ComptimeValue(valid.clone())),
            ),
            function(2, expr(ty.clone(), K::ComptimeValue(valid))),
        ],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for bad in [0, 1] {
        assert_eq!(
            vm.invoke(id(bad), vec![]).unwrap_err().kind,
            ErrorKind::Type
        );
        assert_eq!(
            vm.invoke(id(2), vec![]).unwrap(),
            Value::Optional {
                ty: ty.clone(),
                value: Some(Box::new(Value::Bool(true))),
            }
        );
    }
}

#[test]
fn optional_type_payload_freezing_retains_authorized_declaration_scope() {
    let parameter = id(99);
    let domain = optional(Term::Primitive(P::Type));
    let payload = Term::Rigid {
        declaration: parameter,
        sort: nir::Sort::Type,
    };
    let value = Value::Optional {
        ty: domain.clone(),
        value: Some(Box::new(Value::Type(payload.clone()))),
    };
    assert!(
        value
            .clone()
            .into_normal_term(&mut Default::default())
            .is_err()
    );
    let frozen = value
        .into_scoped_normal_term(
            &mut Default::default(),
            &std::collections::BTreeSet::from([parameter]),
        )
        .unwrap();
    assert_eq!(
        frozen,
        Term::OptionalValue {
            ty: Box::new(domain),
            value: Some(Box::new(payload)),
        }
    );
}

#[test]
fn optional_patterns_distinguish_each_layer_and_bind_the_complete_payload() {
    use nir::program::{MatchArm, Pattern};
    let cases = [
        absent(optional(boolean())),
        present(absent(boolean())),
        present(present(expr(boolean(), K::Literal(Literal::Bool(true))))),
    ];
    let tag = |c| expr(Term::Primitive(P::Char), K::Literal(Literal::Char(c)));
    let arms = vec![
        MatchArm {
            pattern: Pattern::OptionalAbsent,
            guard: None,
            body: tag('u'),
        },
        MatchArm {
            pattern: Pattern::OptionalPresent(Box::new(Pattern::OptionalAbsent)),
            guard: None,
            body: tag('c'),
        },
        MatchArm {
            pattern: Pattern::OptionalPresent(Box::new(Pattern::OptionalPresent(Box::new(
                Pattern::Wild,
            )))),
            guard: None,
            body: tag('s'),
        },
    ];
    let package = Package {
        functions: cases
            .into_iter()
            .enumerate()
            .map(|(i, value)| {
                function(
                    i as u32,
                    expr(
                        Term::Primitive(P::Char),
                        K::Match {
                            value: Box::new(value),
                            arms: arms.clone(),
                        },
                    ),
                )
            })
            .collect(),
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for (i, c) in ['u', 'c', 's'].into_iter().enumerate() {
        assert_eq!(vm.invoke(id(i as u32), vec![]).unwrap(), Value::Char(c));
    }
}

#[test]
fn optional_pattern_guard_failure_rolls_back_bindings_before_next_alternative() {
    use nir::program::{LocalId, MatchArm, Pattern};
    let binding = LocalId { owner: 0, index: 0 };
    let bind = Pattern::Binding {
        binding,
        inner: Box::new(Pattern::Wild),
    };
    let pattern = Pattern::Or(vec![
        Pattern::OptionalPresent(Box::new(Pattern::Guard {
            pattern: Box::new(bind.clone()),
            condition: Box::new(expr(boolean(), K::Literal(Literal::Bool(false)))),
        })),
        Pattern::OptionalPresent(Box::new(bind)),
    ]);
    let package = Package {
        functions: vec![function(
            0,
            expr(
                boolean(),
                K::Match {
                    value: Box::new(present(expr(boolean(), K::Literal(Literal::Bool(true))))),
                    arms: vec![
                        MatchArm {
                            pattern,
                            guard: None,
                            body: expr(boolean(), K::Local(binding)),
                        },
                        MatchArm {
                            pattern: Pattern::Wild,
                            guard: None,
                            body: expr(boolean(), K::Literal(Literal::Bool(false))),
                        },
                    ],
                },
            ),
        )],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(0), vec![]).unwrap(), Value::Bool(true));
    assert_eq!(vm.invoke(id(0), vec![]).unwrap(), Value::Bool(true));
}
