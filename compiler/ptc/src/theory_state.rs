//! Query-local reversible equality closure, validated type facts and integer
//! potentials. Checkers and conflict minimization use fresh theory state.
use super::{Atom, Search, TheoryModel, TypeKind, UnknownReason, integer};

mod members;
mod propagation;

#[cfg(test)]
mod reuse_tests;

use std::{collections::BTreeSet, fmt};

pub(super) enum Check {
    Conflict,
    Consistent,
    Model(TheoryModel),
}

pub(super) struct Classes {
    parent: Vec<usize>,
    size: Vec<usize>,
    // Disjoint circular lists. Swapping two roots' links joins their classes;
    // the same swap undoes that join after later merges have been reverted.
    next: Vec<usize>,
    undo: Vec<(usize, usize)>,
}

impl Classes {
    pub fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
            size: vec![1; count],
            next: (0..count).collect(),
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
        self.next.swap(a, b);
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
            self.next.swap(parent, child);
        }
    }

    fn snapshot(&self) -> Self {
        Self {
            parent: self.parent.clone(),
            size: self.size.clone(),
            next: self.next.clone(),
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
        write!(f, ") (next")?;
        for next in &self.next {
            write!(f, " {next}")?;
        }
        write!(f, ") (undo")?;
        for (child, parent) in &self.undo {
            write!(f, " (merge {child} {parent})")?;
        }
        write!(f, "))")
    }
}

struct Layer {
    atoms: Vec<usize>,
    checkpoint: usize,
    integers: integer::Checkpoint,
}

pub(super) struct State {
    classes: Classes,
    integers: integer::State,
    // Successful type-theory synchronization for the retained facts/classes.
    // Integer additions do not change this closure. Retractions invalidate it;
    // a failed extension must never replace the last committed summary.
    kinds: Option<Vec<(usize, TypeKind, bool)>>,
    not_sized: Option<Vec<(usize, bool)>>,
    // Each admitted atom has exactly one introduction layer, not one copy in
    // every later layer. Pending atoms have not yet earned retained evidence.
    facts: Vec<Option<(bool, usize)>>,
    layers: Vec<Layer>,
    pending: BTreeSet<usize>,
}

fn charge(fuel: &mut usize, work: usize) -> Result<(), UnknownReason> {
    *fuel = fuel
        .checked_sub(work)
        .ok_or(UnknownReason::BudgetExhausted)?;
    Ok(())
}

