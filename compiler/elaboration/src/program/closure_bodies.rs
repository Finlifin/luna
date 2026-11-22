//! Ordinary source checking and selected runtime bodies share a closure return scope.
use super::*;

#[derive(Clone, Default)]
pub(super) enum ClosureConstRegions {
    #[default]
    Unchecked,
    Describing(LocalBindings<()>),
    Checked(LocalBindings<()>),
}

impl ClosureConstRegions {
    pub(super) fn record(&mut self, site: hir::HirId) {
        if let Self::Describing(sites) = self {
            sites.insert(local(site), ());
        }
    }

    pub(super) fn contains(&self, site: hir::HirId) -> bool {
        matches!(self, Self::Checked(sites) if sites.contains_key(&local(site)))
    }
}

impl Checker<'_, '_> {
    pub(super) fn closure(
        &mut self,
        expr: &Expr<'_>,
        parameters: &[hir::expr::ClosureParam<'_>],
        annotation: Option<&Expr<'_>>,
        body_id: hir::BodyId,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let body = self
            .package
            .body(body_id)
            .ok_or_else(|| error(expr.span, "missing lambda body"))?;
        self.closure_body(
            expr, parameters, annotation, body.value, expected, partial, false,
        )
        .map(|(value, _)| value)
    }

    pub(super) fn closure_body(
        &mut self,
        expr: &Expr<'_>,
        parameters: &[hir::expr::ClosureParam<'_>],
        annotation: Option<&Expr<'_>>,
        body: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        tactic: bool,
    ) -> Result<(n::Expr, bool), Error> {
        let expected_shape = expected
            .map(|ty| self.structural_type(ty, expr.span))
            .transpose()?;
        let expected_signature = match expected_shape.as_ref() {
            Some(Term::Closure { signature, .. }) => Some(signature.as_ref()),
            other => other,
        };
        let comptime = !self.in_code_body
            && (self.in_required_comptime
                || self.in_staged_input
                || self
                    .signature_owner
                    .and_then(|owner| self.package.item(owner))
                    .is_some_and(|item| {
                        matches!(
                            &item.kind,
                            ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                                if sig.modifiers.is_comptime
                        )
                    }));
        let contextual = match expected_signature.map(Term::callable_signature) {
            Some(Term::Function { parameters, result }) => Some((parameters, result)),
            Some(_) => {
                return Err(incompatible_context(
                    expr.span,
                    "lambda target is not callable",
                ));
            }
            None => None,
        };
        if expected_signature.is_some_and(|ty| ty.is_comptime_callable() != comptime)
            || partial.is_some_and(|context| context.comptime != comptime)
        {
            return Err(error(
                expr.span,
                "lambda target requires a different execution phase",
            ));
        }
        if let Some((expected, _)) = contextual {
            if expected.len() != parameters.len() {
                return Err(error(
                    expr.span,
                    "lambda parameter count does not match target",
                ));
            }
        }
        if partial.is_some_and(|context| context.parameters.len() != parameters.len()) {
            return Err(error(
                expr.span,
                "lambda parameter count does not match target",
            ));
        }
        let saved_locals = self.locals.clone();
        let saved_control = std::mem::take(&mut self.control);
        let saved_tactic = self.tactic_frame.take();
        // A callback infers its own result from both its tail and explicit
        // returns, even when the callback value is being probed by a parent.
        let saved_branch_probe = std::mem::replace(&mut self.probing_optional_branch, false);
        let saved_unsafe = std::mem::replace(&mut self.unsafe_depth, 0);
        if tactic {
            self.tactic_frame = Some((
                source(body.span),
                parameters.iter().map(|p| p.hir_id).collect(),
                false,
            ));
        }
        let mut outer = self.locals.keys().copied().collect::<BTreeSet<_>>();
        if comptime {
            // Signature inputs can be read before a runtime locals frame exists.
            // Snapshot their binding identities for capture routing; the normal
            // expression path still checks their domain, phase and availability.
            outer.extend(
                self.input_facts
                    .parameters
                    .identities()
                    .map(|(binding, _)| *binding),
            );
            outer.extend(self.input_facts.types.keys().copied());
            outer.extend(self.input_facts.values.keys().copied());
        }
        self.captures.push(CaptureFrame {
            const_regions: ClosureConstRegions::default(),
            site: local(expr.hir_id),
            addresses: BTreeMap::new(),
            comptime,
            outer,
            values: Vec::new(),
        });
        let checked = (|| {
            let mut parameter_types = Vec::new();
            for (index, parameter) in parameters.iter().enumerate() {
                if !matches!(parameter.pat.kind, hir::PatternKind::Binding(_, _, None)) {
                    return Err(error(
                        parameter.span,
                        "lambda parameters must be anchored identifiers",
                    ));
                }
                let context_type = contextual
                    .map(|(parameters, _)| parameters[index].ty.substitute_partial(&[]))
                    .transpose()
                    .map_err(|e| error(parameter.span, e.to_string()))?
                    .or_else(|| partial.and_then(|context| context.parameters[index].clone()))
                    .map(|ty| self.callable_input_view(&ty, parameter.span))
                    .transpose()?;
                let ty = if let Some(annotation) = parameter.ty {
                    let ty = self.type_expr(annotation)?;
                    if let Some(context_type) = context_type.as_ref() {
                        self.require(&ty, context_type, parameter.span)?;
                    }
                    ty
                } else {
                    context_type.ok_or_else(|| {
                        context_error(
                            parameter.span,
                            "lambda parameter needs an annotation or a callable target",
                        )
                    })?
                };
                self.locals.insert(local(parameter.hir_id), ty.clone());
                parameter_types.push(Parameter {
                    sort: Sort::Runtime,
                    ty: ty
                        .abstract_parameters(&[])
                        .map_err(|e| error(parameter.span, e.to_string()))?,
                });
            }
            let contextual_result = contextual
                .map(|(_, result)| result.substitute_partial(&[]))
                .transpose()
                .map_err(|e| error(expr.span, e.to_string()))?
                .or_else(|| partial.and_then(|context| context.result.clone()))
                .map(|ty| self.callable_input_view(&ty, expr.span))
                .transpose()?;
            let result_type = if let Some(annotation) = annotation {
                let ty = self.type_expr(annotation)?;
                if let Some(expected) = contextual_result.as_ref() {
                    self.require(&ty, expected, expr.span)?;
                }
                Some(ty)
            } else {
                contextual_result
            };
            self.control.result = result_type.clone();
            self.control.return_observations = Some(ReturnObservations::Infer(Vec::new()));
            let result_context = partial.and_then(|context| context.result_context.as_deref());
            let (body, result) =
                self.stage_closure_body(body, result_type, result_context, expr.span)?;
            // Definition-only selectors and computed values are formal inputs
            // of the enclosing behavior function. A lambda must acquire the
            // ones it reads, including those forwarded to a nested lambda.
            if !comptime
                && !self.in_code_body
                && self.checking_closure_source()
                && !self.ordinary_behavior_inputs.is_empty()
            {
                let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let free = nir::free_bindings::collect(&body, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                let frame = self.captures.last_mut().unwrap();
                for binding in free.map_err(|message| error(expr.span, message))? {
                    if let Some(ty) = self.ordinary_behavior_inputs.get(&binding)
                        && !frame.values.iter().any(|entry| entry.binding == binding)
                    {
                        frame.values.push(n::Capture {
                            binding,
                            ty: ty.clone(),
                            value: None,
                        });
                    }
                }
            }
            let captures = self.captures.last().unwrap().values.clone();
            let signature = Term::Function {
                parameters: parameter_types,
                result: Box::new(
                    result
                        .abstract_parameters(&[])
                        .map_err(|e| error(expr.span, e.to_string()))?,
                ),
            }
            .with_comptime_phase(comptime);
            let ty = if captures.is_empty() {
                signature
            } else {
                Term::Closure {
                    environment: captures.iter().map(|capture| capture.ty.clone()).collect(),
                    signature: Box::new(signature),
                }
            };
            if let Some(expected) = expected {
                self.require(&ty, expected, expr.span)?;
            }
            Ok(n::Expr {
                ty,
                source: source(expr.span),
                kind: K::Closure {
                    parameters: parameters.iter().map(|p| local(p.hir_id)).collect(),
                    captures,
                    body: std::sync::Arc::new(body),
                },
            })
        })();
        self.captures.pop();
        self.locals = saved_locals;
        self.control = saved_control;
        let used = self.tactic_frame.as_ref().is_some_and(|(_, _, used)| *used);
        self.tactic_frame = saved_tactic;
        self.probing_optional_branch = saved_branch_probe;
        self.unsafe_depth = saved_unsafe;
        checked.map(|value| (value, used)).map_err(|mut failure| {
            if !tactic {
                failure.needs_tactic = false;
            }
            failure
        })
    }

    fn check_closure_result(
        &mut self,
        body: &Expr<'_>,
        result_type: Option<Term>,
        result_context: Option<&crate::calls::InputContext>,
        span: Span,
    ) -> Result<(n::Expr, Term), Error> {
        let mut body = if result_type.is_none() {
            let mut return_hint = None;
            let attempted = self.checking_attempt(
                body.span,
                self.numeric_defaults,
                CheckingDisposition::Commit,
                |checker| {
                    let checked = checker.computation_body_with_input(body, None, result_context);
                    if matches!(&checked, Err(failure) if failure.needs_context) {
                        return_hint = checker
                            .control
                            .return_observations
                            .as_ref()
                            .and_then(|returns| returns.iter().find(|ty| !is_never(ty)).cloned());
                    }
                    checked
                },
            )?;
            match attempted {
                Some(body) => body,
                None => {
                    let hint = return_hint.ok_or_else(|| {
                        context_error(
                            body.span,
                            "callback result needs a tail or explicit return type",
                        )
                    })?;
                    // Retry once with concrete return evidence. The failed
                    // attempt has rolled back captures, pending returns and
                    // local state; only the completed type crosses attempts.
                    self.computation_body_with_input(body, Some(&hint), result_context)?
                }
            }
        } else {
            self.computation_body_with_input(body, result_type.as_ref(), result_context)?
        };
        let returns = self.control.return_observations.as_ref().unwrap().clone();
        let mut result = result_type.unwrap_or_else(|| {
            if !is_never(&body.ty) {
                body.ty.clone()
            } else {
                returns
                    .iter()
                    .find(|ty| {
                        !is_never(ty)
                            && !matches!(ty, Term::ErrorResult { result, .. } if is_never(result))
                    })
                    .cloned()
                    .or_else(|| returns.iter().find(|ty| !is_never(ty)).cloned())
                    .unwrap_or_else(|| p(P::Never))
            }
        });
        let value_context = match result_context {
            Some(crate::calls::InputContext::Computation { result, .. }) => Some(result.as_ref()),
            other => other,
        };
        let allowed_errors = match value_context {
            Some(crate::calls::InputContext::ErrorResult { errors, .. }) => Some(errors.as_slice()),
            _ => None,
        };
        let completed_error_returns =
            !self.control.pending_error_returns.is_empty() || allowed_errors.is_some();
        if completed_error_returns {
            result = self.complete_error_returns(&mut body, result, allowed_errors, span)?;
        }
        if !self.control.pending_optional_returns.is_empty() {
            let value_result = if let Term::Computation { result, .. } = &result {
                result.as_ref()
            } else {
                &result
            };
            self.optional_payload(value_result, span)?;
            let mut remaining = self
                .control
                .pending_optional_returns
                .keys()
                .copied()
                .collect::<BTreeSet<_>>();
            nir::visit::expression_mut(&mut body, &mut 100_000, |node, _| {
                if matches!(node.kind, K::Return(None)) {
                    if let Some(empty_source) = self
                        .control
                        .pending_optional_returns
                        .get(&(node.source.start, node.source.end))
                    {
                        remaining.remove(&(node.source.start, node.source.end));
                        node.kind = K::Return(Some(Box::new(n::Expr {
                            source: *empty_source,
                            ty: value_result.clone(),
                            kind: K::OptionalAbsent,
                        })));
                    }
                }
                Ok(())
            })
            .map_err(|message| error(span, message))?;
            if !remaining.is_empty() {
                return Err(error(span, "unresolved optional return site"));
            }
        }
        self.require(&body.ty, &result, span)?;
        for actual in returns.iter() {
            let value_result = if let Term::Computation { result, .. } = &result {
                result.as_ref()
            } else {
                &result
            };
            if !completed_error_returns {
                self.require(actual, value_result, span)?;
            }
        }
        Ok((body, result))
    }

    fn checking_closure_source(&self) -> bool {
        self.checking_open_behavior
            || (self.open_instance.is_none()
                && self
                    .signature_owner
                    .is_some_and(|owner| self.active_open_definitions.contains(&owner)))
    }

    fn ordinary_closure_result(
        &mut self,
        body: &Expr<'_>,
        result: Option<Term>,
        context: Option<&crate::calls::InputContext>,
        span: Span,
    ) -> Result<(n::Expr, Term), Error> {
        let behavior = std::mem::replace(&mut self.checking_open_behavior, true);
        let spread = std::mem::replace(&mut self.describing_open_spread, true);
        let unselected = std::mem::replace(&mut self.in_unselected_inline, true);
        let selection = self.inline_selection.replace((
            hir::HirId::new(body.hir_id.owner, hir::ItemLocalId::new(u32::MAX)),
            usize::MAX,
        ));
        let regions = std::mem::take(&mut self.definition_refinements.regions);
        let contains_const =
            std::mem::take(&mut self.definition_refinements.block_contains_conditional_const);
        let transfers = std::mem::take(&mut self.open_loop_transfers);
        let checked = self.check_closure_result(body, result, context, span);
        self.definition_refinements.block_contains_conditional_const = contains_const;
        self.definition_refinements.regions = regions;
        self.open_loop_transfers = transfers;
        self.inline_selection = selection;
        self.in_unselected_inline = unselected;
        self.describing_open_spread = spread;
        self.checking_open_behavior = behavior;
        checked
    }

    fn stage_closure_body(
        &mut self,
        body: &Expr<'_>,
        result: Option<Term>,
        context: Option<&crate::calls::InputContext>,
        span: Span,
    ) -> Result<(n::Expr, Term), Error> {
        let owner = self.signature_owner;
        let ordinary = self.checking_closure_source();
        let Some((owner, sig)) = owner.and_then(|owner| match &self.package.item(owner)?.kind {
            ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => Some((owner, sig)),
            _ => None,
        }) else {
            return self.check_closure_result(body, result, context, span);
        };
        if self.in_code_body
            || self.captures.last().unwrap().comptime
            || !self.closure_needs_staging(body, ordinary)?
        {
            return self.check_closure_result(body, result, context, span);
        }
        if ordinary {
            return self.ordinary_closure_result(body, result, context, span);
        }
        // Purity is a declaration promise. The enclosing behavior check sees
        // the whole lambda and follows its calls before any branch is erased.
        if self.definition_refinements.closed_source_body && sig.modifiers.is_pure {
            self.check_closed_const_regions(owner)?;
        }
        let probe = checking_attempts::Attempt::begin(
            self,
            span,
            self.numeric_defaults,
            CheckingDisposition::Probe,
        )?;
        let tactic = self.tactic_frame.clone();
        self.captures.last_mut().unwrap().const_regions =
            ClosureConstRegions::Describing(Default::default());
        let checked = self
            .ordinary_closure_result(body, result, context, span)
            .and_then(|(witness, ty)| {
                self.check_open_fragment(&witness, span, false)?;
                let frame = self.captures.last_mut().unwrap();
                let ClosureConstRegions::Describing(regions) =
                    std::mem::take(&mut frame.const_regions)
                else {
                    unreachable!()
                };
                Ok((regions, ty))
            });
        self.tactic_frame = tactic;
        let (regions, ty) = probe.finish_checked(self, checked)?;
        self.captures.last_mut().unwrap().const_regions = ClosureConstRegions::Checked(regions);
        self.check_closure_result(body, Some(ty), context, span)
    }
}
