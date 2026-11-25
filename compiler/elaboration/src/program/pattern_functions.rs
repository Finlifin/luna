//! Package-wide open pattern functions (`fn foo(case ...)`).
use super::*;

struct CheckedPatternFnArm {
    span: Span,
    pattern: n::Pattern,
    guard: Option<n::Expr>,
    body: n::Expr,
}

fn collect_all_pattern_fn_bases(checker: &Checker<'_, '_>) -> Result<Vec<(OwnerId, Span)>, Error> {
    let mut bases: BTreeMap<OwnerId, Span> = BTreeMap::new();
    for arm in &checker.package.pattern_fn_arms {
        let Some(base_owner) = checker.package.definition_owner(arm.target) else {
            return Err(error(
                arm.span,
                "pattern function arms must be in the same package as the base declaration",
            ));
        };
        let Some(base_item) = checker.package.item(base_owner) else {
            return Err(error(
                arm.span,
                "pattern function arm target must be a declared function",
            ));
        };
        match &base_item.kind {
            ItemKind::FnDeclaration(_) => {
                bases.entry(base_owner).or_insert(base_item.span);
            }
            ItemKind::Fn(..) => {
                return Err(error(
                    arm.span,
                    "pattern function base declaration must not have a body",
                ));
            }
            _ => {
                return Err(error(
                    arm.span,
                    "pattern function arm target must be a declared function",
                ));
            }
        }
    }
    Ok(bases.into_iter().collect())
}

fn check_arm_body_with_facts<'hir>(
    checker: &mut Checker<'_, '_>,
    arm_owner: OwnerId,
    slots: &[(n::Pattern, n::Expr)],
    body_expr: &Expr<'hir>,
    result_ty: &Term,
    span: Span,
) -> Result<n::Expr, Error> {
    if let Some(((pat, scrutinee), rest)) = slots.split_first() {
        checker.under_matched_pattern_type_facts(pat, scrutinee, span, |this| {
            check_arm_body_with_facts(this, arm_owner, rest, body_expr, result_ty, span)
        })
    } else {
        checker.function_body(arm_owner, body_expr, result_ty)
    }
}

fn conjoin_guards(guards: Vec<n::Expr>) -> Option<n::Expr> {
    let mut iter = guards.into_iter();
    let mut acc = iter.next()?;
    for next in iter {
        let span = next.source;
        acc = n::Expr {
            ty: p(P::Bool),
            source: span,
            kind: K::If {
                condition: Box::new(acc),
                then_branch: Box::new(next),
                else_branch: Some(Box::new(n::Expr {
                    ty: p(P::Bool),
                    source: span,
                    kind: K::Literal(n::Literal::Bool(false)),
                })),
            },
        };
    }
    Some(acc)
}

impl Checker<'_, '_> {
    pub(super) fn pattern_fn_arm_parameter_type(
        &mut self,
        arm_owner: OwnerId,
        parameter_index: usize,
        span: Span,
    ) -> Result<Term, Error> {
        let base_owner = *self
            .pattern_fn_arm_targets
            .get(&arm_owner)
            .ok_or_else(|| error(span, "missing base declaration for pattern function arm"))?;
        let base_sig = self
            .signatures
            .get(&base_owner)
            .ok_or_else(|| error(span, "missing base signature for pattern function arm"))?
            .clone();
        let Term::Function { parameters, .. } = base_sig else {
            return Err(error(
                span,
                "pattern function base declaration must be callable",
            ));
        };
        let base_param = parameters.get(parameter_index).ok_or_else(|| {
            error(
                span,
                "pattern function arm parameter count does not match base declaration",
            )
        })?;
        let base_type_params = self
            .type_parameters
            .get(&base_owner)
            .cloned()
            .unwrap_or_default();
        if base_type_params.is_empty() {
            return Ok(base_param.ty.clone());
        }
        let arm_type_params = self
            .type_parameters
            .get(&arm_owner)
            .cloned()
            .unwrap_or_default();
        if arm_type_params.len() != base_type_params.len() {
            return Err(error(
                span,
                "pattern function arm clause parameters do not match base declaration",
            ));
        }
        let base_domains = self.trait_domains(base_owner)?;
        let arm_domains = self.trait_domains(arm_owner)?;
        let arm_rigids: Vec<Term> = arm_type_params
            .iter()
            .zip(arm_domains.iter())
            .map(|(decl, domain)| Term::Rigid {
                declaration: *decl,
                sort: Sort::of_comptime_domain(domain),
            })
            .collect();
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let empty_frames = [vec![]];
        let sub = nir::DeclarationSubstitution::bounded(
            &base_type_params,
            &base_domains,
            &arm_rigids,
            &empty_frames,
            &mut fuel,
        )
        .map_err(|e| error(span, e.to_string()))?;
        let result = sub
            .instantiate_bounded(&base_param.ty, &mut fuel)
            .map_err(|e| error(span, e.to_string()));
        self.required_steps -= (before - fuel) as u64;
        result
    }

