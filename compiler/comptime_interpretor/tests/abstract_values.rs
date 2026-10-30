use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use nir::program::{
    Expr, ExprKind as K, Function, FunctionPhase, Literal, LocalId, MatchArm, Package, Pattern,
    SourceRange,
};
use nir::{DeclarationId, Parameter, Primitive as P, Sort, Term};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 91, local }
}
fn local() -> LocalId {
    LocalId { owner: 0, index: 0 }
}
fn bool_ty() -> Term {
    Term::Primitive(P::Bool)
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 1, end: 2 },
    }
}
fn input() -> Expr {
    expr(bool_ty(), K::Local(local()))
}
fn literal(value: bool) -> Expr {
    expr(bool_ty(), K::Literal(Literal::Bool(value)))
}
fn opaque() -> Value {
    Value::AbstractValue {
        declaration: id(10),
        ty: bool_ty(),
    }
}
fn package(body: Expr) -> Package {
    Package {
        functions: vec![Function {
            declaration: id(0),
            name: "run".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: true,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            signature: Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: bool_ty(),
                }],
                result: Box::new(body.ty.clone()),
            },
            parameters: vec![local()],
            defaults: vec![],
            body,
        }],
        ..Default::default()
    }
}
fn scoped(package: &Package) -> Interpreter<'_> {
    Interpreter::for_comptime(package, Budget::default())
        .unwrap()
        .with_abstract_value_parameters(&[(id(10), bool_ty())])
        .unwrap()
}
#[test]
fn opaque_values_require_explicit_domain_authority() {
    let package = package(input());
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(0), vec![opaque()])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert_eq!(
        scoped(&package).invoke(id(0), vec![opaque()]).unwrap(),
        opaque()
    );
    let forged = Value::AbstractValue {
        declaration: id(10),
        ty: Term::Primitive(P::Char),
    };
    assert_eq!(
        scoped(&package)
            .invoke(id(0), vec![forged])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert!(
        Interpreter::new(&package, Budget::default())
            .unwrap()
            .with_abstract_value_parameters(&[(id(10), bool_ty())])
            .is_err()
    );
}
#[test]
fn opaque_values_can_be_packaged_and_projected_without_observation() {
    let ty = Term::Record(vec![("flag".into(), bool_ty())]);
    let record = expr(
        ty,
        K::AnonymousRecord {
            fields: vec![input()],
        },
    );
    let package = package(record.clone());
    assert_eq!(
        scoped(&package).invoke(id(0), vec![opaque()]).unwrap(),
        Value::AnonymousRecord(vec![("flag".into(), opaque())])
    );
    let projected = expr(
        bool_ty(),
        K::AnonymousField {
            value: Box::new(record),
            index: 0,
            name: "flag".into(),
        },
    );
    let package = crate::package(projected);
    assert_eq!(
        scoped(&package).invoke(id(0), vec![opaque()]).unwrap(),
        opaque()
    );
}
#[test]
fn opaque_values_do_not_decide_conditions_comparisons_or_fallback_matches() {
    let operations = vec![
        expr(
            bool_ty(),
            K::If {
                condition: Box::new(input()),
                then_branch: Box::new(literal(true)),
                else_branch: Some(Box::new(literal(false))),
            },
        ),
        expr(
            bool_ty(),
            K::ScalarEqual(Box::new(input()), Box::new(input())),
        ),
        expr(
            bool_ty(),
            K::Match {
                value: Box::new(input()),
                arms: vec![
                    MatchArm {
                        pattern: Pattern::Literal(Literal::Bool(true)),
                        guard: None,
                        body: literal(true),
                    },
                    MatchArm {
                        pattern: Pattern::Wild,
                        guard: None,
                        body: literal(false),
                    },
                ],
            },
        ),
    ];
    for operation in operations {
        let package = package(operation);
        assert_eq!(
            scoped(&package)
                .invoke(id(0), vec![opaque()])
                .unwrap_err()
                .kind,
            ErrorKind::AbstractValueUnresolved
        );
        assert_eq!(
            scoped(&package)
                .invoke(id(0), vec![Value::Bool(true)])
                .unwrap(),
            Value::Bool(true)
        );
    }
}
#[test]
fn opaque_values_do_not_escape_as_completed_snapshots() {
    let value = Value::AnonymousRecord(vec![("flag".into(), opaque())]);
    assert_eq!(
        value.into_normal_term(&mut Default::default()).unwrap_err(),
        ErrorKind::AbstractValueUnresolved
    );
    assert_eq!(
        opaque()
            .into_scoped_normal_term(&mut Default::default(), &[id(10)].into_iter().collect())
            .unwrap_err(),
        ErrorKind::AbstractValueUnresolved
    );
    assert!(opaque().to_string().contains("(abstract-value-input"));
}
#[test]
fn abstract_domain_authority_rejects_conflicts_and_type_value_confusion() {
    let package = package(input());
    assert!(
        scoped(&package)
            .with_abstract_value_parameters(&[(id(10), Term::Primitive(P::Char))])
            .is_err()
    );
    assert!(
        scoped(&package)
            .with_abstract_type_parameters(&[id(10)])
            .is_err()
    );
    assert!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_abstract_type_parameters(&[id(10)])
            .unwrap()
            .with_abstract_value_parameters(&[(id(10), bool_ty())])
            .is_err()
    );
    assert!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_abstract_value_parameters(&[(id(10), Term::Primitive(P::Type))])
            .is_err()
    );
}

