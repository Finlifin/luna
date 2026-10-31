//! Query-local reversible equality closure. Checkers and conflict minimization
//! deliberately use fresh classes rather than this search acceleration state.
use super::{Search, TheoryModel, UnknownReason};
use std::fmt;

pub(super) struct Classes {
    parent: Vec<usize>,
    size: Vec<usize>,
    undo: Vec<(usize, usize)>,
}
impl Classes {
    pub fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
            size: vec![1; count],
            undo: Vec::new(),
        }
    }
    pub fn root(&self, mut id: usize) -> usize {
        // Path compression would require separate undo entries. Union by size
        // bounds path length while keeping each merge a single reversible step.
        while self.parent[id] != id {
            id = self.parent[id];
        }
        id
    }
    pub fn merge(&mut self, a: usize, b: usize) -> bool {
        let (mut a, mut b) = (self.root(a), self.root(b));
        if a == b {
            return false;
        }
        if self.size[a] < self.size[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.undo.push((b, a));
        self.parent[b] = a;
        self.size[a] += self.size[b];
        true
    }
    fn checkpoint(&self) -> usize {
        self.undo.len()
    }
    fn rollback(&mut self, checkpoint: usize) {
        while self.undo.len() > checkpoint {
            let (child, parent) = self.undo.pop().unwrap();
            self.size[parent] -= self.size[child];
            self.parent[child] = child;
        }
    }
    fn snapshot(&self) -> Self {
        Self {
            parent: self.parent.clone(),
            size: self.size.clone(),
            undo: Vec::new(),
        }
    }
}
impl fmt::Display for Classes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(equality-classes (roots")?;
        for i in 0..self.parent.len() {
            write!(f, " {}", self.root(i))?;
        }
        write!(f, ") (undo")?;
        for (child, parent) in &self.undo {
            write!(f, " (merge {child} {parent})")?;
        }
        write!(f, "))")
    }
}

