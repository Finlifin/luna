//! Explained equality propagation from an already checked, reversible closure.
//! Model representatives never become premises or inferred equalities here.
use super::{Classes, Search};
use crate::{Atom, Conflict, ProofTree};
use std::collections::BTreeSet;

pub(crate) struct Unit {
    pub index: usize,
    pub value: bool,
    pub reason: Conflict,
}
fn charge(fuel: &mut usize, work: usize) -> Option<()> {
    *fuel = fuel.checked_sub(work)?;
    Some(())
}
impl Classes {
    fn metered_root(&self, mut index: usize, fuel: &mut usize) -> Option<usize> {
        loop {
            charge(fuel, 1)?;
            if self.parent[index] == index {
                return Some(index);
            }
            index = self.parent[index];
        }
    }
}
impl Search<'_> {
    // Called only after successful theory synchronization for this assignment.
    // Propagation is optional; interruption publishes neither a literal nor
    // an incomplete explanation and does not modify retained closure state.
    pub(crate) fn equality_unit(&mut self, assignment: &[Option<bool>]) -> Option<Unit> {
        let allowance = (self.steps / 8).min(1024);
        let mut work = allowance;
        let result = self.find_equality_unit(assignment, &mut work);
        self.steps -= allowance - work;
        result
    }
    fn find_equality_unit(&self, assignment: &[Option<bool>], fuel: &mut usize) -> Option<Unit> {
        let state = self.theory_state.as_ref()?;
        if state.layers.is_empty() || !state.pending.is_empty() {
            return None;
        }
        let mut unequal = BTreeSet::new();
        let mut pending = Vec::new();
        for (index, (atom, value)) in self.atoms.iter().zip(assignment).enumerate() {
            charge(fuel, 1)?;
            let Atom::TypeEqual(equality) = atom else {
                continue;
            };
            if *value == Some(true) {
                continue;
            }
            let a = state
                .classes
                .metered_root(self.terms.ids[&equality.0], fuel)?;
            let b = state
                .classes
                .metered_root(self.terms.ids[&equality.1], fuel)?;
            let pair = (a.min(b), a.max(b));
            charge(fuel, 1)?;
            if *value == Some(false) {
                unequal.insert(pair);
            } else {
                pending.push((index, pair));
            }
        }
        for (index, pair) in pending {
            charge(fuel, 1)?;
            let value = if pair.0 == pair.1 {
                true
            } else if unequal.contains(&pair) {
                false
            } else {
                continue;
            };
            // All admitted source premises are retained. The opposite target
            // contradicts the closure without relying on a selected model.
            charge(fuel, assignment.len().saturating_mul(2).saturating_add(1))?;
            let literals = assignment.iter().enumerate().filter_map(|(i, assigned)| {
                let value = if i == index { Some(!value) } else { *assigned };
                value.map(|value| (self.atoms[i].clone(), value))
            });
            let mut support: BTreeSet<_> = assignment
                .iter()
                .enumerate()
                .filter_map(|(i, value)| value.is_some().then_some(i))
                .collect();
            support.insert(index);
            return Some(Unit {
                index,
                value,
                reason: Conflict {
                    tree: ProofTree::TheoryConflict(literals.collect()),
                    support,
                },
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Context, Formula, Terms, collect, learning};
    use nir::{DeclarationId, Sort, Term};

    #[test]
    fn equality_units_replay_across_branches_and_interrupted_budgets() {
        let rigid = |local| Term::Rigid {
            declaration: DeclarationId {
                package: 100,
                local,
            },
            sort: Sort::Type,
        };
        let formula = Formula::And(vec![
            Formula::equal(rigid(0), rigid(1)),
            Formula::equal(rigid(1), rigid(2)),
            Formula::equal(rigid(0), rigid(2)),
            Formula::equal(
                Term::Optional(Box::new(rigid(0))),
                Term::Optional(Box::new(rigid(2))),
            ),
        ]);
        let context = Context::default();
        let mut atoms = Vec::new();
        let mut terms = Terms::default();
        collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
        let mut search = Search {
            boolean: None,
            theory_state: None,
            learned: learning::Clauses::default(),
            context: &context,
            formula: &formula,
            atoms: &atoms,
            terms: &terms,
            steps: 100000,
            max_depth: 128,
        };
        let mut positive = 0;
        let mut negative = 0;
        for code in (0..81).chain((0..81).rev()) {
            let mut code = code;
            let assignment: Vec<_> = (0..4)
                .map(|_| {
                    let value = [None, Some(false), Some(true)][code % 3];
                    code /= 3;
                    value
                })
                .collect();
            search.steps = 100000;
            if search.theory_cached(&assignment).unwrap().is_none() {
                continue;
            }
            let before = search.theory_state.as_ref().unwrap().to_string();
            if let Some(unit) = search.equality_unit(&assignment) {
                assert!(assignment[unit.index].is_none());
                if unit.value {
                    positive += 1;
                } else {
                    negative += 1;
                }
                let mut opposite = assignment.clone();
                opposite[unit.index] = Some(!unit.value);
                search.steps = 100000;
                search
                    .check_tree(&unit.reason.tree, &mut opposite, 0)
                    .unwrap();
                assert!(search.theory(&opposite).unwrap().is_none());
                opposite[unit.index] = Some(unit.value);
                search.steps = 100000;
                assert!(
                    search
                        .check_tree(&unit.reason.tree, &mut opposite, 0)
                        .is_err()
                );
                search.steps = 100000;
                assert!(search.theory(&opposite).unwrap().is_some());
            }
            assert_eq!(search.theory_state.as_ref().unwrap().to_string(), before);
        }
        assert!(positive > 0 && negative > 0);
        let assignment = [Some(true), Some(true), None, None];
        search.steps = 100000;
        assert!(search.theory_cached(&assignment).unwrap().is_some());
        let before = search.theory_state.as_ref().unwrap().to_string();
        let mut interrupted = false;
        let mut propagated = false;
        for budget in 0..512 {
            search.steps = budget;
            if let Some(unit) = search.equality_unit(&assignment) {
                assert_eq!((unit.index, unit.value), (2, true));
                let mut opposite = assignment;
                opposite[2] = Some(false);
                search.steps = 100000;
                search
                    .check_tree(&unit.reason.tree, &mut opposite, 0)
                    .unwrap();
                propagated = true;
            } else {
                interrupted = true;
            }
            assert_eq!(search.theory_state.as_ref().unwrap().to_string(), before);
        }
        assert!(interrupted && propagated);
        // An unconstrained model may pick equal representatives, but this is
        // not a logical equality and must not become a propagated literal.
        search.steps = 100000;
        assert!(search.theory_cached(&[None; 4]).unwrap().is_some());
        assert!(search.equality_unit(&[None; 4]).is_none());
    }
}
