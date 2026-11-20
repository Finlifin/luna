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

#[test]
fn combined_inputs_preserve_declaration_identity_sort_and_nested_scope() {
    let declaration = DeclarationId {
        package: 7,
        local: 3,
    };
    let rigid = Term::Rigid {
        declaration,
        sort: Sort::ComptimeValue,
    };
    let value = Term::NormalForm {
        head: DeclarationId {
            package: 7,
            local: 99,
        },
        sort: Sort::Type,
        arguments: vec![
            rigid.clone(),
            Term::Function {
                parameters: vec![],
                result: Box::new(Term::Tuple(vec![
                    rigid,
                    bound(0, 7, Sort::Type),
                    bound(1, 4, Sort::Type),
                ])),
            },
        ],
    };
    let mut combined_budget = 1000;
    let inputs = nir::copy_budget::term_inputs(&value, &mut combined_budget).unwrap();
    assert_eq!(
        inputs.declarations,
        BTreeMap::from([(declaration, Sort::ComptimeValue)])
    );
    assert_eq!(inputs.parameters, BTreeMap::from([(4, Sort::Type)]));
    assert_eq!(
        inputs.to_string(),
        format!(
            "(term-inputs (declarations (input {declaration} {})) (parameters (input 4 {})))",
            Sort::ComptimeValue,
            Sort::Type
        )
    );
    let mut slot_budget = 1000;
    assert_eq!(
        term_substitution_parameters(&value, &mut slot_budget).unwrap(),
        inputs.parameters
    );
    assert_eq!(
        combined_budget, slot_budget,
        "both maps must share one structural walk"
    );
}

#[test]
fn declaration_sort_conflicts_and_budget_failures_publish_neither_map() {
    let declaration = DeclarationId {
        package: 7,
        local: 3,
    };
    let value = Term::Tuple(vec![
        Term::Rigid {
            declaration,
            sort: Sort::Type,
        },
        Term::Rigid {
            declaration,
            sort: Sort::ComptimeValue,
        },
        bound(0, 2, Sort::Type),
    ]);
    assert!(matches!(
        nir::copy_budget::term_inputs(&value, &mut 1000),
        Err(InputObservationError::Scope(TypeError::WrongSort { .. }))
    ));
    let value = Term::Tuple(vec![
        Term::Rigid {
            declaration,
            sort: Sort::Type,
        },
        bound(0, 2, Sort::Type),
    ]);
    let mut fuel = 1;
    assert_eq!(
        nir::copy_budget::term_inputs(&value, &mut fuel),
        Err(InputObservationError::Limit(CopyBudgetExhausted::Steps))
    );
    assert_eq!(fuel, 0);
    let inputs = nir::copy_budget::term_inputs(&value, &mut 1000).unwrap();
    assert_eq!(
        inputs.declarations,
        BTreeMap::from([(declaration, Sort::Type)])
    );
    assert_eq!(inputs.parameters, BTreeMap::from([(2, Sort::Type)]));
}

#[test]
fn partial_frame_expansion_charges_each_reached_replacement_under_its_binder() {
    use nir::type_computations::Error;
    let wide = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 64]);
    let nested = |outer| Term::Function {
        parameters: vec![],
        result: Box::new(Term::Tuple(vec![bound(0, 0, Sort::Type), outer])),
    };
    let domain = nested(bound(1, 1, Sort::Type));
    let mut caller = vec![Some(wide), Some(bound(0, 2, Sort::Type)), None];
    let expected = nested(bound(1, 2, Sort::Type));
    assert_eq!(
        domain.substitute_partial_bounded(&caller, &mut 12).unwrap(),
        expected
    );
    let repeated = Term::Tuple(vec![bound(0, 0, Sort::Type); 2]);
    assert_eq!(
        repeated.substitute_partial_bounded(&caller, &mut 100),
        Err(Error::Budget)
    );
    assert_eq!(
        repeated
            .substitute_partial_bounded(&caller, &mut 200)
            .unwrap(),
        repeated.substitute_partial(&caller).unwrap()
    );
    caller[1] = None;
    assert_eq!(
        domain.substitute_partial_bounded(&caller, &mut 12),
        Err(Error::Scope(TypeError::MissingArgument(1)))
    );
}

#[test]
fn completing_retained_slots_agrees_with_one_shot_binding_and_sort_refinement() {
    let ty = Term::Primitive(Primitive::Type);
    let boolean = Term::Primitive(Primitive::Bool);
    let signature = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::ComptimeValue,
                ty: boolean.clone(),
            },
            Parameter {
                sort: Sort::Type,
                ty: ty.clone(),
            },
            Parameter {
                sort: Sort::ComptimeValue,
                ty: bound(0, 1, Sort::Type),
            },
            Parameter {
                sort: Sort::Runtime,
                ty: boolean.clone(),
            },
        ],
        result: Box::new(ty.clone()),
    };
    let initial = signature
        .prepare_function_inputs(&[Some(Term::Bool(false)), None, None, None])
        .unwrap();
    let inputs = BTreeMap::from([(1, ty.clone()), (2, boolean.clone())]);
    let completed = initial
        .bind_retained_bounded(inputs.clone(), &mut 1000)
        .unwrap();
    let direct = signature
        .prepare_function_inputs(&[
            Some(Term::Bool(false)),
            Some(ty),
            Some(boolean.clone()),
            None,
        ])
        .unwrap();
    assert_eq!(completed, direct);
    assert_eq!(
        completed.rebind(&bound(0, 2, Sort::ComptimeValue)).unwrap(),
        boolean
    );
    assert!(initial.bind_retained_bounded(inputs, &mut 0).is_err());
    assert!(
        initial
            .bind_retained_bounded(BTreeMap::from([(0, Term::Bool(true))]), &mut 1000)
            .is_err()
    );
}
