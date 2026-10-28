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
