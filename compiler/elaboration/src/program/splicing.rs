//! Ordinary cross-stage conversion produces a runtime trait call, with the
//! source value routed through the existing staged-argument machinery.
use super::*;

/// A checked operand keeps a source program distinct from a value already
/// checked in the consumer's phase. Source does not mean already evaluated.
pub(super) enum Operand {
    Source(n::Expr),
    Value(n::Expr),
}

impl Operand {
    pub(super) fn into_parts(self) -> (n::Expr, bool) {
        match self {
            Self::Source(value) => (value, true),
            Self::Value(value) => (value, false),
        }
    }

    pub(super) fn projected(value: n::Expr, source: bool) -> Self {
        if source {
            Self::Source(value)
        } else {
            Self::Value(value)
        }
    }
}

impl std::fmt::Display for Operand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(value) => write!(f, "(source-operand {value})"),
            Self::Value(value) => write!(f, "(value-operand {value})"),
        }
    }
}

impl Checker<'_, '_> {
    /// Inspect a receiver before deciding whether its result crosses stages.
    /// This boundary owns the checked program; no ambient flag or source-range
    /// cache can transfer an observation to another checking attempt.
    pub(super) fn field_operand(&mut self, expr: &Expr<'_>) -> Result<Operand, Error> {
        if self.runtime_value_context() {
            if let H::ResolvedLocal(_, binding) | H::ResolvedFnParam(_, binding) = expr.kind
                && self
                    .generator_data
                    .get(&local(binding))
                    .is_some_and(Option::is_some)
            {
                // The binding reader retains current snapshot/value-reference
                // checks. Abstract generator inputs are not available values.
                return self.expr(expr, None).map(Operand::Source);
            }
            if let H::ResolvedDef { def_id, .. } = expr.kind
                && let Some(owner) = self.package.definition_owner(def_id)
                && matches!(
                    self.package.item(owner).map(|item| &item.kind),
                    Some(ItemKind::Const(..))
                )
            {
                let mut value = if self.is_build_context(owner) {
                    self.build_context_value(expr, owner)?
                } else {
                    self.constant_value(owner, expr.span)?
                };
                value.source = source(expr.span);
                return Ok(Operand::Source(value));
            }
            if matches!(expr.kind, H::Application(..) | H::ExtendedApplication(..))
                && self.declared_comptime_call_result(expr)?.is_some()
            {
                return self
                    .with_staged_input(true, |checker| checker.expr(expr, None))
                    .map(Operand::Source);
            }
            if let H::Projection(base, field) = expr.kind {
                return self.projection_operand(expr, base, &field, None, None);
            }
        }
        self.expr(expr, None).map(Operand::Value)
    }

    pub(super) fn deliver_operand(
        &mut self,
        expr: &Expr<'_>,
        operand: Operand,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let (value, source) = operand.into_parts();
        if source && self.runtime_value_context() {
            return self.deliver_comptime_result(expr, value, expected);
        }
        if let Some(expected) = expected {
            self.require_expression(&value.ty, expected, expr.span)?;
        }
        Ok(value)
    }

    pub(super) fn with_callee_delivery<R>(
        &mut self,
        parent: hir::HirId,
        delivery: Option<hir::HirId>,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let saved = self.comptime_callee_result;
        if saved == Some(parent) {
            self.comptime_callee_result = delivery;
        }
        let result = check(self);
        self.comptime_callee_result = saved;
        result
    }

    pub(super) fn deliver_comptime_result(
        &mut self,
        expr: &Expr<'_>,
        value: n::Expr,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        // A comptime callable or constructor Type used as the next callee
        // never enters runtime data flow. Keep the checked producer for that
        // invocation or construction instead of requesting a Splice conversion.
        if self.comptime_callee_result == Some(expr.hir_id)
            && (value.ty.is_comptime_callable() || value.ty == p(P::Type))
        {
            return Ok(value);
        }
        self.splice_value(expr, value, expected)
    }

    pub(super) fn runtime_value_context(&self) -> bool {
        if self.observing_type || self.in_required_comptime || self.in_staged_input {
            return false;
        }
        // A quote checks runtime code inside a comptime producer. Nested
        // closures own their execution phase; the enclosing declaration only
        // supplies the phase when neither boundary is active.
        if let Some(frame) = self.captures.last() {
            return !frame.comptime;
        }
        self.in_code_body
            || !self
                .signature_owner
                .and_then(|owner| self.package.item(owner))
                .is_some_and(|item| {
                    matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                        if sig.modifiers.is_comptime)
                })
    }

    pub(super) fn splice_constant(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let mut value = self.constant_value(owner, expr.span)?;
        value.source = source(expr.span);
        self.splice_value(expr, value, expected)
    }

    pub(super) fn splice_value(
        &mut self,
        expr: &Expr<'_>,
        value: n::Expr,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let id = self.protocols.splice.ok_or_else(|| error(expr.span,
            "constant runtime use requires cross-stage Splice conversion with a bound Splice protocol"))?;
        if !self.interfaces_ready {
            return Err(error(
                expr.span,
                "Splice conversion is waiting for the trait implementation world",
            ));
        }
        crate::splice::protocol(&self.checked_traits, id).map_err(|e| error(expr.span, e))?;
        let callee = self
            .selected_method(
                value.ty.clone(),
                Some(n::InterfaceInstance {
                    declaration: id,
                    arguments: vec![],
                }),
                "splice",
                expr.span,
            )?
            .ok_or_else(|| error(expr.span, "Splice conversion has no selected method"))?;
        // A prepared prefix is already an elaborated input; finish_call still
        // checks ordinary contracts, specializes staged inputs and preserves
        // the trait origin. It does not invoke this runtime method at comptime.
        self.finish_call(expr, callee, &[], vec![value], expected, None)
    }
}
