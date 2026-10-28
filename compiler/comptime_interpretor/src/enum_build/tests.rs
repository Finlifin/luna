use super::*;
use nir::Sort;
use std::cell::Cell;
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 90, local }
}
fn p(value: Primitive) -> Term {
    Term::Primitive(value)
}
fn list(element: Term) -> Term {
    Term::NormalForm {
        head: id(1),
        sort: Sort::Type,
        arguments: vec![element],
    }
}
fn world() -> nir::program::Package {
    nir::program::Package {
        normal_forms: vec![nir::normal_forms::Head {
            role: nir::normal_forms::Role::TypeList,
            declaration: id(1),
            requirements: vec![],
            parameters: vec![nir::normal_forms::Parameter {
                declaration: id(2),
                binding: LocalId { owner: 1, index: 0 },
                kind: nir::normal_forms::Kind::Type,
                ty: p(Primitive::Type),
            }],
        }],
        ..Default::default()
    }
}
fn symbols(names: &[&str]) -> Value {
    Value::List {
        ty: list(p(Primitive::Symbol)),
        values: names
            .iter()
            .map(|name| Value::Symbol((*name).into()))
            .collect(),
    }
}
fn input(shape: &str) -> Value {
    Value::Tuple(vec![
        Value::Symbol("Generated".into()),
        symbols(&["Group"]),
        symbols(&["Group.Data"]),
        symbols(&[shape]),
        Value::List {
            ty: list(list(p(Primitive::Symbol))),
            values: vec![symbols(&["flag"])],
        },
        Value::List {
            ty: list(list(p(Primitive::Type))),
            values: vec![Value::TypeList(vec![p(Primitive::Bool)])],
        },
    ])
}
fn expression() -> Expr {
    let source = SourceRange { start: 3, end: 7 };
    Expr {
        source,
        ty: p(Primitive::Type),
        kind: K::BuildEnum(Box::new(Expr {
            source,
            ty: Term::Tuple(vec![
                p(Primitive::Symbol),
                list(p(Primitive::Symbol)),
                list(p(Primitive::Symbol)),
                list(p(Primitive::Symbol)),
                list(list(p(Primitive::Symbol))),
                list(list(p(Primitive::Type))),
            ]),
            kind: K::Local(LocalId { owner: 9, index: 0 }),
        })),
    }
}
fn env(value: Value) -> Environment {
    let mut env = Environment::new();
    env.insert(LocalId { owner: 9, index: 0 }, value);
    env
}
fn prepared<'a>(
    request: &EnumBuildRequest,
    commit: Box<dyn FnOnce() -> Result<(), Error> + 'a>,
) -> PreparedEnumBuild<'a> {
    PreparedEnumBuild {
        result: id(10),
        definitions: vec![nir::program::Enum {
            declaration: id(10),
            name: request.name.clone(),
            is_intrinsic: false,
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            groups: request.groups.clone(),
            variants: request.variants.clone(),
            encoding: None,
            symbol_encoding: request.symbol_encoding.clone(),
        }]
        .into(),
        commit,
    }
}
#[test]
fn build_enum_installs_the_exact_descriptor_and_returns_its_type() {
    let world = world();
    let commits = Cell::new(0);
    let builder = |request: &EnumBuildRequest, _: &mut u64| {
        Ok(prepared(
            request,
            Box::new(|| {
                commits.set(commits.get() + 1);
                Ok(())
            }),
        ))
    };
    let mut vm = Interpreter::for_comptime(&world, Budget::default())
        .unwrap()
        .with_enum_builder(&builder);
    let value = vm.eval(&expression(), &mut env(input("named"))).unwrap();
    let ty = Term::Nominal {
        declaration: id(10),
        arguments: vec![],
    };
    assert!(matches!(value, Flow::Value(Value::Type(ref actual)) if actual == &ty));
    assert_eq!(commits.get(), 1);
    let reflected = vm.reflect_enum(&ty).unwrap();
    assert_eq!(reflected.groups, ["Group"]);
    assert_eq!(reflected.variants[0].fields, [p(Primitive::Bool)]);
    assert!(expression().to_string().contains("(build-enum"));
}
#[test]
fn malformed_shape_and_runtime_operation_do_not_call_the_builder() {
    let world = world();
    let calls = Cell::new(0);
    let builder = |request: &EnumBuildRequest, _: &mut u64| {
        calls.set(calls.get() + 1);
        Ok(prepared(request, Box::new(|| Ok(()))))
    };
    let mut vm = Interpreter::for_comptime(&world, Budget::default())
        .unwrap()
        .with_enum_builder(&builder);
    for shape in ["unit", "tuple", "unknown"] {
        assert_eq!(
            vm.eval(&expression(), &mut env(input(shape)))
                .unwrap_err()
                .kind,
            ErrorKind::InvalidDefinition(DefinitionBuildError::VariantShape {
                name: "Group.Data".into(),
                shape: shape.into(),
                names: 1,
                types: 1
            })
        );
    }
    vm.comptime_only = false;
    assert_eq!(
        vm.eval(&expression(), &mut env(input("named")))
            .unwrap_err()
            .kind,
        ErrorKind::PhaseMismatch
    );
    assert_eq!(calls.get(), 0);
}
#[test]
fn mismatched_host_descriptor_and_failed_commit_leave_no_definition() {
    let world = world();
    let commits = Cell::new(0);
    for mismatch in [true, false] {
        let builder = |request: &EnumBuildRequest, _: &mut u64| {
            let mut result = prepared(
                request,
                Box::new(|| {
                    commits.set(commits.get() + 1);
                    Err(Error {
                        kind: ErrorKind::InvalidCallPlan,
                        source: None,
                    })
                }),
            );
            if mismatch {
                Arc::make_mut(&mut result.definitions)[0].variants[0].name = "Different".into();
            }
            Ok(result)
        };
        let mut vm = Interpreter::for_comptime(&world, Budget::default())
            .unwrap()
            .with_enum_builder(&builder);
        assert!(vm.eval(&expression(), &mut env(input("named"))).is_err());
        assert!(!vm.enums.contains_key(&id(10)));
    }
    assert_eq!(commits.get(), 1);
}
#[test]
fn host_cannot_refund_fuel_and_early_return_never_calls_it() {
    let world = world();
    let calls = Cell::new(0);
    let builder = |request: &EnumBuildRequest, remaining: &mut u64| {
        calls.set(calls.get() + 1);
        *remaining = u64::MAX;
        Ok(prepared(request, Box::new(|| Ok(()))))
    };
    let mut vm = Interpreter::for_comptime(&world, Budget::default())
        .unwrap()
        .with_enum_builder(&builder);
    let mut expression = expression();
    let K::BuildEnum(request) = &mut expression.kind else {
        unreachable!()
    };
    request.kind = K::Return(Some(Box::new(Expr {
        source: request.source,
        ty: p(Primitive::Type),
        kind: K::TypeValue(p(Primitive::Bool)),
    })));
    request.ty = p(Primitive::Never);
    expression.ty = p(Primitive::Never);
    assert!(matches!(
        vm.eval(&expression, &mut Environment::new()).unwrap(),
        Flow::Return(_)
    ));
    assert_eq!(calls.get(), 0);
    let before = vm.remaining_steps();
    vm.eval(&super::tests::expression(), &mut env(input("named")))
        .unwrap();
    assert!(vm.remaining_steps() < before);
    assert_eq!(calls.get(), 1);
}

