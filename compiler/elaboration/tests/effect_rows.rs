use elaboration::ptc::{Budget, Context, Formula, Outcome, ProofError, UnknownReason};
use nir::effects::Operation;
use nir::{DeclarationId, Primitive, Sort, Term};

fn variable(local: u32) -> Term {
    Term::Rigid {
        declaration: DeclarationId { package: 1, local },
        sort: Sort::Type,
    }
}
fn operation(local: u32, arguments: Vec<Term>) -> Operation {
    Operation {
        declaration: DeclarationId { package: 2, local },
        arguments,
    }
}
fn row(operations: Vec<Operation>) -> Term {
    Term::computation(operations, Term::Primitive(Primitive::Bool))
}
fn prove(context: &Context, left: Term, right: Term) {
    let goal = Formula::equal(left, right);
    let Outcome::Entailed(proof) = context.prove(goal, Budget::default()) else {
        panic!("row equality not proved");
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
}

#[test]
fn independent_labels_commute_without_erasing_duplicate_occurrences() {
    let a = operation(1, vec![]);
    let b = operation(2, vec![]);
    let c = operation(3, vec![]);
    let context = Context::default();
    prove(
        &context,
        row(vec![a.clone(), b.clone(), a.clone(), c.clone()]),
        row(vec![c, a.clone(), a.clone(), b.clone()]),
    );
    assert!(!matches!(
        context.prove(
            Formula::equal(row(vec![a.clone(), b.clone(), a.clone()]), row(vec![a, b])),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
}

#[test]
fn generic_instances_need_branch_local_equality_and_disequality_evidence() {
    let a = operation(1, vec![variable(1)]);
    let b = operation(1, vec![variable(2)]);
    let left = row(vec![a.clone(), b.clone(), a.clone()]);
    let right = row(vec![b.clone(), a.clone(), a]);
    let goal = Formula::equal(left.clone(), right.clone());
    let empty = Context::default();
    assert!(matches!(
        empty.prove(goal.clone(), Budget::default()),
        Outcome::Unknown { .. }
    ));
    let mut distinct = Context::default();
    distinct.assume(Formula::equal(variable(1), variable(2)).negate());
    let Outcome::Entailed(proof) = distinct.prove(goal.clone(), Budget::default()) else {
        panic!()
    };
    distinct.verify_proof(&proof, Budget::default()).unwrap();
    assert_eq!(
        empty.verify_proof(&proof, Budget::default()),
        Err(ProofError::Invalid)
    );
    let mut same = Context::default();
    same.assume(Formula::equal(variable(1), variable(2)));
    prove(&same, left, right);
    // Both equality and disequality cases justify this permutation. An
    // explicit exhaustive split may discharge the formerly pending relation.
    let mut conditional = Context::default();
    conditional.assume(Formula::Or(vec![
        Formula::equal(variable(1), variable(2)).negate(),
        Formula::True,
    ]));
    let Outcome::Entailed(proof) = conditional.prove(goal, Budget::default()) else {
        panic!()
    };
    conditional.verify_proof(&proof, Budget::default()).unwrap();
    // A different branch cannot borrow the equality needed to change a label.
    let mut insufficient = Context::default();
    insufficient.assume(Formula::Or(vec![
        Formula::equal(variable(1), variable(2)),
        Formula::equal(variable(3), variable(4)),
    ]));
    let a = operation(1, vec![variable(1)]);
    let b = operation(1, vec![variable(2)]);
    let c = operation(2, vec![]);
    assert!(!matches!(
        insufficient.prove(
            Formula::equal(row(vec![a, c.clone()]), row(vec![c, b])),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
}

#[test]
fn concrete_type_facts_and_builtin_values_can_distinguish_instances() {
    let mut context = Context::default();
    context.assume(Formula::equal(
        variable(1),
        Term::Primitive(Primitive::Bool),
    ));
    context.assume(Formula::equal(
        variable(2),
        Term::Primitive(Primitive::Char),
    ));
    let a = operation(1, vec![variable(1)]);
    let b = operation(1, vec![variable(2)]);
    prove(&context, row(vec![a.clone(), b.clone()]), row(vec![b, a]));
    let a = operation(1, vec![Term::Bool(true)]);
    let b = operation(1, vec![Term::Bool(false)]);
    prove(
        &Context::default(),
        row(vec![a.clone(), b.clone()]),
        row(vec![b, a]),
    );
    let opaque = |local| Term::NormalForm {
        head: DeclarationId { package: 3, local },
        sort: Sort::Type,
        arguments: vec![],
    };
    let a = operation(1, vec![opaque(1)]);
    let b = operation(1, vec![opaque(2)]);
    assert!(!matches!(
        Context::default().prove(
            Formula::equal(row(vec![a.clone(), b.clone()]), row(vec![b, a])),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
}

#[test]
fn closed_row_equalities_agree_with_a_finite_multiset_oracle() {
    let context = Context::default();
    // Every word of length three over three independent labels, including
    // repeated occurrences. The oracle counts multiplicities, not solver moves.
    for left in 0..27usize {
        for right in 0..27usize {
            let decode = |mut word| {
                let mut counts = [0; 3];
                let mut operations = Vec::new();
                for _ in 0..3 {
                    let label = word % 3;
                    word /= 3;
                    counts[label] += 1;
                    operations.push(operation(label as u32, vec![]));
                }
                (counts, row(operations))
            };
            let (a, left) = decode(left);
            let (b, right) = decode(right);
            let goal = Formula::equal(left, right);
            let inverse = context.prove(goal.clone().negate(), Budget::default());
            assert_eq!(matches!(inverse, Outcome::Entailed(_)), a != b);
            if let Outcome::Entailed(proof) = inverse {
                context.verify_proof(&proof, Budget::default()).unwrap();
            }
            let result = context.prove(goal, Budget::default());
            assert_eq!(matches!(result, Outcome::Entailed(_)), a == b);
            if let Outcome::Entailed(proof) = result {
                context.verify_proof(&proof, Budget::default()).unwrap();
            }
        }
    }
}

#[test]
fn row_search_budget_failure_does_not_poison_later_proofs() {
    let context = Context::default();
    let left = row((0..30).map(|id| operation(id, vec![])).collect());
    let right = row((0..30).rev().map(|id| operation(id, vec![])).collect());
    assert!(matches!(
        context.prove(
            Formula::equal(left.clone(), right.clone()),
            Budget {
                steps: 200,
                max_depth: 64
            }
        ),
        Outcome::Unknown {
            reason: UnknownReason::BudgetExhausted,
            ..
        }
    ));
    prove(&context, left, right);
}

#[test]
fn nested_type_distinction_requires_constructor_injectivity() {
    let boolean = Term::Primitive(Primitive::Bool);
    let character = Term::Primitive(Primitive::Char);
    let a = operation(
        1,
        vec![Term::Pointer(Box::new(Term::Tuple(vec![boolean.clone()])))],
    );
    let b = operation(
        1,
        vec![Term::Pointer(Box::new(Term::Tuple(vec![
            character.clone(),
        ])))],
    );
    prove(
        &Context::default(),
        row(vec![a.clone(), b.clone()]),
        row(vec![b, a]),
    );
    let declaration = DeclarationId {
        package: 7,
        local: 1,
    };
    let a = operation(
        1,
        vec![Term::Nominal {
            declaration,
            arguments: vec![boolean],
        }],
    );
    let b = operation(
        1,
        vec![Term::Nominal {
            declaration,
            arguments: vec![character],
        }],
    );
    let left = row(vec![a.clone(), b.clone()]);
    let right = row(vec![b, a]);
    assert!(!matches!(
        Context::default().prove(
            Formula::equal(left.clone(), right.clone()),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
    let mut context = Context::default();
    context.admit_nominal_constructor(declaration);
    prove(&context, left, right);
}

fn prove_distinct(context: &Context, left: Term, right: Term) {
    let goal = Formula::equal(left, right).negate();
    let Outcome::Entailed(proof) = context.prove(goal, Budget::default()) else {
        panic!("computation inequality not proved");
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
}

#[test]
fn computation_inequality_uses_occurrence_counts_and_result_distinction() {
    let context = Context::default();
    let a = operation(1, vec![]);
    let b = operation(2, vec![]);
    prove_distinct(
        &context,
        row(vec![a.clone()]),
        Term::Primitive(Primitive::Bool),
    );
    prove_distinct(&context, row(vec![a.clone()]), row(vec![b.clone()]));
    prove_distinct(
        &context,
        row(vec![a.clone()]),
        row(vec![a.clone(), a.clone()]),
    );
    prove_distinct(
        &context,
        row(vec![a.clone(), a.clone(), b.clone()]),
        row(vec![a.clone(), b.clone(), b]),
    );
    prove_distinct(
        &context,
        row(vec![a.clone()]),
        Term::computation(vec![a], Term::Primitive(Primitive::Char)),
    );
}

#[test]
fn computation_count_refutations_require_label_evidence_and_replay_scope() {
    let a = operation(1, vec![variable(1)]);
    let b = operation(1, vec![variable(2)]);
    let goal = Formula::equal(
        row(vec![a.clone(), a.clone(), b.clone()]),
        row(vec![a, b.clone(), b]),
    )
    .negate();
    let empty = Context::default();
    assert!(!matches!(
        empty.prove(goal.clone(), Budget::default()),
        Outcome::Entailed(_)
    ));
    let mut same = Context::default();
    same.assume(Formula::equal(variable(1), variable(2)));
    assert!(!matches!(
        same.prove(goal.clone(), Budget::default()),
        Outcome::Entailed(_)
    ));
    let mut distinct = Context::default();
    distinct.assume(Formula::equal(variable(1), variable(2)).negate());
    let Outcome::Entailed(proof) = distinct.prove(goal, Budget::default()) else {
        panic!()
    };
    distinct.verify_proof(&proof, Budget::default()).unwrap();
    assert_eq!(
        empty.verify_proof(&proof, Budget::default()),
        Err(ProofError::Invalid)
    );
}

#[test]
fn computation_refutation_never_unfolds_opaque_results_or_ignores_budget() {
    let a = operation(1, vec![]);
    let opaque = Term::NormalForm {
        head: DeclarationId {
            package: 9,
            local: 1,
        },
        sort: Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    let context = Context::default();
    let goal = Formula::equal(
        row(vec![a.clone()]),
        Term::computation(vec![a.clone()], opaque),
    )
    .negate();
    assert!(!matches!(
        context.prove(goal, Budget::default()),
        Outcome::Entailed(_)
    ));
    let goal = Formula::equal(row(vec![a.clone()]), row(vec![a.clone(), a])).negate();
    assert!(matches!(
        context.prove(
            goal,
            Budget {
                steps: 0,
                ..Default::default()
            }
        ),
        Outcome::Unknown { .. }
    ));
}

#[test]
fn computation_prefix_counts_do_not_mistake_unresolved_results_for_empty_tails() {
    let a = operation(1, vec![]);
    let variable = variable(1);
    let left = Term::computation(vec![a.clone()], variable.clone());
    let right = row(vec![a.clone(), a.clone()]);
    let goal = Formula::equal(left, right).negate();
    let mut open = Context::default();
    open.assume(Formula::equal(variable.clone(), row(vec![a.clone()])));
    assert!(!matches!(
        open.prove(goal.clone(), Budget::default()),
        Outcome::Entailed(_)
    ));
    let mut closed = Context::default();
    closed.assume(Formula::equal(variable, Term::Primitive(Primitive::Bool)));
    let Outcome::Entailed(proof) = closed.prove(goal, Budget::default()) else {
        panic!()
    };
    closed.verify_proof(&proof, Budget::default()).unwrap();
    let nested = Term::Computation {
        effects: vec![a.clone()],
        result: Box::new(row(vec![a.clone()])),
    };
    assert!(!matches!(
        Context::default().prove(
            Formula::equal(nested, row(vec![a.clone(), a])).negate(),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
}

#[test]
fn computation_result_aliases_flatten_transitively_and_keep_proof_scope() {
    let a = operation(1, vec![]);
    let b = operation(2, vec![]);
    let c = operation(3, vec![]);
    let mut context = Context::default();
    context.assume(Formula::equal(
        variable(1),
        Term::computation(vec![b.clone()], variable(2)),
    ));
    context.assume(Formula::equal(variable(2), row(vec![c.clone()])));
    let left = Term::computation(vec![a.clone()], variable(1));
    let right = row(vec![c.clone(), a.clone(), b.clone()]);
    let goal = Formula::equal(left.clone(), right.clone());
    let Outcome::Entailed(proof) = context.prove(goal, Budget::default()) else {
        panic!("transitive row view missing");
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
    assert_eq!(
        Context::default().verify_proof(&proof, Budget::default()),
        Err(ProofError::Invalid)
    );
    prove_distinct(&context, left, row(vec![a, b]));
}

#[test]
fn raw_nested_and_empty_computation_wrappers_have_the_canonical_row_meaning() {
    let a = operation(1, vec![]);
    let b = operation(2, vec![]);
    let raw = Term::Computation {
        effects: vec![a.clone()],
        result: Box::new(row(vec![b.clone(), a.clone()])),
    };
    prove(&Context::default(), raw, row(vec![a.clone(), a, b]));
    let empty = Term::Computation {
        effects: vec![],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    prove(&Context::default(), empty, Term::Primitive(Primitive::Bool));
}

#[test]
fn computation_views_do_not_unfold_normal_form_heads() {
    let a = operation(1, vec![]);
    let b = operation(2, vec![]);
    let opaque = Term::NormalForm {
        head: DeclarationId {
            package: 9,
            local: 2,
        },
        sort: Sort::Type,
        arguments: vec![],
    };
    let left = Term::computation(vec![a.clone()], opaque.clone());
    let right = row(vec![a, b.clone()]);
    let empty = Context::default();
    assert!(!matches!(
        empty.prove(
            Formula::equal(left.clone(), right.clone()),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
    let mut known = Context::default();
    known.assume(Formula::equal(opaque, row(vec![b])));
    prove(&known, left, right);
}

#[test]
fn cyclic_computation_alias_views_are_bounded_and_do_not_leak_to_later_queries() {
    let a = operation(1, vec![]);
    let mut cyclic = Context::default();
    cyclic.assume(Formula::equal(
        variable(1),
        Term::computation(vec![a.clone()], variable(1)),
    ));
    let goal = Formula::equal(
        Term::computation(vec![a.clone()], variable(1)),
        row(vec![a.clone()]),
    );
    assert!(matches!(
        cyclic.prove(
            goal,
            Budget {
                steps: 200,
                max_depth: 12,
                ..Default::default()
            }
        ),
        Outcome::Unknown { .. }
    ));
    let mut valid = Context::default();
    valid.assume(Formula::equal(variable(1), row(vec![a.clone()])));
    prove(
        &valid,
        Term::computation(vec![a.clone()], variable(1)),
        row(vec![a.clone(), a]),
    );
}
