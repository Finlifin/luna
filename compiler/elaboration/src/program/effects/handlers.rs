//! Direct handler branches, coverage and lexical environments.
use super::*;

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
                let operation = operations.into_iter().next().unwrap();
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
                    if !matches!(metadata.abi, nir::effects::Abi::Direct | nir::effects::Abi::DirectPassing) {
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
                    index
                };
                let Term::Function { parameters, result } = &handlers[index].signature else {
                    return Err(error(arm.span, "invalid effect signature"));
                };
                if parameters.len() != fields.len() {
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
                self.effect_resume_result = Some(result.as_ref().clone());
                let mut patterns = Vec::new();
                for (field, parameter) in fields.iter().zip(parameters) {
                    patterns.push(self.pattern(field, &parameter.ty)?);
                }
                let pattern = n::Pattern::Tuple(patterns);
                let pattern = if let Some(guard) = arm.guard {
                    n::Pattern::Guard {
                        pattern: Box::new(pattern),
                        condition: Box::new(self.expr(guard, Some(&p(P::Bool)))?),
                    }
                } else {
                    pattern
                };
                // Keep self.result and loop targets: return/break/continue belong
                // to the lexical caller, not the operation result contract.
                let body = self.expr(arm.body, Some(result))?;
                handlers[index].arms.push(n::MatchArm {
                    pattern,
                    guard: None,
                    body,
                });
            }
            self.locals = saved_locals.clone();
            self.effect_resume_result = saved_resume.clone();
            for handler in &mut handlers {
                let Term::Function { parameters, .. } = &handler.signature else {
                    unreachable!()
                };
                let input = Term::Tuple(parameters.iter().map(|p| p.ty.clone()).collect());
                let patterns: Vec<_> = handler.arms.iter().map(|arm| &arm.pattern).collect();
                if self.patterns_exhaustive(&patterns, &input)
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
            let ty = body.ty.clone();
            let computation = n::Expr {
                source: source(value.span),
                ty: Term::computation(installed.clone(), ty.clone()),
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
