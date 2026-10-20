use iir::completed_type;
use nir::{DeclarationId, Parameter, Primitive, RuntimeId, Sort, Term};
use std::collections::BTreeSet;

#[test]
fn staged_type_list_index_preserves_identity_and_checks_bounds_and_depth() {
    use nir::normal_forms::{Head, Kind, Parameter as Input, Role};
    use nir::program::{Expr, ExprKind as K, Literal, LocalId, SourceRange};
    let id = DeclarationId {
        package: 81,
        local: 1,
    };
    let type_ty = Term::Primitive(Primitive::Type);
    let list_ty = Term::NormalForm {
        head: id,
        sort: Sort::Type,
        arguments: vec![type_ty.clone()],
    };
    let heads = [Head {
        declaration: id,
        role: Role::TypeList,
        requirements: vec![],
        parameters: vec![Input {
            declaration: DeclarationId {
                package: 81,
                local: 2,
            },
            binding: LocalId { owner: 1, index: 0 },
            kind: Kind::Type,
            ty: type_ty.clone(),
        }],
    }];
    let source = SourceRange { start: 10, end: 20 };
    let opaque = Term::NormalForm {
        head: id,
        sort: Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    let element = Expr {
        source,
        ty: type_ty.clone(),
        kind: K::TypeValue(opaque.clone()),
    };
    let list = |elements| Expr {
        source,
        ty: list_ty.clone(),
        kind: K::TypeList(elements),
    };
    let index = |values, position: &str| Expr {
        source,
        ty: type_ty.clone(),
        kind: K::TypeListIndex(
            Box::new(values),
            Box::new(Expr {
                source,
                ty: Term::Primitive(Primitive::Integer),
                kind: K::Literal(Literal::Integer(position.into())),
            }),
        ),
    };
    let valid = index(list(vec![element.clone()]), "0");
    let fold =
        |expr: &Expr, fuel: &mut usize| iir::staging::fold_value_with_layouts(&heads, expr, fuel);
    assert_eq!(fold(&valid, &mut 10_000).unwrap(), element);
    let invalid_index = index(list(vec![]), "0");
    assert!(
        fold(
            &index(list(vec![element.clone(), invalid_index.clone()]), "0"),
            &mut 10_000
        )
        .is_err()
    );
    let selected = Expr {
        source,
        ty: type_ty.clone(),
        kind: K::If {
            condition: Box::new(Expr {
                source,
                ty: Term::Primitive(Primitive::Bool),
                kind: K::Literal(Literal::Bool(true)),
            }),
            then_branch: Box::new(element.clone()),
            else_branch: Some(Box::new(invalid_index.clone())),
        },
    };
    assert_eq!(
        fold(&index(list(vec![selected]), "0"), &mut 10_000).unwrap(),
        element
    );
    let discarded = Expr {
        source,
        ty: type_ty.clone(),
        kind: K::Block {
            statements: vec![invalid_index],
            result: Some(Box::new(element.clone())),
        },
    };
    assert!(fold(&discarded, &mut 10_000).is_err());
    for position in ["-1", "1", "340282366920938463463374607431768211456"] {
        assert!(
            fold(&index(list(vec![element.clone()]), position), &mut 10_000)
                .unwrap_err()
                .contains("out of bounds")
        );
    }
    assert!(fold(&index(list(vec![]), "0"), &mut 10_000).is_err());
    assert!(fold(&valid, &mut 0).is_err());
    assert_eq!(fold(&valid, &mut 10_000).unwrap(), element);
    let mut nested = element.clone();
    for _ in 0..40 {
        nested = index(list(vec![nested]), "0");
    }
    assert_eq!(fold(&nested, &mut 100_000).unwrap(), element);
    for _ in 0..40 {
        nested = index(list(vec![nested]), "0");
    }
    assert!(fold(&nested, &mut 100_000).unwrap_err().contains("depth"));
}

#[test]
fn unresolved_associated_types_cannot_enter_iir_even_with_closed_inputs() {
    let projection = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: DeclarationId {
                package: 1,
                local: 1,
            },
            member: DeclarationId {
                package: 1,
                local: 2,
            },
        }),
        subject: Box::new(Term::Primitive(Primitive::Bool)),
        arguments: vec![],
    };
    for term in [projection.clone(), Term::Tuple(vec![projection])] {
        assert!(
            completed_type(&term, &BTreeSet::new(), &mut 100)
                .unwrap_err()
                .contains("unresolved associated type")
        );
    }
}
#[test]
fn completed_types_keep_heads_and_runtime_identity_but_reject_missing_scope() {
    let id = RuntimeId {
        instance: 17,
        local: 3,
    };
    let term = Term::NormalForm {
        head: DeclarationId {
            package: 1,
            local: 9,
        },
        sort: Sort::Type,
        arguments: vec![Term::Runtime(id)],
    };
    let before = term.to_string();
    assert!(
        completed_type(&term, &BTreeSet::new(), &mut 100)
            .unwrap_err()
            .contains("unavailable runtime")
    );
    completed_type(&term, &BTreeSet::from([id]), &mut 100).unwrap();
    assert_eq!(term.to_string(), before);
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: Term::Primitive(Primitive::Type),
        }],
        result: Box::new(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        }),
    };
    completed_type(&signature, &BTreeSet::new(), &mut 100).unwrap();
    let rigid = Term::Rigid {
        declaration: DeclarationId {
            package: 0,
            local: 1,
        },
        sort: Sort::Type,
    };
    assert!(
        completed_type(&rigid, &BTreeSet::new(), &mut 100)
            .unwrap_err()
            .contains("unresolved")
    );
    assert!(
        completed_type(&signature, &BTreeSet::new(), &mut 0)
            .unwrap_err()
            .contains("budget")
    );
}

