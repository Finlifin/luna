//! Theory nogoods are local to one query and its declaration authorities.
//! A clause stores the forbidden conjunction, not unconditional type facts.
use super::conflicts::Conflict;
use super::{ProofTree, Search, UnknownReason};

#[derive(Default)]
pub(super) struct Clauses {
    entries: Vec<Vec<(usize, bool)>>,
    literals: usize,
}
pub(super) enum Propagation {
    None,
    Conflict(Conflict),
    Unit {
        index: usize,
        value: bool,
        reason: Conflict,
    },
}
impl Search<'_> {
    fn conflict_proof(&self, literals: &[(usize, bool)]) -> Conflict {
        Conflict {
            tree: ProofTree::TheoryConflict(
                literals
                    .iter()
                    .map(|(i, b)| (self.atoms[*i].clone(), *b))
                    .collect(),
            ),
            support: literals.iter().map(|(i, _)| *i).collect(),
        }
    }

    /// Deletion checks use a bounded fraction of remaining query fuel. Only
    /// independently established conflicts may replace the original support;
    /// an inconclusive trial never becomes a learned fact.
    pub(super) fn learn_conflict(&mut self, assignment: &[Option<bool>]) -> Conflict {
        let mut support = assignment.to_vec();
        let available = self.steps;
        let allowance = (available / 8).min(1024);
        self.steps = allowance;
        for i in 0..support.len() {
            let Some(value) = support[i] else { continue };
            if self.steps == 0 {
                break;
            }
            support[i] = None;
            match self.theory(&support) {
                Ok(None) => {}
                Ok(Some(_)) => support[i] = Some(value),
                Err(_) => {
                    support[i] = Some(value);
                    break;
                }
            }
        }
        self.steps += available - allowance;
        let literals: Vec<_> = support
            .iter()
            .enumerate()
            .filter_map(|(i, b)| b.map(|b| (i, b)))
            .collect();
        let proof = self.conflict_proof(&literals);
        // Learning is optional; pressure discards optimization state only.
        if literals.len() <= 8192 && self.steps >= literals.len() {
            self.steps -= literals.len();
            if self.learned.entries.len() >= 128 || self.learned.literals + literals.len() > 8192 {
                self.learned.entries.clear();
                self.learned.literals = 0;
            }
            if !self.learned.entries.contains(&literals) {
                self.learned.literals += literals.len();
                self.learned.entries.push(literals);
            }
        }
        proof
    }

    pub(super) fn propagate_learned(
        &mut self,
        assignment: &[Option<bool>],
    ) -> Result<Propagation, UnknownReason> {
        for clause in &self.learned.entries {
            let mut missing = None;
            let mut inactive = false;
            for (index, value) in clause {
                self.steps = self
                    .steps
                    .checked_sub(1)
                    .ok_or(UnknownReason::BudgetExhausted)?;
                match assignment[*index] {
                    Some(actual) if actual != *value => {
                        inactive = true;
                        break;
                    }
                    None => {
                        if missing.is_some() {
                            inactive = true;
                            break;
                        }
                        missing = Some((*index, !*value));
                    }
                    _ => {}
                }
            }
            if inactive {
                continue;
            }
            let reason = self.conflict_proof(clause);
            return Ok(match missing {
                None => Propagation::Conflict(reason),
                Some((index, value)) => Propagation::Unit {
                    index,
                    value,
                    reason,
                },
            });
        }
        Ok(Propagation::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Context, Formula, ProofError, Terms, collect};
    use nir::{DeclarationId, Primitive, Sort, Term};

    #[test]
    fn learned_support_propagates_across_siblings_and_replays_independently() {
        let variable = |local| Term::Rigid {
            declaration: DeclarationId { package: 72, local },
            sort: Sort::Type,
        };
        let formula = Formula::And(vec![
            Formula::equal(variable(1), Term::Primitive(Primitive::Bool)),
            Formula::equal(variable(0), Term::Primitive(Primitive::Bool)),
            Formula::equal(variable(0), Term::Primitive(Primitive::U8)),
        ]);
        let context = Context::default();
        let mut atoms = Vec::new();
        let mut terms = Terms::default();
        collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
        let mut search = Search {
            boolean: None,
            learned: Clauses::default(),
            context: &context,
            formula: &formula,
            atoms: &atoms,
            terms: &terms,
            steps: 100_000,
            max_depth: 128,
        };
        let assignment = [Some(true); 3];
        assert!(search.theory(&assignment).unwrap().is_none());
        let proof = search.learn_conflict(&assignment);
        assert_eq!(search.learned.entries, vec![vec![(1, true), (2, true)]]);
        // The unrelated first atom may change in a sibling branch.
        let mut sibling = [Some(false), Some(true), Some(true)];
        search.check_tree(&proof.tree, &mut sibling, 0).unwrap();
        assert!(matches!(
            search.propagate_learned(&sibling).unwrap(),
            Propagation::Conflict(_)
        ));
        sibling[2] = None;
        let Propagation::Unit {
            index,
            value,
            reason,
        } = search.propagate_learned(&sibling).unwrap()
        else {
            panic!("missing learned unit")
        };
        assert_eq!((index, value), (2, false));
        // The explanation proves the opposing branch, not the propagated one.
        assert_eq!(
            search.check_tree(&reason.tree, &mut sibling, 0),
            Err(ProofError::Invalid)
        );
        sibling[2] = Some(true);
        search.check_tree(&reason.tree, &mut sibling, 0).unwrap();
        sibling[2] = Some(false);
        assert!(matches!(
            search.propagate_learned(&sibling).unwrap(),
            Propagation::None
        ));
        let ProofTree::TheoryConflict(mut support) = reason.tree else {
            unreachable!()
        };
        support.pop();
        sibling[2] = Some(true);
        assert_eq!(
            search.check_tree(&ProofTree::TheoryConflict(support), &mut sibling, 0),
            Err(ProofError::Invalid)
        );
        search.steps = 0;
        assert!(search.propagate_learned(&sibling).is_err());
        search.steps = 100_000;
        search.check_tree(&proof.tree, &mut sibling, 0).unwrap();
        let ProofTree::TheoryConflict(mut duplicate) = proof.tree.clone() else {
            unreachable!()
        };
        duplicate.push(duplicate[0].clone());
        assert_eq!(
            search.check_tree(&ProofTree::TheoryConflict(duplicate), &mut sibling, 0),
            Err(ProofError::Invalid)
        );
        // An interrupted deletion trial must restore its removed premise.
        search.steps = 8;
        let interrupted = search.learn_conflict(&assignment);
        assert!(
            matches!(&interrupted.tree, ProofTree::TheoryConflict(support) if support.len() == 3)
        );
        search.steps = 100_000;
        search
            .check_tree(&interrupted.tree, &mut assignment.clone(), 0)
            .unwrap();
        // A fresh query has no learned support, even with the same atom catalog.
        search.learned = Clauses::default();
        assert!(matches!(
            search.propagate_learned(&sibling).unwrap(),
            Propagation::None
        ));
    }

    #[test]
    fn exhausted_minimization_retains_original_support() {
        let formula = Formula::equal(
            Term::Primitive(Primitive::Bool),
            Term::Primitive(Primitive::U8),
        );
        let context = Context::default();
        let mut atoms = Vec::new();
        let mut terms = Terms::default();
        collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
        let mut search = Search {
            boolean: None,
            learned: Clauses::default(),
            context: &context,
            formula: &formula,
            atoms: &atoms,
            terms: &terms,
            steps: 0,
            max_depth: 128,
        };
        let proof = search.learn_conflict(&[Some(true)]);
        assert!(search.learned.entries.is_empty());
        assert!(matches!(&proof.tree, ProofTree::TheoryConflict(support) if support.len() == 1));
        search.steps = 1000;
        search
            .check_tree(&proof.tree, &mut [Some(true)], 0)
            .unwrap();
        assert_eq!(
            search.check_tree(&proof.tree, &mut [Some(false)], 0),
            Err(ProofError::Invalid)
        );
    }
}