#[test]
fn prepared_normal_inputs_use_the_same_scoped_domain_authority() {
    let package = package(input());
    let term = Term::Rigid {
        declaration: id(10),
        sort: Sort::ComptimeValue,
    };
    assert_eq!(
        scoped(&package)
            .invoke_with_normal_arguments(id(0), vec![(bool_ty(), term.clone())])
            .unwrap(),
        opaque()
    );
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke_with_normal_arguments(id(0), vec![(bool_ty(), term.clone())])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert_eq!(
        scoped(&package)
            .invoke_with_normal_arguments(id(0), vec![(Term::Primitive(P::Char), term)])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
}

#[test]
fn abstract_reification_requires_exact_domains_and_preserves_identity() {
    let types = Default::default();
    let domains = [(id(10), bool_ty())].into_iter().collect();
    let value = Value::AnonymousRecord(vec![("a".into(), opaque()), ("b".into(), opaque())]);
    let term = value
        .into_abstract_invocation_term(&mut Default::default(), &types, &domains)
        .unwrap();
    let input = Term::Rigid {
        declaration: id(10),
        sort: Sort::ComptimeValue,
    };
    assert_eq!(
        term,
        Term::RecordValue(vec![("a".into(), input.clone()), ("b".into(), input)])
    );
    let wrong = [(id(10), Term::Primitive(P::Char))].into_iter().collect();
    assert!(
        opaque()
            .into_abstract_invocation_term(&mut Default::default(), &types, &wrong)
            .is_err()
    );
    assert!(
        opaque()
            .into_abstract_invocation_term(
                &mut Default::default(),
                &[id(10)].into_iter().collect(),
                &domains
            )
            .is_err()
    );
    assert!(
        opaque()
            .into_abstract_invocation_term(
                &mut comptime_runtime::Budget {
                    steps: 0,
                    max_limbs: 4096
                },
                &types,
                &domains
            )
            .is_err()
    );
    let forged = Value::AbstractValue {
        declaration: id(10),
        ty: Term::Primitive(P::Type),
    };
    assert!(
        forged
            .into_abstract_invocation_term(
                &mut Default::default(),
                &types,
                &[(id(10), Term::Primitive(P::Type))].into_iter().collect()
            )
            .is_err()
    );
}

#[test]
fn known_short_circuit_conditions_preserve_opaque_rhs_without_observation() {
    for expression in [
        K::And(Box::new(literal(true)), Box::new(input())),
        K::Or(Box::new(literal(false)), Box::new(input())),
    ] {
        let package = package(expr(bool_ty(), expression));
        assert_eq!(
            scoped(&package).invoke(id(0), vec![opaque()]).unwrap(),
            opaque()
        );
    }
    for (expression, expected) in [
        (
            K::And(
                Box::new(literal(false)),
                Box::new(expr(bool_ty(), K::Not(Box::new(input())))),
            ),
            false,
        ),
        (
            K::Or(
                Box::new(literal(true)),
                Box::new(expr(bool_ty(), K::Not(Box::new(input())))),
            ),
            true,
        ),
    ] {
        let package = package(expr(bool_ty(), expression));
        assert_eq!(
            scoped(&package).invoke(id(0), vec![opaque()]).unwrap(),
            Value::Bool(expected)
        );
    }
    for expression in [
        K::Not(Box::new(input())),
        K::And(Box::new(input()), Box::new(literal(false))),
        K::Or(Box::new(input()), Box::new(literal(true))),
    ] {
        let package = package(expr(bool_ty(), expression));
        assert_eq!(
            scoped(&package)
                .invoke(id(0), vec![opaque()])
                .unwrap_err()
                .kind,
            ErrorKind::AbstractValueUnresolved
        );
    }
}

#[test]
fn numeric_observation_of_abstract_values_is_unresolved_not_a_type_error() {
    for primitive in [P::Integer, P::U8, P::Float, P::F32, P::F64, P::BitVec(8)] {
        let domain = Term::Primitive(primitive);
        let input = expr(domain.clone(), K::Local(local()));
        let kind = match primitive {
            P::Integer | P::U8 => K::IntegerNegate(Box::new(input)),
            P::BitVec(_) => K::Not(Box::new(input)),
            _ => K::FloatNegate(Box::new(input)),
        };
        let mut package = package(expr(domain.clone(), kind));
        let Term::Function { parameters, .. } = &mut package.functions[0].signature else {
            unreachable!()
        };
        parameters[0].ty = domain.clone();
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_abstract_value_parameters(&[(id(10), domain.clone())])
            .unwrap();
        assert_eq!(
            vm.invoke(
                id(0),
                vec![Value::AbstractValue {
                    declaration: id(10),
                    ty: domain
                }]
            )
            .unwrap_err()
            .kind,
            ErrorKind::AbstractValueUnresolved
        );
    }
}

#[test]
fn unknown_guards_do_not_choose_an_arm_or_fallback() {
    let package = package(expr(
        bool_ty(),
        K::Match {
            value: Box::new(literal(true)),
            arms: vec![
                MatchArm {
                    pattern: Pattern::Wild,
                    guard: Some(input()),
                    body: literal(false),
                },
                MatchArm {
                    pattern: Pattern::Wild,
                    guard: None,
                    body: literal(true),
                },
            ],
        },
    ));
    assert_eq!(
        scoped(&package)
            .invoke(id(0), vec![opaque()])
            .unwrap_err()
            .kind,
        ErrorKind::AbstractValueUnresolved
    );
    assert_eq!(
        scoped(&package)
            .invoke(id(0), vec![Value::Bool(true)])
            .unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn rejected_scope_extensions_are_atomic_and_keep_consumed_fuel() {
    let package = package(input());
    let mut vm = scoped(&package);
    let before = vm.remaining_steps();
    assert_eq!(
        vm.authorize_abstract_value_parameters(&[
            (id(11), bool_ty()),
            (id(10), Term::Primitive(P::U8)),
        ])
        .unwrap_err()
        .kind,
        ErrorKind::Type
    );
    assert!(vm.remaining_steps() < before);
    let new_input = Value::AbstractValue {
        declaration: id(11),
        ty: bool_ty(),
    };
    assert_eq!(
        vm.invoke(id(0), vec![new_input.clone()]).unwrap_err().kind,
        ErrorKind::Type
    );
    assert_eq!(vm.invoke(id(0), vec![opaque()]).unwrap(), opaque());
    vm.authorize_abstract_value_parameters(&[(id(11), bool_ty())])
        .unwrap();
    assert_eq!(
        vm.invoke(id(0), vec![new_input.clone()]).unwrap(),
        new_input
    );

    assert_eq!(
        vm.authorize_abstract_type_parameters(&[id(12), id(10)])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    // The rejected Type batch must not reserve id(12) as a Type parameter.
    vm.authorize_abstract_value_parameters(&[(id(12), bool_ty())])
        .unwrap();
    assert_eq!(
        vm.authorize_abstract_value_parameters(&[
            (id(13), bool_ty()),
            (id(13), Term::Primitive(P::Char)),
        ])
        .unwrap_err()
        .kind,
        ErrorKind::Type
    );
    let pending = Value::AbstractValue {
        declaration: id(13),
        ty: bool_ty(),
    };
    assert_eq!(
        vm.invoke(id(0), vec![pending.clone()]).unwrap_err().kind,
        ErrorKind::Type
    );
    vm.authorize_abstract_value_parameters(&[(id(13), bool_ty()), (id(13), bool_ty())])
        .unwrap();
    assert_eq!(vm.invoke(id(0), vec![pending.clone()]).unwrap(), pending);
}

#[test]
fn prepared_owned_roots_preserve_borrow_identity_across_invocations() {
    use comptime_interpretor::Storage;
    let pointer_ty = Term::Pointer(Box::new(bool_ty()));
    let mut package = package(expr(pointer_ty.clone(), K::AddressOf(Box::new(input()))));
    let mut writer = package.functions[0].clone();
    writer.declaration = id(1);
    writer.signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: pointer_ty.clone(),
        }],
        result: Box::new(Term::Primitive(P::Void)),
    };
    writer.body = expr(
        Term::Primitive(P::Void),
        K::WritePointer {
            pointer: Box::new(expr(pointer_ty.clone(), K::Local(local()))),
            value: Box::new(literal(true)),
        },
    );
    let mut reader = writer.clone();
    reader.declaration = id(2);
    reader.signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: pointer_ty.clone(),
        }],
        result: Box::new(bool_ty()),
    };
    reader.body = expr(
        bool_ty(),
        K::ReadPointer(Box::new(expr(pointer_ty.clone(), K::Local(local())))),
    );
    package.functions.extend([writer, reader]);
    let owner = Storage::new(Value::Bool(false));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let pointer = vm
        .invoke_with_typed_values(id(0), vec![(bool_ty(), Value::Storage(owner.clone()))])
        .unwrap();
    assert!(
        pointer
            .clone()
            .into_normal_term(&mut Default::default())
            .is_err()
    );
    let checkpoint = vm
        .checkpoint_owned_graph(&[Value::Storage(owner.clone())])
        .unwrap();
    // A fresh interpreter sees the same invocation-owned storage; no copying
    // or implicit promotion of the weak pointer to an owner takes place.
    let mut next = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    next.invoke_with_typed_values(id(1), vec![(pointer_ty.clone(), pointer.clone())])
        .unwrap();
    assert_eq!(owner.read().unwrap(), Value::Bool(true));
    assert_eq!(
        next.invoke_with_typed_values(id(2), vec![(pointer_ty.clone(), pointer.clone())])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        next.invoke_with_typed_values(
            id(0),
            vec![(Term::Primitive(P::Char), Value::Storage(owner.clone()))]
        )
        .unwrap_err()
        .kind,
        ErrorKind::Type
    );
    assert_eq!(owner.read().unwrap(), Value::Bool(true));
    checkpoint.restore().unwrap();
    assert_eq!(owner.read().unwrap(), Value::Bool(false));
    assert_eq!(
        next.invoke_with_typed_values(id(2), vec![(pointer_ty.clone(), pointer.clone())])
            .unwrap(),
        Value::Bool(false)
    );
    drop(owner);
    assert!(
        next.invoke_with_typed_values(id(2), vec![(pointer_ty, pointer)])
            .is_err()
    );
}

