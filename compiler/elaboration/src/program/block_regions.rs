//! Shared lexical scope, statement evidence and binding completion for blocks.
//! The tail consumer chooses ordinary checking or retained contextual inputs;
//! neither path executes a producer while preparing this region.
use super::*;
use constants::StagedPatternSnapshot;
mod aggregates;
pub(super) mod arithmetic;
mod comparisons;
mod conditionals;
mod destinations;
mod driver;
mod input_roots;
mod logical;
mod matches;
mod normal_applications;
mod tails;
mod unary;
mod when;
use destinations::{CheckedBlock, Destination};
pub(super) use tails::{ContextualTail, OrdinaryTail};

/// The destination rule survives statement checking as data. Its checked child
/// and final output stay distinct; finishing never checks the source a second
/// time. The driver owns destinations; source references still borrow this region.
pub(super) trait Tail: std::fmt::Display {
    type Checked;
    type Output;

    fn references(
        checked: &Self::Checked,
        fuel: &mut usize,
    ) -> Result<BTreeSet<n::LocalId>, nir::preparation::Error>;
    fn finish(checked: Self::Checked, statements: Vec<n::Expr>, span: Span) -> Self::Output;
}

/// The parent lexical environment, not a checking transaction. Leaving a
/// block restores bindings and premises but keeps invalidated outer inputs.
struct LexicalScope {
    locals: BTreeMap<n::LocalId, Term>,
    code: BTreeMap<n::LocalId, std::sync::Arc<cmtt::ExpressionCode>>,
    patterns: BTreeMap<n::LocalId, std::sync::Arc<cmtt::patterns::PatternCode>>,
    context: crate::ptc::Context,
    inputs: input_facts::LexicalBoundary,
}

