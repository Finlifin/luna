//! Bind definition-supplied expression code before checking a shared function body.
use super::*;

impl Checker<'_, '_> {
    /// Model abstract code results as runtime inputs only in analysis witnesses.
    /// Concrete instances still supply and validate the actual code behavior.
    pub(in crate::program) fn abstract_code_inputs(
        &mut self,
        body: &n::Expr,
        span: Span,
    ) -> Result<BTreeMap<n::LocalId, Term>, Error> {
        let mut inputs = BTreeMap::new();
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let observed = nir::visit::expression(body, &mut fuel, |node, fuel| {
            if let K::Local(binding) = node.kind {
                if self.abstract_code_dependencies.contains_key(&binding) {
                    nir::type_computations::charge_term(&node.ty, fuel)
                        .map_err(|failure| failure.to_string())?;
                    inputs.insert(binding, node.ty.clone());
                }
            }
            Ok(())
        });
        self.required_steps -= (before - fuel) as u64;
        observed.map_err(|message| error(span, message))?;
        Ok(inputs)
    }

    pub(in crate::program) fn function_body(
        &mut self,
        owner: OwnerId,
        body: &Expr<'_>,
        result: &Term,
    ) -> Result<n::Expr, Error> {
        self.with_function_template_body(owner, result, |checker| {
            checker.computation_body(body, Some(result))
        })
    }

    pub(in crate::program) fn with_function_template_body<R>(
        &mut self,
        owner: OwnerId,
        result: &Term,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let Some(template) = self.package.function_template_bodies.get(&owner).cloned() else {
            return check(self);
        };
        let saved_locals = self.locals.clone();
        let saved_code = self.code_values.clone();
        let saved_any_result = std::mem::take(&mut self.any_result_code_locals);
        let saved_checking = self.checking_code_generator;
        // The supplied runtime body belongs to this generated function. Check
        // its requests in that function's declared row, not the environment of
        // whichever body was elaborated before code construction started.
        let effects = match result {
            Term::Computation { effects, .. } => Some(effects.clone()),
            _ => None,
        };
        let saved_effects = std::mem::replace(&mut self.effect_environment, effects);
        let checked = (|| {
            let formal = self.code_interface(template.interface.ty)?;
            let code = if let Some(supplied) = template.supplied {
                let mut actual = formal.clone();
                actual.context.clear();
                actual.ports.clear();
                actual.defaults.clear();
                for port in &template.ports {
                    let from = local(
                        port.formal
                            .ok_or_else(|| error(port.name.span, "unlinked template port"))?,
                    );
                    let to = local(port.binding);
                    let ty = formal
                        .context
                        .get(&from)
                        .ok_or_else(|| error(port.name.span, "unknown formal template port"))?
                        .clone();
                    actual.context.insert(to, ty.clone());
                    actual.ports.insert(port.name.name.to_string(), to);
                    self.locals.insert(to, ty);
                }
                let supplied = self.package.body(supplied).ok_or_else(|| {
                    error(template.interface.ty.span, "missing supplied template body")
                })?;
                self.checking_code_generator = false;
                // Return targets belong to the resulting function; ordinary
                // quote construction continues to have no outer return target.
                let return_type = match result {
                    Term::Computation { result, .. } => result.as_ref().clone(),
                    result => result.clone(),
                };
                let code = self.quote_expression(
                    supplied.value,
                    supplied.value,
                    &[],
                    Some(&actual),
                    None,
                    Some(return_type),
                )?;
                self.ascribe_code(code, Some(&formal), supplied.value.span)?
            } else {
                Arc::new(ExpressionCode {
                    body: None,
                    result: formal.result,
                    context: formal.context,
                    ports: formal.ports,
                    defaults: formal.defaults,
                })
            };
            // Contextual ports are virtual while constructing the code. Setup
            // must establish real locals and satisfy each splice connection.
            self.locals = saved_locals.clone();
            self.code_values
                .insert(local(template.interface.parameter), code);
            self.checking_code_generator = template.supplied.is_none();
            check(self)
        })();
        self.locals = saved_locals;
        self.code_values = saved_code;
        self.any_result_code_locals = saved_any_result;
        self.checking_code_generator = saved_checking;
        self.effect_environment = saved_effects;
        checked
    }

