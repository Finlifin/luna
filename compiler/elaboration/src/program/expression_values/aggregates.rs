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
            H::Tuple(items) => return self.check_tuple_fields(expr, items, expected),
            H::Range { .. } => return self.range_value_expression(expr, expected),
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
                    let previous = self.inline_selection.replace((value.hir_id, usize::MAX));
                    let checked = self.expr(value, expected);
                    self.inline_selection = previous;
                    let checked = checked?;
                    self.check_inactive_inline_selectors(value, &checked)?;
                    return Ok(checked);
                }
                // Check the complete ordinary definition before publishing its
                // runtime residual. Unselected nested controls are checked but
                // their conditions are not invoked.
                let H::If(condition, _, _) = value.kind else {
                    unreachable!()
                };
                let mut ptc_selected = None;
                if self.is_type_assumption(condition) {
                    let predicate = self.type_predicate(condition)?;
                    let fact = crate::ptc::Formula::from_predicate(&predicate);
                    let mut budget = crate::ptc::Budget::default();
                    self.context
                        .validate_formula(&fact, &mut budget)
                        .map_err(|e| error(condition.span, e.to_string()))?;
                    if matches!(
                        self.context.prove(fact.clone(), budget),
                        crate::ptc::Outcome::Entailed(_)
                    ) {
                        ptc_selected = Some(true);
                    } else if matches!(
                        self.context.prove(fact.negate(), budget),
                        crate::ptc::Outcome::Entailed(_)
                    ) {
                        ptc_selected = Some(false);
                    } else if Self::predicate_has_open_terms(&predicate) {
                        return self.ordinary_inline_branches(value, expected);
                    }
                }
                let selected = if let Some(selected) = ptc_selected {
                    selected
                } else {
                    let comptime_interpretor::Value::Bool(selected) =
                        self.required_value(condition, P::Bool)?
                    else {
                        return Err(error(
                            condition.span,
                            "inline if condition must produce a comptime bool",
                        ));
                    };
                    selected
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
                    let previous = self.inline_selection.replace((value.hir_id, usize::MAX));
                    let checked = self.expr(value, expected);
                    self.inline_selection = previous;
                    let checked = checked?;
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

    fn predicate_has_open_terms(predicate: &n::TypePredicate) -> bool {
        let term_is_open = |term: &Term| {
            term.structural_terms().any(|node| {
                matches!(
                    node,
                    Term::Bound { .. }
                        | Term::Rigid { .. }
                        | Term::Hole { .. }
                        | Term::AssociatedType { .. }
                        | Term::AssociatedTypeCall(_)
                        | Term::ComptimeCall(_)
                        | Term::DeferredErrorResult { .. }
                        | Term::RuntimeField { .. }
                )
            })
        };
        let mut stack = vec![predicate];
        while let Some(current) = stack.pop() {
            match current {
                n::TypePredicate::Equal(a, b) => {
                    if term_is_open(a) || term_is_open(b) {
                        return true;
                    }
                }
                n::TypePredicate::Trait(req) => {
                    if term_is_open(&req.subject)
                        || req.interface_arguments.iter().any(term_is_open)
                    {
                        return true;
                    }
                }
                n::TypePredicate::Kind(term, _) | n::TypePredicate::NotSized(term) => {
                    if term_is_open(term) {
                        return true;
                    }
                }
                n::TypePredicate::Not(inner) => stack.push(inner),
                n::TypePredicate::And(items) | n::TypePredicate::Or(items) => {
                    stack.extend(items.iter());
                }
                n::TypePredicate::Bool(_) | n::TypePredicate::IntLe(..) => {}
            }
        }
        false
    }

    pub(in crate::program) fn range_expected_element(
        &mut self,
        expected: Option<&Term>,
        decl: DeclarationId,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let Some(expected_ty) = expected else {
            return Ok(None);
        };
        let shape = self.structural_type(expected_ty, span)?;
        Ok(match shape {
            Term::Nominal {
                declaration,
                arguments,
            } if declaration == decl => match arguments.as_slice() {
                [elem] => Some(elem.clone()),
                _ => None,
            },
            _ => None,
        })
    }

    #[inline(never)]
    fn range_value_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let H::Range {
            start,
            end,
            inclusive,
        } = &expr.kind
        else {
            unreachable!()
        };
        let (ty, kind) = match (*start, *end, *inclusive) {
            (None, None, true) | (Some(_), None, true) => {
                return Err(error(expr.span, "inclusive range requires an upper bound"));
            }
            (None, None, false) => {
                let decl = self.protocols.range_full.ok_or_else(|| {
                    error(expr.span, "range expression requires RangeFull binding")
                })?;
                (
                    Term::Nominal {
                        declaration: decl,
                        arguments: vec![],
                    },
                    K::Record {
                        declaration: decl,
                        fields: vec![],
                    },
                )
            }
            (Some(endpoint), None, false)
            | (None, Some(endpoint), false)
            | (None, Some(endpoint), true) => {
                let (decl, label) = match (*start, *inclusive) {
                    (Some(_), _) => (self.protocols.range_from, "RangeFrom"),
                    (None, false) => (self.protocols.range_to, "RangeTo"),
                    (None, true) => (self.protocols.range_to_inclusive, "RangeToInclusive"),
                };
                let decl = decl.ok_or_else(|| {
                    error(
                        expr.span,
                        format!("range expression requires {label} binding"),
                    )
                })?;
                let elem_hint = self.range_expected_element(expected, decl, expr.span)?;
                let checked = self.expr(endpoint, elem_hint.as_ref())?;
                let ty = if is_never(&checked.ty) {
                    p(P::Never)
                } else {
                    Term::Nominal {
                        declaration: decl,
                        arguments: vec![checked.ty.clone()],
                    }
                };
                (
                    ty,
                    K::Record {
                        declaration: decl,
                        fields: vec![checked],
                    },
                )
            }
            (Some(start_expr), Some(end_expr), inclusive) => {
                let (decl, label) = if inclusive {
                    (self.protocols.range_inclusive, "RangeInclusive")
                } else {
                    (self.protocols.range, "Range")
                };
                let decl = decl.ok_or_else(|| {
                    error(
                        expr.span,
                        format!("range expression requires {label} binding"),
                    )
                })?;
                let elem_hint = self.range_expected_element(expected, decl, expr.span)?;
                let mut progress = super::super::operators::peers::Progress::new(
                    self,
                    [start_expr, end_expr],
                    elem_hint.as_ref(),
                    None,
                    None,
                )?;
                while let Some(request) = progress.next()? {
                    let outcome = if request.attempt {
                        let attempt = checking_attempts::Attempt::begin(
                            self,
                            request.input.span,
                            false,
                            CheckingDisposition::Commit,
                        )?;
                        let res = self.expr(request.input, request.expected);
                        attempt.finish(self, res)?
                    } else {
                        Some(self.expr(request.input, request.expected)?)
                    };
                    progress.accept(outcome)?;
                }
                let (start_val, end_val) = progress.finish()?;
                let diverges = is_never(&start_val.ty) || is_never(&end_val.ty);
                let ty = if diverges {
                    p(P::Never)
                } else {
                    let elem_ty = elem_hint.unwrap_or_else(|| start_val.ty.clone());
                    self.require(&start_val.ty, &elem_ty, start_expr.span)?;
                    self.require(&end_val.ty, &elem_ty, end_expr.span)?;
                    Term::Nominal {
                        declaration: decl,
                        arguments: vec![elem_ty],
                    }
                };
                let mut fields = vec![start_val, end_val];
                if inclusive {
                    fields.push(n::Expr {
                        ty: p(P::Bool),
                        source: source(expr.span),
                        kind: K::Literal(n::Literal::Bool(false)),
                    });
                }
                (
                    ty,
                    K::Record {
                        declaration: decl,
                        fields,
                    },
                )
            }
        };
        self.finish_value_expression(expr, expected, ty, kind)
    }
}