impl State {
    fn new(
        count: usize,
        atoms: &[Atom],
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Self, UnknownReason> {
        charge(
            fuel,
            count.saturating_mul(3).saturating_add(assignment.len()),
        )?;
        Ok(Self {
            classes: Classes::new(count),
            integers: integer::State::new(atoms, fuel)?,
            kinds: None,
            not_sized: None,
            facts: vec![None; assignment.len()],
            layers: Vec::new(),
            pending: assignment
                .iter()
                .enumerate()
                .filter_map(|(index, value)| value.is_some().then_some(index))
                .collect(),
        })
    }

    fn prepare(
        &mut self,
        assignment: &[Option<bool>],
        changed: Option<&BTreeSet<usize>>,
        fuel: &mut usize,
    ) -> Result<(), UnknownReason> {
        debug_assert_eq!(assignment.len(), self.facts.len());
        if let Some(changed) = changed {
            for &atom in changed {
                charge(fuel, 1)?;
                self.pending.insert(atom);
            }
        }
        let mut cut = self.layers.len();
        for &atom in &self.pending {
            charge(fuel, 1)?;
            if let Some((value, layer)) = self.facts[atom] {
                if assignment[atom] != Some(value) {
                    cut = cut.min(layer);
                }
            }
        }
        if cut == self.layers.len() {
            return Ok(());
        }
        let checkpoint = if cut == 0 {
            0
        } else {
            self.layers[cut - 1].checkpoint
        };
        let integers = if cut == 0 {
            integer::Checkpoint::default()
        } else {
            self.layers[cut - 1].integers
        };
        // Reserve rollback before changing facts, classes or integer potentials.
        // On interruption only extra pending notifications may have been added.
        for layer in &self.layers[cut..] {
            charge(fuel, layer.atoms.len().saturating_add(1))?;
        }
        charge(fuel, self.classes.checkpoint() - checkpoint)?;
        charge(fuel, self.integers.rollback_work(integers))?;
        while self.layers.len() > cut {
            let layer = self.layers.pop().unwrap();
            for atom in layer.atoms {
                self.facts[atom] = None;
                self.pending.insert(atom);
            }
        }
        self.classes.rollback(checkpoint);
        self.integers.rollback(integers);
        self.kinds = None;
        self.not_sized = None;
        Ok(())
    }

    // Caller reserves pending traversal and snapshot work before publication.
    fn commit(&mut self, assignment: &[Option<bool>]) {
        let atoms: Vec<_> = self
            .pending
            .iter()
            .copied()
            .filter(|atom| self.facts[*atom].is_none() && assignment[*atom].is_some())
            .collect();
        if !atoms.is_empty() || self.layers.is_empty() {
            let layer = self.layers.len();
            for &atom in &atoms {
                self.facts[atom] = Some((assignment[atom].unwrap(), layer));
            }
            self.layers.push(Layer {
                atoms,
                checkpoint: self.classes.checkpoint(),
                integers: self.integers.checkpoint(),
            });
        } else {
            let last = self.layers.last_mut().unwrap();
            last.checkpoint = self.classes.checkpoint();
            last.integers = self.integers.checkpoint();
        }
        self.pending.clear();
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(theory-state {} {} (type-summary",
            self.classes, self.integers
        )?;
        match &self.kinds {
            None => write!(f, " pending")?,
            Some(kinds) => {
                write!(f, " checked")?;
                for (root, kind, truth) in kinds {
                    write!(f, " (kind {root} {kind:?} {truth})")?;
                }
            }
        }
        write!(f, ") (facts")?;
        for (atom, fact) in self.facts.iter().enumerate() {
            if let Some((value, layer)) = fact {
                write!(f, " (atom {atom} {value} (layer {layer}))")?;
            }
        }
        write!(f, ") (layers")?;
        for layer in &self.layers {
            write!(
                f,
                " (layer (checkpoint {}) {} (atoms",
                layer.checkpoint, layer.integers
            )?;
            for atom in &layer.atoms {
                write!(f, " {atom}")?;
            }
            write!(f, "))")?;
        }
        write!(f, ") (pending")?;
        for atom in &self.pending {
            write!(f, " {atom}")?;
        }
        write!(f, "))")
    }
}

impl Search<'_> {
    #[cfg(test)]
    pub(super) fn theory_cached(
        &mut self,
        assignment: &[Option<bool>],
    ) -> Result<Option<TheoryModel>, UnknownReason> {
        self.theory_cached_changes(assignment, Some(&(0..assignment.len()).collect()), true)
            .map(|result| match result {
                Check::Conflict => None,
                Check::Model(model) => Some(model),
                Check::Consistent => unreachable!("requested an owned model"),
            })
    }

    pub(super) fn theory_cached_changes(
        &mut self,
        assignment: &[Option<bool>],
        changed: Option<&BTreeSet<usize>>,
        capture_model: bool,
    ) -> Result<Check, UnknownReason> {
        let mut state = match (self.theory_state.take(), changed) {
            (Some(state), Some(_)) => state,
            _ => State::new(
                self.terms.nodes.len(),
                self.atoms,
                assignment,
                &mut self.steps,
            )?,
        };
        let prepared = state.prepare(assignment, changed, &mut self.steps);
        if let Err(error) = prepared {
            self.theory_state = Some(state);
            return Err(error);
        }
        let checkpoint = state.classes.checkpoint();
        let integers = state.integers.checkpoint();
        let result = (|| {
            let mut type_changed = state.kinds.is_none();
            for &index in &state.pending {
                charge(&mut self.steps, 1)?;
                if state.facts[index].is_none() && assignment[index].is_some() {
                    // Only IntLe is independent of this type closure. New atom
                    // kinds must be rechecked unless their separation is known.
                    type_changed |= !matches!(self.atoms[index], Atom::IntLe(_));
                }
            }
            let next_summary = if type_changed {
                // Building roots, dependency vectors and a first dirty frontier
                // costs at least four passes. Small catalogs are cheaper to scan.
                let count = self.terms.nodes.len();
                let pair_count = count.saturating_mul(count.saturating_sub(1)) / 2;
                let baseline = if state.layers.is_empty() || pair_count <= count.saturating_mul(4) {
                    None
                } else {
                    charge(&mut self.steps, count)?;
                    Some((0..count).map(|id| state.classes.root(id)).collect())
                };
                let Some(summary) =
                    self.type_theory_with_classes(assignment, &mut state.classes, baseline)?
                else {
                    return Ok(Check::Conflict);
                };
                Some(summary)
            } else {
                None
            };
            for &index in &state.pending {
                charge(&mut self.steps, 1)?;
                if state.facts[index].is_none() {
                    if let (Atom::IntLe(atom), Some(value)) =
                        (&self.atoms[index], assignment[index])
                    {
                        state.integers.admit(index, atom, value, &mut self.steps)?;
                    }
                }
            }
            if !state.integers.stabilize(integers, &mut self.steps)? {
                return Ok(Check::Conflict);
            }
            let cost = state
                .pending
                .len()
                .saturating_mul(2)
                .saturating_add(1)
                .saturating_add(if capture_model {
                    self.terms.nodes.len().saturating_mul(3)
                } else {
                    0
                });
            charge(&mut self.steps, cost)?;
            let result = if capture_model {
                let kinds = next_summary
                    .as_ref()
                    .map(|(kinds, _)| kinds)
                    .or(state.kinds.as_ref())
                    .expect("checked type closure");
                let not_sized = next_summary
                    .as_ref()
                    .map(|(_, not_sized)| not_sized)
                    .or(state.not_sized.as_ref())
                    .expect("checked type closure");
                charge(&mut self.steps, kinds.len().saturating_add(not_sized.len()))?;
                Check::Model(TheoryModel {
                    classes: state.classes.snapshot(),
                    kinds: kinds.clone(),
                    not_sized: not_sized.clone(),
                    integers: state.integers.model(&mut self.steps)?,
                })
            } else {
                Check::Consistent
            };
            state.commit(assignment);
            if let Some((kinds, not_sized)) = next_summary {
                state.kinds = Some(kinds);
                state.not_sized = Some(not_sized);
            }
            Ok(result)
        })();
        if matches!(result, Err(_) | Ok(Check::Conflict)) {
            state.classes.rollback(checkpoint);
            state.integers.rollback(integers);
        }
        self.theory_state = Some(state);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_layers_match_fresh_search_and_bounded_models_across_retractions() {
        use crate::{Context, Formula, IntLe, IntegerTerm, Terms, collect, learning};
        use nir::{DeclarationId, Sort, Term};
        use std::collections::BTreeMap;
        let x = DeclarationId {
            package: 107,
            local: 0,
        };
        let y = DeclarationId {
            package: 107,
            local: 1,
        };
        let variable = |id, offset| IntegerTerm::variable(id, offset);
        let bounds = [
            IntLe {
                left: variable(x, 0),
                right: variable(y, 0),
            },
            IntLe {
                left: variable(y, 1),
                right: variable(x, 0),
            },
            IntLe {
                left: variable(x, 0),
                right: IntegerTerm::constant(0),
            },
            IntLe {
                left: IntegerTerm::constant(0),
                right: variable(y, 0),
            },
            IntLe {
                left: variable(y, 0),
                right: IntegerTerm::constant(1),
            },
        ];
        let mut atoms: Vec<_> = bounds.iter().cloned().map(Atom::IntLe).collect();
        // A type-only notification must neither lose nor duplicate integer edges.
        let ty = Term::Rigid {
            declaration: DeclarationId {
                package: 107,
                local: 2,
            },
            sort: Sort::Type,
        };
        let formula = Formula::equal(ty.clone(), ty);
        let mut terms = Terms::default();
        collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
        assert_eq!(atoms.len(), 6);
        let context = Context::default();
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
        let check = |search: &mut Search<'_>, assignment: &[Option<bool>]| {
            search.steps = 100000;
            let warm = search.theory_cached(assignment).unwrap();
            let fresh = search.theory(assignment).unwrap();
            assert_eq!(warm.is_some(), fresh.is_some(), "{assignment:?}");
            let exists = (-4..=4).any(|a| {
                (-4..=4).any(|b| {
                    let values = BTreeMap::from([(x, a), (y, b)]);
                    bounds.iter().zip(assignment).all(|(atom, value)| {
                        value.is_none_or(|value| atom.evaluate(&values) == Some(value))
                    })
                })
            });
            assert_eq!(warm.is_some(), exists, "{assignment:?}");
            if let Some(model) = warm {
                for (atom, value) in bounds.iter().zip(assignment) {
                    if let Some(value) = value {
                        assert_eq!(atom.evaluate(&model.integers), Some(*value));
                    }
                }
            }
        };
        for code in (0..243).chain((0..243).rev()) {
            let mut cursor = code;
            let mut assignment: Vec<_> = (0..5)
                .map(|_| {
                    let value = [None, Some(false), Some(true)][cursor % 3];
                    cursor /= 3;
                    value
                })
                .collect();
            assignment.push((code % 2 == 0).then_some(true));
            check(&mut search, &assignment);
        }
        let prefix = [Some(true), Some(false), None, Some(true), None, None];
        let extension = [
            Some(true),
            Some(false),
            Some(false),
            Some(true),
            Some(false),
            Some(true),
        ];
        for budget in 0..256 {
            check(&mut search, &prefix);
            search.steps = budget;
            let _ = search.theory_cached(&extension);
            check(&mut search, &prefix);
            check(&mut search, &extension);
        }
        search.steps = 100000;
        search.theory_cached(&extension).unwrap();
        let retained = search.theory_state.as_ref().unwrap().integers.to_string();
        let mut type_only = extension;
        type_only[5] = None;
        search
            .theory_cached_changes(&type_only, Some(&BTreeSet::from([5])), false)
            .unwrap();
        assert_eq!(
            search.theory_state.as_ref().unwrap().integers.to_string(),
            retained
        );
        search.steps = 100000;
        search
            .theory_cached_changes(&type_only, Some(&BTreeSet::new()), false)
            .unwrap();
        let warm_cost = 100000 - search.steps;
        search.steps = 100000;
        search.theory(&type_only).unwrap();
        assert!(warm_cost < 100000 - search.steps);
    }

    #[test]
    fn sparse_layers_rollback_atomically_and_readmit_unchanged_facts() {
        for budget in 0..12 {
            let mut assignment = vec![None; 4096];
            let mut state = State::new(4, &[], &assignment, &mut 100000).unwrap();
            for atom in 0..3 {
                assignment[atom] = Some(true);
                state
                    .prepare(&assignment, Some(&BTreeSet::from([atom])), &mut 1000)
                    .unwrap();
                state.classes.merge(atom, atom + 1);
                state.commit(&assignment);
            }
            assert_eq!(state.layers.iter().map(|l| l.atoms.len()).sum::<usize>(), 3);
            state
                .prepare(&assignment, Some(&BTreeSet::new()), &mut 0)
                .unwrap();
            assignment[1] = None;
            let before = state.classes.to_string();
            let facts = state.facts.clone();
            if state
                .prepare(&assignment, Some(&BTreeSet::from([1])), &mut { budget })
                .is_err()
            {
                assert_eq!(state.classes.to_string(), before);
                assert_eq!(state.facts, facts);
            }
            state
                .prepare(&assignment, Some(&BTreeSet::from([1])), &mut 1000)
                .unwrap();
            assert_eq!(state.classes.root(0), state.classes.root(1));
            assert_ne!(state.classes.root(1), state.classes.root(2));
            assert_ne!(state.classes.root(2), state.classes.root(3));
            assert!(state.pending.contains(&2));
            // Rechecking the unchanged later fact must establish a new layer.
            state.classes.merge(2, 3);
            state.commit(&assignment);
            assert_eq!(state.facts[2], Some((true, 1)));
            assignment[2] = Some(false);
            state
                .prepare(&assignment, Some(&BTreeSet::from([2])), &mut 1000)
                .unwrap();
            assert_ne!(state.classes.root(2), state.classes.root(3));
            assert!(
                state
                    .to_string()
                    .starts_with("(theory-state (equality-classes")
            );
        }
    }

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
            "(equality-classes (roots 0 1 2 3 4 5) (next 0 1 2 3 4 5) (undo))"
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
        let mut previous = vec![None; atoms.len()];
        let mut pending = BTreeSet::new();
        let mut cached = |search: &mut Search<'_>, assignment: &[Option<bool>]| {
            for (index, (old, new)) in previous.iter_mut().zip(assignment).enumerate() {
                if old != new {
                    pending.insert(index);
                    *old = *new;
                }
            }
            let result = search.theory_cached_changes(assignment, Some(&pending), true);
            if result.is_ok() {
                pending.clear();
            }
            result.map(|result| match result {
                Check::Model(model) => Some(model),
                Check::Conflict => None,
                Check::Consistent => unreachable!(),
            })
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
            let warm = cached(&mut search, &assignment).unwrap();
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
            let _ = cached(&mut search, &[Some(true), None, Some(true), None]);
            search.steps = 100000;
            let assignment = [Some(false), None, Some(false), Some(true)];
            let warm = cached(&mut search, &assignment).unwrap();
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
