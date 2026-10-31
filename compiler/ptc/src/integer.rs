//! Mathematical integer difference logic. i128 is an implementation capacity,
//! never a source overflow rule: any exhausted arithmetic capacity is Unknown.
use super::UnknownReason;
use nir::DeclarationId;
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IntegerTerm {
    pub variable: Option<DeclarationId>,
    pub offset: i128,
}
impl IntegerTerm {
    pub fn constant(value: i128) -> Self {
        Self {
            variable: None,
            offset: value,
        }
    }
    pub fn variable(variable: DeclarationId, offset: i128) -> Self {
        Self {
            variable: Some(variable),
            offset,
        }
    }
    pub fn evaluate(&self, values: &BTreeMap<DeclarationId, i128>) -> Option<i128> {
        let base = match self.variable {
            Some(id) => *values.get(&id)?,
            None => 0,
        };
        base.checked_add(self.offset)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IntLe {
    pub left: IntegerTerm,
    pub right: IntegerTerm,
}
impl IntLe {
    pub fn evaluate(&self, values: &BTreeMap<DeclarationId, i128>) -> Option<bool> {
        Some(self.left.evaluate(values)? <= self.right.evaluate(values)?)
    }
}

fn tick(steps: &mut usize) -> Result<(), UnknownReason> {
    *steps = steps.checked_sub(1).ok_or(UnknownReason::BudgetExhausted)?;
    Ok(())
}

/// All vertices start at distance zero, equivalently a fresh super-source has
/// zero-cost edges to every vertex. The distinguished zero vertex is shifted
/// back to zero when producing the concrete model.
pub(super) fn solve<'a>(
    literals: impl Iterator<Item = (&'a IntLe, Option<bool>)>,
    steps: &mut usize,
) -> Result<Option<BTreeMap<DeclarationId, i128>>, UnknownReason> {
    let literals: Vec<_> = literals.collect();
    let mut variables = BTreeMap::new();
    for (atom, _) in &literals {
        tick(steps)?;
        for term in [&atom.left, &atom.right] {
            if let Some(id) = term.variable {
                let next = variables.len() + 1;
                variables.entry(id).or_insert(next);
            }
        }
    }
    let index = |term: &IntegerTerm| term.variable.map_or(0, |id| variables[&id]);
    let mut edges = Vec::new();
    for (atom, value) in literals {
        let Some(value) = value else { continue };
        // not (x+a <= y+b) means y-x <= a-b-1 over integers.
        let (from, to, bound) = if value {
            (
                index(&atom.right),
                index(&atom.left),
                atom.right.offset.checked_sub(atom.left.offset),
            )
        } else {
            (
                index(&atom.left),
                index(&atom.right),
                atom.left
                    .offset
                    .checked_sub(atom.right.offset)
                    .and_then(|n| n.checked_sub(1)),
            )
        };
        edges.push((from, to, bound.ok_or(UnknownReason::ArithmeticRange)?));
    }
    let mut distance = vec![0i128; variables.len() + 1];
    for round in 0..distance.len() {
        let mut changed = false;
        for &(from, to, bound) in &edges {
            tick(steps)?;
            let candidate = distance[from]
                .checked_add(bound)
                .ok_or(UnknownReason::ArithmeticRange)?;
            if distance[to] > candidate {
                distance[to] = candidate;
                changed = true;
                if round + 1 == distance.len() {
                    return Ok(None);
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut values = BTreeMap::new();
    for (variable, index) in variables {
        let value = distance[index]
            .checked_sub(distance[0])
            .ok_or(UnknownReason::ArithmeticRange)?;
        values.insert(variable, value);
    }
    Ok(Some(values))
}

impl fmt::Display for IntegerTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.variable {
            Some(id) => write!(f, "(int-offset {id} {})", self.offset),
            None => write!(f, "(integer {})", self.offset),
        }
    }
}
impl fmt::Display for IntLe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(int-le {} {})", self.left, self.right)
    }
}
