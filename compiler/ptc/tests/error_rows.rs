use nir::{DeclarationId, Primitive, Sort, Term};
use ptc::{Budget, Context, Formula, Outcome, ProofError};
fn variable(index: u32) -> Term {
    Term::Rigid {
        declaration: DeclarationId {
            package: 1,
            local: index,
        },
        sort: Sort::Type,
    }
}
fn row(errors: Vec<Term>) -> Term {
    Term::error_result(errors, Term::Primitive(Primitive::Bool))
}
fn proved(context: &Context, left: Term, right: Term) {
    let Outcome::Entailed(proof) = context.prove(Formula::equal(left, right), Budget::default())
    else {
        panic!("not proved")
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
}
#[test]
fn error_labels_commute_without_assuming_distinctness() {
    let a = variable(1);
    let b = variable(2);
    proved(
        &Context::default(),
        row(vec![a.clone(), b.clone(), a.clone()]),
        row(vec![b, a]),
    );
}
#[test]
fn label_coalescing_requires_branch_evidence_and_proof_replays() {
    let a = variable(1);
    let b = variable(2);
    let goal = Formula::equal(row(vec![a.clone(), b.clone()]), row(vec![a.clone()]));
    let empty = Context::default();
    assert!(!matches!(
        empty.prove(goal.clone(), Budget::default()),
        Outcome::Entailed(_)
    ));
    let mut context = Context::default();
    context.assume(Formula::equal(a, b));
    let Outcome::Entailed(proof) = context.prove(goal, Budget::default()) else {
        panic!("not proved")
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
    assert_eq!(
        empty.verify_proof(&proof, Budget::default()),
        Err(ProofError::Invalid)
    );
}
#[test]
fn empty_and_nested_raw_error_qualifiers_have_no_extra_layer() {
    let boolean = Term::Primitive(Primitive::Bool);
    let empty = Term::ErrorResult {
        errors: vec![],
        result: Box::new(boolean.clone()),
    };
    proved(&Context::default(), empty, boolean);
    let nested = Term::ErrorResult {
        errors: vec![variable(1)],
        result: Box::new(row(vec![variable(2), variable(1)])),
    };
    proved(
        &Context::default(),
        nested,
        row(vec![variable(2), variable(1)]),
    );
}
#[test]
fn unmatched_labels_and_different_success_types_are_not_equal_by_default() {
    let context = Context::default();
    for (left, right) in [
        (row(vec![variable(1)]), row(vec![variable(2)])),
        (row(vec![variable(1)]), row(vec![variable(1), variable(2)])),
        (
            row(vec![variable(1)]),
            Term::error_result(vec![variable(1)], Term::Primitive(Primitive::Char)),
        ),
    ] {
        assert!(!matches!(
            context.prove(Formula::equal(left, right), Budget::default()),
            Outcome::Entailed(_)
        ));
    }
}

fn disproved(context: &Context, left: Term, right: Term) {
    let goal = Formula::equal(left, right).negate();
    let Outcome::Entailed(proof) = context.prove(goal, Budget::default()) else {
        panic!("disequality not proved")
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
}
#[test]
fn closed_rows_use_proved_absence_and_success_disjointness() {
    let boolean = Term::Primitive(Primitive::Bool);
    let character = Term::Primitive(Primitive::Char);
    disproved(
        &Context::default(),
        row(vec![boolean.clone()]),
        row(vec![character.clone()]),
    );
    disproved(
        &Context::default(),
        row(vec![boolean.clone()]),
        row(vec![boolean.clone(), character]),
    );
    disproved(&Context::default(), row(vec![variable(1)]), boolean);
    disproved(
        &Context::default(),
        row(vec![variable(1)]),
        Term::error_result(vec![variable(1)], Term::Primitive(Primitive::Char)),
    );
}
#[test]
fn unknown_labels_do_not_prove_absence_but_branch_disequality_does() {
    let a = variable(1);
    let b = variable(2);
    let goal = Formula::equal(row(vec![a.clone()]), row(vec![b.clone()])).negate();
    let empty = Context::default();
    assert!(!matches!(
        empty.prove(goal.clone(), Budget::default()),
        Outcome::Entailed(_)
    ));
    let mut context = Context::default();
    context.assume(Formula::equal(a, b).negate());
    let Outcome::Entailed(proof) = context.prove(goal, Budget::default()) else {
        panic!("not proved")
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
    assert_eq!(
        empty.verify_proof(&proof, Budget::default()),
        Err(ProofError::Invalid)
    );
}
#[test]
fn opaque_success_may_hide_more_errors_until_ordinary_type_is_proven() {
    let a = variable(1);
    let b = variable(2);
    let success = variable(3);
    let left = Term::error_result(vec![a.clone()], success.clone());
    let right = Term::error_result(vec![b.clone()], success.clone());
    let mut context = Context::default();
    context.assume(Formula::equal(a, b).negate());
    assert!(!matches!(
        context.prove(
            Formula::equal(left.clone(), right.clone()).negate(),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
    context.assume(Formula::equal(success, Term::Primitive(Primitive::Bool)));
    disproved(&context, left, right);
}

#[test]
fn closed_error_equality_agrees_with_finite_set_oracle() {
    let labels = [Primitive::Bool, Primitive::Char, Primitive::U8];
    let build = |mask: u8, success: Primitive, reverse: bool| {
        let mut errors: Vec<_> = labels
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1 << index) != 0)
            .map(|(_, label)| Term::Primitive(*label))
            .collect();
        if reverse {
            errors.reverse();
        }
        Term::error_result(errors, Term::Primitive(success))
    };
    let context = Context::default();
    for left in 0u8..8 {
        for right in 0u8..8 {
            for ls in [Primitive::Bool, Primitive::Char] {
                for rs in [Primitive::Bool, Primitive::Char] {
                    let a = build(left, ls, false);
                    let b = build(right, rs, true);
                    if left == right && ls == rs {
                        proved(&context, a, b);
                    } else {
                        disproved(&context, a, b);
                    }
                }
            }
        }
    }
}

#[test]
fn frozen_error_indices_lift_type_equalities_without_value_propositions() {
    let a = variable(1);
    let b = variable(2);
    let head = DeclarationId {
        package: 1,
        local: 99,
    };
    let key = |ty, label| Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![Term::ErrorValue {
            ty: Box::new(ty),
            label: Some(Box::new(label)),
            value: Box::new(Term::Bool(true)),
        }],
    };
    let mut context = Context::default();
    context.admit_congruence(head);
    proved(
        &context,
        key(row(vec![a.clone(), b.clone()]), a.clone()),
        key(row(vec![b.clone(), a.clone()]), a.clone()),
    );
    context.assume(Formula::equal(a.clone(), b.clone()));
    proved(
        &context,
        key(row(vec![a.clone()]), a),
        key(row(vec![b.clone()]), b),
    );
}

#[test]
fn explicit_error_label_cannot_disappear_into_a_closed_destination() {
    let label = variable(101);
    let success = variable(102);
    let open = Term::error_result(vec![label.clone()], success.clone());
    let ordinary = Term::Primitive(Primitive::Bool);
    let context = Context::default();
    disproved(&context, open.clone(), ordinary.clone());
    disproved(&context, ordinary, open.clone());
    // An unknown destination can itself acquire the known source label.
    assert!(!matches!(
        context.prove(
            Formula::equal(open.clone(), variable(103)).negate(),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
    let other = variable(104);
    let mut distinct = context.fork();
    distinct.assume(Formula::equal(label.clone(), other.clone()).negate());
    disproved(&distinct, open.clone(), row(vec![other.clone()]));
    assert!(!matches!(
        context.prove(
            Formula::equal(open.clone(), row(vec![other])).negate(),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
    // The opaque success may contribute exactly the destination's extra labels.
    assert!(!matches!(
        context.prove(
            Formula::equal(open, row(vec![label, variable(105)])).negate(),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
}