    pub(super) fn check_all_pattern_functions(&mut self) -> Result<(), Error> {
        let bases = collect_all_pattern_fn_bases(self)?;
        for (base_owner, span) in bases {
            self.check_and_cache_pattern_function(base_owner, span)?;
        }
        Ok(())
    }

    pub(super) fn check_and_cache_pattern_function(
        &mut self,
        base_owner: OwnerId,
        span: Span,
    ) -> Result<n::Function, Error> {
        if let Some(cached) = self.checked_pattern_functions.get(&base_owner) {
            return Ok(cached.clone());
        }
        if !self.active_pattern_functions.insert(base_owner) {
            return Err(error(span, "cyclic pattern function elaboration"));
        }
        let result = self.synthesize_pattern_function(base_owner, span);
        self.active_pattern_functions.remove(&base_owner);
        result
    }

    fn synthesize_pattern_function(
        &mut self,
        base_owner: OwnerId,
        span: Span,
    ) -> Result<n::Function, Error> {
        let base_item = self
            .package
            .item(base_owner)
            .ok_or_else(|| error(span, "missing pattern function base declaration"))?;
        let ItemKind::FnDeclaration(base_sig) = &base_item.kind else {
            return Err(error(
                span,
                "pattern function base declaration must not have a body",
            ));
        };
        if self.self_types.contains_key(&base_owner)
            || self.self_containers.contains_key(&base_owner)
        {
            return Err(error(
                base_item.span,
                "pattern function arms are not supported on trait or impl methods",
            ));
        }
        if base_sig.modifiers.is_quote
            || base_sig.modifiers.is_function_template
            || base_sig.modifiers.is_normal_form_head
            || base_sig.modifiers.is_test
            || base_sig.modifiers.is_test_template
        {
            return Err(error(
                base_item.span,
                "pattern function base declaration uses unsupported modifiers",
            ));
        }

        self.ensure_signature(base_owner, base_item.span)?;
        self.ensure_contract(base_owner, base_item.span)?;
        self.ensure_defaults(base_owner, base_item.span)?;

        let base_signature = self
            .signatures
            .get(&base_owner)
            .ok_or_else(|| error(base_item.span, "missing base pattern function signature"))?
            .clone();
        let Term::Function {
            parameters: base_params,
            result: base_result,
        } = &base_signature
        else {
            return Err(error(
                base_item.span,
                "pattern function base declaration must have a function type",
            ));
        };
        if base_params.is_empty() {
            return Err(error(
                base_item.span,
                "pattern function must accept at least one parameter",
            ));
        }
        let base_type_params = self
            .type_parameters
            .get(&base_owner)
            .cloned()
            .unwrap_or_default();
        let base_domains = self.trait_domains(base_owner)?;
        let base_requirements = self.definition_requirements(base_owner);
        let base_decl_id = declaration(self.package, &self.package_identities, base_owner);
        let base_rigids: Vec<Term> = base_type_params
            .iter()
            .zip(base_domains.iter())
            .map(|(decl, domain)| Term::Rigid {
                declaration: *decl,
                sort: Sort::of_comptime_domain(domain),
            })
            .collect();

        let mut arms: Vec<(OwnerId, Span)> = self
            .package
            .pattern_fn_arms
            .iter()
            .filter_map(|arm| {
                (self.package.definition_owner(arm.target) == Some(base_owner))
                    .then_some((arm.arm, arm.span))
            })
            .collect();
        arms.sort_by_key(|&(arm_owner, arm_span)| (arm_span.lo().0, arm_owner.def_id.raw()));
        if arms.is_empty() {
            return Err(error(
                base_item.span,
                "pattern function declaration has no matching case arms",
            ));
        }

        let mut checked_arms = Vec::with_capacity(arms.len());
        let mut collected_list_infos = BTreeMap::new();
        let mut collected_range_infos = BTreeMap::new();
        let mut collected_bitvec_infos = BTreeMap::new();
        for (arm_owner, arm_span) in arms {
            let arm_item = self
                .package
                .item(arm_owner)
                .ok_or_else(|| error(arm_span, "missing pattern function arm item"))?;
            let ItemKind::Fn(arm_sig, arm_body_id) = &arm_item.kind else {
                return Err(error(arm_span, "pattern function arm must have a body"));
            };
            if arm_sig.modifiers.is_comptime != base_sig.modifiers.is_comptime
                || arm_sig.modifiers.is_pure != base_sig.modifiers.is_pure
                || arm_sig.modifiers.is_unsafe != base_sig.modifiers.is_unsafe
                || arm_sig.modifiers.is_quote
                || arm_sig.modifiers.is_function_template
                || arm_sig.modifiers.is_normal_form_head
                || arm_sig.modifiers.is_test
                || arm_sig.modifiers.is_test_template
                || arm_sig.modifiers.is_spec
            {
                return Err(error(
                    arm_span,
                    "pattern function arm modifiers do not match base declaration",
                ));
            }
            if arm_sig.params.len() != base_sig.params.len() {
                return Err(error(
                    arm_span,
                    "pattern function arm parameter count does not match base declaration",
                ));
            }
            if arm_sig.clause_params.len() != base_sig.clause_params.len() {
                return Err(error(
                    arm_span,
                    "pattern function arm clause parameters do not match base declaration",
                ));
            }
            for (arm_p, base_p) in arm_sig.params.iter().zip(base_sig.params.iter()) {
                if arm_p.ty.is_implicit() != base_p.ty.is_implicit()
                    || arm_p.ty.is_comptime() != base_p.ty.is_comptime()
                    || arm_p.ty.is_quote() != base_p.ty.is_quote()
                    || arm_p.ty.is_optional() != base_p.ty.is_optional()
                {
                    return Err(error(
                        arm_p.span,
                        "pattern function arm parameter markers do not match base declaration",
                    ));
                }
            }

            self.pattern_fn_arm_targets.insert(arm_owner, base_owner);
            self.ensure_signature(arm_owner, arm_span)?;
            self.ensure_contract(arm_owner, arm_span)?;

            let arm_type_params = self
                .type_parameters
                .get(&arm_owner)
                .cloned()
                .unwrap_or_default();
            if arm_type_params.len() != base_type_params.len() {
                return Err(error(
                    arm_span,
                    "pattern function arm clause parameters do not match base declaration",
                ));
            }
            let arm_domains = self.trait_domains(arm_owner)?;
            let arm_signature = self
                .signatures
                .get(&arm_owner)
                .ok_or_else(|| error(arm_span, "missing pattern function arm signature"))?
                .clone();

            let empty_frames = [vec![]];
            let arm_to_base = if arm_type_params.is_empty() {
                None
            } else {
                let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let sub = nir::DeclarationSubstitution::bounded(
                    &arm_type_params,
                    &arm_domains,
                    &base_rigids,
                    &empty_frames,
                    &mut fuel,
                )
                .map_err(|e| error(arm_span, e.to_string()))?;
                self.required_steps -= (before - fuel) as u64;
                Some(sub)
            };

            let (inst_arm_domains, inst_arm_signature, inst_arm_reqs) =
                if let Some(ref sub) = arm_to_base {
                    let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let mut fuel = before;
                    let doms = arm_domains
                        .iter()
                        .map(|d| sub.instantiate_bounded(d, &mut fuel))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| error(arm_span, e.to_string()))?;
                    let sig = sub
                        .instantiate_bounded(&arm_signature, &mut fuel)
                        .map_err(|e| error(arm_span, e.to_string()))?;
                    let reqs = self
                        .definition_requirements(arm_owner)
                        .into_iter()
                        .map(|r| sub.instantiate_predicate_bounded(&r, &mut fuel))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| error(arm_span, e.to_string()))?;
                    self.required_steps -= (before - fuel) as u64;
                    (doms, sig, reqs)
                } else {
                    (
                        arm_domains.clone(),
                        arm_signature.clone(),
                        self.definition_requirements(arm_owner),
                    )
                };

