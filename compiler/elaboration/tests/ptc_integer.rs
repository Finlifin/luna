use elaboration::ptc::{Budget, Context, Formula as F, IntegerTerm as I, Outcome, UnknownReason};
use nir::{DeclarationId, Primitive, Sort, Term};
use std::collections::BTreeMap;

fn id(index: u32) -> DeclarationId {
    DeclarationId {
        package: 3,
        local: index,
    }
}
fn x(index: u32, offset: i128) -> I {
    I::variable(id(index), offset)
}
fn n(value: i128) -> I {
    I::constant(value)
}
fn le(left: I, right: I) -> F {
    F::int_le(left, right)
}
fn prove(context: &Context, goal: F) {
    let result = context.prove(goal, Budget::default());
    let Outcome::Entailed(proof) = result else {
        panic!("{result}")
    };
    assert_eq!(context.verify_proof(&proof, Budget::default()), Ok(()));
}
fn eval(formula: &F, integers: &BTreeMap<DeclarationId, i128>) -> bool {
    match formula {
        F::Kind(_) => panic!("no kind atoms in this generator"),
        F::True => true,
        F::False => false,
        F::IntLe(atom) => {
            let value = |term: &I| term.variable.map_or(0, |id| integers[&id]) + term.offset;
            value(&atom.left) <= value(&atom.right)
        }
        F::Not(f) => !eval(f, integers),
        F::And(fs) => fs.iter().all(|f| eval(f, integers)),
        F::Or(fs) => fs.iter().any(|f| eval(f, integers)),
        F::Equal(_) | F::Trait(_) => panic!("integer-only oracle"),
    }
}

#[test]
fn constant_relations_and_integer_negation_are_exact() {
    let context = Context::default();
    for a in -3..=3 {
        for b in -3..=3 {
            let goal = le(n(a), n(b));
            if a <= b {
                prove(&context, goal);
            } else {
                let Outcome::NotEntailed(witness) = context.prove(goal, Budget::default()) else {
                    panic!("expected concrete countermodel")
                };
                assert!(!eval(&witness.goal, &witness.integers));
            }
        }
    }
    prove(&context, le(x(1, 0), x(1, 0)));
    prove(&context, le(x(1, 1), x(1, 0)).negate());
}

#[test]
fn difference_paths_prove_transitive_offset_bounds() {
    let mut context = Context::default();
    context.assume(le(x(1, 3), x(2, 0)));
    context.assume(le(x(2, 4), x(3, 0)));
    prove(&context, le(x(1, 7), x(3, 0)));
    let Outcome::NotEntailed(witness) = context.prove(le(x(1, 8), x(3, 0)), Budget::default())
    else {
        panic!("strictly stronger bound must have a counterexample")
    };
    assert!(
        witness
            .assumptions
            .iter()
            .all(|f| eval(f, &witness.integers))
    );
    assert!(!eval(&witness.goal, &witness.integers));
}

#[test]
fn negative_cycles_are_conflicts_but_zero_cycles_are_satisfiable() {
    let mut context = Context::default();
    context.assume(le(x(1, 0), x(2, 0)));
    context.assume(le(x(2, 0), x(3, 0)));
    context.assume(le(x(3, 0), x(1, 0)));
    assert!(matches!(
        context.prove(F::False, Budget::default()),
        Outcome::NotEntailed(_)
    ));
    let mut inconsistent = context.fork();
    inconsistent.assume(le(x(3, 1), x(1, 0)));
    prove(&inconsistent, F::False);
    assert!(matches!(
        context.prove(F::False, Budget::default()),
        Outcome::NotEntailed(_)
    ));
}

#[test]
fn zero_vertex_is_normalized_in_real_countermodels() {
    let mut context = Context::default();
    context.assume(le(n(100), x(1, 0)));
    context.assume(le(x(1, 0), n(102)));
    context.assume(le(x(2, 0), n(-100)));
    let Outcome::NotEntailed(witness) = context.prove(le(x(1, 0), n(99)), Budget::default()) else {
        panic!("expected model")
    };
    assert!((100..=102).contains(&witness.integers[&id(1)]));
    assert!(witness.integers[&id(2)] <= -100);
    assert!(
        witness
            .assumptions
            .iter()
            .all(|f| eval(f, &witness.integers))
    );
}

