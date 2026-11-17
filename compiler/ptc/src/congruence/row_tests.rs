use crate::{Atom, Context, Formula, Search, Terms, collect, learning, theory_state::Check};
use nir::{DeclarationId, Primitive, Sort, Term, effects::Operation};
use std::collections::BTreeSet;

fn variable(local: u32) -> Term {
    Term::Rigid {
        declaration: DeclarationId {
            package: 109,
            local,
        },
        sort: Sort::Type,
    }
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
        steps: 5_000_000,
        max_depth: 128,
    }
}
fn compare(warm: Check, fresh: Option<crate::TheoryModel>, count: usize) {
    match (warm, fresh) {
        (Check::Conflict, None) => {}
        (Check::Model(warm), Some(fresh)) => {
            assert_eq!(warm.integers, fresh.integers);
            assert_eq!(warm.kinds.len(), fresh.kinds.len());
            for a in 0..count {
                for b in 0..count {
                    assert_eq!(
                        warm.classes.root(a) == warm.classes.root(b),
                        fresh.classes.root(a) == fresh.classes.root(b)
                    );
                }
            }
        }
        _ => panic!("row agenda disagrees with full closure"),
    }
}

#[test]
fn sparse_rows_do_not_force_a_full_structural_pair_scan() {
    let mut formulas: Vec<_> = (0..96)
        .map(|i| {
            let ty = Term::Pointer(Box::new(variable(i)));
            Formula::equal(ty.clone(), ty)
        })
        .collect();
    let row = Term::error_result(
        vec![Term::Primitive(Primitive::U8)],
        Term::Primitive(Primitive::Bool),
    );
    formulas.push(Formula::equal(row.clone(), row));
    formulas.push(Formula::equal(variable(0), variable(1)));
    let formula = Formula::And(formulas);
    let context = Context::default();
    let mut atoms = Vec::new();
    let mut terms = Terms::default();
    collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
    let mut search = search(&context, &formula, &atoms, &terms);
    let last = atoms.len() - 1;
    let mut assignment = vec![Some(true); atoms.len()];
    assignment[last] = None;
    assert!(matches!(
        search.theory_cached_changes(&assignment, None, false),
        Ok(Check::Consistent)
    ));
    assignment[last] = Some(true);
    search.steps = 5_000_000;
    let warm = search
        .theory_cached_changes(&assignment, Some(&BTreeSet::from([last])), true)
        .unwrap();
    let warm_cost = 5_000_000 - search.steps;
    search.steps = 5_000_000;
    let fresh = search.theory(&assignment).unwrap();
    let fresh_cost = 5_000_000 - search.steps;
    compare(warm, fresh, terms.nodes.len());
    assert!(
        warm_cost * 2 < fresh_cost,
        "sparse row forced a global scan: warm={warm_cost}, fresh={fresh_cost}"
    );
}

#[test]
fn row_aliases_and_disequalities_survive_warm_checks_and_retractions() {
    let boolean = Term::Primitive(Primitive::Bool);
    let operation = |ty| Operation {
        declaration: DeclarationId {
            package: 109,
            local: 100,
        },
        arguments: vec![ty],
    };
    let effects = |labels: Vec<Term>, result| Term::Computation {
        effects: labels.into_iter().map(operation).collect(),
        result: Box::new(result),
    };
    let errors = |label, result| Term::error_result(vec![label], result);
    let left = effects(vec![variable(0), variable(1)], boolean.clone());
    let right = effects(vec![variable(1), variable(0)], boolean.clone());
    let open_effect = effects(vec![variable(0)], variable(2));
    let tail_effect = effects(vec![variable(1)], boolean.clone());
    let open_error = errors(variable(0), variable(3));
    let closed_error = Term::error_result(vec![variable(0), variable(1)], boolean.clone());
    let tail_error = errors(variable(1), boolean);
    let formula = Formula::And(vec![
        Formula::equal(variable(0), variable(1)),
        Formula::equal(left.clone(), right),
        Formula::equal(variable(2), tail_effect),
        Formula::equal(
            Term::Pointer(Box::new(open_effect)),
            Term::Pointer(Box::new(left)),
        ),
        Formula::equal(variable(3), tail_error),
        Formula::equal(open_error, closed_error),
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
        let mut digits = code;
        let assignment: Vec<_> = (0..atoms.len())
            .map(|_| {
                let value = [None, Some(false), Some(true)][digits % 3];
                digits /= 3;
                value
            })
            .collect();
        for (index, (old, new)) in previous.iter_mut().zip(&assignment).enumerate() {
            if old != new {
                *old = *new;
                pending.insert(index);
            }
        }
        search.steps = code % 128;
        let _ = search.theory_cached_changes(&assignment, Some(&pending), true);
        search.steps = 5_000_000;
        let warm = search
            .theory_cached_changes(&assignment, Some(&pending), true)
            .unwrap();
        pending.clear();
        let fresh = search.theory(&assignment).unwrap();
        compare(warm, fresh, terms.nodes.len());
    }
}