            for (arm_dom, base_dom) in inst_arm_domains.iter().zip(base_domains.iter()) {
                self.require(arm_dom, base_dom, arm_span)?;
            }
            let Term::Function {
                parameters: inst_arm_params,
                result: inst_arm_result,
            } = &inst_arm_signature
            else {
                return Err(error(
                    arm_span,
                    "pattern function arm must have a function type",
                ));
            };
            if inst_arm_params.len() != base_params.len() {
                return Err(error(
                    arm_span,
                    "pattern function arm parameter count does not match base declaration",
                ));
            }
            for (ap, bp) in inst_arm_params.iter().zip(base_params.iter()) {
                if ap.sort != bp.sort {
                    return Err(error(
                        arm_span,
                        "pattern function arm parameter phase does not match base declaration",
                    ));
                }
                self.require(&ap.ty, &bp.ty, arm_span)?;
            }
            self.require(inst_arm_result, base_result, arm_span)?;
            if inst_arm_reqs != base_requirements {
                return Err(error(
                    arm_span,
                    "pattern function arm requirements must match base declaration",
                ));
            }

            let Term::Function {
                parameters: arm_params,
                result: arm_result,
            } = arm_signature
            else {
                unreachable!()
            };
            let arm_body = self
                .package
                .body(*arm_body_id)
                .ok_or_else(|| error(arm_span, "missing pattern function arm body"))?;
            let arm_result_val_ty = match arm_result.as_ref() {
                Term::Computation { result, .. } => (**result).clone(),
                other => other.clone(),
            };

