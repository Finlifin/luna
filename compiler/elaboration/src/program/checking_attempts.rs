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
    behavior_inputs: LocalBindings<Term>,
    behavior_values: LocalBindings<n::Expr>,
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
            behavior_inputs: checker.ordinary_behavior_inputs.clone(),
            behavior_values: checker.definition_refinements.behavior_values.clone(),
            data: checker.generator_data.clone(),
            cells: checker.generator_cells.clone(),
            graph,
        })
    }

    fn restore(self, checker: &mut Checker<'_, '_>, span: Span) -> Result<(), Error> {
        let restored = self.graph.map_or(Ok(()), |graph| graph.restore());
        checker.ordinary_behavior_inputs = self.behavior_inputs;
        checker.definition_refinements.behavior_values = self.behavior_values;
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
        f.write_str(") (behavior-inputs")?;
        for (binding, ty) in &self.behavior_inputs {
            write!(f, " (binding {binding} {ty})")?;
        }
        f.write_str(") (behavior-values")?;
        for (binding, value) in &self.behavior_values {
            write!(f, " (binding {binding} {value})")?;
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
    field_defaults: SourceBindings<n::Expr>,
    inputs: input_facts::State,
    open_staged_locals: BTreeSet<n::LocalId>,
    open_member_locals: BTreeSet<n::LocalId>,
    local_write_versions: BTreeMap<n::LocalId, u64>,
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
            inputs: checker.input_facts.clone(),
            open_staged_locals: checker.open_staged_locals.clone(),
            open_member_locals: checker.open_member_locals.clone(),
            local_write_versions: checker.local_write_versions.clone(),
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
        checker.input_facts = self.inputs;
        checker.open_staged_locals = self.open_staged_locals;
        checker.open_member_locals = self.open_member_locals;
        checker.local_write_versions = self.local_write_versions;
    }
}

/// Compiler state tied to retained elaborated NIR code (`Disposition::RetainCode`)
/// rather than lexical scope: rolled back only on `Probe` or error so quoted
/// blocks keep their synthetic dependency table, runtime origins/deliveries,
/// and type-use stability obligations.
struct RetainedCheckpoint {
    operand_scope: Option<scoped_operands::Scope>,
    abstract_code_dependencies: BTreeMap<n::LocalId, BTreeSet<n::LocalId>>,
    local_runtime_origins: runtime_origins::LocalOrigins,
    local_runtime_deliveries: BTreeMap<nir::RuntimeId, LocalRuntimeDelivery>,
    pending_refinements_len: usize,
    pending_contract_projections_len: usize,
    signature_type_uses_len: usize,
    pending_default_stability_len: usize,
    pending_predicate_stability_len: usize,
}

impl RetainedCheckpoint {
    fn capture(checker: &Checker<'_, '_>) -> Self {
        Self {
            operand_scope: checker.operand_scope.clone(),
            abstract_code_dependencies: checker.abstract_code_dependencies.clone(),
            local_runtime_origins: checker.local_runtime_origins.clone(),
            local_runtime_deliveries: checker.local_runtime_deliveries.clone(),
            pending_refinements_len: checker.pending_refinements.len(),
            pending_contract_projections_len: checker.pending_contract_projections.len(),
            signature_type_uses_len: checker.signature_type_uses.len(),
            pending_default_stability_len: checker.pending_default_stability.len(),
            pending_predicate_stability_len: checker.pending_predicate_stability.len(),
        }
    }

