//! Direct, direct-passing, and capturing handler branches, coverage and lexical environments.
use super::*;

struct PreparedArm<'a, 'hir> {
    handler_index: usize,
    pattern: n::Pattern,
    body: &'a Expr<'hir>,
    span: Span,
    arm_locals: crate::program::local_bindings::Bindings<n::LocalId, Term>,
    continuation_binding: Option<n::LocalId>,
}

fn capturing_continuation_local(pattern: &n::Pattern) -> Option<n::LocalId> {
    match pattern {
        n::Pattern::Binding { binding, .. } => Some(*binding),
        _ => None,
    }
}

fn check_capturing_continuation_sequence(
    parts: &[&n::Expr],
    continuation: n::LocalId,
    span: Span,
    fuel: &mut usize,
) -> Result<(bool, bool), Error> {
    let mut used_k = false;
    let mut resumed = false;
    for part in parts {
        let (k, r) = check_capturing_continuation_resume(part, continuation, span, fuel)?;
        if (used_k && r) || (resumed && k) {
            return Err(error(
                span,
                "cannot both move continuation binding and resume it",
            ));
        }
        used_k |= k;
        resumed |= r;
        if is_never(&part.ty) {
            break;
        }
    }
    Ok((used_k, resumed))
}

fn check_capturing_continuation_resume(
    expr: &n::Expr,
    continuation: n::LocalId,
    span: Span,
    fuel: &mut usize,
) -> Result<(bool, bool), Error> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or_else(|| error(span, "capturing continuation resume check budget exhausted"))?;
    let mut seq = |parts: &[&n::Expr]| -> Result<(bool, bool), Error> {
        check_capturing_continuation_sequence(parts, continuation, span, fuel)
    };
    match &expr.kind {
        K::Local(id) => Ok((*id == continuation, false)),
        K::EffectResume(value) => {
            let (k, r) = check_capturing_continuation_resume(value, continuation, span, fuel)?;
            if k || r {
                return Err(error(
                    span,
                    "cannot both move continuation binding and resume it",
                ));
            }
            Ok((false, true))
        }
        K::Block { statements, result } | K::CallPreparation { statements, result } => {
            let mut parts: Vec<&n::Expr> = statements.iter().collect();
            if let Some(result) = result {
                parts.push(result);
            }
            seq(&parts)
        }
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let (ck, cr) =
                check_capturing_continuation_resume(condition, continuation, span, fuel)?;
            if is_never(&condition.ty) {
                return Ok((ck, cr));
            }
            let (tk, tr) =
                check_capturing_continuation_resume(then_branch, continuation, span, fuel)?;
            let (ek, er) = match else_branch {
                Some(branch) => {
                    check_capturing_continuation_resume(branch, continuation, span, fuel)?
                }
                None => (false, false),
            };
            if (ck && (tr || er)) || (cr && (tk || ek)) {
                return Err(error(
                    span,
                    "cannot both move continuation binding and resume it",
                ));
            }
            Ok((ck || tk || ek, cr || tr || er))
        }
        K::Match { value, arms } => {
            let (vk, vr) = check_capturing_continuation_resume(value, continuation, span, fuel)?;
            if is_never(&value.ty) {
                return Ok((vk, vr));
            }
            let mut branch_k = false;
            let mut branch_r = false;
            for arm in arms {
                let mut guard_k = false;
                let mut guard_r = false;
                if let Some(guard) = &arm.guard {
                    (guard_k, guard_r) =
                        check_capturing_continuation_resume(guard, continuation, span, fuel)?;
                }
                let (bk, br) =
                    check_capturing_continuation_resume(&arm.body, continuation, span, fuel)?;
                if (vk && (guard_r || br))
                    || (vr && (guard_k || bk))
                    || (guard_k && br)
                    || (guard_r && bk)
                {
                    return Err(error(
                        span,
                        "cannot both move continuation binding and resume it",
                    ));
                }
                branch_k |= guard_k || bk;
                branch_r |= guard_r || br;
            }
            Ok((vk || branch_k, vr || branch_r))
        }
        K::Let { value, .. }
        | K::UnsafeScope(value)
        | K::Discard(value)
        | K::Return(Some(value)) => {
            check_capturing_continuation_resume(value, continuation, span, fuel)
        }
        K::Call {
            callee,
            arguments,
            env_arguments,
            ..
        } => {
            let mut parts: Vec<&n::Expr> =
                Vec::with_capacity(1 + arguments.len() + env_arguments.len());
            parts.push(callee);
            parts.extend(arguments.iter().map(|arg| &arg.value));
            parts.extend(env_arguments.iter());
            seq(&parts)
        }
        K::Tuple(items)
        | K::AnonymousRecord { fields: items }
        | K::Record { fields: items, .. }
        | K::Variant { fields: items, .. } => {
            let parts: Vec<&n::Expr> = items.iter().collect();
            seq(&parts)
        }
        _ => {
            let mut uses_k = false;
            let mut has_resume = false;
            nir::visit::expression(expr, fuel, |node, _| {
                if matches!(node.kind, K::Local(id) if id == continuation) {
                    uses_k = true;
                } else if matches!(node.kind, K::EffectResume(_)) {
                    has_resume = true;
                }
                Ok(())
            })
            .map_err(|message| error(span, message))?;
            if uses_k && has_resume {
                return Err(error(
                    span,
                    "cannot both move continuation binding and resume it",
                ));
            }
            Ok((uses_k, has_resume))
        }
    }
}

