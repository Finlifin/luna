//! Shared lexical scope, statement evidence and binding completion for blocks.
//! The tail consumer chooses ordinary checking or retained contextual inputs;
//! neither path executes a producer while preparing this region.
use super::*;
use constants::StagedPatternSnapshot;

impl Checker<'_, '_> {
    pub(super) fn block_binding_liveness_needed(
        &self,
        statements: &[n::Expr],
        initialized: &BTreeMap<usize, StagedPatternSnapshot>,
    ) -> bool {
        !initialized.is_empty()
            || (!self.input_facts.specialized.is_empty()
                && statements.iter().any(|statement| matches!(
                    &statement.kind,
                    K::Let { value, .. } | K::LetPattern { value, guard: None, otherwise: None, .. }
                        if matches!(value.kind, K::Local(_) | K::Field { .. } | K::AnonymousField { .. })
                )))
    }

    pub(super) fn in_block_scope<R>(
        &mut self,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let saved = self.locals.clone();
        let saved_code = self.code_values.clone();
        let saved_patterns = self.pattern_codes.clone();
        let saved_context = self.context.clone();
        let boundary = self.input_facts.lexical_boundary();
        let result = check(self);
        self.input_facts.leave_lexical(&boundary);
        self.locals = saved;
        self.code_values = saved_code;
        self.pattern_codes = saved_patterns;
        self.context = saved_context;
        result
    }

    pub(super) fn check_block_region<R>(
        &mut self,
        block: &hir::Block<'_>,
        finish: impl FnOnce(
            &mut Self,
            Vec<n::Expr>,
            bool,
            BTreeMap<usize, StagedPatternSnapshot>,
        ) -> Result<R, Error>,
    ) -> Result<R, Error> {
        self.in_block_scope(|this| {
            let mut statements = Vec::new();
            let mut staged_patterns = BTreeMap::new();
            let mut diverges = false;
            for statement in block.stmts {
                let value = this.expr(statement, None)?;
                if let K::LetPattern {
                    pattern,
                    value: input,
                    ..
                } = &value.kind
                {
                    if let Some(inputs) =
                        this.completed_pattern_inputs(pattern, input, statement.span)?
                    {
                        staged_patterns.insert(statements.len(), inputs);
                    }
                }
                diverges |= is_never(&value.ty);
                if !diverges {
                    this.assume_statement_type_facts(&value, statement.span)?;
                    if this
                        .signature_owner
                        .is_some_and(|owner| this.open_functions.contains_key(&owner))
                    {
                        this.assume_open_continuation(statement)?;
                    }
                }
                statements.push(value);
            }
            finish(this, statements, diverges, staged_patterns)
        })
    }

    pub(super) fn block_tail_references(
        &mut self,
        tail: Option<&n::Expr>,
        span: Span,
    ) -> Result<BTreeSet<n::LocalId>, Error> {
        let Some(tail) = tail else {
            return Ok(BTreeSet::new());
        };
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let references = nir::preparation::references(tail, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        references.map_err(|failure| error(span, failure.to_string()))
    }

    pub(super) fn finish_block_bindings(
        &mut self,
        statements: &mut Vec<n::Expr>,
        references: &BTreeSet<n::LocalId>,
        block: &hir::Block<'_>,
        staged_patterns: &BTreeMap<usize, StagedPatternSnapshot>,
    ) -> Result<(), Error> {
        self.materialize_mixed_staged_patterns(statements, references, block, staged_patterns)?;
        self.erase_consumed_staged_aliases(statements, references, block.span)?;
        self.stabilize_error_let_bindings(statements, block.span)
    }
}