#[test]
fn segmented_paths_reject_empty_and_embedded_separator_segments_before_host_call() {
    let world = world();
    let calls = Cell::new(0);
    let builder = |request: &EnumBuildRequest, _: &mut u64| {
        calls.set(calls.get() + 1);
        Ok(prepared(request, Box::new(|| Ok(()))))
    };
    let mut vm = Interpreter::for_comptime(&world, Budget::default())
        .unwrap()
        .with_enum_builder(&builder);
    for segments in [vec![], vec![""], vec!["Group.Data"], vec!["Group", ""]] {
        let Value::Tuple(mut values) = input("named") else {
            unreachable!()
        };
        values[2] = Value::List {
            ty: list(list(p(Primitive::Symbol))),
            values: vec![symbols(&segments)],
        };
        assert_eq!(
            vm.eval(&expression(), &mut env(Value::Tuple(values)))
                .unwrap_err()
                .kind,
            ErrorKind::InvalidDefinition(DefinitionBuildError::InvalidPath)
        );
    }
    assert_eq!(calls.get(), 0);
    let Value::Tuple(mut values) = input("named") else {
        unreachable!()
    };
    values[1] = Value::List {
        ty: list(list(p(Primitive::Symbol))),
        values: vec![symbols(&["Group"])],
    };
    values[2] = Value::List {
        ty: list(list(p(Primitive::Symbol))),
        values: vec![symbols(&["Group", "Data"])],
    };
    vm.eval(&expression(), &mut env(Value::Tuple(values)))
        .unwrap();
    assert_eq!(calls.get(), 1);
}