#[test]
fn record_fields_cannot_hide_unfinished_or_unavailable_iir_types() {
    let record = |ty| Term::Record(vec![("payload".into(), ty)]);
    let rigid = Term::Rigid {
        declaration: DeclarationId {
            package: 0,
            local: 1,
        },
        sort: Sort::Type,
    };
    assert!(
        completed_type(&record(rigid), &BTreeSet::new(), &mut 100)
            .unwrap_err()
            .contains("unresolved")
    );
    let runtime = RuntimeId {
        instance: 17,
        local: 3,
    };
    let ty = record(Term::NormalForm {
        head: DeclarationId {
            package: 1,
            local: 9,
        },
        sort: Sort::Type,
        arguments: vec![Term::Runtime(runtime)],
    });
    assert!(
        completed_type(&ty, &BTreeSet::new(), &mut 100)
            .unwrap_err()
            .contains("unavailable runtime")
    );
    completed_type(&ty, &BTreeSet::from([runtime]), &mut 100).unwrap();
    assert!(
        completed_type(&ty, &BTreeSet::from([runtime]), &mut 2)
            .unwrap_err()
            .contains("budget")
    );
    completed_type(&ty, &BTreeSet::from([runtime]), &mut 100).unwrap();
}

#[test]
fn effect_computation_type_completion_does_not_replace_environment_validation() {
    let computation = Term::computation(
        vec![nir::effects::Operation {
            declaration: DeclarationId {
                package: 1,
                local: 99,
            },
            arguments: vec![],
        }],
        Term::Primitive(Primitive::Bool),
    );
    for ty in [computation.clone(), Term::Tuple(vec![computation])] {
        completed_type(&ty, &BTreeSet::new(), &mut 100).unwrap();
    }
}

