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

fn signature(comptime: bool) -> Term {
    Term::Function {
        parameters: vec![],
        result: Box::new(p(P::Bool)),
    }
    .with_comptime_phase(comptime)
}

#[test]
fn environment_fields_execute_once_before_the_signature_and_keep_phase() {
    for comptime in [false, true] {
        let package = Package {
            functions: vec![
                function(
                    0,
                    expr(
                        p(P::Type),
                        K::ClosureType {
                            environment: vec![call(1), call(2)],
                            signature: Box::new(call(3)),
                        },
                    ),
                ),
                function(1, value(p(P::Bool))),
                function(2, value(p(P::Char))),
                function(3, value(signature(comptime))),
            ],
            ..Default::default()
        };
        let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter.invoke(id(0), vec![]).unwrap(),
            Value::Type(Term::Closure {
                environment: vec![p(P::Bool), p(P::Char)],
                signature: Box::new(signature(comptime))
            })
        );
        let entered: Vec<_> = interpreter
            .events()
            .iter()
            .filter_map(|event| match event {
                Event::EnterFunction(id) => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(entered, vec![id(0), id(1), id(2), id(3)]);
    }
}

#[test]
fn invalid_environments_and_pointer_operands_reject_and_recover() {
    let repeated = Term::Closure {
        environment: vec![p(P::Bool)],
        signature: Box::new(signature(false)),
    };
    for body in [
        expr(
            p(P::Type),
            K::ClosureType {
                environment: vec![value(p(P::Bool))],
                signature: Box::new(value(repeated)),
            },
        ),
        expr(
            p(P::Type),
            K::ClosureType {
                environment: vec![value(p(P::Bool))],
                signature: Box::new(value(p(P::Char))),
            },
        ),
        expr(
            p(P::Type),
            K::ClosureType {
                environment: vec![expr(
                    p(P::Bool),
                    K::Literal(nir::program::Literal::Bool(true)),
                )],
                signature: Box::new(call(1)),
            },
        ),
        expr(
            p(P::Type),
            K::PointerType(Box::new(value(Term::Bool(true)))),
        ),
        expr(p(P::Bool), K::PointerType(Box::new(value(p(P::Char))))),
    ] {
        let package = Package {
            functions: vec![function(0, body), function(1, value(signature(false)))],
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
            Value::Type(signature(false))
        );
    }
}

#[test]
fn empty_contexts_eliminate_and_pointer_specialization_preserves_the_template() {
    let parameter = id(9);
    let body = expr(
        p(P::Type),
        K::ClosureType {
            environment: vec![],
            signature: Box::new(expr(
                p(P::Type),
                K::PointerType(Box::new(value(Term::Rigid {
                    declaration: parameter,
                    sort: Sort::Type,
                }))),
            )),
        },
    );
    let specialized = body.instantiate_types(&[parameter], &[p(P::Bool)]).unwrap();
    assert_ne!(body, specialized);
    let package = Package {
        functions: vec![function(0, specialized)],
        ..Default::default()
    };
    assert_eq!(
        Interpreter::new(&package, Budget::default())
            .unwrap()
            .invoke(id(0), vec![])
            .unwrap(),
        Value::Type(Term::Pointer(Box::new(p(P::Bool))))
    );
}

#[test]
fn environment_construction_is_budgeted() {
    let parameter = id(9);
    let body = expr(
        p(P::Type),
        K::ClosureType {
            environment: vec![
                value(Term::Rigid {
                    declaration: parameter,
                    sort: Sort::Type
                });
                16
            ],
            signature: Box::new(value(signature(true))),
        },
    );
    let specialized = body.instantiate_types(&[parameter], &[p(P::Bool)]).unwrap();
    let package = Package {
        functions: vec![function(0, specialized)],
        ..Default::default()
    };
    assert_eq!(
        Interpreter::new(
            &package,
            Budget {
                steps: 8,
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
        Value::Type(Term::Closure {
            environment: vec![p(P::Bool); 16],
            signature: Box::new(signature(true))
        })
    );
}
