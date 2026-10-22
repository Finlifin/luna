use super::*;

fn expression(kind: K, ty: Term) -> n::Expr {
    n::Expr {
        kind,
        ty,
        source: n::SourceRange { start: 0, end: 0 },
    }
}
fn boolean(kind: K) -> n::Expr {
    expression(kind, p(P::Bool))
}
fn conditional(condition: &n::Expr, yes: &n::Expr, no: &n::Expr) -> n::Expr {
    let block = |value: &n::Expr| {
        boolean(K::Block {
            statements: vec![],
            result: Some(Box::new(value.clone())),
        })
    };
    boolean(K::If {
        condition: Box::new(condition.clone()),
        then_branch: Box::new(block(yes)),
        else_branch: Some(Box::new(block(no))),
    })
}

fn foreign_comparison() -> n::Expr {
    boolean(K::ScalarGreater(
        Box::new(expression(
            K::Local(n::LocalId { owner: 0, index: 2 }),
            p(P::I32),
        )),
        Box::new(expression(
            K::Literal(n::Literal::SignedInteger(0)),
            p(P::I32),
        )),
    ))
}

fn subject() -> n::LocalId {
    n::LocalId { owner: 0, index: 0 }
}
fn atom(ty: P) -> n::Expr {
    boolean(K::TypeEqual(
        Box::new(expression(K::Local(subject()), p(P::Type))),
        Box::new(expression(K::TypeValue(p(ty)), p(P::Type))),
    ))
}
fn evaluate(expr: &n::Expr, ty: &Term, flag: bool, foreign: bool) -> bool {
    match &expr.kind {
        K::Literal(n::Literal::Bool(value)) => *value,
        K::Local(_) => flag,
        K::TypeEqual(_, expected) => {
            let K::TypeValue(expected) = &expected.kind else {
                unreachable!()
            };
            ty == expected
        }
        // The independent integer input is +1 when foreign is true, -1 otherwise.
        K::ScalarGreater(_, _) => foreign,
        K::Not(value) => !evaluate(value, ty, flag, foreign),
        K::And(left, right) => {
            evaluate(left, ty, flag, foreign) && evaluate(right, ty, flag, foreign)
        }
        K::Or(left, right) => {
            evaluate(left, ty, flag, foreign) || evaluate(right, ty, flag, foreign)
        }
        K::ScalarEqual(left, right) => {
            evaluate(left, ty, flag, foreign) == evaluate(right, ty, flag, foreign)
        }
        K::If {
            condition,
            then_branch,
            else_branch: Some(no),
        } => evaluate(
            if evaluate(condition, ty, flag, foreign) {
                then_branch
            } else {
                no
            },
            ty,
            flag,
            foreign,
        ),
        K::Block {
            statements,
            result: Some(value),
        } if statements.is_empty() => evaluate(value, ty, flag, foreign),
        _ => unreachable!(),
    }
}

#[test]
fn type_guard_evidence_is_sound_for_finite_boolean_models() {
    let base = vec![
        foreign_comparison(),
        atom(P::Bool),
        atom(P::Char),
        boolean(K::Local(n::LocalId { owner: 0, index: 1 })),
        boolean(K::Literal(n::Literal::Bool(false))),
        boolean(K::Literal(n::Literal::Bool(true))),
    ];
    let mut shallow = base.clone();
    for left in &base {
        shallow.push(boolean(K::Not(Box::new(left.clone()))));
        for right in &base {
            shallow.push(boolean(K::And(
                Box::new(left.clone()),
                Box::new(right.clone()),
            )));
            shallow.push(boolean(K::Or(
                Box::new(left.clone()),
                Box::new(right.clone()),
            )));
        }
    }
    let mut cases = shallow.clone();
    for left in &shallow {
        cases.push(boolean(K::Not(Box::new(left.clone()))));
        for right in &shallow {
            cases.push(boolean(K::And(
                Box::new(left.clone()),
                Box::new(right.clone()),
            )));
            cases.push(boolean(K::Or(
                Box::new(left.clone()),
                Box::new(right.clone()),
            )));
        }
    }
    for left in &shallow {
        for right in &shallow {
            cases.push(boolean(K::ScalarEqual(
                Box::new(left.clone()),
                Box::new(right.clone()),
            )));
        }
    }
    for condition in &shallow {
        for yes in &base {
            for no in &base {
                cases.push(conditional(condition, yes, no));
            }
        }
    }
    for expr in &cases {
        for truth in [false, true] {
            let fact = pattern_type_guard(expr, subject(), truth, &mut 100_000, Span::default(), 0)
                .unwrap()
                .unwrap();
            for ty in [p(P::Bool), p(P::Char), p(P::U8)] {
                for (flag, foreign) in [(false, false), (false, true), (true, false), (true, true)]
                {
                    if evaluate(expr, &ty, flag, foreign) != truth {
                        continue;
                    }
                    match fact {
                        PatternTypeGuard::Impossible => {
                            panic!("satisfiable guard reported impossible")
                        }
                        PatternTypeGuard::Exact(expected) => assert_eq!(&ty, expected),
                        PatternTypeGuard::Unknown => {}
                    }
                }
            }
        }
    }
    assert!(pattern_type_guard(&base[0], subject(), true, &mut 0, Span::default(), 0).is_err());
    assert!(pattern_type_guard(&base[0], subject(), true, &mut 100, Span::default(), 128).is_err());
}