impl Checker<'_, '_> {
    pub(in crate::program) fn effect_handle(
        &mut self,
        expr: &Expr<'_>,
        value: &Expr<'_>,
        arms: &[hir::pattern::PatternArm<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if self.in_required_comptime || self.signature_owner.and_then(|owner| self.package.item(owner)).is_some_and(|item| matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) if sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head)) {
            return Err(error(expr.span, "comptime effect handlers are not implemented"));
        }
        if arms.len() > 4096 {
            return Err(error(expr.span, "effect handler width exhausted"));
        }
        let outer = self.active_effect_environment().unwrap_or(&[]).to_vec();
        let saved_locals = self.locals.clone();
        let saved_resume = self.effect_resume_result.clone();
        let saved_environment = self.effect_environment.clone();
        let checked = (|| {
            let mut handlers: Vec<n::EffectHandler> = Vec::new();
            let mut prepared_arms = Vec::with_capacity(arms.len());
            let mut handler_patterns: Vec<Vec<n::Pattern>> = Vec::new();
            let mut comparisons = 100_000usize;
            for arm in arms {
                self.locals = saved_locals.clone();
                let hir::pattern::PatternKind::AppTuple(head, fields) = &arm.pat.kind else {
                    return Err(error(
                        arm.span,
                        "effect handler requires an operation application pattern",
                    ));
                };
                let hir::pattern::PatternKind::Const(head) = &head.kind else {
                    return Err(error(
                        arm.span,
                        "effect handler requires a resolved operation label",
                    ));
                };
                let operations = self.effect_row(head, 0)?;
                if operations.len() != 1 {
                    return Err(error(
                        arm.span,
                        "effect handler requires one independent operation",
                    ));
                }
                // Even a one-operation group is not itself an operation head.
                let declaration_head = if let H::NFApplication(base, _) = &head.kind {
                    *base
                } else {
                    *head
                };
                if self
                    .effect_head_owner(declaration_head)
                    .ok()
                    .is_some_and(|owner| {
                        matches!(
                            self.package.item(owner).map(|i| &i.kind),
                            Some(ItemKind::EffectGroup(_))
                        )
                    })
                {
                    return Err(error(
                        arm.span,
                        "effect group is not an operation; select a member",
                    ));
                }
                let operation = operations.into_iter().next().ok_or_else(|| {
                    error(
                        arm.span,
                        "effect handler requires one independent operation",
                    )
                })?;
                let mut selected = None;
                for (index, handler) in handlers.iter().enumerate() {
                    comparisons = comparisons.checked_sub(1).ok_or_else(|| {
                        error(arm.span, "effect handler identity budget exhausted")
                    })?;
                    match crate::effects::select_layer(
                        std::slice::from_ref(&handler.operation),
                        &operation,
                        0,
                        &self.context,
                        crate::ptc::Budget::default(),
                    ) {
                        Ok(_) => {
                            selected = Some(index);
                            break;
                        }
                        Err(crate::effects::Error::MissingEntry) => {}
                        Err(failure) => return Err(error(arm.span, failure.to_string())),
                    }
                }
                let index = if let Some(index) = selected {
                    index
                } else {
                    let metadata = self.effect_metadata(&operation, arm.span)?;
                    if !matches!(
                        metadata.abi,
                        nir::effects::Abi::Direct
                            | nir::effects::Abi::DirectPassing
                            | nir::effects::Abi::Capturing
                    ) {
                        return Err(error(
                            arm.span,
                            "non-direct effect handler source lowering is not implemented",
                        ));
                    }
                    let signature =
                        self.formed_effect_signature(&metadata, &operation, arm.span)?;
                    let index = handlers.len();
                    handlers.push(n::EffectHandler {
                        operation,
                        abi: metadata.abi,
                        signature,
                        arms: Vec::new(),
                        fallback: None,
                    });
                    handler_patterns.push(Vec::new());
                    index
                };
                let abi = handlers[index].abi;
                let Term::Function { parameters, result } = &handlers[index].signature else {
                    return Err(error(arm.span, "invalid effect signature"));
                };
                if abi == nir::effects::Abi::Capturing {
                    if parameters.len() + 1 != fields.len() {
                        return Err(error(
                            arm.span,
                            "capturing effect handler requires a trailing continuation pattern",
                        ));
                    }
                } else if parameters.len() != fields.len() {
                    return Err(error(arm.span, "effect handler parameter count mismatch"));
                }
                if result.check_scope(&[]).is_err()
                    || parameters.iter().any(|p| p.ty.check_scope(&[]).is_err())
                {
                    return Err(error(
                        arm.span,
                        "runtime-dependent effect handler source lowering is not implemented",
                    ));
                }
                let mut patterns = Vec::with_capacity(fields.len());
                for (field, parameter) in fields.iter().zip(parameters) {
                    patterns.push(self.pattern(field, &parameter.ty)?);
                }
                let continuation_binding = if abi == nir::effects::Abi::Capturing {
                    let continuation_pattern =
                        self.pattern(&fields[parameters.len()], &p(P::Continuation))?;
                    let binding = capturing_continuation_local(&continuation_pattern);
                    patterns.push(continuation_pattern);
                    binding
                } else {
                    None
                };
                let pattern = n::Pattern::Tuple(patterns);
                let pattern = if let Some(guard) = arm.guard {
                    n::Pattern::Guard {
                        pattern: Box::new(pattern),
                        condition: Box::new(self.expr(guard, Some(&p(P::Bool)))?),
                    }
                } else {
                    pattern
                };
                handler_patterns[index].push(pattern.clone());
                prepared_arms.push(PreparedArm {
                    handler_index: index,
                    pattern,
                    body: arm.body,
                    span: arm.span,
                    arm_locals: self.locals.clone(),
                    continuation_binding,
                });
            }
            self.locals = saved_locals.clone();
            self.effect_resume_result = saved_resume.clone();
            for (handler, patterns) in handlers.iter_mut().zip(&handler_patterns) {
                let Term::Function { parameters, .. } = &handler.signature else {
                    return Err(error(expr.span, "invalid effect signature"));
                };
                let mut input_types: Vec<_> = parameters.iter().map(|p| p.ty.clone()).collect();
                if handler.abi == nir::effects::Abi::Capturing {
                    input_types.push(p(P::Continuation));
                }
                let input = Term::Tuple(input_types);
                let pattern_refs: Vec<_> = patterns.iter().collect();
                if self.patterns_exhaustive(&pattern_refs, &input)
                    != crate::coverage::Coverage::Exhaustive
                {
                    let fallback = crate::effects::select_layer(
                        &outer,
                        &handler.operation,
                        0,
                        &self.context,
                        crate::ptc::Budget::default(),
                    )
                    .map_err(|failure| {
                        error(
                            expr.span,
                            format!("partial effect handler requires an outer entry: {failure}"),
                        )
                    })?;
                    handler.fallback = Some(fallback.index());
                }
            }
            let mut installed: Vec<_> = handlers.iter().map(|h| h.operation.clone()).collect();
            installed.extend_from_slice(&outer);
            self.effect_environment = Some(installed.clone());
            let is_operation = if let H::Application(head, _) = &value.kind {
                let head = if let H::NFApplication(base, _) = &head.kind {
                    *base
                } else {
                    *head
                };
                if let H::Projection(group, _) = &head.kind {
                    let group = if let H::NFApplication(base, _) = &group.kind {
                        *base
                    } else {
                        *group
                    };
                    self.effect_head_owner(group).ok().is_some_and(|owner| {
                        matches!(
                            self.package.item(owner).map(|i| &i.kind),
                            Some(ItemKind::EffectGroup(_))
                        )
                    })
                } else {
                    self.effect_head_owner(head).ok().is_some_and(|owner| {
                        matches!(
                            self.package.item(owner).map(|i| &i.kind),
                            Some(ItemKind::Effect(..))
                        )
                    })
                }
            } else {
                false
            };
            let body = if is_operation {
                self.effect_request(expr, value, expected)?
            } else {
                self.expr(value, expected)?
            };
            let mut ty = body.ty.clone();
            let mut overall_result = expected
                .cloned()
                .filter(|_| is_never(&ty))
                .unwrap_or_else(|| ty.clone());
            self.effect_environment = saved_environment.clone();
            for prepared in prepared_arms {
                self.locals = prepared.arm_locals;
                let handler_abi = handlers[prepared.handler_index].abi;
                let Term::Function { result, .. } = &handlers[prepared.handler_index].signature
                else {
                    return Err(error(prepared.span, "invalid effect signature"));
                };
                let operation_result = result.as_ref().clone();
                self.effect_resume_result = Some(operation_result.clone());
                // Keep self.control.result and loop targets: return/break/continue belong
                // to the lexical caller, not the operation result contract.
                let arm_body = if handler_abi == nir::effects::Abi::Capturing {
                    let expected_branch = (!is_never(&overall_result)).then_some(&overall_result);
                    let checked_arm = self.expr(prepared.body, expected_branch)?;
                    if is_never(&overall_result) && !is_never(&checked_arm.ty) {
                        overall_result = checked_arm.ty.clone();
                    }
                    if is_never(&ty) && !is_never(&checked_arm.ty) {
                        ty = overall_result.clone();
                    }
                    if let Some(continuation) = prepared.continuation_binding {
                        let mut fuel = 100_000usize;
                        if let n::Pattern::Guard { condition, .. } = &prepared.pattern {
                            check_capturing_continuation_sequence(
                                &[condition.as_ref(), &checked_arm],
                                continuation,
                                prepared.span,
                                &mut fuel,
                            )?;
                        } else {
                            check_capturing_continuation_resume(
                                &checked_arm,
                                continuation,
                                prepared.span,
                                &mut fuel,
                            )?;
                        }
                    }
                    checked_arm
                } else {
                    self.expr(prepared.body, Some(&operation_result))?
                };
                handlers[prepared.handler_index].arms.push(n::MatchArm {
                    pattern: prepared.pattern,
                    guard: None,
                    body: arm_body,
                });
            }
            let computation = n::Expr {
                source: source(value.span),
                ty: Term::computation(installed.clone(), body.ty.clone()),
                kind: K::EffectComputation {
                    effects: installed,
                    body: Box::new(body),
                },
            };
            Ok(n::Expr {
                source: source(expr.span),
                ty,
                kind: K::EffectHandle {
                    computation: Box::new(computation),
                    handlers,
                    outer,
                },
            })
        })();
        self.locals = saved_locals;
        self.effect_resume_result = saved_resume;
        self.effect_environment = saved_environment;
        checked
    }
}