#[test]
fn duplicate_payload_diagnostic_does_not_publish_or_poison_recovery() {
    let world = world();
    let calls = Cell::new(0);
    let builder = |request: &EnumBuildRequest, _: &mut u64| {
        calls.set(calls.get() + 1);
        Ok(prepared(request, Box::new(|| Ok(()))))
    };
    let mut vm = Interpreter::for_comptime(&world, Budget::default())
        .unwrap()
        .with_enum_builder(&builder);
    let Value::Tuple(mut values) = input("named") else {
        unreachable!()
    };
    values[4] = Value::List {
        ty: list(list(p(Primitive::Symbol))),
        values: vec![symbols(&["flag", "flag"])],
    };
    values[5] = Value::List {
        ty: list(list(p(Primitive::Type))),
        values: vec![Value::TypeList(vec![
            p(Primitive::Bool),
            p(Primitive::Bool),
        ])],
    };
    let error = vm
        .eval(&expression(), &mut env(Value::Tuple(values)))
        .unwrap_err();
    assert_eq!(
        error.kind,
        ErrorKind::InvalidDefinition(DefinitionBuildError::DuplicateField {
            owner: "Group.Data".into(),
            name: "flag".into(),
        })
    );
    assert_eq!(error.source, Some(expression().source));
    assert_eq!(calls.get(), 0);
    assert!(!vm.enums.contains_key(&id(10)));
    vm.eval(&expression(), &mut env(input("named"))).unwrap();
    assert_eq!(calls.get(), 1);
    assert!(vm.enums.contains_key(&id(10)));
}

#[test]
fn ordinary_enum_builder_rejects_host_added_symbol_encoding_before_commit() {
    let world = world();
    let commits = Cell::new(0);
    let altered = Cell::new(true);
    let builder = |request: &EnumBuildRequest, _: &mut u64| {
        let mut result = prepared(
            request,
            Box::new(|| {
                commits.set(commits.get() + 1);
                Ok(())
            }),
        );
        if altered.get() {
            Arc::make_mut(&mut result.definitions)[0].symbol_encoding = Some(
                nir::enum_encoding::SymbolRepresentation::new(
                    vec![nir::enum_encoding::SymbolVariant {
                        name: "Group.Data".into(),
                        value: "different-abi".into(),
                    }],
                    &mut comptime_runtime::Budget::default(),
                )
                .unwrap(),
            );
        }
        Ok(result)
    };
    let Value::Tuple(mut values) = input("unit") else {
        unreachable!()
    };
    values[4] = Value::List {
        ty: list(list(p(Primitive::Symbol))),
        values: vec![symbols(&[])],
    };
    values[5] = Value::List {
        ty: list(list(p(Primitive::Type))),
        values: vec![Value::TypeList(vec![])],
    };
    let input = Value::Tuple(values);
    let mut vm = Interpreter::for_comptime(&world, Budget::default())
        .unwrap()
        .with_enum_builder(&builder);
    let error = vm.eval(&expression(), &mut env(input.clone())).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Type);
    assert_eq!(commits.get(), 0);
    assert!(!vm.enums.contains_key(&id(10)));
    altered.set(false);
    vm.eval(&expression(), &mut env(input)).unwrap();
    assert_eq!(commits.get(), 1);
    assert!(vm.enums[&id(10)].symbol_encoding.is_none());
}

