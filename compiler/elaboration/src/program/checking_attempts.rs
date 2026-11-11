//! Ownership boundaries for source checking attempts. A probe may inspect a
//! generated expression's type, but must not commit its construction effects.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Disposition {
    Commit,
    Probe,
    RetainCode,
}

impl std::fmt::Display for Disposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Commit => "(checking-disposition commit)",
            Self::Probe => "(checking-disposition probe)",
            Self::RetainCode => "(checking-disposition retain-code)",
        })
    }
}

/// Retains original cell identities and owned graph contents, not cloned VM
/// values with new identities. Work is charged before snapshotting; restoration
/// consumes prepaid work, and never refunds checking/invocation fuel.
struct StorageCheckpoint {
    data: BTreeMap<n::LocalId, Option<n::Expr>>,
    cells: BTreeMap<n::LocalId, (n::Expr, GeneratorCell)>,
    graph: Option<comptime_interpretor::OwnedGraphCheckpoint>,
}
impl StorageCheckpoint {
    fn capture(checker: &mut Checker<'_, '_>, span: Span) -> Result<Self, Error> {
        let work = checker
            .generator_data
            .len()
            .saturating_add(checker.generator_cells.len())
            .saturating_add(checker.quote_holes.len());
        checker.required_steps = checker
            .required_steps
            .checked_sub(work as u64)
            .ok_or_else(|| error(span, "checking storage snapshot budget exhausted"))?;
        for value in checker.generator_data.values().flatten() {
            constants::charge_copy(value, &mut checker.required_steps, span)?;
        }
        for (value, _) in checker.generator_cells.values() {
            constants::charge_copy(value, &mut checker.required_steps, span)?;
        }
        let graph = if checker.generator_cells.is_empty() {
            None
        } else {
            let roots = checker
                .generator_cells
                .values()
                .map(|(_, cell)| cell.value())
                .collect::<Vec<_>>();
            Some(
                comptime_interpretor::OwnedGraphCheckpoint::capture_retained(
                    &roots,
                    &mut checker.required_steps,
                )
                .map_err(|failure| {
                    error(
                        span,
                        format!("checking storage checkpoint failed: {failure:?}"),
                    )
                })?,
            )
        };
        Ok(Self {
            data: checker.generator_data.clone(),
            cells: checker.generator_cells.clone(),
            graph,
        })
    }

    fn restore(self, checker: &mut Checker<'_, '_>, span: Span) -> Result<(), Error> {
        let restored = self.graph.map_or(Ok(()), |graph| graph.restore());
        checker.generator_data = self.data;
        checker.generator_cells = self.cells;
        restored.map_err(|failure| {
            error(
                span,
                format!("checking storage restoration failed: {failure:?}"),
            )
        })
    }
}
impl std::fmt::Display for StorageCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(checking-storage (data")?;
        for (binding, value) in &self.data {
            write!(f, " (binding {binding}")?;
            if let Some(value) = value {
                write!(f, " {value}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (cells")?;
        for (binding, (snapshot, _)) in &self.cells {
            write!(f, " (binding {binding} {snapshot})")?;
        }
        f.write_str(")")?;
        if let Some(graph) = &self.graph {
            write!(f, " {graph}")?;
        }
        f.write_str(")")
    }
}

impl Checker<'_, '_> {
    /// Explicitly distinguish probes from retained quote construction. Both
    /// isolate lexical evidence, but only the latter keeps successful effects.
    pub(super) fn checking_attempt(
        &mut self,
        span: Span,
        allow_defaults: bool,
        disposition: Disposition,
        check: impl FnOnce(&mut Self) -> Result<n::Expr, Error>,
    ) -> Result<Option<n::Expr>, Error> {
        let storage = StorageCheckpoint::capture(self, span)?;
        let quote_holes = self.quote_holes.clone();
        let saved = (
            self.locals.clone(),
            self.code_values.clone(),
            self.pattern_codes.clone(),
            self.aliases.clone(),
            self.control.clone(),
            self.captures.clone(),
            self.immutable.clone(),
            self.context.clone(),
            self.field_defaults.clone(),
            self.active_defaults.clone(),
            self.input_facts.clone(),
        );
        let pending_len = self.pending_type_uses.as_ref().map(Vec::len);
        let old_defaults = std::mem::replace(&mut self.numeric_defaults, allow_defaults);
        let pattern_pending = self.pattern_quote_pending;
        let attempt = check(self);

        self.numeric_defaults = old_defaults;
        if disposition != Disposition::Commit || attempt.is_err() {
            if let (Some(pending), Some(len)) = (&mut self.pending_type_uses, pending_len) {
                pending.truncate(len);
            }
            (
                self.locals,
                self.code_values,
                self.pattern_codes,
                self.aliases,
                self.control,
                self.captures,
                self.immutable,
                self.context,
                self.field_defaults,
                self.active_defaults,
                self.input_facts,
            ) = saved;
        }
        if disposition == Disposition::Probe || attempt.is_err() {
            self.pattern_quote_pending = pattern_pending;
            self.quote_holes = quote_holes;
            storage.restore(self, span)?;
        }
        match attempt {
            Ok(value) => Ok(Some(value)),
            Err(failure) if failure.needs_context => Ok(None),
            Err(failure) => Err(failure),
        }
    }
}