#[test]
fn residual_nominal_types_require_value_domain_authority_and_stay_unexportable() {
    use nir::program::NormalFormArgument;
    let symbolic = Term::Rigid {
        declaration: id(10),
        sort: Sort::ComptimeValue,
    };
    for captures in [0, 1] {
        let mut package = package(expr(
            Term::Primitive(P::Type),
            K::NominalType {
                preparations: vec![],
                bindings: None,
                order: None,
                declaration: id(20),
                arguments: vec![NormalFormArgument::Value(input())],
            },
        ));
        package.newtypes.push(nir::newtypes::Newtype {
            declaration: id(20),
            name: "Indexed".into(),
            type_parameters: vec![id(21)],
            parameter_types: vec![bool_ty()],
            capture_count: captures,
            requirements: vec![],
            capabilities: vec![],
            representation: bool_ty(),
        });
        let expected = Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![symbolic.clone()],
        });
        let value = scoped(&package).invoke(id(0), vec![opaque()]).unwrap();
        assert_eq!(value, expected);
        assert!(
            value
                .clone()
                .into_normal_term(&mut Default::default())
                .is_err()
        );
        assert!(
            value
                .clone()
                .into_scoped_normal_term(&mut Default::default(), &Default::default())
                .is_err()
        );
        assert_eq!(
            value
                .into_abstract_invocation_term(
                    &mut Default::default(),
                    &Default::default(),
                    &[(id(10), bool_ty())].into_iter().collect()
                )
                .unwrap(),
            Term::Nominal {
                declaration: id(20),
                arguments: vec![symbolic.clone()]
            }
        );
        assert!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke(id(0), vec![opaque()])
                .is_err()
        );
        package.newtypes[0].parameter_types = vec![Term::Primitive(P::Char)];
        assert!(scoped(&package).invoke(id(0), vec![opaque()]).is_err());
        // A prebuilt identity also passes through declaration-domain admission.
        package.functions[0].body = expr(
            Term::Primitive(P::Type),
            K::TypeValue(Term::Nominal {
                declaration: id(20),
                arguments: vec![symbolic.clone()],
            }),
        );
        assert!(scoped(&package).invoke(id(0), vec![opaque()]).is_err());
    }
}

