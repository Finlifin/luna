//! Match expression dispatch and candidate-state restoration.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn match_expr(
        &mut self,
        expr: &Expr<'_>,
        value: &Expr<'_>,
        arms: &[hir::pattern::PatternArm<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        self.match_expr_with_input(expr, value, arms, expected, partial, None)
    }
    pub(in crate::program) fn match_arm_pattern(
        &mut self,
        arm: &hir::pattern::PatternArm<'_>,
        value: &n::Expr,
    ) -> Result<n::Pattern, Error> {
        let observed = self.known_pattern_type_value(value);
        let domain = arguments::inference_type(value).unwrap_or(&value.ty);
        let pattern = self.pattern_observed(&arm.pat, domain, observed.as_ref())?;
        let guard = arm
            .guard
            .map(|guard| {
                self.under_matched_pattern_type_facts(&pattern, value, guard.span, |checker| {
                    checker.expr(guard, Some(&p(P::Bool)))
                })
            })
            .transpose()?;
        // Final guard runs after candidate selection, including or-patterns.
        Ok(match guard {
            Some(condition) => n::Pattern::Guard {
                pattern: Box::new(pattern),
                condition: Box::new(condition),
            },
            None => pattern,
        })
    }
    pub(in crate::program) fn match_expr_with_input(
        &mut self,
        expr: &Expr<'_>,
        value: &Expr<'_>,
        arms: &[hir::pattern::PatternArm<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        input_context: Option<&crate::calls::InputContext>,
    ) -> Result<n::Expr, Error> {
        let behavior_pattern = self.checking_open_behavior
            && self
                .inline_selection
                .is_some_and(|(id, _)| id == expr.hir_id);
        let inline_input = self
            .inline_match_input
            .as_ref()
            .filter(|(id, _, _)| *id == expr.hir_id)
            .cloned();
        let staged_match = inline_input.is_some()
            || (self.in_unselected_inline
                && self
                    .inline_selection
                    .is_some_and(|(id, _)| id == expr.hir_id));
        let value = if let Some((_, value, _)) = &inline_input {
            value.clone()
        } else {
            self.expr(value, None)?
        };
        let saved = self.locals.clone();
        let saved_data = inline_input.as_ref().map(|_| self.generator_data.clone());
        let saved_selection = self.inline_selection;
        let saved_suppressed = self.in_unselected_inline;
        let result = (|| {
            let inferred = if expected.is_none() {
                match input_context {
                    Some(context) => self.optional_branch_context(
                        expr.span,
                        context,
                        arms.len(),
                        |checker, index| {
                            let arm = &arms[index];
                            let pattern = checker.match_arm_pattern(arm, &value)?;
                            checker.under_matched_pattern_type_facts(
                                &pattern,
                                &value,
                                arm.body.span,
                                |checker| checker.input_context_expression(arm.body, context),
                            )
                        },
                    )?,
                    None => None,
                }
            } else {
                None
            };
            let expected = expected.or(inferred.as_ref());
            let mut checked = Vec::new();
            let mut result_ty = p(P::Never);
            let mut selected = None;
            let mut selected_bindings = Vec::new();
            for (index, arm) in arms.iter().enumerate() {
                self.locals = saved.clone();
                if let Some(saved_data) = &saved_data {
                    self.generator_data = saved_data.clone();
                }
                let pattern = if behavior_pattern {
                    self.with_staged_input(true, |this| this.match_arm_pattern(arm, &value))?
                } else {
                    self.match_arm_pattern(arm, &value)?
                };
                let staged: BTreeSet<_> = self
                    .locals
                    .keys()
                    .filter(|id| !saved.contains_key(id))
                    .copied()
                    .collect();
                if staged_match && !self.checking_open_behavior {
                    self.check_inline_pattern_availability(&pattern, arm.span)?;
                }
                if let Some((_, _, subject)) = &inline_input {
                    if selected.is_none() {
                        if let Some(bindings) =
                            self.select_inline_pattern(&pattern, subject, arm.body)?
                        {
                            selected = Some(index);
                            selected_bindings =
                                self.materialize_inline_bindings(&bindings, arm.body)?;
                        }
                    }
                    self.inline_selection = Some((expr.hir_id, selected.unwrap_or(usize::MAX)));
                    self.in_unselected_inline = selected != Some(index);
                }
                let arm_context =
                    expected.or_else(|| (!is_never(&result_ty)).then_some(&result_ty));
                let saved_staged = self.open_staged_locals.clone();
                let saved_staged_members = self.open_member_locals.clone();
                if staged_match {
                    self.open_staged_locals.extend(staged);
                }
                let body = self.under_matched_pattern_type_facts(
                    &pattern,
                    &value,
                    arm.body.span,
                    |checker| {
                        checker.with_callee_delivery(
                            expr.hir_id,
                            Some(arm.body.hir_id),
                            |checker| match input_context {
                                Some(input_context) if arm_context.is_none() => {
                                    checker.input_context_expression(arm.body, input_context)
                                }
                                _ => checker.expr_context(arm.body, arm_context, partial),
                            },
                        )
                    },
                );
                self.open_staged_locals = saved_staged;
                self.open_member_locals = saved_staged_members;
                let body = body?;
                if !is_never(&body.ty) {
                    if !is_never(&result_ty) {
                        self.require(&body.ty, &result_ty, arm.span)?;
                    }
                    result_ty = body.ty.clone();
                }
                let mut checked_arm = n::MatchArm {
                    pattern,
                    guard: None,
                    body,
                };
                if inline_input.is_none() {
                    self.stabilize_error_success_binding(&mut checked_arm, &value.ty, arm.span)?;
                }
                checked.push(checked_arm);
            }
            // Desugarings such as `matches` give both synthetic arm bodies the
            // same source span. Cleanup uses body sites as runtime identities:
            // assign distinct generated sites while preserving diagnostics.
            self.required_steps = self
                .required_steps
                .checked_sub(checked.len() as u64)
                .ok_or_else(|| error(expr.span, "match arm source budget exhausted"))?;
            let mut body_sites = BTreeMap::new();
            for arm in &checked {
                *body_sites
                    .entry((arm.body.source.start, arm.body.source.end))
                    .or_insert(0usize) += 1;
            }
            for arm in &mut checked {
                let original = arm.body.source;
                if original == source(expr.span) || body_sites[&(original.start, original.end)] > 1
                {
                    let index = u32::try_from(self.expansion_sources.len())
                        .map_err(|_| error(expr.span, "match arm source space exhausted"))?;
                    self.expansion_sources.push(n::ExpansionSource {
                        original,
                        site: source(expr.span),
                    });
                    arm.body.source = n::SourceRange::generated(index);
                }
            }
            let patterns: Vec<_> = checked
                .iter()
                .filter(|arm| arm.guard.is_none())
                .map(|arm| &arm.pattern)
                .collect();
            let exhaustive = self.patterns_exhaustive(&patterns, &value.ty);
            if exhaustive != crate::coverage::Coverage::Exhaustive {
                return Err(error(
                    expr.span,
                    if exhaustive == crate::coverage::Coverage::Unknown {
                        "match exhaustiveness could not be proved within supported analysis and budget; add an unguarded fallback pattern"
                    } else {
                        "non-exhaustive match; add an unguarded fallback pattern"
                    },
                ));
            }
            if inline_input.is_some() {
                let index =
                    selected.ok_or_else(|| error(expr.span, "inline match has no selected arm"))?;
                let body = checked.swap_remove(index).body;
                return Ok(n::Expr {
                    ty: body.ty.clone(),
                    source: source(expr.span),
                    kind: K::Block {
                        statements: selected_bindings,
                        result: Some(Box::new(body)),
                    },
                });
            }
            if behavior_pattern {
                // Preserve the original typed patterns for branch facts and
                // coverage above; only runtime behavior evidence is abstracted.
                for arm in &mut checked {
                    self.abstract_inline_pattern(&mut arm.pattern, expr.span)?;
                }
            }
            self.stabilize_error_success_patterns(&mut checked, &value.ty, &result_ty, expr.span)?;
            Ok(n::Expr {
                ty: if is_never(&value.ty) {
                    p(P::Never)
                } else {
                    result_ty
                },
                source: source(expr.span),
                kind: K::Match {
                    value: Box::new(value),
                    arms: checked,
                },
            })
        })();
        self.locals = saved;
        if let Some(saved_data) = saved_data {
            self.generator_data = saved_data;
        }
        self.inline_selection = saved_selection;
        self.in_unselected_inline = saved_suppressed;
        result
    }
}
