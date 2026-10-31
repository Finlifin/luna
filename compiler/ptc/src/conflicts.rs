//! Search support is an upper bound on the branch literals a proof needs.
//! It guides backjumping only; exported proofs are still replayed independently.
use super::{Atom, ProofTree};
use std::collections::BTreeSet;

pub(super) struct Conflict {
    pub tree: ProofTree,
    pub support: BTreeSet<usize>,
}
impl Conflict {
    pub fn boolean(assignment: &[Option<bool>]) -> Self {
        Self {
            tree: ProofTree::BooleanConflict,
            support: assignment
                .iter()
                .enumerate()
                .filter_map(|(i, b)| b.map(|_| i))
                .collect(),
        }
    }
    pub fn resolve(index: usize, atom: &Atom, negative: Self, positive: Self) -> Self {
        if !negative.support.contains(&index) {
            return negative;
        }
        if !positive.support.contains(&index) {
            return positive;
        }
        let mut support = negative.support;
        support.extend(positive.support);
        support.remove(&index);
        Self {
            tree: ProofTree::Split {
                atom: atom.clone(),
                negative: Box::new(negative.tree),
                positive: Box::new(positive.tree),
            },
            support,
        }
    }
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(conflict (support")?;
        for index in &self.support {
            write!(f, " {index}")?;
        }
        write!(f, ") (proof {}))", self.tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Context, Formula, Search, SearchResult, Terms, collect, learning::Clauses};
    use nir::{DeclarationId, Primitive, Sort, Term};

    #[test]
    fn boolean_conflicts_backjump_over_irrelevant_decisions() {
        let atom = |local| {
            Formula::equal(
                Term::Rigid {
                    declaration: DeclarationId { package: 79, local },
                    sort: Sort::Type,
                },
                Term::Primitive(Primitive::Bool),
            )
        };
        let a = atom(10);
        let b = atom(11);
        let mut parts: Vec<_> = (0..6)
            .map(|i| Formula::Or(vec![atom(i), atom(i).negate()]))
            .collect();
        parts.extend([
            Formula::Or(vec![a.clone(), b.clone()]),
            Formula::Or(vec![a.clone().negate(), b.clone()]),
            Formula::Or(vec![a.clone(), b.clone().negate()]),
            Formula::Or(vec![a.negate(), b.negate()]),
        ]);
        let formula = Formula::And(parts);
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
        let mut assignment = vec![None; atoms.len()];
        let SearchResult::Unsat(conflict) = search.solve(&mut assignment, 0) else {
            panic!("expected contradiction")
        };
        assert!(assignment.iter().all(Option::is_none));
        assert!(conflict.support.is_empty());
        assert!(
            conflict
                .to_string()
                .starts_with("(conflict (support) (proof (split")
        );
        let mut pending = vec![&conflict.tree];
        let mut splits = 0;
        while let Some(node) = pending.pop() {
            match node {
                ProofTree::Split {
                    atom,
                    negative,
                    positive,
                } => {
                    assert!(
                        atoms[6..].contains(atom),
                        "irrelevant choice retained in proof"
                    );
                    splits += 1;
                    pending.extend([negative.as_ref(), positive.as_ref()]);
                }
                _ => {}
            }
        }
        assert!(splits <= 3, "duplicated search proof: {splits}");
        search.steps = 100_000;
        search
            .check_tree(&conflict.tree, &mut assignment, 0)
            .unwrap();
    }
}