#[test]
fn ordinary_enum_builder_rejects_host_added_metadata_before_commit() {
    for case in 0..5 {
        let world = world();
        let commits = Cell::new(0);
        let altered = Cell::new(true);
        let builder = |request: &EnumBuildRequest, _: &mut u64| {
            let mut result = prepared(
                request,
                Box::new(|| {
                    commits.set(commits.get() + 1);
                    Ok(())
                }),
            );
            if altered.get() {
                let definition = &mut Arc::make_mut(&mut result.definitions)[0];
                match case {
                    0 => definition.is_intrinsic = true,
                    1 => definition.type_parameters.push(id(20)),
                    2 => definition.parameter_types.push(p(Primitive::Type)),
                    3 => definition
                        .requirements
                        .push(nir::program::TypePredicate::Bool(true)),
                    4 => definition
                        .capabilities
                        .push(nir::program::TraitRequirement {
                            interface_arguments: vec![],
                            subject: p(Primitive::Bool),
                            interface: id(21),
                        }),
                    _ => unreachable!(),
                }
            }
            Ok(result)
        };
        let mut vm = Interpreter::for_comptime(&world, Budget::default())
            .unwrap()
            .with_enum_builder(&builder);
        let error = vm
            .eval(&expression(), &mut env(input("named")))
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Type, "case {case}");
        assert_eq!(error.source, Some(expression().source));
        assert_eq!(commits.get(), 0);
        assert!(!vm.enums.contains_key(&id(10)));
        altered.set(false);
        vm.eval(&expression(), &mut env(input("named"))).unwrap();
        assert_eq!(commits.get(), 1);
    }
}

#[test]
fn ordinary_struct_builder_rejects_host_added_metadata_before_commit() {
    // Reuse the same meta.List world to exercise the sibling descriptor protocol.
    for case in 0..4 {
        let world = world();
        let commits = Cell::new(0);
        let altered = Cell::new(true);
        let builder = |request: &StructBuildRequest, _: &mut u64| {
            let mut definition = nir::program::Struct {
                declaration: id(10),
                name: request.name.clone(),
                fields: request
                    .fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.clone()))
                    .collect(),
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
            };
            if altered.get() {
                match case {
                    0 => definition.type_parameters.push(id(20)),
                    1 => definition.parameter_types.push(p(Primitive::Type)),
                    2 => definition
                        .requirements
                        .push(nir::program::TypePredicate::Bool(true)),
                    3 => definition
                        .capabilities
                        .push(nir::program::TraitRequirement {
                            interface_arguments: vec![],
                            subject: p(Primitive::Bool),
                            interface: id(21),
                        }),
                    _ => unreachable!(),
                }
            }
            Ok(PreparedStructBuild {
                result: id(10),
                definitions: vec![definition].into(),
                commit: Box::new(|| {
                    commits.set(commits.get() + 1);
                    Ok(())
                }),
            })
        };
        let source = SourceRange { start: 3, end: 7 };
        let expr = Expr {
            source,
            ty: p(Primitive::Type),
            kind: K::BuildStruct(Box::new(Expr {
                source,
                ty: Term::Tuple(vec![
                    p(Primitive::Symbol),
                    list(p(Primitive::Symbol)),
                    list(p(Primitive::Type)),
                ]),
                kind: K::Local(LocalId { owner: 9, index: 0 }),
            })),
        };
        let input = Value::Tuple(vec![
            Value::Symbol("Generated".into()),
            symbols(&["flag"]),
            Value::TypeList(vec![p(Primitive::Bool)]),
        ]);
        let mut vm = Interpreter::for_comptime(&world, Budget::default())
            .unwrap()
            .with_struct_builder(&builder);
        let error = vm.eval(&expr, &mut env(input.clone())).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Type, "case {case}");
        assert_eq!(error.source, Some(source));
        assert_eq!(commits.get(), 0);
        altered.set(false);
        assert!(
            matches!(vm.eval(&expr, &mut env(input)).unwrap(), Flow::Value(Value::Type(Term::Nominal { declaration, .. })) if declaration == id(10))
        );
        assert_eq!(commits.get(), 1);
    }
}

