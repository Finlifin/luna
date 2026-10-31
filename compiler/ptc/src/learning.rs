//! Explained nogoods are local to one query and its declaration authorities.
//! A clause stores the forbidden conjunction, not unconditional type facts.
mod watches;
use super::conflicts::Conflict;
use super::{ProofTree, Search, UnknownReason};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Default)]
pub(super) struct Clauses {
    entries: Vec<Vec<(usize, bool)>>,
    watches: watches::Watches,
    literals: usize,
    proofs: BTreeMap<Vec<(usize, bool)>, Arc<ProofTree>>,
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
                self.learned = Clauses::default();
            }
            if !self.learned.entries.contains(&literals) {
                self.learned.push(literals);
            }
        }
        proof
    }

    /// Retain a resolved Boolean/theory proof, not just a theory leaf. The
    /// stored proof is immutable and older than every lemma that references it.
    pub(super) fn learn_resolved(
        &mut self,
        mut conflict: Conflict,
        assignment: &[Option<bool>],
    ) -> Conflict {
        if conflict.support.is_empty() || conflict.support.len() > 8192 {
            return conflict;
        }
        let literals: Option<Vec<_>> = conflict
            .support
            .iter()
            .map(|i| assignment[*i].map(|b| (*i, b)))
            .collect();
        let Some(literals) = literals else {
            return conflict;
        };
        if self.learned.entries.contains(&literals) {
            return conflict;
        }
        // Bound retained proof depth/volume before extending the shared DAG.
        let allowance = (self.steps / 8).min(1024);
        let mut work = allowance;
        let mut pending = vec![(&conflict.tree, 0)];
        let mut admitted = true;
        while let Some((tree, depth)) = pending.pop() {
            if work == 0 || depth >= 32 {
                admitted = false;
                break;
            }
            work -= 1;
            match tree {
                ProofTree::Split {
                    negative, positive, ..
                } => {
                    pending.push((negative, depth + 1));
                    pending.push((positive, depth + 1));
                }
                ProofTree::Lemma { proof, .. } => pending.push((proof, depth + 1)),
                ProofTree::ClauseConflict(trace) => {
                    if work < trace.len() {
                        admitted = false;
                        break;
                    }
                    work -= trace.len();
                }
                _ => {}
            }
        }
        self.steps -= allowance - work;
        if !admitted || self.steps < literals.len() {
            return conflict;
        }
        self.steps -= literals.len();
        if self.learned.entries.len() >= 128 || self.learned.literals + literals.len() > 8192 {
            self.learned = Clauses::default();
        }
        let proof = Arc::new(conflict.tree);
        self.learned.push(literals.clone());
        self.learned.proofs.insert(literals.clone(), proof.clone());
        conflict.tree = ProofTree::Lemma {
            premises: literals
                .iter()
                .map(|(i, b)| (self.atoms[*i].clone(), *b))
                .collect(),
            proof,
        };
        conflict
    }

    pub(super) fn retains_conflict(
        &mut self,
        conflict: &Conflict,
        assignment: &[Option<bool>],
    ) -> bool {
        for clause in &self.learned.entries {
            if self.steps == 0 {
                return false;
            }
            self.steps -= 1;
            if clause.len() != conflict.support.len() {
                continue;
            }
            let mut same = true;
            for (index, value) in clause {
                if self.steps == 0 {
                    return false;
                }
                self.steps -= 1;
                if !conflict.support.contains(index) || assignment[*index] != Some(*value) {
                    same = false;
                    break;
                }
            }
            if same {
                return true;
            }
        }
        false
    }

    pub(super) fn propagate_learned(
        &mut self,
        assignment: &[Option<bool>],
    ) -> Result<Propagation, UnknownReason> {
        if let Some((id, missing)) = self.learned.propagate(assignment, &mut self.steps)? {
            let clause = &self.learned.entries[id];
            let mut reason = self.conflict_proof(clause);
            if let Some(proof) = self.learned.proofs.get(clause) {
                let ProofTree::TheoryConflict(premises) = reason.tree else {
                    unreachable!()
                };
                reason.tree = ProofTree::Lemma {
                    premises,
                    proof: proof.clone(),
                };
            }
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
            theory_state: None,
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
            theory_state: None,
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

#[cfg(test)]
mod resolved_tests {
    use super::*;
    use crate::{Context, Formula, ProofError, SearchResult, Terms, collect};
    use nir::{DeclarationId, Primitive, Sort, Term};

    #[test]
    fn resolved_boolean_clause_propagates_with_a_replayed_proof() {
        let atom = |local| {
            Formula::equal(
                Term::Rigid {
                    declaration: DeclarationId { package: 73, local },
                    sort: Sort::Type,
                },
                Term::Primitive(Primitive::Bool),
            )
        };
        let a = atom(0);
        let b = atom(1);
        let c = atom(2);
        let mut clauses = Vec::new();
        for left in [b.clone(), b.negate()] {
            for right in [c.clone(), c.clone().negate()] {
                clauses.push(Formula::Or(vec![a.clone().negate(), left.clone(), right]));
            }
        }
        let formula = Formula::And(clauses);
        let context = Context::default();
        let mut atoms = Vec::new();
        let mut terms = Terms::default();
        collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
        let mut search = Search {
            boolean: None,
            theory_state: None,
            learned: Clauses::default(),
            context: &context,
            formula: &formula,
            atoms: &atoms,
            terms: &terms,
            steps: 100_000,
            max_depth: 128,
        };
        let mut assignment = vec![Some(true), None, None];
        let SearchResult::Unsat(conflict) = search.solve(&mut assignment, 0) else {
            panic!("expected resolved Boolean contradiction")
        };
        assert_eq!(assignment, [Some(true), None, None]);
        assert_eq!(
            conflict.support.iter().copied().collect::<Vec<_>>(),
            vec![0]
        );
        assert!(search.learned.proofs.contains_key(&vec![(0, true)]));
        search.steps = 100_000;
        search
            .check_tree(&conflict.tree, &mut assignment, 0)
            .unwrap();
        assignment[0] = None;
        let Propagation::Unit {
            index,
            value,
            reason,
        } = search.propagate_learned(&assignment).unwrap()
        else {
            panic!("expected learned Boolean unit")
        };
        assert_eq!((index, value), (0, false));
        assert!(matches!(&reason.tree, ProofTree::Lemma { .. }));
        assert!(
            reason
                .tree
                .to_string()
                .starts_with("(lemma (premises (literal true")
        );
        assert_eq!(
            search.check_tree(&reason.tree, &mut assignment, 0),
            Err(ProofError::Invalid)
        );
        assignment[0] = Some(true);
        search.check_tree(&reason.tree, &mut assignment, 0).unwrap();
        let ProofTree::Lemma { premises, proof } = reason.tree else {
            unreachable!()
        };
        let forged = ProofTree::Lemma {
            premises: premises.clone(),
            proof: Arc::new(ProofTree::BooleanConflict),
        };
        assert_eq!(
            search.check_tree(&forged, &mut assignment, 0),
            Err(ProofError::Invalid)
        );
        let missing = ProofTree::Lemma {
            premises: vec![],
            proof: proof.clone(),
        };
        assert_eq!(
            search.check_tree(&missing, &mut assignment, 0),
            Err(ProofError::Invalid)
        );
        // Eviction may remove search state but cannot invalidate an exported
        // immutable proof or make the next query inherit learned assignments.
        search.learned = Clauses::default();
        let retained = ProofTree::Lemma { premises, proof };
        search.check_tree(&retained, &mut assignment, 0).unwrap();
        assert!(matches!(
            search.propagate_learned(&assignment).unwrap(),
            Propagation::None
        ));
    }
}