            let (mut checked_pat, mut checked_body) =
                self.in_declaration_scope(arm_owner, |this| {
                    this.active_body_declaration = Some(base_decl_id);
                    this.context = this.definition_context(arm_owner);
                    this.unsafe_depth = usize::from(arm_sig.modifiers.is_unsafe);

                    let mut slot_patterns = Vec::with_capacity(arm_sig.params.len());
                    let mut case_fact_slots = Vec::new();
                    for (index, (param, param_ty)) in
                        arm_sig.params.iter().zip(arm_params.iter()).enumerate()
                    {
                        if let TyParamKind::Case(hir_pat) = param.ty.kind {
                            let (inner_pat, guard) = match &hir_pat.kind {
                                hir::pattern::PatternKind::Guard(p, g) => (*p, Some(*g)),
                                _ => (hir_pat, None),
                            };
                            let hir_arm = hir::pattern::PatternArm {
                                hir_id: hir_pat.hir_id,
                                pat: inner_pat.clone(),
                                guard,
                                body: arm_body.value,
                                span: param.span,
                            };
                            let synthetic_scrutinee = n::Expr {
                                source: source(param.span),
                                ty: param_ty.ty.clone(),
                                kind: K::Unit,
                            };
                            let pat = this.match_arm_pattern(&hir_arm, &synthetic_scrutinee)?;
                            case_fact_slots.push((pat.clone(), synthetic_scrutinee));
                            slot_patterns.push(pat);
                        } else {
                            let binding = local(param.hir_id);
                            this.locals.insert(binding, param_ty.ty.clone());
                            this.input_facts.parameters.bind(
                                binding,
                                index,
                                param_ty.sort,
                                &param_ty.ty,
                            );
                            slot_patterns.push(n::Pattern::Binding {
                                binding,
                                inner: Box::new(n::Pattern::Wild),
                            });
                        }
                    }

                    let mut body = check_arm_body_with_facts(
                        this,
                        arm_owner,
                        &case_fact_slots,
                        arm_body.value,
                        &arm_result,
                        arm_span,
                    )?;
                    let before = usize::try_from(this.required_steps).unwrap_or(usize::MAX);
                    let mut fuel = before;
                    nir::inline_return::simplify_tail_returns(
                        &mut body,
                        &arm_result_val_ty,
                        &mut fuel,
                    )
                    .map_err(|message| error(arm_span, message))?;
                    this.required_steps -= (before - fuel) as u64;

                    let combined_pat = if slot_patterns.len() == 1 {
                        slot_patterns.pop().expect("len == 1")
                    } else {
                        let mut inner_slots = Vec::with_capacity(slot_patterns.len());
                        let mut guards = Vec::new();
                        for pat in slot_patterns {
                            match pat {
                                n::Pattern::Guard { pattern, condition } => {
                                    inner_slots.push(*pattern);
                                    guards.push(*condition);
                                }
                                other => inner_slots.push(other),
                            }
                        }
                        let tuple_pat = n::Pattern::Tuple(inner_slots);
                        if let Some(guard_expr) = conjoin_guards(guards) {
                            n::Pattern::Guard {
                                pattern: Box::new(tuple_pat),
                                condition: Box::new(guard_expr),
                            }
                        } else {
                            tuple_pat
                        }
                    };
                    collected_list_infos.extend(std::mem::take(&mut this.list_pattern_infos));
                    collected_range_infos.extend(std::mem::take(&mut this.range_pattern_infos));
                    collected_bitvec_infos.extend(std::mem::take(&mut this.bitvec_pattern_infos));
                    Ok((combined_pat, body))
                })?;

