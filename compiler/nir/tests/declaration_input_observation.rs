use nir::copy_budget::{CopyBudgetExhausted, expression_declaration_inputs};
use nir::program::{
    Capture, EffectHandler, Expr, ExprKind as K, LocalId, MatchArm, Pattern, SourceRange,
};
use nir::{DeclarationId, Primitive, Sort, Term};
use std::{collections::BTreeSet, sync::Arc};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 91, local }
}
fn input(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}
fn expr(kind: K) -> Expr {
    Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Bool),
        kind,
    }
}

#[test]
fn observation_preserves_shared_bodies_and_residual_value_nodes() {
    let body = Arc::new(expr(K::Tuple(vec![
        expr(K::TypeValue(input(2))),
        expr(K::StagedValue(Term::Rigid {
            declaration: id(4),
            sort: Sort::ComptimeValue,
        })),
    ])));
    let value = expr(K::Closure {
        parameters: vec![],
        body: body.clone(),
        captures: vec![Capture {
            binding: LocalId { owner: 1, index: 0 },
            ty: input(1),
            value: Some(Box::new(expr(K::TypeValue(input(3))))),
        }],
    });
    let before = value.to_string();
    assert_eq!(
        expression_declaration_inputs(&value, &mut 1000).unwrap(),
        (1..=4).map(id).collect()
    );
    assert_eq!(value.to_string(), before);
    let K::Closure { body: observed, .. } = &value.kind else {
        panic!()
    };
    assert!(Arc::ptr_eq(&body, observed));
    assert_eq!(Arc::strong_count(&body), 2);
    let K::Tuple(fields) = &observed.kind else {
        panic!()
    };
    assert!(matches!(fields[1].kind, K::StagedValue(_)));
}

#[test]
fn operation_metadata_patterns_and_call_identities_keep_hidden_inputs() {
    let operation = |n| nir::effects::Operation {
        declaration: id(99),
        arguments: vec![input(n)],
    };
    let value = expr(K::EffectHandle {
        computation: Box::new(expr(K::Unit)),
        outer: vec![operation(1)],
        handlers: vec![EffectHandler {
            operation: operation(2),
            abi: nir::effects::Abi::Direct,
            signature: input(3),
            fallback: None,
            arms: vec![MatchArm {
                pattern: Pattern::ErrorResult {
                    label: Some(input(4)),
                    inner: Box::new(Pattern::Wild),
                },
                guard: Some(expr(K::TraitMethod {
                    interface: id(99),
                    member: id(98),
                    subject: input(5),
                    interface_arguments: vec![input(6)],
                    method_arguments: vec![input(7)],
                })),
                body: expr(K::FunctionInstance {
                    declaration: id(97),
                    arguments: vec![input(8)],
                }),
            }],
        }],
    });
    assert_eq!(
        expression_declaration_inputs(&value, &mut 1000).unwrap(),
        (1..=8).map(id).collect()
    );
}

#[test]
fn value_payloads_keep_inputs_without_unfolding_opaque_heads() {
    let value = expr(K::ComptimeValue(Term::NominalValue {
        ty: Box::new(Term::NormalForm {
            head: id(99),
            sort: Sort::Type,
            arguments: vec![input(1)],
        }),
        variant: None,
        fields: vec![input(2)],
    }));
    assert_eq!(
        expression_declaration_inputs(&value, &mut 1000).unwrap(),
        BTreeSet::from([id(1), id(2)])
    );
    let K::ComptimeValue(Term::NominalValue { ty, .. }) = &value.kind else {
        panic!()
    };
    assert!(matches!(**ty, Term::NormalForm { head, .. } if head == id(99)));
}

#[test]
fn budget_failures_publish_no_input_set_and_preserve_retry() {
    let value = expr(K::Tuple(vec![
        expr(K::TypeValue(input(1))),
        expr(K::TypeValue(input(2))),
    ]));
    let mut fuel = 5;
    assert_eq!(
        expression_declaration_inputs(&value, &mut fuel),
        Err(CopyBudgetExhausted::Steps)
    );
    assert!(fuel < 5);
    assert_eq!(
        expression_declaration_inputs(&value, &mut 1000).unwrap(),
        BTreeSet::from([id(1), id(2)])
    );
    let mut deep = value.clone();
    for _ in 0..nir::copy_budget::MAX_COPY_DEPTH {
        deep = expr(K::Discard(Box::new(deep)));
    }
    let mut fuel = 1_000_000;
    assert_eq!(
        expression_declaration_inputs(&deep, &mut fuel),
        Err(CopyBudgetExhausted::Depth)
    );
    assert!(fuel > 0);
    assert_eq!(
        expression_declaration_inputs(&value, &mut fuel).unwrap(),
        BTreeSet::from([id(1), id(2)])
    );
}