#[test]
fn type_list_constants_validate_embedded_types_and_domain_authority() {
    use nir::normal_forms::{Head, Kind, Parameter as Input, Role};
    use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, LocalId, SourceRange};
    let id = |local| DeclarationId { package: 51, local };
    let primitive = |p| Term::Primitive(p);
    let list = Term::NormalForm {
        head: id(1),
        sort: Sort::Type,
        arguments: vec![primitive(Primitive::Type)],
    };
    let key = iir::InstanceKey {
        declaration: id(3),
        arguments: vec![],
    };
    let mut package = iir::Package {
        roots: vec![key.clone()],
        normal_forms: vec![
            Head {
                declaration: id(1),
                role: Role::Opaque,
                requirements: vec![],
                parameters: vec![Input {
                    declaration: id(10),
                    binding: LocalId { owner: 1, index: 0 },
                    kind: Kind::Type,
                    ty: primitive(Primitive::Type),
                }],
            },
            Head {
                declaration: id(2),
                role: Role::Tuple,
                requirements: vec![],
                parameters: vec![Input {
                    declaration: id(20),
                    binding: LocalId { owner: 2, index: 0 },
                    kind: Kind::TypeList,
                    ty: list.clone(),
                }],
            },
        ],
        functions: vec![iir::FunctionInstance {
            key,
            function: Function {
                declaration: id(3),
                name: "elements".into(),
                phase: FunctionPhase::Comptime,
                inline: false,
                pure: true,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(list.clone()),
                },
                body: Expr {
                    source: SourceRange { start: 0, end: 1 },
                    ty: list.clone(),
                    kind: K::TypeListValue(vec![]),
                },
                parameters: vec![],
                defaults: vec![],
                type_parameters: vec![],
                requirements: vec![],
                capabilities: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
            },
        }],
        ..Default::default()
    };
    package.validate(&mut 10_000).unwrap();
    let base = package.clone();
    for element in [
        primitive(Primitive::Bool),
        Term::NormalForm {
            head: id(2),
            sort: Sort::Type,
            arguments: vec![Term::TypeListValue(vec![primitive(Primitive::U8)])],
        },
    ] {
        package.functions[0].function.body.kind = K::TypeListValue(vec![element]);
        package.validate(&mut 10_000).unwrap();
    }
    for element in [
        Term::Bool(true),
        Term::Rigid {
            declaration: id(99),
            sort: Sort::Type,
        },
        Term::NormalForm {
            head: id(99),
            sort: Sort::Type,
            arguments: vec![],
        },
    ] {
        package.functions[0].function.body.kind = K::TypeListValue(vec![element]);
        assert!(package.validate(&mut 10_000).is_err());
    }
    package.functions[0].function.body.kind = K::TypeListValue(vec![primitive(Primitive::Bool)]);
    assert!(package.validate(&mut 0).is_err());
    package.validate(&mut 10_000).unwrap();
    package.functions[0].function.body.ty = primitive(Primitive::Type);
    assert!(
        package
            .validate(&mut 10_000)
            .unwrap_err()
            .contains("type-list constant")
    );
    package.functions[0].function.body.ty = list;
    package.normal_forms.pop();
    assert!(
        package
            .validate(&mut 10_000)
            .unwrap_err()
            .contains("type-list constant")
    );
    package.normal_forms[0].role = Role::TypeList;
    package.validate(&mut 10_000).unwrap();
    let value = package.functions[0].function.body.clone();
    assert_eq!(
        iir::staging::fold_value_with_layouts(&package.normal_forms, &value, &mut 10_000).unwrap(),
        value
    );
    let construction = Expr {
        source: value.source,
        ty: value.ty.clone(),
        kind: K::TypeList(vec![Expr {
            source: value.source,
            ty: primitive(Primitive::Type),
            kind: K::TypeValue(primitive(Primitive::Bool)),
        }]),
    };
    package.functions[0].function.body = construction.clone();
    package.validate(&mut 10_000).unwrap();
    assert_eq!(
        iir::staging::fold_value_with_layouts(&package.normal_forms, &construction, &mut 10_000)
            .unwrap(),
        value
    );
    assert!(
        iir::staging::fold_value_with_layouts(&package.normal_forms, &construction, &mut 0)
            .is_err()
    );
    let K::TypeList(fields) = &mut package.functions[0].function.body.kind else {
        unreachable!()
    };
    fields[0].ty = primitive(Primitive::Bool);
    assert!(package.validate(&mut 10_000).is_err());
    package.functions[0].function.body = construction;
    package.normal_forms[0].parameters.clear();
    assert!(package.validate(&mut 10_000).is_err());

    let mut contract_package = base;
    let list_value = contract_package.functions[0].function.body.clone();
    let predicate = Expr {
        source: list_value.source,
        ty: primitive(Primitive::Bool),
        kind: K::Block {
            statements: vec![list_value.clone()],
            result: Some(Box::new(Expr {
                source: list_value.source,
                ty: primitive(Primitive::Bool),
                kind: K::Literal(nir::program::Literal::Bool(true)),
            })),
        },
    };
    contract_package.interfaces.push(iir::InterfaceContract {
        interface: nir::program::InterfaceInstance {
            declaration: id(4),
            arguments: vec![],
        },
        subject: primitive(Primitive::Bool),
        methods: vec![nir::program::MethodContract {
            is_quote: false,
            pure: false,
            declaration: id(5),
            name: "check".into(),
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(primitive(Primitive::Bool)),
            }
            .into(),
            parameter_bindings: vec![],
            type_arguments: vec![],
            type_parameters: vec![],
            capabilities: vec![],
            requirements: vec![],
            by_value_receiver: false,
            by_reference_receiver: false,
            default: None,
            program: nir::contracts::ProgramContract {
                preconditions: vec![predicate],
                ..Default::default()
            },
        }],
    });
    let mut method = contract_package.functions[0].function.clone();
    method.declaration = id(6);
    method.phase = FunctionPhase::Runtime;
    method.signature = Term::Function {
        parameters: vec![],
        result: Box::new(primitive(Primitive::Bool)),
    };
    method.body = Expr {
        source: list_value.source,
        ty: primitive(Primitive::Bool),
        kind: K::Literal(nir::program::Literal::Bool(true)),
    };
    contract_package.functions.push(iir::FunctionInstance {
        key: iir::InstanceKey {
            declaration: id(6),
            arguments: vec![],
        },
        function: method,
    });
    contract_package
        .dispatch
        .push(nir::program::DispatchInstance {
            associated_initializers: vec![],
            interface: id(4),
            interface_arguments: vec![],
            subject: primitive(Primitive::Bool),
            implementation: id(7),
            arguments: vec![],
            associated: vec![],
            methods: vec![nir::program::MethodInstance {
                member: id(5),
                body: id(6),
                arguments: vec![],
                method_arguments: vec![],
            }],
        });
    contract_package.validate(&mut 10_000).unwrap();
    let mut pending = contract_package.clone();
    let initializer = nir::associated_initializers::AssociatedInitializer::new(
        Expr {
            source: list_value.source,
            ty: primitive(Primitive::Bool),
            kind: K::Literal(nir::program::Literal::Bool(true)),
        },
        &mut 1000,
    )
    .unwrap();
    pending.dispatch[0]
        .associated_initializers
        .push((id(8), initializer));
    assert!(
        pending
            .validate(&mut 10_000)
            .unwrap_err()
            .contains("associated initializers")
    );
    let mut forged = contract_package.clone();
    forged.interfaces[0].methods[0].signature = nir::code_interfaces::MethodSignature::Generator(
        nir::code_interfaces::CodeFunctionSignature {
            clause_domains: vec![],
            inputs: vec![],
            output: nir::code_interfaces::ExpressionCodeInterface {
                infer_context: false,
                context: Default::default(),
                ports: Default::default(),
                defaults: Default::default(),
                result: primitive(Primitive::Bool),
            },
        },
    );
    let failure = forged.validate(&mut 10_000).unwrap_err();
    assert!(
        failure.contains("not callable") || failure.contains("contextual code"),
        "{failure}"
    );
    contract_package.validate(&mut 10_000).unwrap();
    let predicate = &mut contract_package.interfaces[0].methods[0]
        .program
        .preconditions[0];
    let K::Block { statements, .. } = &mut predicate.kind else {
        unreachable!()
    };
    statements[0].kind = K::TypeListValue(vec![Term::Rigid {
        declaration: id(99),
        sort: Sort::Type,
    }]);
    assert!(
        contract_package
            .validate(&mut 10_000)
            .unwrap_err()
            .contains("unresolved")
    );
}

