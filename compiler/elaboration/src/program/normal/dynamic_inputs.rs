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
        let parameters = self.open_normal_input_telescope(parameters, expr.span)?;
        let operands = self
            .planned_positional_inputs(inputs, expr.span, parameters, *collector)?
            .operands;
        let diverges = operands.expressions().any(|value| is_never(&value.ty));
        let ty = if diverges { p(P::Never) } else { p(P::Type) };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        // Retain the callee before any input producer. Even a selected function
        // value may come from an effectful invocation or a mutable storage read.
        let binding = self.spread_binding(expr.span)?;
        let callee_type = self.copy_alias_type(&callee.ty, expr.span)?;
        let anchor = n::SourceRange {
            start: expr.span.lo().0,
            end: expr.span.lo().0,
        };
        let mut statements = Vec::with_capacity(operands.statements.len() + 1);
        statements.push(n::Expr {
            source: anchor,
            ty: p(P::Void),
            kind: K::Let {
                binding,
                value: Box::new(callee),
            },
        });
        statements.extend(operands.statements);
        let call = n::Expr {
            source: source(expr.span),
            ty: p(P::Type),
            kind: K::Call {
                callee: Box::new(n::Expr {
                    source: anchor,
                    ty: callee_type,
                    kind: K::Local(binding),
                }),
                arguments: operands
                    .arguments
                    .into_iter()
                    .enumerate()
                    .map(|(parameter, value)| n::CallArgument {
                        passing: Default::default(),
                        parameter,
                        value,
                        is_default: false,
                    })
                    .collect(),
                parameter_bindings: Vec::new(),
            },
        };
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::CallPreparation {
                statements,
                result: Some(Box::new(call)),
            },
        })
    }

    /// Open only this invocation's binder frame. Fresh rigid slots are private
    /// dependency keys; completed argument expressions retain caller identities.
    fn open_normal_input_telescope(
        &mut self,
        parameters: &[Parameter],
        span: Span,
    ) -> Result<Vec<(DeclarationId, Sort, Term)>, Error> {
        if self
            .package_identities
            .values()
            .any(|id| *id == function_types::INPUT_NAMESPACE)
        {
            return Err(error(
                span,
                "package identity conflicts with callable input namespace",
            ));
        }
        let mut opened = Vec::new();
        let mut arguments = Vec::new();
        for parameter in parameters {
            let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let charge = nir::type_computations::charge_term(&parameter.ty, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            charge.map_err(|failure| {
                error(
                    span,
                    format!("callable input telescope budget exhausted: {failure}"),
                )
            })?;
            let domain = parameter
                .ty
                .substitute(&arguments)
                .map_err(|failure| error(span, failure.to_string()))?;
            let declaration = DeclarationId {
                package: function_types::INPUT_NAMESPACE,
                local: self.spread_binding(span)?.index,
            };
            arguments.push(Term::Rigid {
                declaration,
                sort: parameter.sort,
            });
            opened.push((declaration, parameter.sort, domain));
        }
        Ok(opened)
    }
}
