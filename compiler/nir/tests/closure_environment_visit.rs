use nir::program::{
    CallArgument, Capture, EffectHandler, Expr, ExprKind as K, Literal, LocalId, MatchArm, Pattern,
    SourceRange,
};
use nir::{DeclarationId, Parameter, Primitive, Sort, Term};
use std::collections::BTreeSet;
use std::sync::Arc;

fn void() -> Term {
    Term::Primitive(Primitive::Void)
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn expr(at: u32, ty: Term, kind: K) -> Expr {
    Expr {
        source: SourceRange {
            start: at,
            end: at + 1,
        },
        ty,
        kind,
    }
}
fn unit(at: u32) -> Expr {
    expr(at, void(), K::Unit)
}
fn flag(at: u32) -> Expr {
    expr(at, boolean(), K::Literal(Literal::Bool(true)))
}
fn tree() -> Expr {
    let closure = expr(
        1,
        Term::Closure {
            environment: vec![boolean()],
            signature: Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(void()),
            }),
        },
        K::Closure {
            parameters: vec![],
            captures: vec![Capture {
                binding: LocalId { owner: 1, index: 0 },
                ty: boolean(),
                value: Some(Box::new(flag(3))),
            }],
            body: Arc::new(unit(2)),
        },
    );
    let call = expr(
        4,
        void(),
        K::Call {
            callee: Box::new(expr(
                5,
                Term::Function {
                    parameters: vec![Parameter {
                        sort: Sort::Runtime,
                        ty: boolean(),
                    }],
                    result: Box::new(void()),
                },
                K::Function(DeclarationId {
                    package: 1,
                    local: 1,
                }),
            )),
            arguments: vec![CallArgument {
                parameter: 0,
                value: flag(6),
                is_default: true,
            }],
            parameter_bindings: vec![],
        },
    );
    let handler = expr(
        7,
        void(),
        K::EffectHandle {
            computation: Box::new(unit(8)),
            handlers: vec![EffectHandler {
                operation: nir::effects::Operation {
                    declaration: DeclarationId {
                        package: 1,
                        local: 2,
                    },
                    arguments: vec![],
                },
                abi: nir::effects::Abi::Direct,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(void()),
                },
                arms: vec![MatchArm {
                    pattern: Pattern::Guard {
                        pattern: Box::new(Pattern::Unit),
                        condition: Box::new(flag(11)),
                    },
                    guard: Some(flag(9)),
                    body: unit(10),
                }],
                fallback: None,
            }],
            outer: vec![],
        },
    );
    expr(
        0,
        void(),
        K::Block {
            statements: vec![closure, call, handler],
            result: None,
        },
    )
}

#[test]
fn closure_environment_keeps_initializers_defaults_and_handler_accesses() {
    let tree = tree();
    let mut seen = BTreeSet::new();
    nir::visit::expression_in_closure_environment(&tree, &mut 100, |expr, _| {
        seen.insert(expr.source.start);
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, (0..12).filter(|id| *id != 2).collect());
    let mut full = BTreeSet::new();
    nir::visit::expression(&tree, &mut 100, |expr, _| {
        full.insert(expr.source.start);
        Ok(())
    })
    .unwrap();
    assert_eq!(full, (0..12).collect());
    let mut scoped = BTreeSet::new();
    nir::visit::expression_in_scope(&tree, &mut 100, |expr, _| {
        scoped.insert(expr.source.start);
        Ok(())
    })
    .unwrap();
    assert_eq!(scoped, BTreeSet::from([0, 1, 3, 4, 5, 7, 8]));
}

#[test]
fn closure_environment_budget_failure_does_not_visit_unadmitted_nodes() {
    let tree = tree();
    let mut count = 0;
    assert!(
        nir::visit::expression_in_closure_environment(&tree, &mut 0, |_, _| {
            count += 1;
            Ok(())
        })
        .is_err()
    );
    assert_eq!(count, 0);
    nir::visit::expression_in_closure_environment(&tree, &mut 100, |_, _| {
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 11);
}

#[test]
fn mutable_closure_environment_preserves_bodies_and_rewrites_formation() {
    let original = tree();
    let mut changed = original.clone();
    let mut seen = BTreeSet::new();
    nir::visit::expression_mut_in_closure_environment(&mut changed, &mut 100, |expr, _| {
        seen.insert(expr.source.start);
        expr.source.end += 100;
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, (0..12).filter(|id| *id != 2).collect());
    fn body(tree: &Expr) -> &Arc<Expr> {
        let K::Block { statements, .. } = &tree.kind else {
            panic!()
        };
        let K::Closure { body, .. } = &statements[0].kind else {
            panic!()
        };
        body
    }
    assert!(Arc::ptr_eq(body(&original), body(&changed)));
    assert_eq!(body(&changed).source.end, 3);
    nir::visit::expression(&changed, &mut 100, |expr, _| {
        assert_eq!(
            expr.source.end,
            expr.source.start + if expr.source.start == 2 { 1 } else { 101 }
        );
        Ok(())
    })
    .unwrap();
    nir::visit::expression_mut(&mut changed, &mut 100, |expr, _| {
        expr.source.end += 1;
        Ok(())
    })
    .unwrap();
    assert!(!Arc::ptr_eq(body(&original), body(&changed)));
    assert_eq!(body(&original).source.end, 3);
    assert_eq!(body(&changed).source.end, 4);
}

#[test]
fn mutable_closure_environment_budget_failure_recovers_on_fresh_input() {
    let mut tree = tree();
    let original = tree.clone();
    let mut count = 0;
    assert!(
        nir::visit::expression_mut_in_closure_environment(&mut tree, &mut 0, |_, _| {
            count += 1;
            Ok(())
        })
        .is_err()
    );
    assert_eq!(count, 0);
    assert_eq!(tree, original);
    nir::visit::expression_mut_in_closure_environment(&mut tree, &mut 100, |_, _| {
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 11);
}

#[test]
fn mutable_handler_scope_keeps_nested_arm_and_closure_control_separate() {
    let mut tree = tree();
    let mut seen = BTreeSet::new();
    nir::visit::expression_mut_in_handler_scope(&mut tree, &mut 100, |expr, _| {
        seen.insert(expr.source.start);
        expr.source.end += 100;
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, BTreeSet::from([0, 1, 3, 4, 5, 6, 7, 8]));
    nir::visit::expression(&tree, &mut 100, |expr, _| {
        assert_eq!(
            expr.source.end,
            expr.source.start
                + if seen.contains(&expr.source.start) {
                    101
                } else {
                    1
                }
        );
        Ok(())
    })
    .unwrap();
}
