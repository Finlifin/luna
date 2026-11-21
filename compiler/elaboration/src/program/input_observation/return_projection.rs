//! A direct comptime call can forward an operand or a closed type binding.
//! Follow the resolved return expression without entering a VM or importing
//! the caller's local facts into the callee. The call remains executable.
use super::*;

impl<'h> Checker<'_, 'h> {
    /// Only signature substitution consumes this map. Admission still receives
    /// the original operands, including every invocation-time obligation.
    pub(in crate::program) fn signature_return_inputs(
        &mut self,
        signature: &Term,
        telescope: &declaration_inputs::PreparedTelescope,
        inputs: &declaration_inputs::PreparedDeclarationInputs,
        span: Span,
    ) -> Result<BTreeMap<DeclarationId, Term>, Error> {
        let plan = &inputs.plan;
        dependent_inputs::tasks::spend(&mut self.required_steps, inputs.arguments.len(), span)?;
        if !inputs.arguments.iter().any(|arg| {
            matches!(arg, n::NormalFormArgument::Value(e)
            if matches!(e.kind, K::Call { .. }) && e.ty == p(P::Type))
        }) {
            return Ok(BTreeMap::new());
        }
        let needed = input_observation::term(signature, &mut self.required_steps, span)?;
        if needed.is_empty() {
            return Ok(BTreeMap::new());
        }
        let mut live = BTreeMap::new();
        dependent_inputs::tasks::spend(&mut self.required_steps, plan.preparations.len(), span)?;
        for step in n::InputStep::in_order(
            plan.order.iter().copied(),
            plan.preparations.iter().map(|p| (p.before, &p.expression)),
        ) {
            match step {
                n::InputStep::Prepare(expr) => {
                    self.with_observation_budget(span, |_, budget| {
                        Ok(Self::invalidate_mutated_type_bindings(
                            expr,
                            &mut live,
                            &mut budget.steps,
                        )?)
                    })?;
                }
                n::InputStep::Supply(slot) => {
                    let arg = &inputs.arguments[slot];
                    self.with_observation_budget(span, |_, budget| {
                        Ok(nir::free_bindings::retain_unmodified_argument(
                            arg,
                            &mut live,
                            &mut budget.steps,
                        )?)
                    })?;
                    let (id, _, domain) = &telescope.local[slot];
                    if needed.contains(id)
                        && *domain == p(P::Type)
                        && let n::NormalFormArgument::Value(expr) = arg
                        && let Some(result) = self.projected_type_return(expr, span)?
                    {
                        live.insert(plan.bindings[slot], (*id, result));
                    }
                }
            }
        }
        Ok(live.into_values().collect())
    }

    pub(in crate::program) fn projected_type_return(
        &mut self,
        producer: &n::Expr,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if producer.ty != p(P::Type) {
            return Ok(None);
        }
        let K::Call {
            callee, arguments, ..
        } = &producer.kind
        else {
            return Ok(None);
        };
        let K::Function(id) = callee.kind else {
            return Ok(None);
        };
        let Some(item) = self.source_declarations.item(id) else {
            return Ok(None);
        };
        let ItemKind::Fn(sig, body) = &item.kind else {
            return Ok(None);
        };
        if !sig.modifiers.is_comptime
            || sig.modifiers.is_normal_form_head
            || sig.modifiers.is_quote
            || sig.modifiers.is_function_template
        {
            return Ok(None);
        }
        let Some(body) = self.package.body(*body).map(|body| body.value) else {
            return Ok(None);
        };
        let mut paths = vec![body];
        let mut result = None;
        while let Some(expr) = paths.pop() {
            dependent_inputs::tasks::spend(&mut self.required_steps, 1, span)?;
            let value = match &expr.kind {
                H::Block(block) if block.stmts.is_empty() => {
                    let Some(tail) = block.expr else {
                        return Ok(None);
                    };
                    paths.push(tail);
                    continue;
                }
                H::Return(Some(value)) => {
                    paths.push(value);
                    continue;
                }
                H::If(test, yes, Some(no)) => {
                    // An arbitrary test can itself return from the function.
                    // A plain formal read has no such exit even if its value
                    // is unknown. In that case both branches must agree.
                    let selected = match &test.kind {
                        H::Lit(hir::Lit {
                            kind: hir::LitKind::Bool(value),
                            ..
                        }) => Some(*value),
                        H::ResolvedFnParam(_, id) => {
                            match self.projected_argument_identity(sig, arguments, *id, span)? {
                                Some(Term::Bool(selector)) => Some(selector),
                                _ => None,
                            }
                        }
                        _ => return Ok(None),
                    };
                    if selected != Some(true) {
                        paths.push(no);
                    }
                    if selected != Some(false) {
                        let Some(value) = yes.expr.filter(|_| yes.stmts.is_empty()) else {
                            return Ok(None);
                        };
                        paths.push(value);
                    }
                    continue;
                }
                H::ResolvedFnParam(_, id) => {
                    let Some(identity) =
                        self.projected_argument_identity(sig, arguments, *id, span)?
                    else {
                        return Ok(None);
                    };
                    if identity.sort() != Sort::Type {
                        return Ok(None);
                    }
                    identity
                }
                _ => {
                    let Some(value) = self.closed_return_type(expr, span)? else {
                        return Ok(None);
                    };
                    value
                }
            };
            let agrees = self.with_observation_budget(span, |_, budget| {
                observation::optional(exits::merge_identity(&mut result, value, budget))
            })?;
            if agrees.is_none() {
                return Ok(None);
            }
        }
        Ok(result)
    }

    /// An ordinary operand is evaluated at its own source position. Only use
    /// facts established inside that producer: free reads may have been
    /// invalidated by an earlier argument, including a named argument.
    fn projected_argument_identity(
        &mut self,
        signature: &hir::item::FnSig<'_>,
        arguments: &[n::CallArgument],
        binding: hir::HirId,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let Some(input) = projected_argument(
            signature,
            arguments,
            binding,
            &mut self.required_steps,
            span,
        )?
        else {
            return Ok(None);
        };
        self.with_observation_budget(span, |checker, budget| {
            if !nir::free_bindings::collect(input, &mut budget.steps)?.is_empty() {
                return Ok(None);
            }
            observation::hint(checker.observe_preserved_input_in(
                input,
                BTreeMap::new(),
                BTreeSet::new(),
                budget,
            ))
        })
    }

    /// Follow transparent, parameterless alias references. A nominal without any
    /// declaration or capture inputs already has its closed identity here.
    fn closed_return_type(
        &mut self,
        mut expr: &'h Expr<'h>,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let mut visited = BTreeSet::new();
        loop {
            dependent_inputs::tasks::spend(&mut self.required_steps, 1, span)?;
            match expr.kind {
                H::ResolvedDef { def_id, .. } => {
                    let Some(owner) = self.package.definition_owner(def_id) else {
                        return Ok(None);
                    };
                    let Some(item) = self.package.item(owner) else {
                        return Ok(None);
                    };
                    let id = declaration(self.package, &self.package_identities, owner);
                    if source_declarations::nominal_kind(&item.kind).is_some()
                        && self.type_parameters.get(&owner).is_some_and(Vec::is_empty)
                        && !self.self_containers.contains_key(&owner)
                    {
                        return Ok(Some(Term::Nominal {
                            declaration: id,
                            arguments: Vec::new(),
                        }));
                    }
                    let ItemKind::TypeAlias(target, [], []) = &item.kind else {
                        return Ok(None);
                    };
                    if !visited.insert(owner) {
                        return Ok(None);
                    }
                    expr = target;
                }
                _ => {
                    return Ok(match self.atomic_type_data(expr)? {
                        Some(CompletedType::Identity(ty)) => Some(ty),
                        _ => None,
                    });
                }
            }
        }
    }
}

fn projected_argument<'a>(
    signature: &hir::item::FnSig<'_>,
    arguments: &'a [n::CallArgument],
    binding: hir::HirId,
    steps: &mut u64,
    span: Span,
) -> Result<Option<&'a n::Expr>, Error> {
    dependent_inputs::tasks::spend(
        steps,
        signature.params.len().saturating_add(arguments.len()),
        span,
    )?;
    let Some(slot) = signature.params.iter().position(|p| p.hir_id == binding) else {
        return Ok(None);
    };
    Ok(arguments
        .iter()
        .find(|arg| arg.parameter == slot && arg.passing == n::ArgumentPassing::Value)
        .map(|arg| &arg.value))
}