fn evaluate_boolean_subject(expr: &n::Expr, candidate: bool, flag: bool, foreign: bool) -> bool {
    match &expr.kind {
        K::Literal(n::Literal::Bool(value)) => *value,
        K::Local(id) => {
            if *id == subject() {
                candidate
            } else {
                flag
            }
        }
        // The independent integer input is +1 when foreign is true, -1 otherwise.
        K::ScalarGreater(_, _) => foreign,
        K::Not(value) => !evaluate_boolean_subject(value, candidate, flag, foreign),
        K::And(left, right) => {
            evaluate_boolean_subject(left, candidate, flag, foreign)
                && evaluate_boolean_subject(right, candidate, flag, foreign)
        }
        K::Or(left, right) => {
            evaluate_boolean_subject(left, candidate, flag, foreign)
                || evaluate_boolean_subject(right, candidate, flag, foreign)
        }
        K::ScalarEqual(left, right) => {
            evaluate_boolean_subject(left, candidate, flag, foreign)
                == evaluate_boolean_subject(right, candidate, flag, foreign)
        }
        K::If {
            condition,
            then_branch,
            else_branch: Some(no),
        } => evaluate_boolean_subject(
            if evaluate_boolean_subject(condition, candidate, flag, foreign) {
                then_branch
            } else {
                no
            },
            candidate,
            flag,
            foreign,
        ),
        K::Block {
            statements,
            result: Some(value),
        } if statements.is_empty() => evaluate_boolean_subject(value, candidate, flag, foreign),
        _ => unreachable!(),
    }
}

#[test]
fn boolean_guard_evidence_contains_all_concrete_outcomes() {
    let base = vec![
        foreign_comparison(),
        boolean(K::Local(subject())),
        boolean(K::Local(n::LocalId { owner: 0, index: 1 })),
        boolean(K::Literal(n::Literal::Bool(false))),
        boolean(K::Literal(n::Literal::Bool(true))),
    ];
    fn expand(base: &[n::Expr]) -> Vec<n::Expr> {
        let mut cases = base.to_vec();
        for left in base {
            cases.push(boolean(K::Not(Box::new(left.clone()))));
            for right in base {
                cases.push(boolean(K::And(
                    Box::new(left.clone()),
                    Box::new(right.clone()),
                )));
                cases.push(boolean(K::Or(
                    Box::new(left.clone()),
                    Box::new(right.clone()),
                )));
                cases.push(boolean(K::ScalarEqual(
                    Box::new(left.clone()),
                    Box::new(right.clone()),
                )));
            }
        }
        cases
    }
    let shallow = expand(&base);
    let mut cases = expand(&shallow);
    for condition in &shallow {
        for yes in &base {
            for no in &base {
                cases.push(conditional(condition, yes, no));
            }
        }
    }
    for expr in &cases {
        for candidate in [false, true] {
            let outcomes = normal::boolean_guard_values(
                expr,
                subject(),
                candidate,
                &mut 100_000,
                Span::default(),
                0,
            )
            .unwrap()
            .unwrap();
            for (flag, foreign) in [(false, false), (false, true), (true, false), (true, true)] {
                let actual = evaluate_boolean_subject(expr, candidate, flag, foreign);
                assert_ne!(outcomes & (1 << u8::from(actual)), 0);
            }
        }
    }
    assert!(
        normal::boolean_guard_values(&base[0], subject(), true, &mut 0, Span::default(), 0)
            .is_err()
    );
    assert!(
        normal::boolean_guard_values(&base[0], subject(), true, &mut 100, Span::default(), 128)
            .is_err()
    );
}

#[test]
fn boolean_equality_constant_wrappers_fit_linear_budget() {
    let mut expr = atom(P::Bool);
    for _ in 0..40 {
        expr = boolean(K::ScalarEqual(
            Box::new(expr),
            Box::new(boolean(K::Literal(n::Literal::Bool(true)))),
        ));
    }
    let evidence = pattern_type_guard(&expr, subject(), true, &mut 128, Span::default(), 0)
        .unwrap()
        .unwrap();
    assert!(matches!(evidence, PatternTypeGuard::Exact(ty) if *ty == p(P::Bool)));
}
