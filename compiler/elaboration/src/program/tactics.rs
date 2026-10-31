//! A marked input introduces a lexical placeholder boundary. Try ordinary
//! value formation transactionally first; only a free placeholder requests a
//! tactic closure. Nested marked inputs and explicit lambdas own their frames.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn tactic_parameter(
        &mut self,
        expr: &Expr<'_>,
        index: usize,
    ) -> Result<hir::HirId, Error> {
        let Some((scope, parameters, used)) = &mut self.tactic_frame else {
            return Err(Error {
                needs_tactic: true,
                ..error(
                    expr.span,
                    "tactic placeholder requires a lambda-marked argument",
                )
            });
        };
        let range = source(expr.span);
        if range.start < scope.start || range.end > scope.end {
            return Err(error(
                expr.span,
                "tactic placeholder is outside its lexical boundary",
            ));
        }
        *used = true;
        parameters
            .get(index)
            .copied()
            .ok_or_else(|| error(expr.span, "tactic placeholder index exceeds target arity"))
    }

    pub(super) fn tactic_argument(
        &mut self,
        input: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        if let H::PostLambdaBody(body) = input.kind {
            let body = self
                .package
                .body(body)
                .ok_or_else(|| error(input.span, "missing post do body"))?;
            return self.tactic_closure(body.value, expected, partial, false);
        }
        let saved = self.tactic_frame.take();
        let first = self.argument_attempt(input, None, None);
        let contextual = matches!(&first, Ok(None));
        let attempt = match first {
            Ok(None) => self.argument_attempt(input, expected, partial),
            result => result,
        };
        self.tactic_frame = saved;
        match attempt {
            Ok(Some(value)) => {
                if let Some(expected) = expected {
                    self.require(&value.ty, expected, input.span)?;
                }
                Ok(value)
            }
            Err(failure)
                if failure.needs_tactic
                    && failure.source.start >= input.span.lo().0
                    && failure.source.end <= input.span.hi().0 =>
            {
                self.tactic_closure(input, expected, partial, true)
            }
            Err(failure) if !contextual => Err(failure),
            ordinary => {
                // Contextual constructors can need the callback RESULT type
                // before reaching their placeholder. Accept this interpretation
                // only if a parameter of this exact frame was actually used.
                match self.tactic_closure(input, expected, partial, true) {
                    Ok(value) => Ok(value),
                    Err(_) => match ordinary {
                        Err(failure) => Err(failure),
                        _ => Err(context_error(
                            input.span,
                            "lambda-marked input needs a callable context",
                        )),
                    },
                }
            }
        }
    }

    fn tactic_closure(
        &mut self,
        input: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        require_placeholder: bool,
    ) -> Result<n::Expr, Error> {
        let shape = expected
            .map(|ty| self.structural_type(ty, input.span))
            .transpose()?;
        let arity = match shape.as_ref().map(Term::callable_signature) {
            Some(Term::Function { parameters, .. }) => parameters.len(),
            Some(_) => return Err(error(input.span, "tactic lambda target is not callable")),
            None => partial
                .map(|context| context.parameters.len())
                .ok_or_else(|| {
                    context_error(input.span, "tactic lambda needs the target parameter list")
                })?,
        };
        let mut parameters = Vec::with_capacity(arity);
        for _ in 0..arity {
            let binding = self.spread_binding(input.span)?;
            let id = hir::HirId::new(hir::OwnerId::INVALID, hir::ItemLocalId::new(binding.index));
            parameters.push(hir::expr::ClosureParam {
                hir_id: id,
                pat: hir::Pattern {
                    hir_id: id,
                    kind: hir::PatternKind::Binding(
                        hir::common::BindingMode::ByValue,
                        hir::common::Ident::new("$tactic", input.span),
                        None,
                    ),
                    span: input.span,
                },
                ty: None,
                span: input.span,
            });
        }
        let (value, used) =
            self.closure_body(input, &parameters, None, input, expected, partial, true)?;
        if require_placeholder && !used {
            return Err(error(
                input.span,
                "expression has no placeholder in this lambda boundary",
            ));
        }
        Ok(value)
    }
}
