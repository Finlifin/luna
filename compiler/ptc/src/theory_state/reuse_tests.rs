use super::*;
use crate::{Context, Formula, IntLe, IntegerTerm, Terms, TypeKind, collect, learning};
use nir::{DeclarationId, Primitive, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId {
        package: 108,
        local,
    }
}
fn input(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}
fn trait_query(subject: Term) -> Formula {
    Formula::Trait(crate::TraitQuery {
        interface: id(10),
        subject,
        arguments: vec![],
    })
}
fn integer_query() -> Formula {
    Formula::IntLe(IntLe {
        left: IntegerTerm::variable(id(20), 0),
        right: IntegerTerm::constant(0),
    })
}
fn search<'a>(
    context: &'a Context,
    formula: &'a Formula,
    atoms: &'a [Atom],
    terms: &'a Terms,
) -> Search<'a> {
    Search {
        boolean: None,
        theory_state: None,
        learned: learning::Clauses::default(),
        context,
        formula,
        atoms,
        terms,
        steps: 1_000_000,
        max_depth: 128,
    }
}

#[test]
fn integer_extensions_do_not_recheck_an_unchanged_type_closure() {
    let mut left = input(0);
    let mut right = Term::Primitive(Primitive::U8);
    for _ in 0..24 {
        left = Term::Optional(Box::new(left));
        right = Term::Optional(Box::new(right));
    }
    let formula = Formula::And(vec![
        Formula::equal(
            Term::error_result(vec![Term::Primitive(Primitive::Bool)], left),
            Term::error_result(vec![Term::Primitive(Primitive::Bool)], right),
        ),
        Formula::kind(input(0), TypeKind::UnsignedInteger),
        trait_query(input(0)),
        integer_query(),
    ]);
    let context = Context::default();
    let mut atoms = Vec::new();
    let mut terms = Terms::default();
    collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
    let mut search = search(&context, &formula, &atoms, &terms);
    let mut assignment = [Some(true), Some(true), Some(true), None];
    assert!(matches!(
        search.theory_cached_changes(&assignment, None, false),
        Ok(Check::Consistent)
    ));
    assignment[3] = Some(true);
    search.steps = 256;
    assert!(
        matches!(
            search.theory_cached_changes(&assignment, Some(&BTreeSet::from([3])), false),
            Ok(Check::Consistent)
        ),
        "integer-only extension rechecked the type closure"
    );
    search.steps = 1;
    assert!(matches!(
        search.theory_cached_changes(&assignment, Some(&BTreeSet::new()), false),
        Ok(Check::Consistent)
    ));
    // Copying a model is still work, even when consistency is already known.
    search.steps = 1;
    assert!(matches!(
        search.theory_cached_changes(&assignment, Some(&BTreeSet::new()), true),
        Err(UnknownReason::BudgetExhausted)
    ));
    search.steps = 1_000_000;
    let Check::Model(model) = search
        .theory_cached_changes(&assignment, Some(&BTreeSet::new()), true)
        .unwrap()
    else {
        panic!("missing model")
    };
    assert_eq!(model.kinds.len(), 1);
    assert!(model.integers[&id(20)] <= 0);
    // The closure contains trait congruence, not an implementation-world model.
    assert!(search.witness(&model).is_none());
}

#[test]
fn retained_type_facts_match_fresh_checks_after_retractions_and_interruptions() {
    let formula = Formula::And(vec![
        Formula::equal(input(0), input(1)),
        trait_query(input(0)),
        trait_query(input(1)),
        Formula::kind(input(0), TypeKind::UnsignedInteger),
        integer_query(),
        Formula::equal(
            Term::error_result(vec![input(0)], Term::Primitive(Primitive::Bool)),
            Term::error_result(vec![input(1)], Term::Primitive(Primitive::Bool)),
        ),
    ]);
    let context = Context::default();
    let mut atoms = Vec::new();
    let mut terms = Terms::default();
    collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
    let mut search = search(&context, &formula, &atoms, &terms);
    let mut previous = vec![None; atoms.len()];
    let mut pending = BTreeSet::new();
    let combinations = 3usize.pow(atoms.len() as u32);
    for code in (0..combinations).chain((0..combinations).rev()) {
        let mut remaining = code;
        let assignment: Vec<_> = (0..atoms.len())
            .map(|_| {
                let value = [None, Some(false), Some(true)][remaining % 3];
                remaining /= 3;
                value
            })
            .collect();
        for (i, (old, new)) in previous.iter_mut().zip(&assignment).enumerate() {
            if old != new {
                pending.insert(i);
                *old = *new;
            }
        }
        // Interrupted checks retain pending changes until a complete retry.
        search.steps = code % 64;
        let _ = search.theory_cached_changes(&assignment, Some(&pending), true);
        search.steps = 1_000_000;
        let warm = search
            .theory_cached_changes(&assignment, Some(&pending), true)
            .unwrap();
        pending.clear();
        let fresh = search.theory(&assignment).unwrap();
        match (warm, fresh) {
            (Check::Conflict, None) => {}
            (Check::Model(warm), Some(fresh)) => {
                assert_eq!(warm.integers, fresh.integers);
                for a in 0..terms.nodes.len() {
                    for b in 0..terms.nodes.len() {
                        assert_eq!(
                            warm.classes.root(a) == warm.classes.root(b),
                            fresh.classes.root(a) == fresh.classes.root(b)
                        );
                    }
                }
                assert_eq!(warm.kinds.len(), fresh.kinds.len());
                for ((a, kind, truth), (b, other, value)) in warm.kinds.iter().zip(&fresh.kinds) {
                    assert_eq!((kind, truth), (other, value));
                    assert_eq!(warm.classes.root(*b), *a);
                }
            }
            _ => panic!("cached theory disagrees for {assignment:?}"),
        }
    }
}
