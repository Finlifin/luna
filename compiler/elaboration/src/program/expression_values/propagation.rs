//! Optional and error result construction and propagation.
use super::*;

impl Checker<'_, '_> {
    #[inline(never)]
    pub(super) fn propagation_value_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        _partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let (ty, kind) = match &expr.kind {
            H::ErrorNew(input) => {
                let hint = match expected {
                    Some(Term::ErrorResult { errors, .. }) if errors.len() == 1 => errors.first(),
                    _ => None,
                };
                let value = self.expr(input, hint)?;
                if is_never(&value.ty) {
                    return Ok(value);
                }
                let inferred = Term::error_result(vec![value.ty.clone()], p(P::Never));
                let target = expected.unwrap_or(&inferred);
                return self.convert_error_value(
                    value,
                    target,
                    nir::error_results::Conversion::Failure,
                    expr.span,
                );
            }
            H::ErrorPropagate(input) => {
                let target = self.result.clone();
                if target.is_none()
                    && !matches!(self.return_observations, Some(ReturnObservations::Infer(_)))
                {
                    return Err(context_error(
                        expr.span,
                        "error propagation requires a function result context",
                    ));
                }
                let input = self.expr(input, None)?;
                if is_never(&input.ty) {
                    return Ok(input);
                }
                let Term::ErrorResult {
                    errors,
                    result: payload,
                } = &input.ty
                else {
                    return Err(error(
                        expr.span,
                        "error propagation requires an error-qualified operand",
                    ));
                };
                if let Some(expected) = expected {
                    self.require(payload, expected, expr.span)?;
                }
                self.required_steps = self
                    .required_steps
                    .checked_sub(errors.len() as u64)
                    .ok_or_else(|| error(expr.span, "error propagation budget exhausted"))?;
                let payload = (**payload).clone();
                let labels = errors.clone();
                let binding = self.spread_binding(expr.span)?;
                let mut arms = vec![n::MatchArm {
                    pattern: n::Pattern::ErrorResult {
                        label: None,
                        inner: Box::new(n::Pattern::Binding {
                            binding,
                            inner: Box::new(n::Pattern::Wild),
                        }),
                    },
                    guard: None,
                    body: n::Expr {
                        source: self.error_generated_source(expr.span)?,
                        ty: payload.clone(),
                        kind: K::Local(binding),
                    },
                }];
                for label in labels {
                    let binding = self.spread_binding(expr.span)?;
                    let value = n::Expr {
                        source: self.error_generated_source(expr.span)?,
                        ty: label.clone(),
                        kind: K::Local(binding),
                    };
                    let provisional = Term::error_result(vec![label.clone()], p(P::Never));
                    let failure = self.convert_error_value(
                        value,
                        target.as_ref().unwrap_or(&provisional),
                        nir::error_results::Conversion::Failure,
                        expr.span,
                    )?;
                    let return_source = self.error_generated_source(expr.span)?;
                    if target.is_none() {
                        self.pending_error_returns
                            .insert((return_source.start, return_source.end), label.clone());
                    }
                    arms.push(n::MatchArm {
                        pattern: n::Pattern::ErrorResult {
                            label: Some(label),
                            inner: Box::new(n::Pattern::Binding {
                                binding,
                                inner: Box::new(n::Pattern::Wild),
                            }),
                        },
                        guard: None,
                        body: n::Expr {
                            source: return_source,
                            ty: p(P::Never),
                            kind: K::Return(Some(Box::new(failure))),
                        },
                    });
                }
                if let (Some(observations), Some(target)) = (&mut self.return_observations, target)
                {
                    observations.push(target);
                }
                return Ok(n::Expr {
                    source: source(expr.span),
                    ty: payload,
                    kind: K::Match {
                        value: Box::new(input),
                        arms,
                    },
                });
            }
            H::OptionPropagate(input) => {
                let result = self.result.clone();
                let deferred = self.return_observations.is_some()
                    && matches!(
                        &self.partial_return_context,
                        Some(crate::calls::InputContext::Optional(_))
                    );
                if result.is_none() && !deferred {
                    return Err(context_error(
                        expr.span,
                        "optional propagation requires an optional function result context",
                    ));
                }
                let result_layout = result
                    .as_ref()
                    .map(|result| self.optional_payload(result, expr.span))
                    .transpose()?;
                let input = if let Some(payload) = expected {
                    if let Some(value) = self.argument_attempt(input, None, None)? {
                        value
                    } else {
                        let target = Term::Optional(Box::new(payload.clone()));
                        // Supply the missing operand context without implicitly
                        // injecting an already nonoptional operand at this root.
                        self.expr_context_unadapted(input, Some(&target), None)?
                    }
                } else {
                    self.expr(input, None)?
                };
                if is_never(&input.ty) {
                    return Ok(input);
                }
                let payload = self.optional_payload(&input.ty, expr.span)?;
                let binding = self.spread_binding(expr.span)?;
                // Match, successful projection, empty value and Return need distinct
                // source anchors: cleanup uses them to identify runtime boundaries.
                let return_source = n::SourceRange {
                    start: expr.span.lo().0,
                    end: expr.span.lo().0,
                };
                let empty_source = n::SourceRange {
                    start: expr.span.lo().0,
                    end: expr.span.lo().0 + 1,
                };
                let empty = if result_layout.is_some() {
                    let result = result.unwrap();
                    if let Some(observations) = &mut self.return_observations {
                        observations.push(result.clone());
                    }
                    Some(Box::new(n::Expr {
                        source: empty_source,
                        ty: result,
                        kind: K::OptionalAbsent,
                    }))
                } else {
                    // This private control-exit site is completed before the closure is exported.
                    self.pending_optional_returns
                        .insert((return_source.start, return_source.end), empty_source);
                    None
                };
                if let Some(expected) = expected {
                    self.require(&payload, expected, expr.span)?;
                }
                let match_source = n::SourceRange {
                    start: input.source.start,
                    end: expr.span.hi().0,
                };
                return Ok(n::Expr {
                    source: match_source,
                    ty: payload.clone(),
                    kind: K::Match {
                        value: Box::new(input),
                        arms: vec![
                            n::MatchArm {
                                pattern: n::Pattern::OptionalPresent(Box::new(
                                    n::Pattern::Binding {
                                        binding,
                                        inner: Box::new(n::Pattern::Wild),
                                    },
                                )),
                                guard: None,
                                body: n::Expr {
                                    source: n::SourceRange {
                                        start: expr.span.hi().0,
                                        end: expr.span.hi().0,
                                    },
                                    ty: payload,
                                    kind: K::Local(binding),
                                },
                            },
                            n::MatchArm {
                                pattern: n::Pattern::OptionalAbsent,
                                guard: None,
                                body: n::Expr {
                                    source: return_source,
                                    ty: p(P::Never),
                                    kind: K::Return(empty),
                                },
                            },
                        ],
                    },
                });
            }
            _ => unreachable!("value expression category was checked by the dispatcher"),
        };
        self.finish_value_expression(expr, expected, ty, kind)
    }
}