#[test]
fn residual_type_lists_compose_with_aggregates_without_becoming_closed_constants() {
    let ty = Term::Nominal {
        declaration: id(20),
        arguments: vec![Term::Rigid {
            declaration: id(10),
            sort: Sort::ComptimeValue,
        }],
    };
    let types = ordinary_type_list(vec![bool_ty(), ty.clone(), ty.clone()]);
    let expected = frozen_types(vec![bool_ty(), ty.clone(), ty.clone()]);
    let authority = [(id(10), bool_ty())].into_iter().collect();
    for (value, expected) in [
        (types.clone(), expected.clone()),
        (
            Value::Tuple(vec![Value::Type(ty.clone()), types.clone()]),
            Term::TupleValue(vec![ty.clone(), expected.clone()]),
        ),
        (
            Value::AnonymousRecord(vec![("types".into(), types)]),
            Term::RecordValue(vec![("types".into(), expected)]),
        ),
    ] {
        assert_eq!(
            value
                .clone()
                .into_abstract_invocation_term(
                    &mut Default::default(),
                    &Default::default(),
                    &authority
                )
                .unwrap(),
            expected
        );
        assert!(
            value
                .clone()
                .into_normal_term(&mut Default::default())
                .is_err()
        );
        assert!(
            value
                .clone()
                .into_scoped_normal_term(&mut Default::default(), &Default::default())
                .is_err()
        );
        assert!(
            value
                .clone()
                .into_abstract_invocation_term(
                    &mut Default::default(),
                    &Default::default(),
                    &Default::default()
                )
                .is_err()
        );
        let mut exhausted = comptime_runtime::Budget::default();
        exhausted.steps = 0;
        assert!(
            value
                .clone()
                .into_abstract_invocation_term(&mut exhausted, &Default::default(), &authority)
                .is_err()
        );
        assert_eq!(
            value
                .into_abstract_invocation_term(
                    &mut Default::default(),
                    &Default::default(),
                    &authority
                )
                .unwrap(),
            expected
        );
    }
}

