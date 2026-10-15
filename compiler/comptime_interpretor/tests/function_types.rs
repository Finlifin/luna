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
fn constructing_callable_types_preserves_phase_order_and_parameter_types() {
    for comptime in [false, true] {
        let body = expr(
            p(P::Type),
            K::FunctionResultType {
                signature: Box::new(expr(
                    p(P::Type),
                    K::FunctionType {
                        comptime,
                        parameters: vec![call(1)],
                    },
                )),
                result: Box::new(call(2)),
            },
        );
        let package = Package {
            functions: vec![
                function(0, body),
                function(1, value(p(P::Bool))),
                function(2, value(p(P::Char))),
            ],
            ..Default::default()
        };
        let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let ty = Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Runtime,
                ty: p(P::Bool),
            }],
            result: Box::new(p(P::Char)),
        }
        .with_comptime_phase(comptime);
        assert_eq!(interpreter.invoke(id(0), vec![]).unwrap(), Value::Type(ty));
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
}

#[test]
fn malformed_signatures_fail_before_result_evaluation_and_recover() {
    for bad in [
        expr(
            p(P::Type),
            K::FunctionResultType {
                signature: Box::new(value(p(P::Bool))),
                result: Box::new(call(1)),
            },
        ),
        expr(
            p(P::Type),
            K::FunctionType {
                comptime: false,
                parameters: vec![expr(
                    p(P::Bool),
                    K::Literal(nir::program::Literal::Bool(true)),
                )],
            },
        ),
        expr(
            p(P::Bool),
            K::FunctionType {
                comptime: false,
                parameters: vec![],
            },
        ),
    ] {
        let package = Package {
            functions: vec![function(0, bad), function(1, value(p(P::Char)))],
            ..Default::default()
        };
        let mut interpreter = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter.invoke(id(0), vec![]).unwrap_err().kind,
            ErrorKind::Type
        );
        assert!(!interpreter.events().contains(&Event::EnterFunction(id(1))));
        assert_eq!(
            interpreter.invoke(id(1), vec![]).unwrap(),
            Value::Type(p(P::Char))
        );
    }
}

#[test]
fn computed_signature_specialization_is_budgeted_and_preserves_its_template() {
    let parameter = id(9);
    let rigid = Term::Rigid {
        declaration: parameter,
        sort: Sort::Type,
    };
    let body = expr(
        p(P::Type),
        K::FunctionResultType {
            signature: Box::new(expr(
                p(P::Type),
                K::FunctionType {
                    comptime: true,
                    parameters: vec![value(rigid.clone()); 16],
                },
            )),
            result: Box::new(value(rigid)),
        },
    );
    let copy = body.instantiate_types(&[parameter], &[p(P::Bool)]).unwrap();
    assert_ne!(body, copy);
    let package = Package {
        functions: vec![function(0, copy)],
        ..Default::default()
    };
    let mut limited = Interpreter::new(
        &package,
        Budget {
            steps: 8,
            depth: 128,
        },
    )
    .unwrap();
    assert_eq!(
        limited.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    let expected = Term::Function {
        parameters: vec![
            nir::Parameter {
                sort: Sort::Runtime,
                ty: p(P::Bool)
            };
            16
        ],
        result: Box::new(p(P::Bool)),
    }
    .with_comptime_phase(true);
    assert_eq!(
        Interpreter::new(&package, Budget::default())
            .unwrap()
            .invoke(id(0), vec![])
            .unwrap(),
        Value::Type(expected)
    );
}

#[test]
fn result_replacement_preserves_captured_environment_and_callable_phase() {
    for comptime in [false, true] {
        let input = Term::Closure {
            environment: vec![p(P::Bool), p(P::Char)],
            signature: Box::new(
                Term::Function {
                    parameters: vec![],
                    result: Box::new(p(P::Bool)),
                }
                .with_comptime_phase(comptime),
            ),
        };
        let package = Package {
            functions: vec![function(
                0,
                expr(
                    p(P::Type),
                    K::FunctionResultType {
                        signature: Box::new(value(input)),
                        result: Box::new(value(p(P::Char))),
                    },
                ),
            )],
            ..Default::default()
        };
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke(id(0), vec![])
                .unwrap(),
            Value::Type(Term::Closure {
                environment: vec![p(P::Bool), p(P::Char)],
                signature: Box::new(
                    Term::Function {
                        parameters: vec![],
                        result: Box::new(p(P::Char))
                    }
                    .with_comptime_phase(comptime)
                ),
            })
        );
    }
}
