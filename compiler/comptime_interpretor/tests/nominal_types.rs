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
        inline: false, pure: false,
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

fn record(requirements: Vec<nir::program::TypePredicate>) -> nir::program::Struct {
    nir::program::Struct {
        declaration: id(20),
        name: "Box".into(),
        type_parameters: vec![id(9)],
        requirements,
        capabilities: vec![],
        fields: vec![(
            "value".into(),
            Term::Rigid {
                declaration: id(9),
                sort: Sort::Type,
            },
        )],
    }
}
fn nominal(argument: Expr) -> Expr {
    expr(
        p(P::Type),
        K::NominalType {
            declaration: id(20),
            arguments: vec![argument],
        },
    )
}

#[test]
fn evaluated_arguments_are_checked_against_the_nominal_contract_before_escape() {
    let package = Package {
        structs: vec![record(vec![nir::program::TypePredicate::Equal(
            Term::Rigid {
                declaration: id(9),
                sort: Sort::Type,
            },
            p(P::Bool),
        )])],
        functions: vec![
            function(0, nominal(call(1))),
            function(1, value(p(P::Char))),
            function(2, nominal(value(p(P::Bool)))),
        ],
        ..Default::default()
    };
    let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::TypeContractUnresolved
    );
    let entered: Vec<_> = interpreter
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(entered, vec![id(0), id(1)]);
    assert_eq!(
        interpreter.invoke(id(2), vec![]).unwrap(),
        Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![p(P::Bool)]
        })
    );
}

#[test]
fn missing_descriptors_arity_and_non_type_inputs_do_not_construct_nominals() {
    for body in [
        expr(
            p(P::Type),
            K::NominalType {
                declaration: id(99),
                arguments: vec![value(p(P::Bool))],
            },
        ),
        expr(
            p(P::Type),
            K::NominalType {
                declaration: id(20),
                arguments: vec![],
            },
        ),
        nominal(expr(
            p(P::Bool),
            K::Literal(nir::program::Literal::Bool(true)),
        )),
        nominal(value(Term::Bool(true))),
    ] {
        let package = Package {
            structs: vec![record(vec![])],
            functions: vec![function(0, body), function(1, nominal(value(p(P::Bool))))],
            ..Default::default()
        };
        let mut interpreter = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter.invoke(id(0), vec![]).unwrap_err().kind,
            ErrorKind::Type
        );
        assert!(interpreter.invoke(id(1), vec![]).is_ok());
    }
}

#[test]
fn specialization_preserves_the_template_and_bounded_construction_recovers() {
    let body = nominal(value(Term::Rigid {
        declaration: id(9),
        sort: Sort::Type,
    }));
    let specialized = body.instantiate_types(&[id(9)], &[p(P::Char)]).unwrap();
    assert_ne!(body, specialized);
    let package = Package {
        structs: vec![record(vec![])],
        functions: vec![function(0, specialized)],
        ..Default::default()
    };
    assert_eq!(
        Interpreter::new(
            &package,
            Budget {
                steps: 1,
                depth: 128
            }
        )
        .unwrap()
        .invoke(id(0), vec![])
        .unwrap_err()
        .kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(
        Interpreter::new(&package, Budget::default())
            .unwrap()
            .invoke(id(0), vec![])
            .unwrap(),
        Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![p(P::Char)]
        })
    );
}