#[test]
fn residual_type_reification_rejects_value_roots_even_with_authority() {
    let authority = [(id(10), bool_ty())].into_iter().collect();
    for ty in [
        Term::Bool(true),
        Term::Rigid {
            declaration: id(10),
            sort: Sort::ComptimeValue,
        },
    ] {
        for value in [Value::Type(ty.clone()), ordinary_type_list(vec![ty])] {
            assert!(
                value
                    .into_abstract_invocation_term(
                        &mut Default::default(),
                        &Default::default(),
                        &authority
                    )
                    .is_err()
            );
        }
    }
}

#[test]
fn residual_nominal_payload_reification_keeps_indices_tags_and_closed_export_rules() {
    let symbol = Term::Rigid {
        declaration: id(10),
        sort: Sort::ComptimeValue,
    };
    let ty = Term::Nominal {
        declaration: id(20),
        arguments: vec![symbol.clone()],
    };
    let authority = [(id(10), bool_ty())].into_iter().collect();
    for (value, variant) in [
        (
            Value::Record {
                declaration: id(20),
                arguments: vec![symbol.clone()],
                fields: vec![Value::Bool(true)],
            },
            None,
        ),
        (
            Value::Variant {
                declaration: id(20),
                arguments: vec![symbol.clone()],
                index: 2,
                fields: vec![Value::Bool(true)],
            },
            Some(2),
        ),
        (
            Value::Newtype {
                ty: ty.clone(),
                value: Box::new(Value::Bool(true)),
            },
            None,
        ),
    ] {
        let expected = Term::NominalValue {
            ty: Box::new(ty.clone()),
            variant,
            fields: vec![Term::Bool(true)],
        };
        assert_eq!(
            value
                .clone()
                .into_abstract_invocation_term(
                    &mut Default::default(),
                    &Default::default(),
                    &authority
                )
                .unwrap(),
            expected
        );
        assert!(
            value
                .clone()
                .into_normal_term(&mut Default::default())
                .is_err()
        );
        assert!(
            value
                .clone()
                .into_scoped_normal_term(&mut Default::default(), &Default::default())
                .is_err()
        );
        assert!(
            value
                .clone()
                .into_abstract_invocation_term(
                    &mut Default::default(),
                    &Default::default(),
                    &Default::default()
                )
                .is_err()
        );
        let mut exhausted = comptime_runtime::Budget::default();
        exhausted.steps = 0;
        assert!(
            value
                .clone()
                .into_abstract_invocation_term(&mut exhausted, &Default::default(), &authority)
                .is_err()
        );
        assert_eq!(
            value
                .into_abstract_invocation_term(
                    &mut Default::default(),
                    &Default::default(),
                    &authority
                )
                .unwrap(),
            expected
        );
    }
}

