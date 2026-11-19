//! Function-value NF inputs use the same slot, packet and collection preparation
//! as declarations. The current callable type exposes only positional entries.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn dynamic_normal_form_application(
        &mut self,
        expr: &Expr<'_>,
        callee: n::Expr,
        inputs: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if self.protocols.splice.is_some() && self.runtime_value_context() {
            let value = self.with_staged_input(true, |this| {
                this.prepare_dynamic_normal_inputs(expr, callee, inputs, None)
            })?;
            return self.deliver_comptime_result(expr, value, expected);
        }
        self.prepare_dynamic_normal_inputs(expr, callee, inputs, expected)
    }

    fn prepare_dynamic_normal_inputs(
        &mut self,
        expr: &Expr<'_>,
        callee: n::Expr,
        inputs: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let Term::NormalFormFunction(signature, collector) = &callee.ty else {
            return Err(error(
                expr.span,
                "normal-form application requires a normal-form callable",
            ));
        };
        let Term::Function { parameters, result } = signature.callable_signature() else {
            return Err(error(
                expr.span,
                "normal-form callable has no input telescope",
            ));
        };
        if **result != p(P::Type) {
            return Err(error(
                expr.span,
                "non-Type normal-form results are not implemented",
            ));
        }
        let ids: Vec<_> = self
            .fresh_callable_inputs(parameters.len(), expr.span)?
            .collect();
        let parameters = nominal_fields::metered(&mut self.required_steps, |fuel| {
            nir::normal_forms::open_inputs(&callee.ty, &ids, fuel)
        })
        .map_err(|failure| error(expr.span, failure.to_string()))?;
        let inputs = self.planned_positional_inputs(inputs, expr.span, parameters, *collector)?;
        let ty = if inputs.diverges() {
            p(P::Never)
        } else {
            p(P::Type)
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::ApplyNormalForm {
                callee: std::sync::Arc::new(callee),
                input_ids: ids,
                arguments: inputs.arguments,
                plan: Some(inputs.plan),
            },
        })
    }
}