fn symbol_input(codes: &[&str]) -> Value {
    let Value::Tuple(mut values) = input("unit") else {
        unreachable!()
    };
    values[4] = Value::List {
        ty: list(list(p(Primitive::Symbol))),
        values: vec![symbols(&[])],
    };
    values[5] = Value::List {
        ty: list(list(p(Primitive::Type))),
        values: vec![Value::TypeList(vec![])],
    };
    values.push(symbols(codes));
    Value::Tuple(values)
}
fn symbol_expression() -> Expr {
    let mut expr = expression();
    let K::BuildEnum(inner) = &mut expr.kind else {
        unreachable!()
    };
    let Term::Tuple(types) = &mut inner.ty else {
        unreachable!()
    };
    types.push(list(p(Primitive::Symbol)));
    expr
}
#[test]
fn symbol_enum_builder_installs_encoding_and_rejects_host_replacement() {
    let world = world();
    let commits = Cell::new(0);
    let tamper = Cell::new(true);
    let builder = |request: &EnumBuildRequest, _: &mut u64| {
        assert!(request.to_string().contains("(symbol-encoding"));
        let mut result = prepared(
            request,
            Box::new(|| {
                commits.set(commits.get() + 1);
                Ok(())
            }),
        );
        if tamper.get() {
            Arc::make_mut(&mut result.definitions)[0].symbol_encoding = Some(
                nir::enum_encoding::SymbolRepresentation::new(
                    vec![nir::enum_encoding::SymbolVariant {
                        name: "Group.Data".into(),
                        value: "wrong".into(),
                    }],
                    &mut comptime_runtime::Budget::default(),
                )
                .unwrap(),
            );
        }
        Ok(result)
    };
    let mut vm = Interpreter::for_comptime(&world, Budget::default())
        .unwrap()
        .with_enum_builder(&builder);
    let error = vm
        .eval(&symbol_expression(), &mut env(symbol_input(&["wire"])))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Type);
    assert_eq!(commits.get(), 0);
    assert!(!vm.enums.contains_key(&id(10)));
    tamper.set(false);
    vm.eval(&symbol_expression(), &mut env(symbol_input(&["wire"])))
        .unwrap();
    assert_eq!(commits.get(), 1);
    assert_eq!(
        vm.enums[&id(10)]
            .symbol_encoding
            .as_ref()
            .unwrap()
            .encode(0),
        Some("wire")
    );
}
#[test]
fn symbol_enum_builder_rejects_code_count_before_calling_host() {
    let world = world();
    let calls = Cell::new(0);
    let builder = |request: &EnumBuildRequest, _: &mut u64| {
        calls.set(calls.get() + 1);
        Ok(prepared(request, Box::new(|| Ok(()))))
    };
    let mut vm = Interpreter::for_comptime(&world, Budget::default())
        .unwrap()
        .with_enum_builder(&builder);
    for codes in [&[][..], &["a", "b"][..]] {
        let error = vm
            .eval(&symbol_expression(), &mut env(symbol_input(codes)))
            .unwrap_err();
        assert_eq!(
            error.kind,
            ErrorKind::InvalidDefinition(DefinitionBuildError::SymbolCodeCount {
                variants: 1,
                codes: codes.len()
            })
        );
        assert_eq!(error.source, Some(symbol_expression().source));
        assert_eq!(calls.get(), 0);
    }
    vm.eval(&symbol_expression(), &mut env(symbol_input(&["wire"])))
        .unwrap();
    assert_eq!(calls.get(), 1);
}