#[test]
fn frozen_error_values_round_trip_and_reject_invalid_branch_or_payload() {
    let ty = Term::error_result(vec![bool_ty(), Term::Primitive(P::Char)], bool_ty());
    let mut package = package(expr(ty.clone(), K::Local(local())));
    let Term::Function { parameters, .. } = &mut package.functions[0].signature else {
        unreachable!()
    };
    parameters[0].ty = ty.clone();
    for (branch, payload) in [
        (None, Value::Bool(true)),
        (Some(0), Value::Bool(false)),
        (Some(1), Value::Char('x')),
    ] {
        let value = Value::ErrorResult {
            ty: ty.clone(),
            error: branch,
            value: Box::new(payload),
        };
        let frozen = value
            .clone()
            .into_normal_term(&mut Default::default())
            .unwrap();
        frozen.check_scope(&[]).unwrap();
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke_with_normal_arguments(id(0), vec![(ty.clone(), frozen.clone())])
                .unwrap(),
            value
        );
        assert!(matches!(frozen, Term::ErrorValue { .. }));
    }
    for frozen in [
        Term::ErrorValue {
            ty: Box::new(ty.clone()),
            label: Some(Box::new(Term::Primitive(P::Str))),
            value: Box::new(Term::String("x".into())),
        },
        Term::ErrorValue {
            ty: Box::new(ty.clone()),
            label: Some(Box::new(bool_ty())),
            value: Box::new(Term::Char('x')),
        },
        Term::ErrorValue {
            ty: Box::new(ty.clone()),
            label: None,
            value: Box::new(Term::Char('x')),
        },
    ] {
        assert!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke_with_normal_arguments(id(0), vec![(ty.clone(), frozen)])
                .is_err()
        );
    }
    let invalid = Value::ErrorResult {
        ty,
        error: Some(2),
        value: Box::new(Value::Bool(true)),
    };
    assert!(invalid.into_normal_term(&mut Default::default()).is_err());
}

