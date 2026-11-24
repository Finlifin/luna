//! A16–A18 row predicates (`RowContains`, `RowSubset`, `RowDisjoint`) per
//! `docs/type/propositional-constraints/atoms.md`.
//!
//! Open row tails never imply label absence, and effect membership never
//! synthesizes handler routing or coercion evidence.
use super::*;
use crate::effects::Relation;
use nir::effects::Operation;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowKind {
    Error,
    Effect,
}

impl fmt::Display for RowKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Error => "error",
            Self::Effect => "effect",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RowLabel {
    Error(Term),
    Effect(Operation),
}

impl fmt::Display for RowLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error(ty) => ty.fmt(f),
            Self::Effect(op) => op.fmt(f),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RowContainsQuery {
    pub kind: RowKind,
    pub row: Term,
    pub label: RowLabel,
}

impl fmt::Display for RowContainsQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(row-contains {} {} {})",
            self.kind, self.row, self.label
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RowSubsetQuery {
    pub left: Term,
    pub right: Term,
}

impl fmt::Display for RowSubsetQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(row-subset error {} {})", self.left, self.right)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RowDisjointQuery {
    pub left: Term,
    pub right: Term,
}

impl fmt::Display for RowDisjointQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(row-disjoint error {} {})", self.left, self.right)
    }
}

struct ErrorRowSummary {
    labels: BTreeSet<usize>,
    tail_root: usize,
    tail_closed: bool,
}

struct EffectRowSummary<'a> {
    operations: Vec<&'a Operation>,
    tail_closed: bool,
}

fn closed_error_labels(ty: &Term, fuel: &std::cell::Cell<usize>) -> Option<Vec<Term>> {
    let mut labels = Vec::new();
    let mut current = ty;
    for _ in 0..128 {
        let next = fuel.get().checked_sub(1)?;
        fuel.set(next);
        match current {
            Term::ErrorResult { errors, result } => {
                labels.extend(errors.iter().cloned());
                current = result.as_ref();
            }
            _ => return Some(labels),
        }
    }
    None
}

fn closed_effect_operations(ty: &Term, fuel: &std::cell::Cell<usize>) -> Option<Vec<Operation>> {
    let mut ops = Vec::new();
    let mut current = ty;
    for _ in 0..128 {
        let next = fuel.get().checked_sub(1)?;
        fuel.set(next);
        match current {
            Term::Computation { effects, result } => {
                ops.extend(effects.iter().cloned());
                current = result.as_ref();
            }
            _ => return Some(ops),
        }
    }
    None
}

pub(super) fn eval_model_row_contains(
    query: &RowContainsQuery,
    row: &Term,
    label: &RowLabel,
    fuel: &std::cell::Cell<usize>,
) -> Option<bool> {
    match (query.kind, label) {
        (RowKind::Error, RowLabel::Error(target)) => {
            let labels = closed_error_labels(row, fuel)?;
            Some(labels.iter().any(|item| item == target))
        }
        (RowKind::Effect, RowLabel::Effect(target)) => {
            let ops = closed_effect_operations(row, fuel)?;
            Some(ops.iter().any(|item| item == target))
        }
        _ => None,
    }
}

pub(super) fn eval_model_row_subset(
    left: &Term,
    right: &Term,
    fuel: &std::cell::Cell<usize>,
) -> Option<bool> {
    let left_labels = closed_error_labels(left, fuel)?;
    let right_labels = closed_error_labels(right, fuel)?;
    Some(
        left_labels
            .iter()
            .all(|l| right_labels.iter().any(|r| r == l)),
    )
}

pub(super) fn eval_model_row_disjoint(
    left: &Term,
    right: &Term,
    fuel: &std::cell::Cell<usize>,
) -> Option<bool> {
    let left_labels = closed_error_labels(left, fuel)?;
    let right_labels = closed_error_labels(right, fuel)?;
    Some(
        left_labels
            .iter()
            .all(|l| right_labels.iter().all(|r| r != l)),
    )
}

