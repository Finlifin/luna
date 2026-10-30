//! Thread one original result through catch phases; never clone an affine payload.
use super::*;

impl Checker<'_, '_> {
    fn catch_local(
        &mut self,
        binding: n::LocalId,
        ty: &Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        Ok(n::Expr {
            source: self.error_generated_source(span)?,
            ty: ty.clone(),
            kind: K::Local(binding),
        })
    }

    fn catch_arm(
        &mut self,
        arm: &hir::pattern::PatternArm<'_>,
        input: &n::Expr,
        payload: bool,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::MatchArm, Error> {
        let wrapped = hir::pattern::Pattern {
            hir_id: arm.pat.hir_id,
            span: arm.pat.span,
            kind: hir::pattern::PatternKind::ErrorErr(&arm.pat),
        };
        let mut pattern = self.pattern(if payload { &wrapped } else { &arm.pat }, &input.ty)?;
        if let Some(guard) = arm.guard {
            let condition =
                self.under_matched_pattern_type_facts(&pattern, input, guard.span, |this| {
                    this.expr(guard, Some(&p(P::Bool)))
                })?;
            pattern = n::Pattern::Guard {
                pattern: Box::new(pattern),
                condition: Box::new(condition),
            };
        }
        let body =
            self.under_matched_pattern_type_facts(&pattern, input, arm.body.span, |this| {
                if let Some(expected) = expected {
                    this.expr_context(arm.body, Some(expected), partial)
                } else {
                    this.expr_context(arm.body, None, partial)
                }
            })?;
        Ok(n::MatchArm {
            pattern,
            guard: None,
            body,
        })
    }

    // A normally completing phase must return the original whole value. A
    // plain whole binding already owns it; other patterns observe constituents
    // under an as-binding. Non-Copy constituent transfers cannot coexist with
    // later delivery of the whole. A diverging arm needs no retained whole.
    fn retain_catch_input(
        &mut self,
        mut arm: n::MatchArm,
        ty: &Term,
        span: Span,
    ) -> Result<n::MatchArm, Error> {
        if is_never(&arm.body.ty) {
            return Ok(arm);
        }
        let mut binding_pattern = &arm.pattern;
        while let n::Pattern::Guard { pattern, .. } = binding_pattern {
            binding_pattern = pattern;
        }
        let binding = match binding_pattern {
            n::Pattern::Binding { binding, inner }
                if matches!(inner.as_ref(), n::Pattern::Wild) =>
            {
                *binding
            }
            _ => {
                let binding = self.spread_binding(span)?;
                arm.pattern = n::Pattern::Binding {
                    binding,
                    inner: Box::new(arm.pattern),
                };
                binding
            }
        };
        let discard = n::Expr {
            source: self.error_generated_source(span)?,
            ty: p(P::Void),
            kind: K::Discard(Box::new(arm.body)),
        };
        let result = self.catch_local(binding, ty, span)?;
        arm.body = n::Expr {
            source: self.error_generated_source(span)?,
            ty: ty.clone(),
            kind: K::Block {
                statements: vec![discard],
                result: Some(Box::new(result)),
            },
        };
        Ok(arm)
    }

    fn catch_fallback(&mut self, ty: &Term, span: Span) -> Result<n::MatchArm, Error> {
        let binding = self.spread_binding(span)?;
        Ok(n::MatchArm {
            pattern: n::Pattern::Binding {
                binding,
                inner: Box::new(n::Pattern::Wild),
            },
            guard: None,
            body: self.catch_local(binding, ty, span)?,
        })
    }

    #[inline(never)]
    pub(super) fn error_catch_handle(
        &mut self,
        expression: &Expr<'_>,
        input: n::Expr,
        before: Option<&hir::pattern::PatternArm<'_>>,
        arms: &[hir::pattern::PatternArm<'_>],
        after: &hir::pattern::PatternArm<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let Term::ErrorResult { errors, result } = &input.ty else {
            unreachable!()
        };
        let labels = errors.clone();
        let success = (**result).clone();
        let input_ty = input.ty.clone();
        let saved = self.locals.clone();
        let outcome = (|| {
            let before = if let Some(before) = before {
                Some(self.catch_arm(before, &input, false, None, partial)?)
            } else {
                None
            };
            let mut ordinary = vec![];
            for arm in arms {
                self.locals = saved.clone();
                ordinary.push(self.catch_arm(arm, &input, true, None, partial)?);
            }
            self.locals = saved.clone();
            let mut after = self.catch_arm(after, &input, false, expected, partial)?;
            // Only an initial run of diverging ordinary arms is guaranteed
            // to exit on its matched inputs; an earlier normal arm may shadow
            // later arms and still continue into the trailing catch.
            let mut resolved = vec![&after.pattern];
            if let Some(before) = &before {
                if is_never(&before.body.ty) {
                    resolved.push(&before.pattern);
                }
            }
            resolved.extend(
                ordinary
                    .iter()
                    .take_while(|arm| is_never(&arm.body.ty))
                    .map(|arm| &arm.pattern),
            );
            let mut residual = vec![];
            for label in &labels {
                let domain = Term::error_result(vec![label.clone()], p(P::Never));
                let allowance = usize::try_from(self.required_steps)
                    .unwrap_or(usize::MAX)
                    .min(100_000);
                let (coverage, used) = crate::coverage::check_metered_with_collections_in(
                    &self.structs,
                    &self.enums,
                    &domain,
                    &resolved,
                    allowance,
                    &self.context,
                    self.protocols.tuple,
                    &|domain, fuel| self.collection_domain_budgeted(domain, fuel),
                    self.target,
                );
                self.required_steps -= used as u64;
                if coverage != crate::coverage::Coverage::Exhaustive {
                    residual.push(label.clone());
                }
            }
            let target = if let Some(expected) = expected {
                expected.clone()
            } else {
                let mut output_errors = residual.clone();
                let mut output = success.clone();
                if !is_never(&after.body.ty) {
                    let returned = if let Term::ErrorResult { errors, result } = &after.body.ty {
                        output_errors.extend(errors.iter().cloned());
                        result.as_ref()
                    } else {
                        &after.body.ty
                    };
                    if !is_never(returned) {
                        if is_never(&output) {
                            output = returned.clone();
                        } else {
                            self.require(&output, returned, expression.span)?;
                        }
                    }
                }
                Term::error_result(output_errors, output)
            };
            after.body = self.error_delivery(after.body, &target, expression.span)?;
            let mut final_arms = vec![after];
            for label in residual {
                let binding = self.spread_binding(expression.span)?;
                let value = self.catch_local(binding, &label, expression.span)?;
                let mut body = self.convert_error_value(
                    value,
                    &target,
                    nir::error_results::Conversion::Failure,
                    expression.span,
                )?;
                body.source = self.error_generated_source(expression.span)?;
                final_arms.push(n::MatchArm {
                    pattern: n::Pattern::ErrorResult {
                        label: Some(label),
                        inner: Box::new(n::Pattern::Binding {
                            binding,
                            inner: Box::new(n::Pattern::Wild),
                        }),
                    },
                    guard: None,
                    body,
                });
            }
            let original = self.spread_binding(expression.span)?;
            let mut threaded = self.catch_local(original, &input_ty, expression.span)?;
            if let Some(before) = before {
                let first = self.retain_catch_input(before, &input_ty, expression.span)?;
                let fallback = self.catch_fallback(&input_ty, expression.span)?;
                threaded = n::Expr {
                    source: self.error_generated_source(expression.span)?,
                    ty: input_ty.clone(),
                    kind: K::Match {
                        value: Box::new(threaded),
                        arms: vec![first, fallback],
                    },
                };
            }
            if !ordinary.is_empty() {
                let mut selected = vec![];
                for arm in ordinary {
                    selected.push(self.retain_catch_input(arm, &input_ty, expression.span)?);
                }
                selected.push(self.catch_fallback(&input_ty, expression.span)?);
                threaded = n::Expr {
                    source: self.error_generated_source(expression.span)?,
                    ty: input_ty.clone(),
                    kind: K::Match {
                        value: Box::new(threaded),
                        arms: selected,
                    },
                };
            }
            let failure = n::Expr {
                source: self.error_generated_source(expression.span)?,
                ty: target.clone(),
                kind: K::Match {
                    value: Box::new(threaded),
                    arms: final_arms,
                },
            };
            let binding = self.spread_binding(expression.span)?;
            let value = self.catch_local(binding, &success, expression.span)?;
            let success_arm = n::MatchArm {
                pattern: n::Pattern::ErrorResult {
                    label: None,
                    inner: Box::new(n::Pattern::Binding {
                        binding,
                        inner: Box::new(n::Pattern::Wild),
                    }),
                },
                guard: None,
                body: self.error_delivery(value, &target, expression.span)?,
            };
            let errors = n::Pattern::Or(
                labels
                    .into_iter()
                    .map(|label| n::Pattern::ErrorResult {
                        label: Some(label),
                        inner: Box::new(n::Pattern::Wild),
                    })
                    .collect(),
            );
            Ok(n::Expr {
                source: source(expression.span),
                ty: target,
                kind: K::Match {
                    value: Box::new(input),
                    arms: vec![
                        success_arm,
                        n::MatchArm {
                            pattern: n::Pattern::Binding {
                                binding: original,
                                inner: Box::new(errors),
                            },
                            guard: None,
                            body: failure,
                        },
                    ],
                },
            })
        })();
        self.locals = saved;
        outcome
    }
}
