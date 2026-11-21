//! Observe ordinary pure comptime results under the actual call's stable input
//! identities. The function, its operands and its contracts still execute at
//! the original call site. Normal-form heads have a separate identity rule.
use super::*;

mod control;
use control::Paths;

enum Caller<'a, 'result> {
    Source(Option<(&'a [DeclarationId], &'a [Term])>),
    Function(&'a mut control::Observation<'result>),
}

enum Operand {
    Parameter(usize, Term),
    Collected,
    Tactic,
}

const FORMALS: Option<(&[DeclarationId], &[Term])> = Some((&[], &[]));

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
        Ok(self
            .call_result_paths(
                callee,
                arguments,
                Some(domain),
                depth,
                Caller::Source(bindings),
            )?
            .and_then(Paths::known))
    }

    // None means the source call has no independent routing/signature evidence.
    // A routed call can instead close the caller's path during preparation.
    fn call_result_paths(
        &mut self,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Term>,
        depth: usize,
        mut caller: Caller<'_, '_>,
    ) -> Result<Option<Paths>, Error> {
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
        {
            return Ok(None);
        }
        let Some(Term::Function { parameters, result }) =
            self.signatures.get(&owner).map(Term::callable_signature)
        else {
            return Ok(None);
        };
        let observed_domain = expected.filter(|domain| *domain == result.as_ref());
        if parameters.len() != decl.params.len()
            || (matches!(caller, Caller::Source(_)) && observed_domain.is_none())
        {
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
        // A collector can group inputs on opposite sides of a named supply.
        // Map destinations once, then visit the original sequence of operands.
        let mut destinations: Vec<_> = (0..arguments.len()).map(|_| Operand::Collected).collect();
        for route in plan.routes {
            if route.lambda {
                for position in route.sources {
                    destinations[position] = Operand::Tactic;
                }
            } else if !route.collected {
                let ty = declaration_inputs::copy_input_domain(
                    &parameters[route.parameter].ty,
                    &mut self.required_steps,
                    callee.span,
                )?;
                destinations[route.sources[0]] = Operand::Parameter(route.parameter, ty);
            }
        }
        let mut inputs = vec![None; decl.params.len()];
        for (arg, destination) in arguments.iter().zip(destinations) {
            if matches!(destination, Operand::Tactic) {
                // Its source may be wrapped in a delayed body by call checking.
                if !self.observing_call_results.is_empty() {
                    return Ok(None);
                }
                continue;
            }
            let (Arg::Named(_, arg) | Arg::Positional(arg)) = arg else {
                return Ok(None);
            };
            let ty = match &destination {
                Operand::Parameter(_, ty) => Some(ty),
                _ => None,
            };
            let value = match &mut caller {
                Caller::Function(flow) => match flow.expr(self, arg, ty, depth + 1)? {
                    Paths::Closed => return Ok(Some(Paths::Closed)),
                    normal => normal.known(),
                },
                Caller::Source(bindings) => {
                    let value = match ty {
                        Some(ty) => {
                            self.generator_static_value_hint(arg, ty, depth + 1, *bindings)?
                        }
                        None => None,
                    };
                    if value.is_none()
                        && !self.observing_call_results.is_empty()
                        && !direct_read(arg)
                    {
                        return Ok(None);
                    }
                    value
                }
            };
            if let Operand::Parameter(slot, ty) = destination {
                inputs[slot] = value.map(|value| (ty, value));
            }
        }
        // Defaults and returns inside the called body belong to that call.
        // A discarded or recursive result need not be known to account for
        // lexical returns that have already occurred in its explicit inputs.
        let Some(domain) = observed_domain else {
            return Ok(Some(Paths::Unknown));
        };
        if self.observing_call_results.contains(&owner) {
            return Ok(Some(Paths::Unknown));
        }
        self.observing_call_results.insert(owner);
        let result = self.in_formal_declaration(owner, |this| {
            this.in_required_comptime = true;
            // Defaults bind only their declaration prefix. Even an unknown
            // explicit operand suppresses its default at this call site.
            for ((parameter, assigned), explicit) in
                decl.params.iter().zip(plan.assigned).zip(inputs)
            {
                let evidence = if assigned {
                    explicit
                } else if let TyParamKind::Optional(_, annotation, default) = parameter.ty.kind {
                    let Some(ty) =
                        this.generator_static_type_hint(annotation, depth + 1, FORMALS)?
                    else {
                        continue;
                    };
                    this.generator_static_value_hint(default, &ty, depth + 1, FORMALS)?
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
            control::result(this, body, domain, depth + 1)
        });
        self.observing_call_results.remove(&owner);
        result.map(Some)
    }
}
