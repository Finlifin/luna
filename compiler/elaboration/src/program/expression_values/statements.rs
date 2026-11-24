//! Statements, transfers and checked control predicates.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn generated_expression_source(
        &mut self,
        original: n::SourceRange,
        span: Span,
    ) -> Result<n::SourceRange, Error> {
        let index = u32::try_from(self.expansion_sources.len())
            .map_err(|_| error(span, "generated expression source space exhausted"))?;
        self.expansion_sources.push(n::ExpansionSource {
            original,
            site: source(span),
        });
        Ok(n::SourceRange::generated(index))
    }

    pub(in crate::program) fn discard_result(
        &mut self,
        value: n::Expr,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if is_never(&value.ty) || value.ty == p(P::Void) {
            return Ok(value);
        }
        let source = self.generated_expression_source(value.source, span)?;
        Ok(n::Expr {
            ty: p(P::Void),
            source,
            kind: K::Discard(Box::new(value)),
        })
    }

    pub(in crate::program) fn finish_semicolon(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        value: n::Expr,
    ) -> Result<n::Expr, Error> {
        let ty = p(if is_never(&value.ty) {
            P::Never
        } else {
            P::Void
        });
        let mut discard =
            self.finish_value_expression(expr, expected, ty, K::Discard(Box::new(value)))?;
        if !discard.shares_discard_temporary_boundary() {
            discard.source = self.generated_expression_source(discard.source, expr.span)?;
        }
        Ok(discard)
    }

    #[inline(never)]
    pub(super) fn control_value_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let (ty, kind) = match &expr.kind {
            H::Semi(inner) => {
                let value = self.expr(inner, None)?;
                return self.finish_semicolon(expr, expected, value);
            }
            H::For {..} => return self.iterator_loop(expr, expected),
            H::Loop(block) => {
                let identity = local(expr.hir_id);
                let mut body = self.elaborate_loop_body_with_barrier(expr.span, identity, |this| {
                    this.control.loops.push(identity);
                    let body = this.block(block, None);
                    this.control.loops.pop();
                    body
                })?;
                let mut source_fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = source_fuel;
                let freshened = control::freshen_loop_sites(&mut body, source(expr.span), &mut self.expansion_sources, &mut source_fuel);
                self.required_steps -= (before - source_fuel) as u64;
                freshened.map_err(|reason| error(expr.span, reason))?;
                let may_complete = control::loop_may_complete(&body, identity, &mut 100_000)
                    .map_err(|reason| error(expr.span, reason))?;
                (
                    p(if may_complete { P::Void } else { P::Never }),
                    K::Loop { identity, body: Box::new(body) },
                )
            }
            H::BreakTo(target) | H::ContinueTo(target) => {
                let identity = local(*target);
                if !self.control.loops.contains(&identity) {
                    return Err(error(
                        expr.span,
                        "loop transfer outside a loop in this function or lambda",
                    ));
                }
                let kind = if matches!(expr.kind, H::BreakTo(_)) {
                    K::Break(identity)
                } else {
                    K::Continue(identity)
                };
                (p(P::Never), kind)
            }
            H::Break(label) | H::Continue(label) => {
                if label.name != hir::Symbol::invalid() {
                    return Err(error(expr.span, "named loop transfer is not implemented"));
                }
                let identity = *self.control
                    .loops
                    .last()
                    .ok_or_else(|| error(expr.span, "loop transfer outside a loop"))?;
                let kind = if matches!(expr.kind, H::Break(_)) {
                    K::Break(identity)
                } else {
                    K::Continue(identity)
                };
                (p(P::Never), kind)
            }
            H::Let(decl) => {
                let in_comptime_body=!self.in_code_body && self.captures.last().map(|frame|frame.comptime)
                    .unwrap_or_else(||self.in_required_comptime || self.in_staged_input
                        || self.signature_owner.and_then(|owner|self.package.item(owner)).is_some_and(|item|
                            matches!(&item.kind,ItemKind::Fn(sig,_) if sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head)));
                if decl.comptime && (!in_comptime_body
                    || decl.init.map(|value|self.code_producer(value)).transpose()?.unwrap_or(false)
                    || decl.ty.is_some_and(|ty|matches!(ty.kind,H::ContextualExprType(..)))) {
                    return self.code_declaration(decl, expected);
                }
                let annotation = decl.ty
                    .map(|ty| self.local_binding_annotation(expr.hir_id, ty))
                    .transpose()?;
                let init = decl.init.ok_or_else(|| {
                    error(
                        decl.span,
                        "let/var declaration requires an initializer",
                    )
                })?;
                let value = if let (Some(head), Some(ty)) = (decl.ty, annotation.as_ref()) {
                    let inputs = self.acquire_constructor_inputs(head, ty)?;
                    let init = self.expr(init, Some(ty))?;
                    self.finish_constructor_inputs(inputs, init, decl.span)?
                } else {
                    self.expr(init, None)?
                };
                // Preparation can exit before an operation with an already
                // formed result. Retain that domain for static pattern checks;
                // the initializer itself stays Never and binds no runtime value.
                let binding_type = annotation.unwrap_or_else(||
                    arguments::inference_type(&value).unwrap_or(&value.ty).clone());
                // The failure branch cannot observe bindings from this declaration.
                let otherwise = decl
                    .otherwise
                    .map(|body| self.check_failure_branch(body))
                    .transpose()?;
                if let Some(otherwise) = &otherwise {
                    if !is_never(&otherwise.ty) {
                        return Err(error(decl.span, "let else branch must diverge"));
                    }
                }
                // Binding mutability applies while checking computations inside
                // the pattern, including noncanonical or-pattern candidates.
                if !decl.mutable {
                    self.immutable.extend(pattern_binding_ids(&decl.pattern));
                }
                let observed = self.known_pattern_type_value(&value, decl.span)?;
                let pattern = self.pattern_observed(&decl.pattern, &binding_type, observed.as_ref())?;
                if !decl.mutable {
                    if in_comptime_body {
                        if decl.comptime {
                            self.record_known_staged_pattern(&pattern, &value, decl.span)?;
                        } else {
                            self.record_observed_initializer(&pattern, &value, decl.span)?;
                        }
                    }
                    self.record_staged_pattern(&pattern, &value, decl.span)?;
                }
                let guard = decl
                    .guard
                    .map(|guard| self.under_matched_pattern_type_facts(&pattern,&value,guard.span,|checker|
                        checker.expr(guard, Some(&p(P::Bool)))))
                    .transpose()?;
                if otherwise.is_none()
                    && !is_never(&value.ty)
                    && (guard.is_some() || !self.pattern_exhaustive(&pattern, &binding_type))
                {
                    return Err(error(
                        decl.span,
                        "refutable let pattern requires a diverging else branch",
                    ));
                }
                let ty = if is_never(&value.ty) || guard.as_ref().is_some_and(|g| is_never(&g.ty)) {
                    p(P::Never)
                } else {
                    p(P::Void)
                };
                let kind = match pattern {
                    n::Pattern::Binding { binding, ref inner }
                        if matches!(**inner, n::Pattern::Wild)
                            && guard.is_none()
                            && otherwise.is_none() =>
                    {
                        K::Let {
                            binding,
                            value: Box::new(value),
                        }
                    }
                    pattern => K::LetPattern {
                        pattern,
                        value: Box::new(value),
                        guard: guard.map(Box::new),
                        otherwise: otherwise.map(Box::new),
                    },
                };
                (ty, kind)
            }
            H::Defer(value) if self.in_required_comptime || self.in_staged_input
                || self.signature_owner.and_then(|owner|self.package.item(owner))
                    .is_some_and(|item|matches!(&item.kind,ItemKind::Fn(sig,_) if sig.modifiers.is_comptime)) => {
                // These contexts retain checked code in a comptime body or
                // input/default plan. That invocation owns when it runs;
                // checking the operand must not invoke it eagerly or weaken
                // ordinary checking of its operations and result type.
                return self.expr(value, expected);
            }
            H::Defer(_) => {
                return Err(error(expr.span, "deferred value computation requires instantiation support, which is not yet implemented"));
            }
            H::Assumes(condition) if self.is_type_assumption(condition) => {
                // Discharge pure PTC type/integer assumptions directly without
                // lowering them to runtime/VM program predicates.
                let predicate = self.type_predicate(condition)?;
                if !self.in_unselected_inline {
                    self.check_source_refinement(predicate.clone(), condition.span)?;
                }
                let fact = crate::ptc::Formula::from_predicate(&predicate);
                self.context
                    .validate_formula(&fact, &mut crate::ptc::Budget::default())
                    .map_err(|failure| error(condition.span, failure.to_string()))?;
                self.context.assume(fact);
                (p(P::Void), K::Unit)
            }
            H::Assumes(condition) => {
                let condition = self.boolean_predicate(condition)?;
                if !self.program_predicate_shape(&condition, false, expr.span)? {
                    if self.program_predicate_shape(&condition, true, expr.span)? {
                        self.require_stable_predicate(&condition, expr.span)?;
                        let at = source(expr.span);
                        let failure = Box::new(n::Expr {
                            source: at, ty: p(P::Void),
                            kind: K::CheckPredicate(Box::new(n::Expr {
                                source: at, ty: p(P::Bool),
                                kind: K::Literal(n::Literal::Bool(false)),
                            })),
                        });
                        // Evaluate the decision once with normal short circuiting.
                        // The true branch retains its observed facts; the failed
                        // check makes the false branch unreachable. Keeping this
                        // control edge avoids losing relations through a temporary
                        // bool whose value domain alone cannot describe them.
                        return self.finish_value_expression(expr, expected, p(P::Void), K::If {
                            condition: Box::new(condition),
                            then_branch: Box::new(n::Expr {
                                source: at, ty: p(P::Void), kind: K::Unit,
                            }),
                            else_branch: Some(failure),
                        });
                    }
                    return Err(error(
                        expr.span,
                        "assumes requires a supported effect-free program predicate; calls and type propositions need further contract elaboration",
                    ));
                }
                (p(P::Void), K::CheckPredicate(Box::new(condition)))
            }
            H::Return(value) => {
                if self.control.result.is_none() && self.control.return_observations.is_none() {
                    let crosses_required_boundary = self.in_required_comptime
                        && !self.in_code_body
                        && self.signature_owner.is_some_and(|owner| {
                            self.package.item(owner).is_some_and(|item|
                                matches!(item.kind, ItemKind::Fn(..) | ItemKind::FnDeclaration(..)))
                        });
                    return Err(error(expr.span, if crosses_required_boundary {
                        "return across a required comptime boundary is not supported"
                    } else {
                        "return outside a function"
                    }));
                }
                let expected_return = self.control.result.clone().or_else(|| self.control.return_observations.as_ref()
                    .and_then(|observations| observations.iter().find(|ty| !is_never(ty) && !matches!(ty, Term::ErrorResult { result, .. } if is_never(result))).cloned()));
                let partial = self.control.partial_return_context.clone();
                // Absence supplies no payload evidence. Keep the control exit
                // private until the callback's successful paths determine ?T,
                // just as for the missing branch of optional propagation.
                if expected_return.is_none()
                    && matches!(&partial, Some(crate::calls::InputContext::Optional(_)))
                    && value.is_some_and(|value| matches!(value.kind, H::Null))
                {
                    let return_source = source(expr.span);
                    self.control.pending_optional_returns.insert(
                        (return_source.start, return_source.end),
                        source(value.unwrap().span),
                    );
                    return Ok(n::Expr {
                        source: return_source,
                        ty: p(P::Never),
                        kind: K::Return(None),
                    });
                }
                let value = if let Some(value) = value {
                    Some(match (&expected_return, &partial) {
                        (None, Some(context)) => self.input_context_expression(value, context),
                        (expected, _) => self.expr(value, expected.as_ref()),
                    }?)
                } else {
                    self.implicit_error_success(expected_return.as_ref(), expr.span)?
                };
                let ty = value
                    .as_ref()
                    .map(|value| value.ty.clone())
                    .unwrap_or_else(|| p(P::Void));
                if let Some(expected) = &expected_return {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                if let Some(returned) = &value {
                    self.verify_returned_type_guarantees(returned, expr.span)?;
                }
                if let Some(observations) = &mut self.control.return_observations {
                    observations.push(ty);
                }
                if self.control.result.is_none() && matches!(self.control.return_observations, Some(ReturnObservations::Infer(_))) {
                    let site = source(expr.span);
                    self.control.inferred_return_sites.insert((site.start, site.end));
                }
                (p(P::Never), K::Return(value.map(Box::new)))
            }
            _ => unreachable!("value expression category was checked by the dispatcher"),
        };
        self.finish_value_expression(expr, expected, ty, kind)
    }
}