impl<'a> Search<'a> {
    fn is_closed_non_error_class(
        &mut self,
        root: usize,
        classes: &Classes,
    ) -> Result<bool, UnknownReason> {
        for index in classes.ordered_members(root, &mut self.steps)? {
            self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
            let term = &self.terms.nodes[index];
            if constructor(term).is_some()
                || matches!(term, Term::Record(_))
                || self.context.is_tuple_type(term)
                || self.context.is_list_type(term)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn is_closed_non_effect_class(
        &mut self,
        root: usize,
        classes: &Classes,
    ) -> Result<bool, UnknownReason> {
        for index in classes.ordered_members(root, &mut self.steps)? {
            self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
            let term = &self.terms.nodes[index];
            if constructor(term).is_some()
                || matches!(term, Term::Record(_))
                || self.context.is_tuple_type(term)
                || self.context.is_list_type(term)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn error_row_summary(
        &mut self,
        start: usize,
        classes: &Classes,
    ) -> Result<Option<ErrorRowSummary>, UnknownReason> {
        let root = classes.root(start);
        let mut entry = start;
        if !matches!(self.terms.nodes[start], Term::ErrorResult { .. }) {
            for member in classes.ordered_members(root, &mut self.steps)? {
                self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
                if matches!(self.terms.nodes[member], Term::ErrorResult { .. }) {
                    entry = member;
                    break;
                }
            }
        }
        let Some((labels, tail_root)) = self.error_result_view(entry, classes)? else {
            return Ok(None);
        };
        let tail_closed = self.is_closed_non_error_class(tail_root, classes)?;
        Ok(Some(ErrorRowSummary {
            labels,
            tail_root,
            tail_closed,
        }))
    }

    fn effect_row_summary(
        &mut self,
        start: usize,
        classes: &Classes,
    ) -> Result<Option<EffectRowSummary<'a>>, UnknownReason> {
        let root = classes.root(start);
        let mut entry = start;
        if !matches!(self.terms.nodes[start], Term::Computation { .. }) {
            for member in classes.ordered_members(root, &mut self.steps)? {
                self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
                if matches!(self.terms.nodes[member], Term::Computation { .. }) {
                    entry = member;
                    break;
                }
            }
        }
        let Some((operations, tail_root)) = self.computation_view(entry, classes)? else {
            return Ok(None);
        };
        let tail_closed = self.is_closed_non_effect_class(tail_root, classes)?;
        Ok(Some(EffectRowSummary {
            operations,
            tail_closed,
        }))
    }

    pub(super) fn evaluate_row_contains(
        &mut self,
        query: &RowContainsQuery,
        classes: &Classes,
        assignment: &[Option<bool>],
    ) -> Result<Option<bool>, UnknownReason> {
        let row_id = self.terms.ids[&query.row];
        match (query.kind, &query.label) {
            (RowKind::Error, RowLabel::Error(label)) => {
                let Some(summary) = self.error_row_summary(row_id, classes)? else {
                    return Ok(None);
                };
                let label_root = classes.root(self.terms.ids[label]);
                if summary.labels.contains(&label_root) {
                    return Ok(Some(true));
                }
                if !summary.tail_closed {
                    return Ok(None);
                }
                for existing in &summary.labels {
                    if !self.distinct_type_classes(
                        *existing,
                        label_root,
                        classes,
                        assignment,
                        &mut BTreeSet::new(),
                        0,
                    )? {
                        return Ok(None);
                    }
                }
                Ok(Some(false))
            }
            (RowKind::Effect, RowLabel::Effect(op)) => {
                let Some(summary) = self.effect_row_summary(row_id, classes)? else {
                    return Ok(None);
                };
                let mut all_different = true;
                for existing in &summary.operations {
                    match self.operation_relation(existing, op, classes, assignment)? {
                        Relation::Same => return Ok(Some(true)),
                        Relation::Pending => all_different = false,
                        Relation::Different => {}
                    }
                }
                Ok((summary.tail_closed && all_different).then_some(false))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn evaluate_row_subset(
        &mut self,
        query: &RowSubsetQuery,
        classes: &Classes,
        assignment: &[Option<bool>],
    ) -> Result<Option<bool>, UnknownReason> {
        let left_id = self.terms.ids[&query.left];
        let right_id = self.terms.ids[&query.right];
        if classes.root(left_id) == classes.root(right_id) {
            return Ok(Some(true));
        }
        let (Some(left), Some(right)) = (
            self.error_row_summary(left_id, classes)?,
            self.error_row_summary(right_id, classes)?,
        ) else {
            return Ok(None);
        };
        if (left.tail_closed || left.tail_root == right.tail_root)
            && left.labels.iter().all(|label| right.labels.contains(label))
        {
            return Ok(Some(true));
        }
        if right.tail_closed {
            for label in &left.labels {
                let mut absent = true;
                for candidate in &right.labels {
                    if label == candidate
                        || !self.distinct_type_classes(
                            *label,
                            *candidate,
                            classes,
                            assignment,
                            &mut BTreeSet::new(),
                            0,
                        )?
                    {
                        absent = false;
                        break;
                    }
                }
                if absent {
                    return Ok(Some(false));
                }
            }
        }
        Ok(None)
    }

    pub(super) fn evaluate_row_disjoint(
        &mut self,
        query: &RowDisjointQuery,
        classes: &Classes,
        assignment: &[Option<bool>],
    ) -> Result<Option<bool>, UnknownReason> {
        let left_id = self.terms.ids[&query.left];
        let right_id = self.terms.ids[&query.right];
        let (Some(left), Some(right)) = (
            self.error_row_summary(left_id, classes)?,
            self.error_row_summary(right_id, classes)?,
        ) else {
            return Ok(None);
        };
        for label in &left.labels {
            if right.labels.contains(label) {
                return Ok(Some(false));
            }
        }
        if (left.tail_closed && left.labels.is_empty())
            || (right.tail_closed && right.labels.is_empty())
        {
            return Ok(Some(true));
        }
        if !left.tail_closed || !right.tail_closed {
            return Ok(None);
        }
        for l in &left.labels {
            for r in &right.labels {
                if !self.distinct_type_classes(
                    *l,
                    *r,
                    classes,
                    assignment,
                    &mut BTreeSet::new(),
                    0,
                )? {
                    return Ok(None);
                }
            }
        }
        Ok(Some(true))
    }

    /// Check direct and relational consistency for A16–A18 row atoms under `assignment`.
    pub(super) fn check_row_atoms(
        &mut self,
        classes: &Classes,
        assignment: &[Option<bool>],
    ) -> Result<bool, UnknownReason> {
        // 1. Direct evaluation of every assigned row atom.
        let mut err_contains: Vec<(usize, usize, bool)> = Vec::new();
        let mut subsets_true: Vec<(usize, usize)> = Vec::new();
        let mut subsets_false: Vec<(usize, usize)> = Vec::new();
        let mut disjoints: Vec<(usize, usize, bool)> = Vec::new();
        for (atom, value) in self.atoms.iter().zip(assignment) {
            let Some(truth) = *value else { continue };
            match atom {
                Atom::RowContains(query) => {
                    if let Some(actual) = self.evaluate_row_contains(query, classes, assignment)?
                        && actual != truth
                    {
                        return Ok(false);
                    }
                    if let (RowKind::Error, RowLabel::Error(label)) = (query.kind, &query.label) {
                        err_contains.push((
                            classes.root(self.terms.ids[&query.row]),
                            classes.root(self.terms.ids[label]),
                            truth,
                        ));
                    }
                }
                Atom::RowSubset(query) => {
                    if let Some(actual) = self.evaluate_row_subset(query, classes, assignment)?
                        && actual != truth
                    {
                        return Ok(false);
                    }
                    let l = classes.root(self.terms.ids[&query.left]);
                    let r = classes.root(self.terms.ids[&query.right]);
                    if truth {
                        subsets_true.push((l, r));
                    } else {
                        subsets_false.push((l, r));
                    }
                }
                Atom::RowDisjoint(query) => {
                    if let Some(actual) = self.evaluate_row_disjoint(query, classes, assignment)?
                        && actual != truth
                    {
                        return Ok(false);
                    }
                    let l = classes.root(self.terms.ids[&query.left]);
                    let r = classes.root(self.terms.ids[&query.right]);
                    disjoints.push((l, r, truth));
                }
                _ => {}
            }
        }
        // 2. Relational consistency: transitivity of RowSubset, monotonicity of RowContains,
        //    and RowDisjoint vs shared RowContains.
        if !subsets_true.is_empty() {
            let mut adj: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
            for &(l, r) in &subsets_true {
                self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
                adj.entry(l).or_default().push(r);
            }
            let reaches =
                |start: usize, target: usize, steps: &mut usize| -> Result<bool, UnknownReason> {
                    let mut stack = vec![start];
                    let mut visited = BTreeSet::new();
                    while let Some(node) = stack.pop() {
                        *steps = steps.checked_sub(1).ok_or(UnknownReason::BudgetExhausted)?;
                        if node == target {
                            return Ok(true);
                        }
                        if !visited.insert(node) {
                            continue;
                        }
                        if let Some(nexts) = adj.get(&node) {
                            stack.extend(nexts.iter().copied());
                        }
                    }
                    Ok(false)
                };
            for &(l, r) in &subsets_false {
                if reaches(l, r, &mut self.steps)? {
                    return Ok(false);
                }
            }
            // If R <= S, l in R, and l not in S -> conflict.
            for &(r_row, r_label, r_truth) in &err_contains {
                if !r_truth {
                    continue;
                }
                for &(s_row, s_label, s_truth) in &err_contains {
                    if !s_truth && r_label == s_label && reaches(r_row, s_row, &mut self.steps)? {
                        return Ok(false);
                    }
                }
            }
        }
        // Check direct positive/negative RowContains clashes on the same (row, label),
        // and RowDisjoint(R, S) = true vs l in R and l in S (or via subset).
        for i in 0..err_contains.len() {
            let (r1, l1, t1) = err_contains[i];
            for &(r2, l2, t2) in &err_contains[i + 1..] {
                if r1 == r2 && l1 == l2 && t1 != t2 {
                    return Ok(false);
                }
            }
        }
        for &(a, b, is_disjoint) in &disjoints {
            if !is_disjoint {
                // Disjoint(R, S) = false requires intersection; contradictory if already known disjoint
                // is checked in evaluate_row_disjoint above. Also R <= S and Disjoint(S, S) etc.
                continue;
            }
            for &(r1, l1, t1) in &err_contains {
                if !t1 {
                    continue;
                }
                for &(r2, l2, t2) in &err_contains {
                    if !t2 || l1 != l2 {
                        continue;
                    }
                    if (r1 == a && r2 == b) || (r1 == b && r2 == a) {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }
}
