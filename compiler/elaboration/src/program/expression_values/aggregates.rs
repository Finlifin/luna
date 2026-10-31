//! Aggregate construction and branch values.
use super::*;

impl Checker<'_, '_> {
    #[inline(never)]
    pub(super) fn aggregates_value_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let (ty, kind) = match &expr.kind {
            H::CollectedProperties(fields) => {
                return self.property_packet(expr, fields, expected);
            }
            H::Object(entries) => {
                let mut names = Vec::with_capacity(entries.len());
                let mut fields = Vec::with_capacity(entries.len());
                for entry in entries {
                    let (name, value) = match entry {
                        hir::expr::ObjectEntry::Value(value) => (None, *value),
                        hir::expr::ObjectEntry::Property(field) => {
                            (Some(field.ident.name.as_str().to_owned()), field.expr)
                        }
                    };
                    names.push(name);
                    fields.push(self.expr(value, None)?);
                }
                let ty = if fields.iter().any(|field| is_never(&field.ty)) {
                    p(P::Never)
                } else {
                    p(P::Object)
                };
                (ty, K::Object { names, fields })
            }
            H::Tuple(items) => {
                // A proven type equation can expose a tuple's element contexts
                // without unfolding an opaque normal-form head. Keep the final
                // equality check against the original expected type below.
                let expected_shape = expected
                    .map(|ty| self.structural_type(ty, expr.span))
                    .transpose()?;
                let expected_items = if let Some(Term::Tuple(items)) = expected_shape.as_ref() {
                    Some(items)
                } else {
                    None
                };
                let values = items
                    .iter()
                    .enumerate()
                    .map(|(i, x)| self.expr(x, expected_items.and_then(|xs| xs.get(i))))
                    .collect::<Result<Vec<_>, _>>()?;
                let ty = if values.iter().any(|x| is_never(&x.ty)) {
                    p(P::Never)
                } else {
                    self.tuple_value_type(values.iter().map(|x| x.ty.clone()).collect(), expr.span)?
                };
                (ty, K::Tuple(values))
            }
            H::If(condition, yes, no) => {
                let selection = self
                    .inline_selection
                    .filter(|(id, _)| *id == expr.hir_id)
                    .map(|(_, choice)| choice);
                // An inline selector remains a comptime input while checking
                // both ordinary branches; it is not delivered to runtime.
                let saved_comptime = self.in_required_comptime;
                self.in_required_comptime |= selection.is_some();
                let condition = self.expr(condition, Some(&p(P::Bool)));
                self.in_required_comptime = saved_comptime;
                let condition = condition?;
                let suppressed = self.in_unselected_inline;
                self.in_unselected_inline = suppressed || selection == Some(1);
                let yes = self.under_type_condition(&condition, true, expr.span, |checker| {
                    checker.with_callee_delivery(
                        expr.hir_id,
                        yes.expr.map(|tail| tail.hir_id),
                        |checker| {
                            checker.block_context(
                                yes,
                                if no.is_some() { expected } else { None },
                                if no.is_some() { partial } else { None },
                            )
                        },
                    )
                });
                self.in_unselected_inline = suppressed;
                let yes = yes?;
                let yes = if no.is_none() {
                    discard_result(yes)
                } else {
                    yes
                };
                let branch_context = expected.or_else(|| (!is_never(&yes.ty)).then_some(&yes.ty));
                self.in_unselected_inline = suppressed || selection == Some(0);
                let no = no
                    .map(|x| {
                        self.under_type_condition(&condition, false, expr.span, |checker| {
                            checker.with_callee_delivery(expr.hir_id, Some(x.hir_id), |checker| {
                                checker.expr_context(x, branch_context, partial)
                            })
                        })
                    })
                    .transpose();
                self.in_unselected_inline = suppressed;
                let no = no?;
                let other = no.as_ref().map(|x| x.ty.clone()).unwrap_or(p(P::Void));
                let ty = if control::conditional_never(&condition, &yes.ty, &other) {
                    p(P::Never)
                } else if no.is_some() && branch_context.is_some() {
                    // Branch-local type equalities already justified every
                    // completing delivery, including a sole surviving branch.
                    branch_context.unwrap().clone()
                } else if is_never(&yes.ty) {
                    other
                } else if is_never(&other) {
                    yes.ty.clone()
                } else {
                    self.require(&other, &yes.ty, expr.span)?;
                    yes.ty.clone()
                };
                (
                    ty,
                    K::If {
                        condition: Box::new(condition),
                        then_branch: Box::new(yes),
                        else_branch: no.map(Box::new),
                    },
                )
            }
            H::InlineControl(value)
                if matches!(value.kind, H::If(..) | H::When(..) | H::Match(..))
                    && self.closed_comptime_control_body() =>
            {
                // This body executes wholly at comptime. Keep typed branch
                // control in the VM so ordinary arguments and mutable locals
                // are observed at invocation, without changing their ABI sorts.
                // Quoted runtime bodies and open residuals retain specialization.
                return self.expr(value, expected);
            }
            H::InlineControl(value) if matches!(value.kind, H::If(..)) => {
                if self.checking_open_behavior {
                    return self.ordinary_inline_branches(value, expected);
                }
                if self.in_unselected_inline
                    && self
                        .inline_selection
                        .is_some_and(|(id, _)| id.owner == expr.hir_id.owner)
                {
                    let checked = self.expr(value, expected)?;
                    self.check_inactive_inline_selectors(value, &checked)?;
                    return Ok(checked);
                }
                // Check the complete ordinary definition before publishing its
                // runtime residual. Unselected nested controls are checked but
                // their conditions are not invoked.
                let H::If(condition, _, _) = value.kind else {
                    unreachable!()
                };
                let comptime_interpretor::Value::Bool(selected) =
                    self.required_value(condition, P::Bool)?
                else {
                    return Err(error(
                        condition.span,
                        "inline if condition must produce a comptime bool",
                    ));
                };
                let previous = self
                    .inline_selection
                    .replace((value.hir_id, usize::from(!selected)));
                let previous_suppressed = std::mem::replace(&mut self.in_unselected_inline, false);
                let checked = self.expr(value, expected);
                self.inline_selection = previous;
                self.in_unselected_inline = previous_suppressed;
                let checked = checked?;
                let K::If {
                    then_branch,
                    else_branch,
                    ..
                } = checked.kind
                else {
                    unreachable!()
                };
                return Ok(if selected {
                    *then_branch
                } else {
                    else_branch.map(|branch| *branch).unwrap_or(n::Expr {
                        source: source(expr.span),
                        ty: p(P::Void),
                        kind: K::Unit,
                    })
                });
            }
            H::InlineControl(value) if matches!(value.kind, H::When(_)) => {
                if self.checking_open_behavior {
                    return self.ordinary_inline_branches(value, expected);
                }
                if self.in_unselected_inline
                    && self
                        .inline_selection
                        .is_some_and(|(id, _)| id.owner == expr.hir_id.owner)
                {
                    let checked = self.expr(value, expected)?;
                    self.check_inactive_inline_selectors(value, &checked)?;
                    return Ok(checked);
                }
                let H::When(arms) = value.kind else {
                    unreachable!()
                };
                if arms
                    .iter()
                    .position(|arm| arm.cond.is_none())
                    .is_some_and(|index| index + 1 != arms.len())
                {
                    return Err(error(expr.span, "when fallback must be the final arm"));
                }
                let mut selected = arms.len();
                for (index, arm) in arms.iter().enumerate() {
                    let matches = if let Some(condition) = arm.cond {
                        let comptime_interpretor::Value::Bool(value) =
                            self.required_value(condition, P::Bool)?
                        else {
                            return Err(error(
                                condition.span,
                                "inline when condition must produce a comptime bool",
                            ));
                        };
                        value
                    } else {
                        true
                    };
                    if matches {
                        selected = index;
                        break;
                    }
                }
                let previous = self.inline_selection.replace((value.hir_id, selected));
                let previous_suppressed = std::mem::replace(&mut self.in_unselected_inline, false);
                let checked = self.expr(value, expected);
                self.inline_selection = previous;
                self.in_unselected_inline = previous_suppressed;
                return checked;
            }
            H::InlineControl(value) if matches!(value.kind, H::Match(..)) => {
                if self.checking_open_behavior {
                    return self.ordinary_inline_branches(value, expected);
                }
                if self.in_unselected_inline
                    && self
                        .inline_selection
                        .is_some_and(|(id, _)| id.owner == expr.hir_id.owner)
                {
                    let previous = self.inline_selection.replace((value.hir_id, usize::MAX));
                    let checked = self.expr(value, expected);
                    self.inline_selection = previous;
                    let checked = checked?;
                    self.check_inactive_inline_selectors(value, &checked)?;
                    return Ok(checked);
                }
                let H::Match(subject, _) = value.kind else {
                    unreachable!()
                };
                let (completed, ty) = self.required_typed_expression(subject, None)?;
                let term = self.required_normal_term(completed.clone(), subject.span)?;
                let input = self.materialize_local_constant(completed, ty, subject.span)?;
                let previous = self.inline_match_input.replace((value.hir_id, input, term));
                let checked = self.expr(value, expected);
                self.inline_match_input = previous;
                return checked;
            }
            H::InlineControl(value) if matches!(value.kind, H::For { .. }) => {
                return self.expand_inline_range(value, expected);
            }
            H::InlineControl(_) => {
                return Err(error(
                    expr.span,
                    "inline control-flow expansion is not yet implemented; it cannot be treated as ordinary runtime control flow",
                ));
            }
            _ => unreachable!("value expression category was checked by the dispatcher"),
        };
        self.finish_value_expression(expr, expected, ty, kind)
    }
}
