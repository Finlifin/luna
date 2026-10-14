//! Ordinary cross-stage conversion produces a runtime trait call, with the
//! source value routed through the existing staged-argument machinery.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn deliver_comptime_result(
        &mut self,
        expr: &Expr<'_>,
        value: n::Expr,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        // An immediately invoked comptime callable never enters runtime data
        // flow. Keep its producer expression inside the next staged invocation.
        if self.comptime_callee_result == Some(expr.hir_id) && value.ty.is_comptime_callable() {
            return Ok(value);
        }
        self.splice_value(expr, value, expected)
    }

    pub(super) fn runtime_value_context(&self) -> bool {
        !(self.observing_type
            || self.in_required_comptime
            || self.in_staged_input
            || self
                .signature_owner
                .and_then(|owner| self.package.item(owner))
                .is_some_and(
                    |item| matches!(&item.kind, ItemKind::Fn(sig, _) if sig.modifiers.is_comptime),
                ))
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