    pub(in crate::program) fn check_all_quote_fn_arms(&mut self) -> Result<(), Error> {
        for (&instance_owner, &template_owner) in &self.package.function_template_origins {
            let is_arm = self.package.quote_fn_arms.contains_key(&instance_owner);
            let for_arms = self
                .quote_fn_template_flags
                .get(&template_owner)
                .is_some_and(|flags| flags.for_match_arms);
            let span = self
                .package
                .item(instance_owner)
                .map(|item| item.span)
                .unwrap_or_default();
            if is_arm && !for_arms {
                return Err(error(
                    span,
                    "case arm instance requires a template quote fn marked with attrs.quote_fn_template.for_match_arms",
                ));
            }
            if !is_arm && for_arms {
                return Err(error(
                    span,
                    "template quote fn marked with attrs.quote_fn_template.for_match_arms only accepts case arm instances",
                ));
            }
        }
        let arms: Vec<(OwnerId, hir::item::QuoteFnArm<'_>)> = self
            .package
            .quote_fn_arms
            .iter()
            .map(|(owner, arm)| (*owner, arm.clone()))
            .collect();
        for (arm_owner, arm) in arms {
            let checked = self.check_quote_fn_arm(arm_owner, &arm)?;
            self.quote_fn_arms_by_template
                .entry(arm.template_owner)
                .or_default()
                .push(arm_owner);
            self.checked_quote_fn_arms.insert(arm_owner, checked);
        }
        let grouped: Vec<(OwnerId, Vec<OwnerId>)> = self
            .quote_fn_arms_by_template
            .iter()
            .map(|(template, owners)| (*template, owners.clone()))
            .collect();
        for (_template_owner, owners) in grouped {
            for i in 0..owners.len() {
                for j in (i + 1)..owners.len() {
                    let a = &self.checked_quote_fn_arms[&owners[i]];
                    let b = &self.checked_quote_fn_arms[&owners[j]];
                    let same_specialization = match (&a.applied_args, &b.applied_args) {
                        (Some(lhs), Some(rhs)) => lhs == rhs,
                        _ => true,
                    };
                    if same_specialization && patterns_overlap(&a.pattern, &b.pattern) {
                        return Err(error(
                            b.span,
                            "conflicting or overlapping quote fn arm patterns on match-arm template",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn check_quote_fn_arm(
        &mut self,
        arm_owner: OwnerId,
        arm: &hir::item::QuoteFnArm<'_>,
    ) -> Result<CheckedQuoteFnArm, Error> {
        let flags = self
            .quote_fn_template_flags
            .get(&arm.template_owner)
            .copied()
            .ok_or_else(|| {
                error(
                    arm.span,
                    "case arm instance requires a template quote fn marked with attrs.quote_fn_template.for_match_arms",
                )
            })?;
        if !flags.for_match_arms {
            return Err(error(
                arm.span,
                "case arm instance requires a template quote fn marked with attrs.quote_fn_template.for_match_arms",
            ));
        }
        let item = self
            .package
            .item(arm_owner)
            .ok_or_else(|| error(arm.span, "missing quote fn arm declaration"))?;
        let ItemKind::FnDeclaration(sig) = &item.kind else {
            return Err(error(arm.span, "quote fn arm must be a declaration item"));
        };
        let declared = self
            .declared_signature(arm_owner)
            .ok_or_else(|| error(arm.span, "missing quote fn arm signature"))?
            .into_owned();
        let Term::Function { parameters, result } = &declared else {
            return Err(error(arm.span, "quote fn arm signature must be callable"));
        };
        if parameters.is_empty() {
            return Err(error(
                arm.span,
                "quote fn arm requires a scrutinee parameter",
            ));
        }
        let scrutinee_ty = parameters[0].ty.clone();
        let result_ty = result.as_ref().clone();
        let contextual_parameters: Vec<_> = sig
            .params
            .iter()
            .zip(parameters.iter())
            .skip(1)
            .map(|(param, param_ty)| (param.name.name, local(param.hir_id), param_ty.ty.clone()))
            .collect();
        let type_parameters = self
            .type_parameters
            .get(&arm_owner)
            .cloned()
            .unwrap_or_default();
        let applied_args =
            if let Some(app) = self.package.function_template_applications.get(&arm_owner) {
                if let H::NFApplication(_, args) = app.template.kind {
                    let mut evaluated = Vec::with_capacity(args.len());
                    for arg in args {
                        let hir::common::Arg::Positional(expr) = arg else {
                            return Err(error(
                                arm.span,
                                "match-arm template application requires positional type arguments",
                            ));
                        };
                        evaluated.push(self.type_expr(expr)?);
                    }
                    Some(evaluated)
                } else {
                    None
                }
            } else {
                None
            };
        let arm_body = self
            .package
            .body(arm.body)
            .ok_or_else(|| error(arm.span, "missing quote fn arm body"))?;
        let (pattern, body) = self.in_declaration_scope(arm_owner, |this| {
            this.context = this.definition_context(arm_owner);
            this.unsafe_depth = usize::from(sig.modifiers.is_unsafe);
            for (index, (parameter, ty)) in sig.params.iter().zip(parameters.iter()).enumerate() {
                let binding = local(parameter.hir_id);
                this.locals.insert(binding, ty.ty.clone());
                this.input_facts
                    .parameters
                    .bind(binding, index, ty.sort, &ty.ty);
            }
            this.control.result = Some(result_ty.clone());
            this.in_code_body = true;
            this.in_required_comptime = false;
            this.in_staged_input = false;
            let (inner_pat, guard) = match &arm.pattern.kind {
                hir::pattern::PatternKind::Guard(pat, guard) => (*pat, Some(*guard)),
                _ => (arm.pattern, arm.guard),
            };
            let hir_arm = hir::pattern::PatternArm {
                hir_id: arm.pattern.hir_id,
                pat: inner_pat.clone(),
                guard,
                body: arm_body.value,
                span: arm.span,
            };
            let synthetic_scrutinee = n::Expr {
                source: source(arm.span),
                ty: scrutinee_ty.clone(),
                kind: K::Unit,
            };
            let checked_pattern = this.match_arm_pattern(&hir_arm, &synthetic_scrutinee)?;
            let checked_body = this.under_matched_pattern_type_facts(
                &checked_pattern,
                &synthetic_scrutinee,
                arm.span,
                |this| this.computation_body(arm_body.value, Some(&result_ty)),
            )?;
            if !flags.allow_non_exhaustive_instance
                && this.patterns_exhaustive(&[&checked_pattern], &scrutinee_ty)
                    != crate::coverage::Coverage::Exhaustive
            {
                return Err(error(
                    arm.span,
                    "non-exhaustive quote fn arm; add a fallback pattern or mark the template with attrs.quote_fn_template.allow_non_exhaustive_instance",
                ));
            }
            Ok((checked_pattern, checked_body))
        })?;
        Ok(CheckedQuoteFnArm {
            owner: arm_owner,
            template_owner: arm.template_owner,
            type_parameters,
            applied_args,
            scrutinee_ty,
            contextual_parameters,
            result_ty,
            pattern,
            body,
            span: arm.span,
        })
    }

    pub(in crate::program) fn expand_quote_fn_arms_import(
        &mut self,
        import_expr: &Expr<'_>,
        scrutinee: &n::Expr,
        expected_result: Option<&Term>,
        lower_returns: bool,
    ) -> Result<Vec<(n::Pattern, n::Expr)>, Error> {
        let (head, explicit_args) = match &import_expr.kind {
            H::NFApplication(head, args) => (*head, Some(*args)),
            _ => (import_expr, None),
        };
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return Err(error(
                import_expr.span,
                "match arm import requires a template quote fn marked with attrs.quote_fn_template.for_match_arms",
            ));
        };
        let template_owner = self.package.definition_owner(def_id).ok_or_else(|| {
            error(
                import_expr.span,
                "match arm import target declaration is missing",
            )
        })?;
        if !self
            .quote_fn_template_flags
            .get(&template_owner)
            .is_some_and(|flags| flags.for_match_arms)
        {
            return Err(error(
                import_expr.span,
                "match arm import requires a template quote fn marked with attrs.quote_fn_template.for_match_arms",
            ));
        }
        if !self.runtime_value_context() {
            return Err(error(
                import_expr.span,
                "quote fn match arm import requires a runtime phase",
            ));
        }
        let template_type_params = self
            .type_parameters
            .get(&template_owner)
            .cloned()
            .unwrap_or_default();
        let type_args = if template_type_params.is_empty() {
            if explicit_args.is_some() {
                return Err(error(
                    import_expr.span,
                    "match-arm quote template takes no type arguments",
                ));
            }
            Vec::new()
        } else if let Some(args) = explicit_args {
            if args.len() != template_type_params.len() {
                return Err(error(
                    import_expr.span,
                    "match-arm quote template type argument count mismatch",
                ));
            }
            let mut evaluated = Vec::with_capacity(args.len());
            for arg in args {
                let hir::common::Arg::Positional(expr) = arg else {
                    return Err(error(
                        import_expr.span,
                        "match-arm quote template requires positional type arguments",
                    ));
                };
                evaluated.push(self.type_expr(expr)?);
            }
            self.check_type_application(template_owner, &evaluated, import_expr.span)?;
            evaluated
        } else {
            let declared = self
                .declared_signature(template_owner)
                .ok_or_else(|| {
                    error(
                        import_expr.span,
                        "match-arm quote template signature missing",
                    )
                })?
                .into_owned();
            let Term::Function {
                parameters: ordinary,
                result,
            } = declared
            else {
                return Err(error(
                    import_expr.span,
                    "match-arm quote template signature is not callable",
                ));
            };
            let binders = self.input_binders(&template_type_params);
            let domains: Vec<Term> = template_type_params
                .iter()
                .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
                .collect();
            let caller_scope = self.call_inference_domains();
            let scheme = crate::calls::Scheme {
                parameter_sorts: binders.iter().map(|p| p.sort).collect(),
                type_parameters: template_type_params.len(),
                signature: Term::Function {
                    parameters: ordinary.clone(),
                    result,
                }
                .abstract_parameters(&binders)
                .map_err(|e| error(import_expr.span, e.to_string()))?,
            };
            let mut actual = vec![None; ordinary.len()];
            if !actual.is_empty() {
                actual[0] = Some(scrutinee.ty.clone());
            }
            if let Some(ItemKind::FnDeclaration(sig)) =
                self.package.item(template_owner).map(|item| &item.kind)
            {
                for (index, param) in sig.params.iter().enumerate().skip(1) {
                    if let Some(&target) = self
                        .package
                        .lexical_call_locals
                        .get(&import_expr.hir_id)
                        .and_then(|map| map.get(param.name.name.as_str()))
                        && let Ok(ty) = self.validate_contextual_quote_binding(
                            local(target),
                            param.name.name.as_str(),
                            import_expr.span,
                            "match",
                        )
                    {
                        actual[index] = Some(ty);
                    }
                }
            }
            let instance = scheme
                .infer_declared_sparse_types_mode(
                    &template_type_params,
                    &domains,
                    &actual,
                    expected_result,
                    None,
                    &self.context,
                    crate::ptc::Budget::default(),
                    false,
                    &caller_scope,
                )
                .map_err(|e| {
                    error(
                        import_expr.span,
                        format!("cannot infer match-arm template type arguments: {e:?}"),
                    )
                })?
                .finish()
                .map_err(|e| {
                    error(
                        import_expr.span,
                        format!("cannot finish match-arm template type arguments: {e:?}"),
                    )
                })?;
            self.check_type_application(
                template_owner,
                &instance.type_arguments,
                import_expr.span,
            )?;
            instance.type_arguments
        };
        let arm_owners = self
            .quote_fn_arms_by_template
            .get(&template_owner)
            .cloned()
            .unwrap_or_default();
        let mut expanded = Vec::with_capacity(arm_owners.len());
        for arm_owner in arm_owners {
            let arm = self.checked_quote_fn_arms[&arm_owner].clone();
            if let Some(applied) = &arm.applied_args
                && *applied != type_args
            {
                continue;
            }
            let mut wrapped = n::Expr {
                source: source(import_expr.span),
                ty: arm.result_ty.clone(),
                kind: K::Match {
                    value: Box::new(n::Expr {
                        source: source(import_expr.span),
                        ty: arm.scrutinee_ty.clone(),
                        kind: K::Unit,
                    }),
                    arms: vec![n::MatchArm {
                        pattern: arm.pattern,
                        guard: None,
                        body: arm.body,
                    }],
                },
            };
            let mut scrutinee_ty = arm.scrutinee_ty;
            let mut result_ty = arm.result_ty;
            let mut contextual_parameters = arm.contextual_parameters;
            if !arm.type_parameters.is_empty() {
                let domains: Vec<Term> = arm
                    .type_parameters
                    .iter()
                    .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
                    .collect();
                let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let empty_frames = [vec![]];
                let substitution = nir::DeclarationSubstitution::bounded(
                    &arm.type_parameters,
                    &domains,
                    &type_args,
                    &empty_frames,
                    &mut fuel,
                )
                .map_err(|e| error(import_expr.span, e.to_string()))?;
                wrapped = substitution
                    .instantiate_expression_bounded(&wrapped, &mut fuel)
                    .map_err(|e| error(import_expr.span, e.to_string()))?;
                scrutinee_ty = substitution
                    .instantiate_bounded(&scrutinee_ty, &mut fuel)
                    .map_err(|e| error(import_expr.span, e.to_string()))?;
                result_ty = substitution
                    .instantiate_bounded(&result_ty, &mut fuel)
                    .map_err(|e| error(import_expr.span, e.to_string()))?;
                for (_, _, domain) in &mut contextual_parameters {
                    *domain = substitution
                        .instantiate_bounded(domain, &mut fuel)
                        .map_err(|e| error(import_expr.span, e.to_string()))?;
                }
                self.required_steps -= (before - fuel) as u64;
            }
            self.require(&scrutinee.ty, &scrutinee_ty, import_expr.span)?;
            let mut connections = BTreeMap::new();
            for (name, arm_binding, domain) in contextual_parameters {
                let Some(target_hir_id) = self
                    .package
                    .lexical_call_locals
                    .get(&import_expr.hir_id)
                    .and_then(|locals| locals.get(name.as_str()).copied())
                else {
                    return Err(error(
                        import_expr.span,
                        format!("missing contextual quote input `{name}` in match lexical scope"),
                    ));
                };
                let caller_binding = local(target_hir_id);
                let caller_ty = self.validate_contextual_quote_binding(
                    caller_binding,
                    name.as_str(),
                    import_expr.span,
                    "match",
                )?;
                self.require_expression(&caller_ty, &domain, import_expr.span)?;
                connections.insert(arm_binding, caller_binding);
            }
            nir::freshening::expression(
                &mut wrapped,
                connections,
                &mut self.next_temporary_binding,
                &mut self.expansion_sources,
                source(import_expr.span),
            )
            .map_err(|message| error(import_expr.span, message))?;
            let K::Match { mut arms, .. } = wrapped.kind else {
                unreachable!()
            };
            let arm_entry = arms.pop().expect("wrapped arm");
            let mut body = arm_entry.body;
            let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            nir::inline_return::simplify_tail_returns(&mut body, &result_ty, &mut fuel)
                .map_err(|message| error(import_expr.span, message))?;
            let needs_boundary = nir::inline_return::has_lexical_return(&body, &mut fuel)
                .map_err(|message| error(import_expr.span, message))?;
            self.required_steps -= (before - fuel) as u64;
            if needs_boundary {
                if !lower_returns {
                    // Speculative branch probes only inspect the arm's delivered
                    // type and must not publish return packets into Checker state.
                    body.ty = result_ty;
                } else {
                    let slot = self.spread_binding(import_expr.span)?;
                    let exit = self.spread_binding(import_expr.span)?;
                    let result = self.spread_binding(import_expr.span)?;
                    let unreachable = self.spread_binding(import_expr.span)?;
                    let packet = DeclarationId {
                        package: u64::MAX - 4,
                        local: slot.index,
                    };
                    if self.enums.contains_key(&packet) {
                        return Err(error(
                            import_expr.span,
                            "quote fn arm result packet identity collision",
                        ));
                    }
                    let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let mut fuel = before;
                    let (lowered_body, descriptor, empty_replacements) = nir::inline_return::lower(
                        body,
                        result_ty,
                        nir::inline_return::Boundary {
                            packet,
                            slot,
                            exit,
                            result,
                            unreachable,
                        },
                        &mut self.expansion_sources,
                        source(import_expr.span),
                        &mut fuel,
                    )
                    .map_err(|message| error(import_expr.span, message))?;
                    self.required_steps -= (before - fuel) as u64;
                    for empty in empty_replacements {
                        self.internal_return_empty_replacements
                            .insert((empty.start, empty.end), packet);
                    }
                    self.computed_scalar_packets.insert(packet);
                    self.enums.insert(packet, descriptor);
                    body = lowered_body;
                }
            }
            expanded.push((arm_entry.pattern, body));
        }
        Ok(expanded)
    }
}

fn peel_pattern(mut pat: &n::Pattern) -> &n::Pattern {
    loop {
        match pat {
            n::Pattern::Binding { inner, .. }
            | n::Pattern::Observe { inner, .. }
            | n::Pattern::Remap { pattern: inner, .. } => pat = inner,
            _ => return pat,
        }
    }
}

fn patterns_overlap(lhs: &n::Pattern, rhs: &n::Pattern) -> bool {
    let lhs = peel_pattern(lhs);
    let rhs = peel_pattern(rhs);
    match (lhs, rhs) {
        (n::Pattern::Guard { .. }, n::Pattern::Guard { .. }) => false,
        (n::Pattern::Guard { pattern, .. }, other) | (other, n::Pattern::Guard { pattern, .. }) => {
            patterns_overlap(pattern, other)
        }
        (n::Pattern::Or(ps), other) | (other, n::Pattern::Or(ps)) => {
            ps.iter().any(|p| patterns_overlap(p, other))
        }
        (n::Pattern::Wild, _) | (_, n::Pattern::Wild) => true,
        (n::Pattern::Unit, n::Pattern::Unit)
        | (n::Pattern::OptionalAbsent, n::Pattern::OptionalAbsent) => true,
        (n::Pattern::OptionalPresent(a), n::Pattern::OptionalPresent(b)) => patterns_overlap(a, b),
        (
            n::Pattern::ErrorResult {
                label: l1,
                inner: i1,
            },
            n::Pattern::ErrorResult {
                label: l2,
                inner: i2,
            },
        ) => l1 == l2 && patterns_overlap(i1, i2),
        (n::Pattern::Literal(a), n::Pattern::Literal(b)) => a == b,
        (
            n::Pattern::Variant {
                declaration: d1,
                index: i1,
                fields: f1,
            },
            n::Pattern::Variant {
                declaration: d2,
                index: i2,
                fields: f2,
            },
        ) => {
            d1 == d2
                && i1 == i2
                && f1.len() == f2.len()
                && f1.iter().zip(f2).all(|(a, b)| patterns_overlap(a, b))
        }
        (n::Pattern::Tuple(f1), n::Pattern::Tuple(f2))
        | (n::Pattern::TypeTuple(f1), n::Pattern::TypeTuple(f2)) => {
            f1.len() == f2.len() && f1.iter().zip(f2).all(|(a, b)| patterns_overlap(a, b))
        }
        (
            n::Pattern::Record {
                declaration: d1,
                fields: f1,
            },
            n::Pattern::Record {
                declaration: d2,
                fields: f2,
            },
        )
        | (
            n::Pattern::NominalType {
                declaration: d1,
                fields: f1,
            },
            n::Pattern::NominalType {
                declaration: d2,
                fields: f2,
            },
        ) => {
            d1 == d2
                && f1.len() == f2.len()
                && f1.iter().zip(f2).all(|(a, b)| patterns_overlap(a, b))
        }
        (
            n::Pattern::AnonymousRecord {
                names: n1,
                fields: f1,
            },
            n::Pattern::AnonymousRecord {
                names: n2,
                fields: f2,
            },
        ) => {
            n1 == n2
                && f1.len() == f2.len()
                && f1.iter().zip(f2).all(|(a, b)| patterns_overlap(a, b))
        }
        (
            n::Pattern::NormalForm {
                head: h1,
                arguments: a1,
            },
            n::Pattern::NormalForm {
                head: h2,
                arguments: a2,
            },
        ) => {
            h1 == h2
                && a1.len() == a2.len()
                && a1.iter().zip(a2).all(|(a, b)| patterns_overlap(a, b))
        }
        (
            n::Pattern::Then {
                first: a1,
                next: b1,
                ..
            },
            n::Pattern::Then {
                first: a2,
                next: b2,
                ..
            },
        ) => patterns_overlap(a1, a2) && patterns_overlap(b1, b2),
        (n::Pattern::Then { first, .. }, other) | (other, n::Pattern::Then { first, .. }) => {
            patterns_overlap(first, other)
        }
        _ => false,
    }
}
