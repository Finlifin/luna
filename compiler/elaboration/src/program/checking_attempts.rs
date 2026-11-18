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

/// Candidate-private lexical observations. This is not a persistent world or
/// a VM checkpoint: shared declaration tables retain their existing owners.
struct LexicalCheckpoint {
    locals: LocalBindings<Term>,
    code: LocalBindings<cmtt::ExpressionCode>,
    patterns: LocalBindings<cmtt::patterns::PatternCode>,
    aliases: BTreeSet<OwnerId>,
    control: control::Context,
    captures: Vec<CaptureFrame>,
    immutable: BTreeSet<n::LocalId>,
    context: crate::ptc::Context,
    field_defaults: BTreeMap<(OwnerId, Option<usize>, usize), n::Expr>,
    active_defaults: BTreeSet<(OwnerId, Option<usize>, usize)>,
    inputs: input_facts::State,
    pending_len: Option<usize>,
}

impl LexicalCheckpoint {
    fn capture(checker: &Checker<'_, '_>) -> Self {
        Self {
            locals: checker.locals.clone(),
            code: checker.code_values.clone(),
            patterns: checker.pattern_codes.clone(),
            aliases: checker.aliases.clone(),
            control: checker.control.clone(),
            captures: checker.captures.clone(),
            immutable: checker.immutable.clone(),
            context: checker.context.clone(),
            field_defaults: checker.field_defaults.clone(),
            active_defaults: checker.active_defaults.clone(),
            inputs: checker.input_facts.clone(),
            pending_len: checker.pending_type_uses.as_ref().map(Vec::len),
        }
    }

    fn restore(self, checker: &mut Checker<'_, '_>) {
        if let (Some(pending), Some(len)) = (&mut checker.pending_type_uses, self.pending_len) {
            pending.truncate(len);
        }
        checker.locals = self.locals;
        checker.code_values = self.code;
        checker.pattern_codes = self.patterns;
        checker.aliases = self.aliases;
        checker.control = self.control;
        checker.captures = self.captures;
        checker.immutable = self.immutable;
        checker.context = self.context;
        checker.field_defaults = self.field_defaults;
        checker.active_defaults = self.active_defaults;
        checker.input_facts = self.inputs;
    }
}

/// Owns the pre-attempt state across child checks. It deliberately cannot be
/// cloned: completion consumes rollback authority exactly once. There is no
/// Drop-based restoration because the active checker is not borrowed here.
#[must_use = "finish this checking attempt before leaving its active checker"]
pub(super) struct Attempt {
    span: Span,
    disposition: Disposition,
    lexical: LexicalCheckpoint,
    storage: StorageCheckpoint,
    quote_holes: BTreeSet<(u32, u32)>,
    defaults: bool,
    pattern_pending: Option<bool>,
}

impl Attempt {
    pub(super) fn begin(
        checker: &mut Checker<'_, '_>,
        span: Span,
        allow_defaults: bool,
        disposition: Disposition,
    ) -> Result<Self, Error> {
        // Failures while acquiring storage must not change the active numeric
        // mode. Fuel already spent on a failed acquisition is not restored.
        let storage = StorageCheckpoint::capture(checker, span)?;
        let lexical = LexicalCheckpoint::capture(checker);
        Ok(Self {
            span,
            disposition,
            lexical,
            storage,
            quote_holes: checker.quote_holes.clone(),
            defaults: std::mem::replace(&mut checker.numeric_defaults, allow_defaults),
            pattern_pending: checker.pattern_quote_pending,
        })
    }

    pub(super) fn finish<T>(
        self,
        checker: &mut Checker<'_, '_>,
        result: Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        checker.numeric_defaults = self.defaults;
        if self.disposition != Disposition::Commit || result.is_err() {
            self.lexical.restore(checker);
        }
        if self.disposition == Disposition::Probe || result.is_err() {
            checker.pattern_quote_pending = self.pattern_pending;
            checker.quote_holes = self.quote_holes;
            self.storage.restore(checker, self.span)?;
        }
        // Missing context is the only recoverable absence. Domain, budget and
        // restoration failures must still escape to the owning checking rule.
        match result {
            Ok(value) => Ok(Some(value)),
            Err(failure) if failure.needs_context => Ok(None),
            Err(failure) => Err(failure),
        }
    }
}