#[test]
fn pending_error_branch_cannot_be_reconstructed_as_a_selected_success() {
    let inner = Term::error_result(vec![bool_ty()], bool_ty());
    let ty = Term::error_result(vec![Term::Primitive(P::Char)], inner.clone());
    let pending = Term::error_value(
        vec![Term::Primitive(P::Char)],
        inner,
        None,
        Term::Rigid {
            declaration: id(10),
            sort: Sort::ComptimeValue,
        },
    )
    .unwrap();
    let mut package = package(expr(ty.clone(), K::Local(local())));
    let Term::Function { parameters, .. } = &mut package.functions[0].signature else {
        unreachable!()
    };
    parameters[0].ty = ty.clone();
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke_with_normal_arguments(id(0), vec![(ty, pending)])
            .unwrap_err()
            .kind,
        ErrorKind::AbstractValueUnresolved
    );
}

#[test]
fn explicit_physical_error_rows_freeze_and_use_checked_input_transport() {
    let errors = vec![Term::Primitive(P::Char), bool_ty()];
    let ty = Term::ErrorResult {
        errors: errors.clone(),
        result: Box::new(bool_ty()),
    };
    let canonical = Term::error_result(errors.clone(), bool_ty());
    let mut package = package(expr(canonical.clone(), K::Local(local())));
    let Term::Function { parameters, .. } = &mut package.functions[0].signature else {
        unreachable!()
    };
    parameters[0].ty = canonical.clone();
    let transport = nir::error_results::transport_plan(
        &ty,
        &canonical,
        false,
        &mut 10000,
        &mut |a, b| a == b,
        &|_| false,
    )
    .unwrap();
    for (error, payload) in [
        (None, Value::Bool(true)),
        (Some(0), Value::Char('x')),
        (Some(1), Value::Bool(false)),
    ] {
        let value = Value::ErrorResult {
            ty: ty.clone(),
            error,
            value: Box::new(payload.clone()),
        };
        let frozen = value
            .clone()
            .into_normal_term(&mut Default::default())
            .unwrap();
        assert!(
            matches!(&frozen,Term::ErrorValue {ty:domain,..} if domain.as_ref()==&ty),
            "{frozen}"
        );
        assert!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke_with_normal_arguments(id(0), vec![(ty.clone(), frozen.clone())])
                .is_err()
        );
        let transported = transport.frozen(&frozen, &mut 10000).unwrap();
        let Term::ErrorResult {
            errors: canonical_errors,
            ..
        } = &canonical
        else {
            unreachable!()
        };
        let expected = Value::ErrorResult {
            ty: canonical.clone(),
            error: error.map(|index| {
                canonical_errors
                    .iter()
                    .position(|label| label == &errors[index])
                    .unwrap()
            }),
            value: Box::new(payload),
        };
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke_with_normal_arguments(id(0), vec![(canonical.clone(), transported)])
                .unwrap(),
            expected
        );
    }
}

fn ordinary_type_list(types: Vec<nir::Term>) -> Value {
    Value::List {
        ty: nir::Term::NormalForm {
            head: nir::DeclarationId {
                package: 4294967294,
                local: 0,
            },
            sort: nir::Sort::Type,
            arguments: vec![nir::Term::Primitive(nir::Primitive::Type)],
        },
        values: types.into_iter().map(Value::Type).collect(),
    }
}

fn frozen_types(fields: Vec<Term>) -> Term {
    Term::NominalValue {
        ty: Box::new(Term::NormalForm {
            head: DeclarationId {
                package: 4294967294,
                local: 0,
            },
            sort: nir::Sort::Type,
            arguments: vec![Term::Primitive(nir::Primitive::Type)],
        }),
        variant: None,
        fields,
    }
}