pub(super) struct State {
    classes: Classes,
    layers: Vec<(Vec<Option<bool>>, usize)>,
}
impl State {
    fn new(count: usize) -> Self {
        Self {
            classes: Classes::new(count),
            layers: Vec::new(),
        }
    }
    fn prepare(
        &mut self,
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<(), UnknownReason> {
        while let Some((previous, _)) = self.layers.last() {
            let mut compatible = true;
            for (old, new) in previous.iter().zip(assignment) {
                *fuel = fuel.checked_sub(1).ok_or(UnknownReason::BudgetExhausted)?;
                if old.is_some() && old != new {
                    compatible = false;
                    break;
                }
            }
            if compatible {
                break;
            }
            self.layers.pop();
            let checkpoint = self.layers.last().map_or(0, |(_, checkpoint)| *checkpoint);
            self.classes.rollback(checkpoint);
        }
        Ok(())
    }
}
impl Search<'_> {
    pub(super) fn theory_cached(
        &mut self,
        assignment: &[Option<bool>],
    ) -> Result<Option<TheoryModel>, UnknownReason> {
        let mut state = self
            .theory_state
            .take()
            .unwrap_or_else(|| State::new(self.terms.nodes.len()));
        let prepared = state.prepare(assignment, &mut self.steps);
        if let Err(error) = prepared {
            self.theory_state = Some(state);
            return Err(error);
        }
        let checkpoint = state.classes.checkpoint();
        // Building roots, dependency vectors and a first dirty frontier costs
        // at least four linear passes. Small catalogs are cheaper to scan.
        let count = self.terms.nodes.len();
        let pair_count = count.saturating_mul(count.saturating_sub(1)) / 2;
        let baseline = if state.layers.is_empty() || pair_count <= count.saturating_mul(4) {
            Ok(None)
        } else if let Some(remaining) = self.steps.checked_sub(self.terms.nodes.len()) {
            self.steps = remaining;
            Ok(Some(
                (0..self.terms.nodes.len())
                    .map(|id| state.classes.root(id))
                    .collect(),
            ))
        } else {
            Err(UnknownReason::BudgetExhausted)
        };
        let result = baseline.and_then(|baseline| {
            self.theory_with_classes(assignment, &mut state.classes, baseline)
        });
        let result = match result {
            Ok(Some(evidence)) => {
                let cost = assignment
                    .len()
                    .saturating_add(self.terms.nodes.len().saturating_mul(2));
                if let Some(remaining) = self.steps.checked_sub(cost) {
                    self.steps = remaining;
                    if !state
                        .layers
                        .last()
                        .is_some_and(|(previous, _)| previous == assignment)
                    {
                        state
                            .layers
                            .push((assignment.to_vec(), state.classes.checkpoint()));
                    }
                    Ok(Some(TheoryModel {
                        classes: state.classes.snapshot(),
                        kinds: evidence.kinds,
                        integers: evidence.integers,
                    }))
                } else {
                    state.classes.rollback(checkpoint);
                    Err(UnknownReason::BudgetExhausted)
                }
            }
            other => {
                state.classes.rollback(checkpoint);
                other.map(|_| None)
            }
        };
        self.theory_state = Some(state);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rollback_restores_roots_sizes_and_nested_checkpoints() {
        let mut classes = Classes::new(6);
        classes.merge(0, 1);
        let first = classes.checkpoint();
        classes.merge(2, 3);
        classes.merge(3, 4);
        let second = classes.checkpoint();
        classes.merge(0, 4);
        classes.merge(5, 0);
        assert!(!classes.merge(1, 5));
        classes.rollback(second);
        assert_ne!(classes.root(0), classes.root(2));
        assert_eq!(classes.size[classes.root(2)], 3);
        assert_eq!(classes.root(5), 5);
        classes.rollback(first);
        assert_eq!(classes.size[classes.root(0)], 2);
        assert_eq!(classes.root(4), 4);
        classes.rollback(0);
        assert_eq!(
            classes.to_string(),
            "(equality-classes (roots 0 1 2 3 4 5) (undo))"
        );
        assert_eq!(classes.size, vec![1; 6]);
    }
    #[test]
    fn warm_closure_matches_fresh_checks_across_branches_and_interruptions() {
        use crate::{Context, Formula, Terms, collect, learning};
        use nir::{DeclarationId, Primitive, Sort, Term};
        let rigid = |local| Term::Rigid {
            declaration: DeclarationId { package: 93, local },
            sort: Sort::Type,
        };
        let x = rigid(0);
        let y = rigid(1);
        let nested = |mut term| {
            for _ in 0..6 {
                term = Term::Optional(Box::new(term));
            }
            term
        };
        let formula = Formula::And(vec![
            Formula::equal(x.clone(), y.clone()),
            Formula::equal(nested(x.clone()), nested(y.clone())),
            Formula::equal(y, Term::Primitive(Primitive::Bool)),
            Formula::equal(x, Term::Primitive(Primitive::U8)),
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
        let compare = |warm: Option<TheoryModel>, fresh: Option<TheoryModel>| {
            assert_eq!(warm.is_some(), fresh.is_some());
            if let (Some(warm), Some(fresh)) = (warm, fresh) {
                assert_eq!(warm.integers, fresh.integers);
                for a in 0..terms.nodes.len() {
                    for b in 0..terms.nodes.len() {
                        assert_eq!(
                            warm.classes.root(a) == warm.classes.root(b),
                            fresh.classes.root(a) == fresh.classes.root(b)
                        );
                    }
                }
            }
        };
        let mut consistent = 0;
        let mut inconsistent = 0;
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
            let warm = search.theory_cached(&assignment).unwrap();
            if warm.is_some() {
                consistent += 1;
            } else {
                inconsistent += 1;
            }
            let fresh = search.theory(&assignment).unwrap();
            compare(warm, fresh);
        }
        assert!(consistent > 0 && inconsistent > 0);
        for budget in 0..256 {
            search.steps = budget;
            let _ = search.theory_cached(&[Some(true), None, Some(true), None]);
            search.steps = 100000;
            let assignment = [Some(false), None, Some(false), Some(true)];
            let warm = search.theory_cached(&assignment).unwrap();
            let fresh = search.theory(&assignment).unwrap();
            compare(warm, fresh);
        }
    }
    #[test]
    fn retained_structural_closure_avoids_repeating_decomposition_rounds() {
        use crate::{Context, Formula, Terms, collect, learning};
        use nir::{DeclarationId, Primitive, Sort, Term};
        let mut left = Term::Rigid {
            declaration: DeclarationId {
                package: 94,
                local: 0,
            },
            sort: Sort::Type,
        };
        let mut right = Term::Primitive(Primitive::Bool);
        for _ in 0..8 {
            left = Term::Optional(Box::new(left));
            right = Term::Optional(Box::new(right));
        }
        let formula = Formula::equal(left, right);
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
        assert!(search.theory_cached(&[Some(true)]).unwrap().is_some());
        search.steps = 100000;
        let warm = search.theory_cached(&[Some(true)]).unwrap().unwrap();
        let warm_cost = 100000 - search.steps;
        search.steps = 100000;
        let fresh = search.theory(&[Some(true)]).unwrap().unwrap();
        let fresh_cost = 100000 - search.steps;
        assert!(
            warm_cost < fresh_cost,
            "warm={warm_cost}, fresh={fresh_cost}"
        );
        for a in 0..terms.nodes.len() {
            for b in 0..terms.nodes.len() {
                assert_eq!(
                    warm.classes.root(a) == warm.classes.root(b),
                    fresh.classes.root(a) == fresh.classes.root(b)
                );
            }
        }
    }
}
