//! Function-value NF inputs use the same slot, packet and collection preparation
//! as declarations. The receiving source interface controls named supplies.
use super::*;

pub(in crate::program) struct OpenedCallable {
    pub(in crate::program) input_ids: Vec<DeclarationId>,
    pub(in crate::program) parameters: declaration_inputs::Parameters,
    pub(in crate::program) result: Term,
    pub(in crate::program) collector: Option<nir::normal_forms::Collector>,
}

impl Checker<'_, '_> {
    pub(in crate::program) fn dynamic_normal_form_application(
        &mut self,
        expr: &Expr<'_>,
        head: &Expr<'_>,
        callee: n::Expr,
        inputs: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if self.protocols.splice.is_some() && self.runtime_value_context() {
            let value = self.with_staged_input(true, |this| {
                this.prepare_dynamic_normal_inputs(expr, head, callee, inputs, None)
            })?;
            return self.deliver_comptime_result(expr, value, expected);
        }
        self.prepare_dynamic_normal_inputs(expr, head, callee, inputs, expected)
    }

    fn prepare_dynamic_normal_inputs(
        &mut self,
        expr: &Expr<'_>,
        head: &Expr<'_>,
        callee: n::Expr,
        inputs: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let callable = self.open_normal_callable(head, &callee.ty, expr.span)?;
        let parameters = callable.parameters;
        let result_template = callable.result;
        let collector = callable.collector;
        let input_ids = callable.input_ids;
        let inputs = self.planned_callable_inputs(
            inputs,
            expr.span,
            parameters.clone(),
            collector.as_ref(),
        )?;
        let diverges = inputs.diverges();
        let (ty, _) = self.instantiate_normal_result(
            &parameters,
            &result_template,
            inputs.observations,
            diverges,
            expr.span,
        )?;
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::ApplyNormalForm {
                callee: std::sync::Arc::new(callee),
                input_ids,
                arguments: inputs.arguments,
                plan: Some(inputs.plan),
            },
        })
    }

    /// Open the public telescope with fresh declaration identities. Both
    /// concrete input plans and definition-side descriptions use these slots.
    pub(in crate::program) fn open_normal_callable(
        &mut self,
        head: &Expr<'_>,
        ty: &Term,
        span: Span,
    ) -> Result<OpenedCallable, Error> {
        let Term::NormalFormFunction(signature, collector) = ty else {
            return Err(error(
                span,
                "normal-form application requires a normal-form callable",
            ));
        };
        let Term::Function { parameters, .. } = signature.callable_signature() else {
            return Err(error(span, "normal-form callable has no input telescope"));
        };
        let ids: Vec<_> = self
            .fresh_callable_inputs(parameters.len(), span)?
            .collect();
        let (parameters, result) = nominal_fields::metered(&mut self.required_steps, |fuel| {
            nir::normal_forms::open_signature(ty, &ids, fuel)
        })
        .map_err(|failure| error(span, failure.to_string()))?;
        let collector = self.value_collector(head, *collector, span)?;
        Ok(OpenedCallable {
            input_ids: ids,
            parameters,
            result,
            collector,
        })
    }
}