impl LexicalScope {
    fn enter(checker: &mut Checker<'_, '_>, span: Span) -> Result<Self, Error> {
        // Shared code/context payloads are retained, not recursively copied.
        // Account for map entries and boundary sets before allocating them.
        let facts = &checker.input_facts;
        let entries = checker
            .locals
            .len()
            .saturating_add(checker.code_values.len())
            .saturating_add(checker.pattern_codes.len())
            .saturating_add(facts.specialized.len())
            .saturating_add(facts.types.len())
            .saturating_add(facts.partial.len())
            .saturating_add(facts.values.len())
            .saturating_add(facts.derived.len());
        checker.required_steps = checker
            .required_steps
            .checked_sub(entries as u64)
            .ok_or_else(|| error(span, "lexical scope snapshot budget exhausted"))?;
        let mut locals = BTreeMap::new();
        for (binding, ty) in &checker.locals {
            let before = usize::try_from(checker.required_steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let copied = nir::copy_budget::clone_term(ty, &mut fuel);
            checker.required_steps -= (before - fuel) as u64;
            let ty = copied.map_err(|failure| {
                error(
                    span,
                    format!("lexical scope type snapshot budget exhausted: {failure:?}"),
                )
            })?;
            locals.insert(*binding, ty);
        }
        Ok(Self {
            locals,
            code: checker.code_values.clone(),
            patterns: checker.pattern_codes.clone(),
            context: checker.context.clone(),
            inputs: checker.input_facts.lexical_boundary(),
        })
    }

    fn leave(self, checker: &mut Checker<'_, '_>) {
        checker.input_facts.leave_lexical(&self.inputs);
        checker.locals = self.locals;
        checker.code_values = self.code;
        checker.pattern_codes = self.patterns;
        checker.context = self.context;
    }
}

impl std::fmt::Display for LexicalScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(lexical-parent (locals")?;
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
        write!(f, ") {} {})", self.context, self.inputs)
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Statements,
    Child,
    Tail,
    TailChild,
}

/// Owned statement results survive between child checks. Source references
/// borrow this HIR snapshot; they are never published in stable code or worlds.
struct Progress<'b, 'h, T> {
    block: &'b hir::Block<'h>,
    phase: Phase,
    statements: Vec<n::Expr>,
    staged_patterns: BTreeMap<usize, StagedPatternSnapshot>,
    diverges: bool,
    tail: T,
}

impl<'b, 'h, T: Tail> Progress<'b, 'h, T> {
    fn new(block: &'b hir::Block<'h>, tail: T) -> Self {
        Self {
            block,
            phase: Phase::Statements,
            statements: Vec::new(),
            staged_patterns: BTreeMap::new(),
            diverges: false,
            tail,
        }
    }

    fn next(&mut self) -> Result<Option<&'h Expr<'h>>, Error> {
        if !matches!(self.phase, Phase::Statements) {
            return Err(error(
                self.block.span,
                "block child check is already pending or complete",
            ));
        }
        let next = self.block.stmts.get(self.statements.len());
        self.phase = if next.is_some() {
            Phase::Child
        } else {
            Phase::Tail
        };
        Ok(next)
    }

    fn accept(&mut self, checker: &mut Checker<'_, '_>, value: n::Expr) -> Result<(), Error> {
        if !matches!(self.phase, Phase::Child) {
            return Err(error(self.block.span, "block has no pending child result"));
        }
        let statement = &self.block.stmts[self.statements.len()];
        let initialized = if let K::LetPattern {
            pattern,
            value: input,
            ..
        } = &value.kind
        {
            checker.completed_pattern_inputs(pattern, input, statement.span)?
        } else {
            None
        };
        let diverges = self.diverges || is_never(&value.ty);
        if !diverges {
            checker.assume_statement_type_facts(&value, statement.span)?;
            if checker
                .signature_owner
                .is_some_and(|owner| checker.open_functions.contains_key(&owner))
            {
                checker.assume_open_continuation(statement)?;
            }
        }
        if let Some(inputs) = initialized {
            self.staged_patterns.insert(self.statements.len(), inputs);
        }
        self.statements.push(value);
        self.diverges = diverges;
        self.phase = Phase::Statements;
        Ok(())
    }

    fn request_tail(&mut self) -> Result<&T, Error> {
        if !matches!(self.phase, Phase::Tail) {
            return Err(error(self.block.span, "block tail check is not ready"));
        }
        self.phase = Phase::TailChild;
        Ok(&self.tail)
    }

    fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        checked: T::Checked,
    ) -> Result<T::Output, Error> {
        if !matches!(self.phase, Phase::TailChild) {
            return Err(error(self.block.span, "block has no pending tail result"));
        }
        let mut fuel = usize::try_from(checker.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let references =
            if checker.block_binding_liveness_needed(&self.statements, &self.staged_patterns) {
                T::references(&checked, &mut fuel)
            } else {
                Ok(BTreeSet::new())
            };
        checker.required_steps -= (before - fuel) as u64;
        let references =
            references.map_err(|failure| error(self.block.span, failure.to_string()))?;
        let mut statements = self.statements;
        checker.finish_block_bindings(
            &mut statements,
            &references,
            self.block,
            &self.staged_patterns,
        )?;
        Ok(T::finish(checked, statements, self.block.span))
    }
}

impl<T: Tail> std::fmt::Display for Progress<'_, '_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let phase = match self.phase {
            Phase::Statements => "statements",
            Phase::Child => "child",
            Phase::Tail => "tail",
            Phase::TailChild => "tail-child",
        };
        write!(
            f,
            "(block-check (source {}) (phase {phase}) (next {}) (diverges {}) (statements",
            source(self.block.span),
            self.statements.len(),
            self.diverges
        )?;
        for statement in &self.statements {
            write!(f, " {statement}")?;
        }
        f.write_str(") (staged-patterns")?;
        for (index, inputs) in &self.staged_patterns {
            write!(f, " (statement {index} {inputs})")?;
        }
        write!(f, ") (tail {}))", self.tail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_dump_keeps_source_progress_separate_from_checked_results() {
        let id = hir::HirId::new(
            hir::OwnerId::new(hir::LocalDefId::new(1)),
            hir::ItemLocalId::new(0),
        );
        let statements = [Expr {
            hir_id: id,
            kind: H::Unit,
            span: Span::default(),
        }];
        let block = hir::Block {
            hir_id: id,
            stmts: &statements,
            expr: None,
            span: Span::default(),
        };
        let mut progress = Progress::new(&block, ContextualTail { depth: 4 });
        assert!(
            progress
                .to_string()
                .contains("(tail (contextual-tail (depth 4)))")
        );
        assert!(progress.request_tail().is_err());
        assert!(progress.to_string().contains("(phase statements) (next 0)"));
        assert_eq!(progress.next().unwrap().unwrap().hir_id, id);
        assert!(progress.to_string().contains("(phase child) (next 0)"));
        assert!(
            progress.next().is_err(),
            "pending source must not be dispatched twice"
        );
        assert!(progress.statements.is_empty());
        let empty = hir::Block {
            stmts: &[],
            ..block
        };
        let mut finished = Progress::new(&empty, ContextualTail { depth: 4 });
        assert!(finished.next().unwrap().is_none());
        assert!(finished.to_string().contains("(phase tail) (next 0)"));
        assert!(
            finished.next().is_err(),
            "tail-ready state cannot restart statements"
        );
        finished.request_tail().unwrap();
        assert!(finished.to_string().contains("(phase tail-child)"));
        assert!(
            finished.request_tail().is_err(),
            "tail must not be dispatched twice"
        );
        assert!(finished.next().is_err());
    }
}

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
        span: Span,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let scope = LexicalScope::enter(self, span)?;
        let result = check(self);
        scope.leave(self);
        result
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
