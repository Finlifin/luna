use comptime_interpretor::{Budget, ErrorKind, Event, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Package, SourceRange};
use nir::{DeclarationId, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn p(value: P) -> Term {
    Term::Primitive(value)
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 1, end: 2 },
    }
}
fn value(ty: Term) -> Expr {
    expr(p(P::Type), K::TypeValue(ty))
}
fn function(local: u32, body: Expr) -> Function {
    Function {
        declaration: id(local),
        name: format!("Type{local}"),
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
            result: Box::new(p(P::Type)),
        },
        parameters: vec![],
        defaults: vec![],
        body,
    }
}
fn call(local: u32) -> Expr {
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(p(P::Type)),
    }
    .with_comptime_phase(true);
    expr(
        p(P::Type),
        K::Call {
            parameter_bindings: vec![],
            callee: Box::new(expr(signature, K::Function(id(local)))),
            arguments: vec![],
        },
    )
}

#[test]
fn fields_execute_once_in_order_and_construct_types_not_tuple_values() {
    let package = Package {
        functions: vec![
            function(0, expr(p(P::Type), K::TupleType(vec![call(1), call(2)]))),
            function(1, value(p(P::Bool))),
            function(2, value(p(P::Char))),
        ],
        ..Default::default()
    };
    let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap(),
        Value::Type(Term::Tuple(vec![p(P::Bool), p(P::Char)]))
    );
    let entered: Vec<_> = interpreter
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(entered, vec![id(0), id(1), id(2)]);
}

#[test]
fn malformed_fields_fail_and_a_later_valid_invocation_recovers() {
    for malformed in [
        expr(p(P::Bool), K::TupleType(vec![value(p(P::Bool))])),
        expr(
            p(P::Type),
            K::TupleType(vec![expr(
                p(P::Bool),
                K::Literal(nir::program::Literal::Bool(true)),
            )]),
        ),
        expr(p(P::Type), K::TupleType(vec![value(Term::Bool(true))])),
    ] {
        let package = Package {
            functions: vec![function(0, malformed), function(1, value(p(P::Char)))],
            ..Default::default()
        };
        let mut interpreter = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter.invoke(id(0), vec![]).unwrap_err().kind,
            ErrorKind::Type
        );
        assert_eq!(
            interpreter.invoke(id(1), vec![]).unwrap(),
            Value::Type(p(P::Char))
        );
    }
}

#[test]
fn tuple_type_construction_is_budgeted_and_specialization_visits_fields() {
    let parameter = id(9);
    let body = expr(
        p(P::Type),
        K::TupleType(vec![
            value(Term::Rigid {
                declaration: parameter,
                sort: Sort::Type
            });
            16
        ]),
    );
    let specialized = body.instantiate_types(&[parameter], &[p(P::Bool)]).unwrap();
    assert_ne!(body, specialized);
    assert!(body.to_string().contains("tuple-type-value"));
    let package = Package {
        functions: vec![function(0, specialized)],
        ..Default::default()
    };
    let mut interpreter = Interpreter::new(
        &package,
        Budget {
            steps: 8,
            depth: 128,
        },
    )
    .unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    let mut interpreter = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap(),
        Value::Type(Term::Tuple(vec![p(P::Bool); 16]))
    );
}

#[test]
fn a_failed_field_does_not_execute_later_fields() {
    let bad_width = expr(
        p(P::Type),
        K::BitVecType(Box::new(expr(
            p(P::Integer),
            K::Literal(nir::program::Literal::Integer("-1".into())),
        ))),
    );
    let package = Package {
        functions: vec![
            function(0, expr(p(P::Type), K::TupleType(vec![bad_width, call(1)]))),
            function(1, value(p(P::Bool))),
        ],
        ..Default::default()
    };
    let mut interpreter = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::InvalidBitWidth
    );
    assert!(!interpreter.events().contains(&Event::EnterFunction(id(1))));
    assert_eq!(
        interpreter.invoke(id(1), vec![]).unwrap(),
        Value::Type(p(P::Bool))
    );
}