#[test]
fn type_and_integer_atoms_share_boolean_branches_without_sharing_sorts() {
    let t = Term::Rigid {
        declaration: id(1),
        sort: Sort::Type,
    };
    let is_bool = F::equal(t.clone(), Term::Primitive(Primitive::Bool));
    let mut context = Context::default();
    context.assume(F::Or(vec![is_bool.clone(), le(x(1, 0), n(4))]));
    context.assume(le(n(5), x(1, 0)));
    prove(&context, is_bool.clone());
    let Outcome::NotEntailed(witness) = context.prove(F::False, Budget::default()) else {
        panic!("mixed theory model")
    };
    assert_eq!(witness.types[&id(1)], Term::Primitive(Primitive::Bool));
    assert!(witness.integers[&id(1)] >= 5);
    let mut child = context.fork();
    child.assume(is_bool.negate());
    prove(&child, F::False);
    prove(&context, F::equal(t, Term::Primitive(Primitive::Bool)));
}

#[test]
fn disjunctive_integer_branches_backtrack_after_conflict() {
    let mut context = Context::default();
    context.assume(F::Or(vec![
        F::And(vec![le(x(1, 0), n(0)), le(n(1), x(1, 0))]),
        F::And(vec![le(n(7), x(1, 0)), le(x(1, 0), n(7))]),
    ]));
    prove(&context, le(n(7), x(1, 0)));
    prove(&context, le(x(1, 0), n(7)));
}

#[test]
fn arithmetic_capacity_and_search_budget_never_become_false_facts() {
    let context = Context::default();
    assert!(matches!(
        context.prove(le(n(i128::MIN), n(i128::MAX)), Budget::default()),
        Outcome::Unknown {
            reason: UnknownReason::ArithmeticRange,
            ..
        }
    ));
    assert!(matches!(
        context.prove(
            le(x(1, 0), n(0)),
            Budget {
                steps: 1,
                max_depth: 128
            }
        ),
        Outcome::Unknown {
            reason: UnknownReason::BudgetExhausted,
            ..
        }
    ));
    prove(&context, le(n(-1000), n(1000)));
}

#[test]
fn bounded_formula_family_agrees_with_exhaustive_integer_oracle() {
    let atoms = [
        le(x(1, 1), x(2, 0)),
        le(x(2, 0), n(0)),
        le(n(1), x(1, 0)),
        le(x(1, 0), x(2, -1)),
    ];
    for a in &atoms {
        for b in &atoms {
            for goal in &atoms {
                let facts = vec![
                    le(n(-2), x(1, 0)),
                    le(x(1, 0), n(2)),
                    le(n(-2), x(2, 0)),
                    le(x(2, 0), n(2)),
                    F::Or(vec![a.clone(), b.clone().negate()]),
                ];
                let mut context = Context::default();
                for f in &facts {
                    context.assume(f.clone());
                }
                let mut counterexample = false;
                for u in -2..=2 {
                    for v in -2..=2 {
                        let env = BTreeMap::from([(id(1), u), (id(2), v)]);
                        counterexample |= facts.iter().all(|f| eval(f, &env)) && !eval(goal, &env);
                    }
                }
                match context.prove(goal.clone(), Budget::default()) {
                    Outcome::Entailed(proof) => {
                        assert!(!counterexample, "unsound entailment: {goal}");
                        assert_eq!(context.verify_proof(&proof, Budget::default()), Ok(()));
                    }
                    Outcome::NotEntailed(witness) => {
                        assert!(counterexample);
                        assert!(facts.iter().all(|f| eval(f, &witness.integers)));
                        assert!(!eval(goal, &witness.integers));
                    }
                    other => panic!("bounded difference fragment must resolve: {other}"),
                }
            }
        }
    }
}

#[test]
fn integer_witness_replay_rejects_missing_or_falsified_assignments() {
    let mut context = Context::default();
    context.assume(le(n(0), x(1, 0)));
    let Outcome::NotEntailed(witness) = context.prove(le(x(1, 0), n(-1)), Budget::default()) else {
        panic!("expected a nonnegative counterexample")
    };
    assert_eq!(context.verify_witness(&witness, Budget::default()), Ok(()));
    let mut changed = witness.clone();
    changed.integers.insert(id(1), -2);
    assert_eq!(
        context.verify_witness(&changed, Budget::default()),
        Err(elaboration::ptc::ProofError::Invalid)
    );
    changed.integers.clear();
    assert_eq!(
        context.verify_witness(&changed, Budget::default()),
        Err(elaboration::ptc::ProofError::Invalid)
    );
}
