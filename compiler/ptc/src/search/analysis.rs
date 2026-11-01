//! First-UIP resolution over live trail reasons. Analysis does not unwind the
//! assignment or alter propagation cursors; every intermediate result is a proof.
use super::{Conflict, Reason, Search, SearchResult, Trail};
use crate::ProofTree;

#[cfg(test)]
thread_local! { pub(super) static RESOLUTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

fn charge(fuel: &mut usize, work: usize) -> Option<()> {
    *fuel = fuel.checked_sub(work)?;
    Some(())
}

// Proof nodes and payloads are immutable and shared. Reusing a reason copies
// only the query-local support set and a constant-size proof root.
fn copy_reason(reason: &Conflict, fuel: &mut usize) -> Option<Conflict> {
    charge(fuel, reason.support.len().saturating_add(1))?;
    Some(Conflict {
        tree: reason.tree.clone(),
        support: reason.support.clone(),
    })
}

impl Search<'_> {
    pub(super) fn analyze_conflict(
        &mut self,
        result: SearchResult,
        assignment: &Trail<'_>,
        level: usize,
    ) -> SearchResult {
        if level == 0 {
            return result;
        }
        let SearchResult::Unsat(mut conflict) = result else {
            return result;
        };
        // Optional analysis must leave fuel for proof admission and search.
        let allowance = (self.steps / 8).min(2048);
        let mut work = allowance;
        let mut cursor = assignment.len();
        loop {
            if charge(&mut work, conflict.support.len()).is_none() {
                break;
            }
            let current = conflict
                .support
                .iter()
                .filter(|index| assignment.levels()[**index] == level)
                .count();
            if current <= 1 {
                break;
            }
            let mut pivot = None;
            while cursor > 0 {
                if charge(&mut work, 1).is_none() {
                    break;
                }
                cursor -= 1;
                let (index, value, entry_level, reason) = assignment.entry(cursor);
                if entry_level == level && conflict.support.contains(&index) {
                    pivot = Some((index, value, reason));
                    break;
                }
            }
            let Some((index, value, reason)) = pivot else {
                break;
            };
            let reason = match reason {
                Reason::Decision => break,
                Reason::Boolean(antecedents) => {
                    if charge(&mut work, antecedents.len().saturating_add(1)).is_none() {
                        break;
                    }
                    let mut support = (**antecedents).clone();
                    support.insert(index);
                    Conflict {
                        tree: ProofTree::BooleanConflict,
                        support,
                    }
                }
                Reason::RootClause(reason) | Reason::Learned(reason) | Reason::Theory(reason) => {
                    let Some(reason) = copy_reason(reason, &mut work) else {
                        break;
                    };
                    reason
                }
            };
            // Meter the support union before publishing another proof node.
            if charge(
                &mut work,
                conflict.support.len().saturating_add(reason.support.len()),
            )
            .is_none()
            {
                break;
            }
            let (negative, positive) = if value {
                (reason, conflict)
            } else {
                (conflict, reason)
            };
            conflict = Conflict::resolve(index, &self.atoms[index], negative, positive);
            #[cfg(test)]
            RESOLUTIONS.with(|n| n.set(n.get() + 1));
        }
        self.steps -= allowance - work;
        SearchResult::Unsat(conflict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Context, Formula, Terms, collect, learning};
    use nir::{DeclarationId, Primitive, Sort, Term};
    use std::{collections::BTreeSet, sync::Arc};

    #[test]
    fn deep_reason_reuse_shares_nodes_but_not_validation_authority() {
        let atom = |local| {
            Formula::equal(
                Term::Rigid {
                    declaration: DeclarationId { package: 99, local },
                    sort: Sort::Type,
                },
                Term::Primitive(Primitive::Bool),
            )
        };
        let mut parts = vec![Formula::False];
        parts.extend((0..64).map(|i| Formula::Or(vec![atom(i), atom(i).negate()])));
        let formula = Formula::And(parts);
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
        let leaf = Arc::new(ProofTree::BooleanConflict);
        let mut tree = ProofTree::BooleanConflict;
        for atom in atoms.iter().rev() {
            tree = ProofTree::Split {
                atom: Arc::new(atom.clone()),
                negative: Arc::new(tree),
                positive: leaf.clone(),
            };
        }
        let original = Conflict {
            tree,
            support: BTreeSet::new(),
        };
        assert!(copy_reason(&original, &mut 0).is_none());
        // Copy work depends on support size, not the 64-level proof body.
        let copied = copy_reason(&original, &mut 1).unwrap();
        let (
            ProofTree::Split {
                atom: a,
                negative: n,
                positive: p,
            },
            ProofTree::Split {
                atom: ca,
                negative: cn,
                positive: cp,
            },
        ) = (&original.tree, &copied.tree)
        else {
            panic!("expected shared split")
        };
        assert!(Arc::ptr_eq(a, ca) && Arc::ptr_eq(n, cn) && Arc::ptr_eq(p, cp));
        assert_eq!(original.tree.to_string(), copied.tree.to_string());
        drop(original);
        let mut values = vec![None; atoms.len()];
        search.check_tree(&copied.tree, &mut values, 0).unwrap();
        assert!(values.iter().all(Option::is_none));
        let mut forged = copied.tree.clone();
        let ProofTree::Split { atom, .. } = &mut forged else {
            unreachable!()
        };
        *Arc::make_mut(atom) = crate::Atom::TypeEqual(crate::Equality(
            Term::Primitive(Primitive::U8),
            Term::Primitive(Primitive::U16),
        ));
        search.steps = 100000;
        assert_eq!(
            search.check_tree(&forged, &mut values, 0),
            Err(crate::ProofError::Invalid)
        );
        search.steps = 100000;
        search.check_tree(&copied.tree, &mut values, 0).unwrap();
        search.max_depth = 8;
        search.steps = 100000;
        assert_eq!(
            search.check_tree(&copied.tree, &mut values, 0),
            Err(crate::ProofError::BudgetExhausted)
        );
        assert!(values.iter().all(Option::is_none));
    }

    #[test]
    fn mixed_reasons_find_the_uip_without_unwinding_or_notifying() {
        let atom = |local| {
            Formula::equal(
                Term::Rigid {
                    declaration: DeclarationId { package: 98, local },
                    sort: Sort::Type,
                },
                Term::Primitive(Primitive::Bool),
            )
        };
        let formula = Formula::And(vec![
            Formula::Or(vec![atom(0).negate(), atom(1)]),
            Formula::Or(vec![atom(1).negate(), atom(2)]),
            Formula::Or(vec![atom(1).negate(), atom(3)]),
            Formula::Or(vec![atom(2).negate(), atom(3).negate()]),
            Formula::Or(vec![atom(0).negate(), atom(4)]),
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
        let mut values = vec![None; atoms.len()];
        let mut trail = Trail::new(&mut values);
        trail.assign(0, true, 1, Reason::Decision);
        trail.assign(1, true, 1, Reason::Boolean(Arc::new(BTreeSet::from([0]))));
        trail.assign(
            2,
            true,
            1,
            Reason::RootClause(Conflict {
                tree: ProofTree::BooleanConflict,
                support: BTreeSet::from([1, 2]),
            }),
        );
        trail.assign(
            3,
            true,
            1,
            Reason::Learned(Conflict {
                tree: ProofTree::Lemma {
                    premises: vec![(atoms[1].clone(), true), (atoms[3].clone(), false)].into(),
                    proof: Arc::new(ProofTree::BooleanConflict),
                },
                support: BTreeSet::from([1, 3]),
            }),
        );
        // A later unrelated propagation must not delay or enter the learned cut.
        trail.assign(4, true, 1, Reason::Boolean(Arc::new(BTreeSet::from([0]))));
        for consumer in [
            super::super::Consumer::Root,
            super::super::Consumer::Learned,
            super::super::Consumer::Theory,
        ] {
            trail.acknowledge_changes(consumer);
        }
        let before = trail.to_string();
        let initial = || {
            SearchResult::Unsat(Conflict {
                tree: ProofTree::BooleanConflict,
                support: BTreeSet::from([2, 3]),
            })
        };
        let mut completed = false;
        let mut interrupted = false;
        for budget in 0..512 {
            search.steps = budget;
            let SearchResult::Unsat(proof) = search.analyze_conflict(initial(), &trail, 1) else {
                panic!("analysis lost contradiction")
            };
            assert!(search.steps <= budget);
            assert_eq!(trail.to_string(), before);
            assert!(
                trail
                    .changes(super::super::Consumer::Root)
                    .unwrap()
                    .is_empty()
            );
            assert!(
                trail
                    .changes(super::super::Consumer::Learned)
                    .unwrap()
                    .is_empty()
            );
            assert!(
                trail
                    .changes(super::super::Consumer::Theory)
                    .unwrap()
                    .is_empty()
            );
            search.steps = 100000;
            let mut premises: Vec<_> = trail
                .values()
                .iter()
                .enumerate()
                .map(|(i, value)| {
                    if proof.support.contains(&i) {
                        *value
                    } else {
                        None
                    }
                })
                .collect();
            search.check_tree(&proof.tree, &mut premises, 0).unwrap();
            if proof.support == BTreeSet::from([1]) {
                completed = true;
                premises[1] = Some(false);
                search.steps = 100000;
                assert!(search.check_tree(&proof.tree, &mut premises, 0).is_err());
            } else {
                interrupted = true;
            }
        }
        assert!(completed && interrupted);
    }
}