            let mut arm_guard = None;
            if let n::Pattern::Guard { pattern, condition } = checked_pat {
                checked_pat = *pattern;
                arm_guard = Some(*condition);
            }

            if let Some(ref sub) = arm_to_base {
                let mut wrapped = n::Expr {
                    source: source(arm_span),
                    ty: checked_body.ty.clone(),
                    kind: K::Match {
                        value: Box::new(n::Expr {
                            source: source(arm_span),
                            ty: p(P::Void),
                            kind: K::Unit,
                        }),
                        arms: vec![n::MatchArm {
                            pattern: checked_pat,
                            guard: arm_guard,
                            body: checked_body,
                        }],
                    },
                };
                let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                wrapped = sub
                    .instantiate_expression_bounded(&wrapped, &mut fuel)
                    .map_err(|e| error(arm_span, e.to_string()))?;
                self.required_steps -= (before - fuel) as u64;
                let K::Match { mut arms, .. } = wrapped.kind else {
                    unreachable!()
                };
                let arm = arms.pop().expect("wrapped arm");
                checked_pat = arm.pattern;
                arm_guard = arm.guard;
                checked_body = arm.body;
            }

            checked_arms.push(CheckedPatternFnArm {
                span: arm_span,
                pattern: checked_pat,
                guard: arm_guard,
                body: checked_body,
            });
        }

        let scrutinee_ty = if base_params.len() == 1 {
            base_params[0].ty.clone()
        } else {
            Term::Tuple(base_params.iter().map(|p| p.ty.clone()).collect())
        };

        let function = self.in_declaration_scope(base_owner, |this| {
            this.list_pattern_infos = collected_list_infos;
            this.range_pattern_infos = collected_range_infos;
            this.bitvec_pattern_infos = collected_bitvec_infos;

            // OQ-6: Enforce strict pairwise disjointness across all collected package arms.
            for i in 0..checked_arms.len() {
                for j in (i + 1)..checked_arms.len() {
                    let a = &checked_arms[i];
                    let b = &checked_arms[j];
                    let overlap = crate::automata::patterns_overlap(
                        this,
                        &a.pattern,
                        &b.pattern,
                        &scrutinee_ty,
                    );
                    match overlap {
                        Some(false) => {}
                        _ => {
                            return Err(error(
                                b.span,
                                "overlapping case function arms across order-independent package collection",
                            ));
                        }
                    }
                }
            }

            this.active_body_declaration = Some(base_decl_id);
            let mut interface =
                this.bind_source_declaration(base_owner, base_signature.clone(), base_item.span)?;
            interface.defaults = this.defaults[&base_owner]
                .iter()
                .enumerate()
                .filter_map(|(index, value)| value.clone().map(|value| (index, value)))
                .collect();

            let arm_spans: Vec<Span> = checked_arms.iter().map(|arm| arm.span).collect();
            let nir_arms: Vec<n::MatchArm> = checked_arms
                .into_iter()
                .map(|arm| n::MatchArm {
                    pattern: arm.pattern,
                    guard: arm.guard,
                    body: arm.body,
                })
                .collect();
            let coverage_pats: Vec<n::Pattern> = nir_arms
                .iter()
                .map(|arm| match &arm.guard {
                    Some(cond) => n::Pattern::Guard {
                        pattern: Box::new(arm.pattern.clone()),
                        condition: Box::new(cond.clone()),
                    },
                    None => arm.pattern.clone(),
                })
                .collect();
            let coverage_refs: Vec<&n::Pattern> = coverage_pats.iter().collect();
            this.require_match_coverage(
                &coverage_refs,
                &scrutinee_ty,
                base_item.span,
                &arm_spans,
                true,
            )?;

            let scrutinee_expr = if base_sig.params.len() == 1 {
                n::Expr {
                    source: source(base_sig.params[0].span),
                    ty: base_params[0].ty.clone(),
                    kind: K::Local(local(base_sig.params[0].hir_id)),
                }
            } else {
                let elems = base_sig
                    .params
                    .iter()
                    .zip(base_params.iter())
                    .map(|(param, param_ty)| n::Expr {
                        source: source(param.span),
                        ty: param_ty.ty.clone(),
                        kind: K::Local(local(param.hir_id)),
                    })
                    .collect();
                n::Expr {
                    source: source(base_item.span),
                    ty: scrutinee_ty.clone(),
                    kind: K::Tuple(elems),
                }
            };
            let result_val_ty = match base_result.as_ref() {
                Term::Computation { result, .. } => (**result).clone(),
                other => other.clone(),
            };
            let body = n::Expr {
                source: source(base_item.span),
                ty: result_val_ty,
                kind: K::Match {
                    value: Box::new(scrutinee_expr),
                    arms: nir_arms,
                },
            };
            this.verify_returned_type_guarantees(&body, base_item.span)?;
            let decreases = this.program_decreases(base_owner)?;
            let safety_level = if base_sig.modifiers.is_spec {
                n::SafetyLevel::Spec
            } else if base_sig.modifiers.is_verified {
                n::SafetyLevel::Verified
            } else if base_sig.modifiers.is_unsafe {
                n::SafetyLevel::Unsafe
            } else {
                n::SafetyLevel::Safe
            };
            if safety_level != n::SafetyLevel::Safe {
                this.safety_contracts
                    .levels
                    .insert(base_decl_id, safety_level);
            }
            if !decreases.is_empty() {
                this.safety_contracts
                    .decreases
                    .insert(base_decl_id, decreases);
            }
            let runtime_deliveries = this
                .local_runtime_deliveries
                .iter()
                .map(|(id, entry)| (*id, entry.expr.clone()))
                .collect();
            Ok(n::Function {
                interface,
                body,
                runtime_deliveries,
            })
        })?;

        self.checked_pattern_functions
            .insert(base_owner, function.clone());
        Ok(function)
    }
}