#[test]
fn incomplete_type_list_concatenation_cannot_enter_iir() {
    let input = DeclarationId {
        package: 91,
        local: 1,
    };
    let term = Term::type_list_concat(vec![
        Term::TypeListValue(vec![Term::Primitive(Primitive::Bool)]),
        Term::Rigid {
            declaration: input,
            sort: Sort::ComptimeValue,
        },
    ]);
    assert!(
        completed_type(&term, &BTreeSet::new(), &mut 100)
            .unwrap_err()
            .contains("type-list concatenation")
    );
    let completed = term
        .instantiate_parameters(
            &[nir::DeclarationParameter {
                declaration: input,
                sort: Sort::ComptimeValue,
            }],
            &[Term::TypeListValue(vec![])],
        )
        .unwrap();
    completed_type(&completed, &BTreeSet::new(), &mut 100).unwrap();
}

#[test]
fn staged_callable_keys_cannot_hide_unfinished_inputs() {
    let declaration = DeclarationId {
        package: 1,
        local: 9,
    };
    let runtime = RuntimeId {
        instance: 17,
        local: 3,
    };
    for snapshot in [false, true] {
        let runtime_type = Term::NormalForm {
            head: declaration,
            sort: Sort::Type,
            arguments: vec![Term::Runtime(runtime)],
        };
        let key = |argument| {
            if snapshot {
                Term::ClosureSnapshot {
                    factory: declaration,
                    arguments: vec![argument],
                }
            } else {
                Term::FunctionValue {
                    declaration,
                    arguments: vec![argument],
                }
            }
        };
        let rigid = Term::Rigid {
            declaration,
            sort: Sort::Type,
        };
        assert!(
            completed_type(&key(rigid), &BTreeSet::new(), &mut 100)
                .unwrap_err()
                .contains("unresolved")
        );
        assert!(
            completed_type(&key(runtime_type.clone()), &BTreeSet::new(), &mut 100)
                .unwrap_err()
                .contains("unavailable runtime")
        );
        completed_type(
            &key(runtime_type.clone()),
            &BTreeSet::from([runtime]),
            &mut 100,
        )
        .unwrap();
        assert!(
            completed_type(
                &key(Term::Primitive(Primitive::Bool)),
                &BTreeSet::new(),
                &mut 1
            )
            .is_err()
        );
        completed_type(
            &key(Term::Primitive(Primitive::Bool)),
            &BTreeSet::new(),
            &mut 100,
        )
        .unwrap();
    }
}

#[test]
fn captured_comptime_values_must_materialize_before_iir_validation() {
    use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, SourceRange};
    let id = DeclarationId {
        package: 1,
        local: 900,
    };
    let key = iir::InstanceKey {
        declaration: id,
        arguments: vec![],
    };
    let boolean = Term::Primitive(Primitive::Bool);
    let mut package = iir::Package {
        roots: vec![key.clone()],
        functions: vec![iir::FunctionInstance {
            key,
            function: Function {
                declaration: id,
                name: "read".into(),
                phase: FunctionPhase::Runtime,
                inline: false,
                pure: true,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(boolean.clone()),
                },
                parameters: vec![],
                type_parameters: vec![],
                defaults: vec![],
                requirements: vec![],
                capabilities: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
                body: Expr {
                    source: SourceRange { start: 1, end: 2 },
                    ty: boolean,
                    kind: K::ComptimeValue(Term::Bool(true)),
                },
            },
        }],
        ..Default::default()
    };
    let error = package.validate(&mut 10000).unwrap_err();
    assert!(
        error.contains("captured comptime value materialization"),
        "{error}"
    );
    package.functions[0].function.body.kind = K::Literal(nir::program::Literal::Bool(true));
    package.validate(&mut 10000).unwrap();
}
