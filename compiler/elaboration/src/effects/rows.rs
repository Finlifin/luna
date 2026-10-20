//! Temporary, scope-bound occurrence maps. These justify row positions only;
//! operation formation, handler coverage, ABI and resource checks are separate.
use super::{Error, Relation, charge, relation};
use crate::ptc::{Budget, Context};
use nir::effects::Operation;
use std::fmt;

#[derive(Debug)]
pub struct RowEmbedding<'a> {
    source: &'a [Operation],
    target: &'a [Operation],
    _context: &'a Context,
    indices: Vec<usize>,
}
impl<'a> RowEmbedding<'a> {
    /// Each source occurrence has its own target slot; repeated requests reuse
    /// this mapping and do not consume the slots.
    pub fn target_index(&self, source_index: usize) -> Option<usize> {
        self.indices.get(source_index).copied()
    }
    /// Compose call-boundary maps without confusing equal spellings from
    /// different scopes. A substituted middle row needs a fresh embedding.
    pub fn then(&self, next: &RowEmbedding<'a>, budget: Budget) -> Result<RowEmbedding<'a>, Error> {
        if !std::ptr::eq(self._context, next._context) {
            return Err(Error::InvalidInput);
        }
        let mut work = Work::new(budget, 0);
        work.row(self.target)?;
        work.row(next.source)?;
        if self.target != next.source {
            return Err(Error::InvalidInput);
        }
        work.spend(self.indices.len())?;
        let indices = self
            .indices
            .iter()
            .map(|&middle| next.indices.get(middle).copied().ok_or(Error::InvalidInput))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RowEmbedding {
            source: self.source,
            target: next.target,
            _context: self._context,
            indices,
        })
    }
    pub fn source(&self) -> &[Operation] {
        self.source
    }
    pub fn target(&self) -> &[Operation] {
        self.target
    }
}
impl fmt::Display for RowEmbedding<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(effect-row-embedding (source")?;
        for operation in self.source {
            write!(f, " {operation}")?;
        }
        f.write_str(") (target")?;
        for operation in self.target {
            write!(f, " {operation}")?;
        }
        f.write_str(") (mapping")?;
        for (source, target) in self.indices.iter().enumerate() {
            write!(f, " (occurrence {source} {target})")?;
        }
        f.write_str("))")
    }
}

#[derive(Debug)]
pub struct RowLayer<'a> {
    row: &'a [Operation],
    _context: &'a Context,
    index: usize,
}
impl RowLayer<'_> {
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn operation(&self) -> &Operation {
        &self.row[self.index]
    }
    /// Removing this position is a row operation, not proof that a handler
    /// covers all requests. Partial handlers still require an outer route.
    pub fn remaining(&self) -> impl Iterator<Item = &Operation> {
        self.row
            .iter()
            .enumerate()
            .filter_map(|(index, op)| (index != self.index).then_some(op))
    }
}
impl fmt::Display for RowLayer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(effect-row-layer (index {}) (operation {}) (remaining",
            self.index,
            self.operation()
        )?;
        for operation in self.remaining() {
            write!(f, " {operation}")?;
        }
        f.write_str("))")
    }
}

struct Work {
    fuel: usize,
    comparisons: usize,
    depth: usize,
}
impl Work {
    fn new(budget: Budget, comparisons: usize) -> Self {
        Self {
            fuel: budget.steps,
            comparisons,
            depth: budget.max_depth.min(64),
        }
    }
    fn spend(&mut self, count: usize) -> Result<(), Error> {
        self.fuel = self.fuel.checked_sub(count).ok_or(Error::BudgetExhausted)?;
        Ok(())
    }
    fn operation(&mut self, operation: &Operation, validate: bool) -> Result<(), Error> {
        self.spend(1)?;
        if operation.arguments.len() > self.fuel {
            return Err(Error::BudgetExhausted);
        }
        for argument in &operation.arguments {
            charge(argument, &mut self.fuel, self.depth)?;
            if validate {
                argument.check_scope(&[]).map_err(|_| Error::InvalidInput)?;
            }
        }
        Ok(())
    }
    fn row(&mut self, row: &[Operation]) -> Result<(), Error> {
        if row.len() > self.fuel {
            return Err(Error::BudgetExhausted);
        }
        for operation in row {
            self.operation(operation, true)?;
        }
        Ok(())
    }
    fn compare(
        &mut self,
        a: &Operation,
        b: &Operation,
        context: &Context,
    ) -> Result<Relation, Error> {
        // Repeated comparisons also pay for recursive Eq and copying inputs.
        self.operation(a, false)?;
        self.operation(b, false)?;
        if a.declaration != b.declaration || a.arguments.len() != b.arguments.len() {
            self.comparisons = self.comparisons.saturating_sub(1);
            return Ok(Relation::Different);
        }
        if a == b {
            self.comparisons = self.comparisons.saturating_sub(1);
            return Ok(Relation::Same);
        }
        let allowance = self.fuel / self.comparisons.max(1);
        if allowance == 0 {
            return Err(Error::BudgetExhausted);
        }
        self.spend(allowance)?;
        self.comparisons = self.comparisons.saturating_sub(1);
        Ok(relation(
            a,
            b,
            context,
            Budget {
                steps: allowance,
                max_depth: self.depth,
            },
        ))
    }
}

/// Embed a required row in a common allowed environment. Every occurrence maps
/// to a distinct slot, in the original order for equal labels. Extra allowed
/// labels are not erased; the caller retains the target environment.
pub fn embed_row<'a>(
    source: &'a [Operation],
    target: &'a [Operation],
    context: &'a Context,
    budget: Budget,
) -> Result<RowEmbedding<'a>, Error> {
    let comparisons = source
        .len()
        .checked_mul(target.len())
        .ok_or(Error::BudgetExhausted)?;
    let mut work = Work::new(budget, comparisons);
    work.row(source)?;
    work.row(target)?;
    if source.len() > target.len() {
        return Err(Error::MissingEntry);
    }
    work.spend(target.len().saturating_add(source.len()))?;
    let mut consumed = vec![false; target.len()];
    let mut indices = Vec::with_capacity(source.len());
    for operation in source {
        let mut found = None;
        for (index, candidate) in target.iter().enumerate() {
            work.spend(1)?;
            if consumed[index] {
                continue;
            }
            match work.compare(operation, candidate, context)? {
                Relation::Same => {
                    found = Some(index);
                    break;
                }
                Relation::Different => {}
                Relation::Pending => return Err(Error::PendingIdentity),
            }
        }
        let index = found.ok_or(Error::MissingEntry)?;
        consumed[index] = true;
        indices.push(index);
    }
    Ok(RowEmbedding {
        source,
        target,
        _context: context,
        indices,
    })
}

/// Select the zero-based layer of one complete operation instance. Unknown
/// earlier identities block selection rather than silently changing its rank.
pub fn select_layer<'a>(
    row: &'a [Operation],
    operation: &Operation,
    layer: usize,
    context: &'a Context,
    budget: Budget,
) -> Result<RowLayer<'a>, Error> {
    let mut work = Work::new(budget, row.len());
    work.row(row)?;
    work.operation(operation, true)?;
    let mut remaining = layer;
    for (index, candidate) in row.iter().enumerate() {
        match work.compare(operation, candidate, context)? {
            Relation::Different => {}
            Relation::Pending => return Err(Error::PendingIdentity),
            Relation::Same if remaining == 0 => {
                return Ok(RowLayer {
                    row,
                    _context: context,
                    index,
                });
            }
            Relation::Same => remaining -= 1,
        }
    }
    Err(Error::MissingEntry)
}
