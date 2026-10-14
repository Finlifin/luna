use nir::program::{Capture, Expr, ExprKind as K, Literal, LocalId, SourceRange};
use nir::{Primitive, Term};
use std::sync::Arc;
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn id(index: u32) -> LocalId {
    LocalId { owner: 7, index }
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 0, end: 1 },
    }
}
fn signature(result: Term) -> Term {
    Term::ComptimeFunction(Box::new(Term::Function {
        parameters: vec![],
        result: Box::new(result),
    }))
}
fn frozen() -> Expr {
    let ty = Term::Closure {
        environment: vec![boolean()],
        signature: Box::new(signature(boolean())),
    };
    let closure = expr(
        ty.clone(),
        K::Closure {
            parameters: vec![],
            captures: vec![Capture {
                binding: id(1),
                ty: boolean(),
                value: Some(Box::new(expr(boolean(), K::Literal(Literal::Bool(true))))),
            }],
            body: Arc::new(expr(boolean(), K::Local(id(1)))),
        },
    );
    let body = expr(
        ty.clone(),
        K::Block {
            statements: vec![expr(
                Term::Primitive(Primitive::Void),
                K::Let {
                    binding: id(2),
                    value: Box::new(closure),
                },
            )],
            result: Some(Box::new(expr(ty.clone(), K::Local(id(2))))),
        },
    );
    expr(
        ty.clone(),
        K::Call {
            callee: Box::new(expr(
                signature(ty),
                K::Closure {
                    parameters: vec![],
                    captures: vec![],
                    body: Arc::new(body),
                },
            )),
            arguments: vec![],
            parameter_bindings: vec![],
        },
    )
}
fn validate(value: &Expr) -> Result<(), &'static str> {
    nir::completed_values::validate(value, &mut 10000)
}
#[test]
fn frozen_closures_require_closed_explicit_captures_and_private_bindings() {
    let original = frozen();
    validate(&original).unwrap();
    for mutation in 0..4 {
        let mut value = original.clone();
        let K::Call { callee, .. } = &mut value.kind else {
            unreachable!()
        };
        let K::Closure { body, .. } = &mut callee.kind else {
            unreachable!()
        };
        let K::Block { statements, result } = &mut Arc::make_mut(body).kind else {
            unreachable!()
        };
        if mutation == 0 {
            result.as_mut().unwrap().kind = K::Local(id(99));
        } else if mutation == 1 {
            statements.push(statements[0].clone());
        } else {
            let K::Let { value, .. } = &mut statements[0].kind else {
                unreachable!()
            };
            let K::Closure { captures, .. } = &mut value.kind else {
                unreachable!()
            };
            if mutation == 2 {
                captures[0].value = None;
            } else {
                captures[0].value = Some(Box::new(expr(boolean(), K::Local(id(99)))));
            }
        }
        assert!(validate(&value).is_err(), "mutation {mutation}");
    }
    validate(&original).unwrap();
}
#[test]
fn reconstruction_cannot_execute_arbitrary_declarations_or_statements() {
    let mut value = frozen();
    let K::Call { callee, .. } = &mut value.kind else {
        unreachable!()
    };
    let original = callee.clone();
    callee.kind = K::Function(nir::DeclarationId {
        package: 0,
        local: 1,
    });
    assert!(validate(&value).is_err());
    let K::Call { callee, .. } = &mut value.kind else {
        unreachable!()
    };
    *callee = original;
    let K::Closure { body, .. } = &mut callee.kind else {
        unreachable!()
    };
    let K::Block { statements, .. } = &mut Arc::make_mut(body).kind else {
        unreachable!()
    };
    statements.push(expr(boolean(), K::Literal(Literal::Bool(true))));
    assert!(validate(&value).is_err());
}
#[test]
fn completed_graph_validation_recovers_after_budget_exhaustion() {
    let value = frozen();
    assert!(nir::completed_values::validate(&value, &mut 0).is_err());
    validate(&value).unwrap();
    let mut deep = expr(boolean(), K::Literal(Literal::Bool(true)));
    for _ in 0..70 {
        deep = expr(Term::Tuple(vec![deep.ty.clone()]), K::Tuple(vec![deep]));
    }
    assert!(validate(&deep).is_err());
    validate(&value).unwrap();
}

#[test]
fn reconstruction_wrapper_cannot_claim_an_implicit_environment() {
    let mut value = frozen();
    let K::Call { callee, .. } = &mut value.kind else {
        unreachable!()
    };
    callee.ty = Term::Closure {
        environment: vec![boolean()],
        signature: Box::new(callee.ty.clone()),
    };
    assert!(validate(&value).is_err());
}

#[test]
fn wide_graph_bindings_are_ordered_and_scoped() {
    let mut statements = Vec::new();
    for index in 0..2048 {
        let value = if index == 0 {
            K::Literal(Literal::Bool(true))
        } else {
            K::Local(id(index - 1))
        };
        statements.push(expr(
            Term::Primitive(Primitive::Void),
            K::Let {
                binding: id(index),
                value: Box::new(expr(boolean(), value)),
            },
        ));
    }
    let graph = expr(
        boolean(),
        K::Block {
            statements,
            result: Some(Box::new(expr(boolean(), K::Local(id(2047))))),
        },
    );
    nir::completed_values::validate(&graph, &mut 20000).unwrap();
    assert!(nir::completed_values::validate(&graph, &mut 100).is_err());
    // Adjacent blocks can reuse private identities, but neither exports them.
    let pair = expr(
        Term::Tuple(vec![boolean(), boolean()]),
        K::Tuple(vec![graph.clone(), graph]),
    );
    nir::completed_values::validate(&pair, &mut 40000).unwrap();
    let escaped = expr(
        Term::Tuple(vec![pair.ty.clone(), boolean()]),
        K::Tuple(vec![pair, expr(boolean(), K::Local(id(2047)))]),
    );
    assert!(nir::completed_values::validate(&escaped, &mut 50000).is_err());
}