impl std::fmt::Display for LexicalCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(checking-lexical (locals")?;
        for (binding, ty) in &self.locals {
            write!(f, " (binding {binding} {ty})")?;
        }
        f.write_str(") (code")?;
        for (binding, code) in &self.code {
            write!(f, " (binding {binding} {code})")?;
        }
        f.write_str(") (patterns")?;
        for (binding, code) in &self.patterns {
            write!(f, " (binding {binding} {code})")?;
        }
        f.write_str(") (aliases")?;
        for alias in &self.aliases {
            write!(f, " {alias}")?;
        }
        f.write_str(") (captures")?;
        for capture in &self.captures {
            write!(f, " (capture-frame (comptime {}) (outer", capture.comptime)?;
            for binding in &capture.outer {
                write!(f, " {binding}")?;
            }
            f.write_str(") (values")?;
            for value in &capture.values {
                write!(f, " {value}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (immutable")?;
        for binding in &self.immutable {
            write!(f, " {binding}")?;
        }
        f.write_str(") (field-defaults")?;
        for ((owner, variant, index), value) in &self.field_defaults {
            write!(f, " (field {owner} (variant")?;
            if let Some(variant) = variant {
                write!(f, " {variant}")?;
            }
            write!(f, ") (index {index}) {value})")?;
        }
        f.write_str(") (active-defaults")?;
        for (owner, variant, index) in &self.active_defaults {
            write!(f, " (field {owner} (variant")?;
            if let Some(variant) = variant {
                write!(f, " {variant}")?;
            }
            write!(f, ") (index {index}))")?;
        }
        f.write_str(") (pending-type-uses")?;
        if let Some(len) = self.pending_len {
            write!(f, " {len}")?;
        }
        write!(f, ") {} {} {})", self.control, self.context, self.inputs)
    }
}

impl std::fmt::Display for Attempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(checking-attempt (source {}) {} (parent-defaults {}) (pattern-pending",
            source(self.span),
            self.disposition,
            self.defaults
        )?;
        if let Some(pending) = self.pattern_pending {
            write!(f, " {pending}")?;
        }
        f.write_str(") (quote-holes")?;
        for (start, end) in &self.quote_holes {
            write!(f, " (source {start} {end})")?;
        }
        write!(f, ") {} {})", self.lexical, self.storage)
    }
}

impl Checker<'_, '_> {
    /// Synchronous adapter over the owned attempt boundary. Child drivers may
    /// retain the same boundary without retaining this Rust closure frame.
    pub(super) fn checking_attempt(
        &mut self,
        span: Span,
        allow_defaults: bool,
        disposition: Disposition,
        check: impl FnOnce(&mut Self) -> Result<n::Expr, Error>,
    ) -> Result<Option<n::Expr>, Error> {
        let attempt = Attempt::begin(self, span, allow_defaults, disposition)?;
        let result = check(self);
        attempt.finish(self, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_attempt_dump_separates_lexical_observations_and_storage() {
        let id = n::LocalId { owner: 2, index: 3 };
        let lexical = LexicalCheckpoint {
            locals: LocalBindings::from([(
                id,
                Term::NormalForm {
                    head: DeclarationId {
                        package: 1,
                        local: 4,
                    },
                    sort: Sort::Type,
                    arguments: vec![Term::String("parent\"\n".into())],
                },
            )]),
            code: LocalBindings::default(),
            patterns: LocalBindings::default(),
            aliases: BTreeSet::new(),
            control: control::Context {
                result: Some(p(P::Bool)),
                ..Default::default()
            },
            captures: vec![],
            immutable: BTreeSet::from([id]),
            context: crate::ptc::Context::default(),
            field_defaults: BTreeMap::new(),
            active_defaults: BTreeSet::new(),
            inputs: input_facts::State::default(),
            pending_len: Some(7),
        };
        let attempt = Attempt {
            span: Span::default(),
            disposition: Disposition::RetainCode,
            lexical,
            storage: StorageCheckpoint {
                data: BTreeMap::new(),
                cells: BTreeMap::new(),
                graph: None,
            },
            quote_holes: BTreeSet::from([(11, 13)]),
            defaults: false,
            pattern_pending: Some(true),
        };
        let dump = attempt.to_string();
        assert!(dump.starts_with("(checking-attempt "), "{dump}");
        for expected in [
            "(checking-disposition retain-code)",
            "(parent-defaults false)",
            "(pattern-pending true)",
            "(quote-holes (source 11 13))",
            "(pending-type-uses 7)",
            "(checking-lexical ",
            "(checking-storage ",
            "parent\\\"\\n",
        ] {
            assert!(dump.contains(expected), "missing {expected}: {dump}");
        }
        assert!(!dump.contains('\n'), "{dump}");
    }
}
