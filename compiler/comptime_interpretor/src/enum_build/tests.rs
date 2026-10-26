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
            symbol_encoding: None,
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
            ErrorKind::Type
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