    fn restore(self, checker: &mut Checker<'_, '_>) {
        checker.operand_scope = self.operand_scope;
        checker
            .pending_refinements
            .truncate(self.pending_refinements_len);
        checker
            .pending_contract_projections
            .truncate(self.pending_contract_projections_len);
        checker
            .signature_type_uses
            .truncate(self.signature_type_uses_len);
        checker
            .pending_default_stability
            .truncate(self.pending_default_stability_len);
        checker
            .pending_predicate_stability
            .truncate(self.pending_predicate_stability_len);
        checker.abstract_code_dependencies = self.abstract_code_dependencies;
        checker.local_runtime_origins = self.local_runtime_origins;
        checker.local_runtime_deliveries = self.local_runtime_deliveries;
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
    retained: RetainedCheckpoint,
    storage: StorageCheckpoint,
    generated_functions: generated_functions::Checkpoint,
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
        let retained = RetainedCheckpoint::capture(checker);
        let generated_functions = checker.generated_functions.checkpoint();
        Ok(Self {
            span,
            disposition,
            lexical,
            retained,
            storage,
            generated_functions,
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
        // Missing context is the only recoverable absence. Domain, budget and
        // restoration failures must still escape to the owning checking rule.
        match self.finish_checked(checker, result) {
            Ok(value) => Ok(Some(value)),
            Err(failure) if failure.needs_context => Ok(None),
            Err(failure) => Err(failure),
        }
    }

    /// Finish a mandatory source check without discarding its contextual
    /// diagnostic. The enclosing callable owns any retry with stronger inputs.
    pub(super) fn finish_checked<T>(
        self,
        checker: &mut Checker<'_, '_>,
        result: Result<T, Error>,
    ) -> Result<T, Error> {
        checker.numeric_defaults = self.defaults;
        if self.disposition != Disposition::Commit || result.is_err() {
            self.lexical.restore(checker);
        }
        if self.disposition == Disposition::Probe || result.is_err() {
            self.retained.restore(checker);
            checker.pattern_quote_pending = self.pattern_pending;
            checker.quote_holes = self.quote_holes;
            checker
                .generated_functions
                .rollback(self.generated_functions);
            self.storage.restore(checker, self.span)?;
        }
        result
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
        for (field, value) in &self.field_defaults {
            write!(f, " (field {field} {value})")?;
        }
        f.write_str(") (open-staged-locals")?;
        for binding in &self.open_staged_locals {
            write!(f, " {binding}")?;
        }
        f.write_str(") (open-member-locals")?;
        for binding in &self.open_member_locals {
            write!(f, " {binding}")?;
        }
        f.write_str(") (local-write-versions")?;
        for (binding, version) in &self.local_write_versions {
            write!(f, " (binding {binding} {version})")?;
        }
        f.write_str(") (pending-type-uses")?;
        if let Some(len) = self.pending_len {
            write!(f, " {len}")?;
        }
        write!(f, ") {} {} {})", self.control, self.context, self.inputs)
    }
}

impl std::fmt::Display for RetainedCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(checking-retained (abstract-code-dependencies")?;
        for (binding, deps) in &self.abstract_code_dependencies {
            write!(f, " (binding {binding}")?;
            for dep in deps {
                write!(f, " {dep}")?;
            }
            f.write_str(")")?;
        }
        write!(
            f,
            ") (local-runtime-origins {}) (local-runtime-deliveries {}) (pending-refinements {}) (pending-contract-projections {}) (signature-type-uses {}) (pending-default-stability {}) (pending-predicate-stability {}))",
            self.local_runtime_origins.ids.len(),
            self.local_runtime_deliveries.len(),
            self.pending_refinements_len,
            self.pending_contract_projections_len,
            self.signature_type_uses_len,
            self.pending_default_stability_len,
            self.pending_predicate_stability_len,
        )
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
        write!(
            f,
            ") {} {} {} {})",
            self.generated_functions, self.lexical, self.retained, self.storage
        )
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
            field_defaults: SourceBindings::default(),
            inputs: input_facts::State::default(),
            open_staged_locals: BTreeSet::new(),
            open_member_locals: BTreeSet::new(),
            local_write_versions: BTreeMap::new(),
            pending_len: Some(7),
        };
        let retained = RetainedCheckpoint {
            operand_scope: None,
            abstract_code_dependencies: BTreeMap::new(),
            local_runtime_origins: Default::default(),
            local_runtime_deliveries: BTreeMap::new(),
            pending_refinements_len: 0,
            pending_contract_projections_len: 0,
            signature_type_uses_len: 0,
            pending_default_stability_len: 0,
            pending_predicate_stability_len: 0,
        };
        let attempt = Attempt {
            span: Span::default(),
            disposition: Disposition::RetainCode,
            lexical,
            retained,
            storage: StorageCheckpoint {
                behavior_inputs: LocalBindings::default(),
                behavior_values: LocalBindings::default(),
                data: BTreeMap::new(),
                cells: BTreeMap::new(),
                graph: None,
            },
            generated_functions: generated_functions::GeneratedFunctions::default().checkpoint(),
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
            "(checking-retained ",
            "(checking-storage ",
            "parent\\\"\\n",
        ] {
            assert!(dump.contains(expected), "missing {expected}: {dump}");
        }
        assert!(!dump.contains('\n'), "{dump}");
    }
}
