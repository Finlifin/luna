//! Observe ordinary pure comptime results under the actual call's stable input
//! identities. The function, its operands and its contracts still execute at
//! the original call site. Normal-form heads have a separate identity rule.
use super::*;

// An unknown direct read has no lexical control transfer. An unobserved call
// operand or guard can itself return from the function whose result we seek.
fn direct_read(expr: &Expr<'_>) -> bool {
    matches!(
        expr.kind,
        H::ResolvedLocal(..)
            | H::ResolvedFnParam(..)
            | H::ResolvedClauseParam(..)
            | H::ResolvedDef { .. }
            | H::Lit(_)
    )
}

impl Checker<'_, '_> {
    pub(super) fn static_conditional_result(
        &mut self,
        guard: &Expr<'_>,
        yes: &hir::expr::Block<'_>,
        no: &Expr<'_>,
        domain: &Term,
        depth: usize,
        bindings: Option<(&[DeclarationId], &[Term])>,
    ) -> Result<Option<Term>, Error> {
        let selected = self.generator_static_value_hint(guard, &p(P::Bool), depth + 1, bindings)?;
        if !self.observing_call_results.is_empty()
            && !matches!(selected, Some(Term::Bool(_)))
            && !direct_read(guard)
        {
            return Ok(None);
        }
        let observe = |this: &mut Self, value| {
            this.generator_static_value_hint(value, domain, depth + 1, bindings)
        };
        if selected == Some(Term::Bool(false)) {
            return observe(self, no);
        }
        let Some(yes) = yes.expr.filter(|_| yes.stmts.is_empty()) else {
            return Ok(None);
        };
        let Some(left) = observe(self, yes)? else {
            return Ok(None);
        };
        if selected == Some(Term::Bool(true)) {
            return Ok(Some(left));
        }
        let right = observe(self, no)?;
        Ok((right.as_ref() == Some(&left)).then_some(left))
    }

    pub(super) fn static_call_result(
        &mut self,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        domain: &Term,
        depth: usize,
        bindings: Option<(&[DeclarationId], &[Term])>,
    ) -> Result<Option<Term>, Error> {
        let H::ResolvedDef { def_id, .. } = callee.kind else {
            return Ok(None);
        };
        let Some(owner) = self.package.definition_owner(def_id) else {
            return Ok(None);
        };
        let Some(ItemKind::Fn(decl, body)) = self.package.item(owner).map(|item| &item.kind) else {
            return Ok(None);
        };
        if !decl.modifiers.is_pure
            || !decl.modifiers.is_comptime
            || decl.modifiers.is_normal_form_head
            || self
                .type_parameters
                .get(&owner)
                .is_none_or(|params| !params.is_empty())
            || self.observing_call_results.contains(&owner)
        {
            return Ok(None);
        }
        let Some(Term::Function { parameters, result }) =
            self.signatures.get(&owner).map(Term::callable_signature)
        else {
            return Ok(None);
        };
        if result.as_ref() != domain || parameters.len() != decl.params.len() {
            return Ok(None);
        }
        self.required_steps = self
            .required_steps
            .checked_sub(arguments.len().saturating_add(parameters.len()) as u64)
            .ok_or_else(|| error(callee.span, "call result evidence budget exhausted"))?;
        let Ok(plan) =
            arguments::argument_routes(arguments, Some(decl.params), parameters.len(), callee.span)
        else {
            return Ok(None);
        };
        let Some(body) = self.package.body(*body).map(|body| body.value) else {
            return Ok(None);
        };
        // Copy only the domains of individually routed operands before entering
        // an operand observation, which can itself query another declaration.
        let mut operands = Vec::new();
        for route in plan.routes {
            if route.collected || route.lambda {
                if !self.observing_call_results.is_empty() {
                    return Ok(None);
                }
                continue;
            }
            let arg = match arguments[route.sources[0]] {
                Arg::Named(_, value) | Arg::Positional(value) => value,
                _ => return Ok(None),
            };
            let ty = declaration_inputs::copy_input_domain(
                &parameters[route.parameter].ty,
                &mut self.required_steps,
                arg.span,
            )?;
            operands.push((route.parameter, arg, ty));
        }
        let mut inputs = vec![None; decl.params.len()];
        for (slot, arg, ty) in operands {
            if let Some(value) = self.generator_static_value_hint(arg, &ty, depth + 1, bindings)? {
                inputs[slot] = Some((ty, value));
            } else if !self.observing_call_results.is_empty() && !direct_read(arg) {
                return Ok(None);
            }
        }
        self.observing_call_results.insert(owner);
        let result = self.in_formal_declaration(owner, |this| {
            this.in_required_comptime = true;
            // Defaults bind only their declaration prefix. Even an unknown
            // explicit operand suppresses its default at this call site.
            let formals: Option<(&[DeclarationId], &[Term])> = Some((&[], &[]));
            for ((parameter, assigned), explicit) in
                decl.params.iter().zip(plan.assigned).zip(inputs)
            {
                let evidence = if assigned {
                    explicit
                } else if let TyParamKind::Optional(_, annotation, default) = parameter.ty.kind {
                    let Some(ty) =
                        this.generator_static_type_hint(annotation, depth + 1, formals)?
                    else {
                        continue;
                    };
                    this.generator_static_value_hint(default, &ty, depth + 1, formals)?
                        .map(|value| (ty, value))
                } else {
                    None
                };
                let Some((ty, value)) = evidence else {
                    continue;
                };
                let id = local(parameter.hir_id);
                this.locals.insert(id, ty);
                this.immutable.insert(id);
                this.generator_data.insert(id, None);
                this.input_facts
                    .partial
                    .insert(id, constants::PartialStagedValue::known(value));
            }
            this.generator_static_value_hint(body, domain, depth + 1, formals)
        });
        self.observing_call_results.remove(&owner);
        result
    }
}
