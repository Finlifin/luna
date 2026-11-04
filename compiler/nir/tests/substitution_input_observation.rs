use nir::copy_budget::{
    CopyBudgetExhausted, InputObservationError, expression_substitution_parameters,
    term_substitution_parameters,
};
use nir::program::{Capture, Expr, ExprKind as K, LocalId, Pattern, SourceRange};
use nir::{DeclarationId, Parameter, Primitive, Sort, Term, TypeError};
use std::{collections::BTreeMap, sync::Arc};

fn bound(depth: u32, index: usize, sort: Sort) -> Term {
    Term::Bound { depth, index, sort }
}

fn expr(kind: K) -> Expr {
    Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Bool),
        kind,
    }
}

#[test]
fn nested_callables_distinguish_local_outer_and_more_distant_frames() {
    let value = Term::Closure {
        environment: vec![bound(0, 1, Sort::Type)],
        signature: Box::new(Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Type,
                ty: Term::Tuple(vec![
                    bound(0, 9, Sort::Type),
                    bound(1, 2, Sort::Type),
                    bound(2, 8, Sort::Type),
                ]),
            }],
            result: Box::new(Term::UnsafeFunction(Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(Term::Tuple(vec![
                    bound(0, 7, Sort::Type),
                    bound(1, 6, Sort::Type),
                    bound(2, 3, Sort::ComptimeValue),
                    bound(3, 5, Sort::Type),
                ])),
            }))),
        }))),
    };
    assert_eq!(
        term_substitution_parameters(&value, &mut 1000).unwrap(),
        BTreeMap::from([(1, Sort::Type), (2, Sort::Type), (3, Sort::ComptimeValue)])
    );
}

#[test]
fn shared_bodies_value_payloads_and_patterns_are_observed_without_rewriting() {
    let body = Arc::new(expr(K::Tuple(vec![
        expr(K::StagedValue(bound(0, 2, Sort::ComptimeValue))),
        expr(K::TypeValue(Term::NormalForm {
            head: DeclarationId {
                package: 11,
                local: 4,
            },
            sort: Sort::Type,
            arguments: vec![bound(0, 3, Sort::Type)],
        })),
    ])));
    let closure = expr(K::Closure {
        parameters: vec![],
        captures: vec![Capture {
            binding: LocalId { owner: 1, index: 1 },
            ty: bound(0, 1, Sort::Type),
            value: None,
        }],
        body: body.clone(),
    });
    let value = expr(K::Match {
        value: Box::new(expr(K::Unit)),
        arms: vec![nir::program::MatchArm {
            pattern: Pattern::ErrorResult {
                label: Some(bound(0, 4, Sort::Type)),
                inner: Box::new(Pattern::Wild),
            },
            guard: None,
            body: closure,
        }],
    });
    let before = value.to_string();
    assert_eq!(
        expression_substitution_parameters(&value, &mut 1000).unwrap(),
        BTreeMap::from([
            (1, Sort::Type),
            (2, Sort::ComptimeValue),
            (3, Sort::Type),
            (4, Sort::Type),
        ])
    );
    assert_eq!(value.to_string(), before);
    let K::Match { arms, .. } = &value.kind else {
        panic!()
    };
    let K::Closure { body: observed, .. } = &arms[0].body.kind else {
        panic!()
    };
    assert!(Arc::ptr_eq(&body, observed));
    assert_eq!(Arc::strong_count(&body), 2);
    let K::Tuple(fields) = &observed.kind else {
        panic!()
    };
    assert!(matches!(fields[0].kind, K::StagedValue(_)));
}

#[test]
fn conflicting_slot_sorts_do_not_produce_a_dependency_set() {
    let conflicting = Term::Tuple(vec![
        bound(0, 1, Sort::Type),
        bound(0, 1, Sort::ComptimeValue),
    ]);
    assert!(matches!(
        term_substitution_parameters(&conflicting, &mut 1000),
        Err(InputObservationError::Scope(TypeError::WrongSort { .. }))
    ));
    let conflicting = expr(K::Tuple(vec![
        expr(K::TypeValue(bound(0, 1, Sort::Type))),
        expr(K::StagedValue(bound(0, 1, Sort::ComptimeValue))),
    ]));
    assert!(matches!(
        expression_substitution_parameters(&conflicting, &mut 1000),
        Err(InputObservationError::Scope(TypeError::WrongSort { .. }))
    ));
    assert_eq!(
        term_substitution_parameters(&bound(0, 1, Sort::Type), &mut 1000).unwrap(),
        BTreeMap::from([(1, Sort::Type)])
    );
}

#[test]
fn exhaustion_never_publishes_partial_slots_and_fresh_observation_recovers() {
    let value = expr(K::Tuple(vec![
        expr(K::TypeValue(bound(0, 1, Sort::Type))),
        expr(K::TypeValue(bound(0, 2, Sort::Type))),
    ]));
    let mut fuel = 5;
    assert_eq!(
        expression_substitution_parameters(&value, &mut fuel),
        Err(InputObservationError::Limit(CopyBudgetExhausted::Steps))
    );
    assert!(fuel < 5);
    let expected = BTreeMap::from([(1, Sort::Type), (2, Sort::Type)]);
    assert_eq!(
        expression_substitution_parameters(&value, &mut 1000).unwrap(),
        expected
    );
    let mut deep = value.clone();
    for _ in 0..nir::copy_budget::MAX_COPY_DEPTH {
        deep = expr(K::Discard(Box::new(deep)));
    }
    let mut fuel = 1_000_000;
    assert_eq!(
        expression_substitution_parameters(&deep, &mut fuel),
        Err(InputObservationError::Limit(CopyBudgetExhausted::Depth))
    );
    assert!(fuel > 0);
    assert_eq!(
        expression_substitution_parameters(&value, &mut fuel).unwrap(),
        expected
    );
}
