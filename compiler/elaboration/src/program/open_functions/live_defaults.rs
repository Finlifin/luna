//! Mutable declaration slots across signature formation and the residual call.
use super::*;

// Rebindings in the executed prefix of a comptime call. Live values can
// share capture storage with each other and with delivered arguments.
pub(super) type LiveWrites = BTreeMap<n::LocalId, (Term, comptime_interpretor::Value)>;

impl Checker<'_, '_> {
    pub(super) fn prepare_open_live_suffix(
        &mut self,
        owner: OwnerId,
        sig: &hir::item::FnSig<'_>,
        parameters: &[Parameter],
        symbols: &[Option<Term>],
        writes: &mut LiveWrites,
        pending: impl Iterator<Item = usize>,
        span: Span,
    ) -> Result<BTreeMap<usize, n::Expr>, Error> {
        let mut pending = pending.peekable();
        let mut residuals = BTreeMap::new();
        if pending.peek().is_none() || writes.is_empty() {
            writes.clear();
            return Ok(residuals);
        }
        let slots: BTreeMap<_, _> = sig
            .params
            .iter()
            .enumerate()
            .map(|(slot, param)| (local(param.hir_id), slot))
            .collect();
        let mut symbols = symbols.to_vec();
        for (id, (_, value)) in writes.iter() {
            symbols[slots[id]] = self.optional_normal_term(value.clone(), span)?;
        }
        let mut referenced = BTreeSet::new();
        for slot in pending {
            let (default, _) = self.prepare_open_default(
                owner,
                slot,
                parameters,
                &symbols,
                &mut BTreeMap::new(),
                false,
                span,
            )?;
            let access = self.with_observation_budget(span, |_, budget| {
                Ok(nir::free_bindings::access(
                    &default,
                    |id| slots.contains_key(&id),
                    &mut budget.steps,
                )?)
            })?;
            referenced.extend(access.readonly);
            for id in access.writable {
                // The suffix executes later. Its entry evidence cannot cross
                // a possible write while checking another default.
                symbols[slots[&id]] = None;
                referenced.insert(id);
            }
            residuals.insert(slot, default);
        }
        writes.retain(|id, _| referenced.contains(id));
        Ok(residuals)
    }

    /// Resume the unexecuted suffix with the actual mutable preparation
    /// slots. Graph initializers live in the caller alongside the arguments
    /// from which they can share storage.
    pub(super) fn resume_open_default_frame(
        &mut self,
        expression: &mut n::Expr,
        mut writes: BTreeMap<n::LocalId, n::Expr>,
        span: Span,
    ) -> Result<(), Error> {
        let mut call = expression;
        while !matches!(call.kind, K::Call { .. }) {
            match &mut call.kind {
                K::CallPreparation {
                    result: Some(tail), ..
                }
                | K::Block {
                    result: Some(tail), ..
                } => {
                    call = tail;
                }
                _ => return Err(error(span, "missing prepared comptime call")),
            }
        }
        let K::Call {
            arguments,
            parameter_bindings,
            ..
        } = &mut call.kind
        else {
            unreachable!()
        };
        let first = arguments
            .iter()
            .position(|arg| arg.is_default)
            .ok_or_else(|| error(span, "missing residual comptime defaults"))?;
        let (supplied, defaults) = arguments.split_at_mut(first);
        let mut statements = Vec::new();
        // Complete caller-owned operands before installing any declaration
        // locals, even for a recursive call with the same parameter LocalIds.
        for arg in supplied.iter_mut() {
            statements.push(self.prepare_call_operand(&mut arg.value, span)?);
        }
        let bind = |binding, value: n::Expr| n::Expr {
            source: value.source,
            ty: p(P::Void),
            kind: K::Let {
                binding,
                value: Box::new(value),
            },
        };
        for arg in supplied {
            let id = parameter_bindings[arg.parameter];
            let initial = writes.remove(&id).unwrap_or_else(|| arg.value.clone());
            statements.push(bind(id, initial));
        }
        if !writes.is_empty() {
            return Err(error(
                span,
                "default slot was not supplied before resumption",
            ));
        }
        for arg in defaults {
            statements.push(self.prepare_call_operand(&mut arg.value, span)?);
            statements.push(bind(parameter_bindings[arg.parameter], arg.value.clone()));
            arg.is_default = false;
        }
        let result = n::Expr {
            source: call.source,
            ty: call.ty.clone(),
            kind: std::mem::replace(&mut call.kind, K::Unit),
        };
        call.kind = K::Block {
            statements,
            result: Some(Box::new(result)),
        };
        Ok(())
    }

    pub(super) fn prepare_open_live_default(
        &mut self,
        owner: OwnerId,
        slot: usize,
        parameters: &[Parameter],
        symbols: &[Option<Term>],
        live: &[Option<comptime_interpretor::Value>],
        writes: &mut LiveWrites,
        span: Span,
        unresolved: &[DeclarationId],
    ) -> Result<Option<(comptime_interpretor::Value, Term)>, Error> {
        let assumptions = self.context.assumptions().to_vec();
        self.in_open_function_scope(owner, None, |this| {
            this.import_closed_caller_assumptions(assumptions);
            let ItemKind::Fn(sig, _) = &this.package.item(owner).unwrap().kind else {
                unreachable!()
            };
            let TyParamKind::Optional(_, _, default) = sig.params[slot].ty.kind else {
                return Err(error(
                    span,
                    "missing default for an unsupplied open parameter",
                ));
            };
            this.default_input_scope = Some(
                parameters
                    .iter()
                    .map(|p| (p.sort, Some(p.ty.clone())))
                    .collect(),
            );
            this.default_input_owner = Some(owner);
            this.in_staged_input = true;
            let mut graph = Vec::new();
            let mut domains = BTreeMap::new();
            for (index, parameter) in sig.params.iter().enumerate().take(slot) {
                let binding = local(parameter.hir_id);
                let domain = &parameters[index];
                this.locals.insert(binding, domain.ty.clone());
                if live[index].is_some() || symbols[index].is_some() {
                    domains.insert(binding, &domain.ty);
                }
                if let Some((_, value)) = writes.get(&binding) {
                    graph.push((binding, domain.ty.clone(), value.clone()));
                    if let Some(symbol) = this.optional_normal_term(value.clone(), default.span)? {
                        if domain.ty == p(P::Type) {
                            this.input_facts.types.insert(binding, symbol);
                        } else {
                            this.input_facts
                                .values
                                .insert(binding, (symbol, domain.ty.clone()));
                        }
                    }
                } else if let Some(value) = &live[index] {
                    graph.push((binding, domain.ty.clone(), value.clone()));
                } else if let Some(value) = &symbols[index] {
                    this.input_facts
                        .specialized
                        .push((binding, domain.ty.clone(), value.clone()));
                    if domain.ty == p(P::Type) {
                        this.input_facts.types.insert(binding, value.clone());
                    } else {
                        this.input_facts
                            .values
                            .insert(binding, (value.clone(), domain.ty.clone()));
                    }
                }
            }
            let (prepared, outputs) = this.prepare_required_root_with_live_inputs(
                default,
                required::RootKind::Expression,
                Vec::new(),
                false,
                &graph,
                |this| {
                    let value = this.expr(default, Some(&parameters[slot].ty))?;
                    if domains.is_empty() {
                        return Ok((value, None));
                    }
                    dependent_inputs::DefaultOutputs::prepare(
                        this,
                        value,
                        |id| domains.get(&id).copied(),
                        default.span,
                    )
                },
            )?;
            if !unresolved.is_empty()
                && this.open_computation_uses_clauses(&prepared, unresolved, default.span)?
            {
                return Ok(None);
            }
            let selected: BTreeSet<_> = prepared
                .inputs
                .iter()
                .map(|(binding, _, _)| *binding)
                .collect();
            let (value, ty) = this.invoke_required_publishing(
                prepared,
                graph
                    .into_iter()
                    .filter(|(binding, _, _)| selected.contains(binding))
                    .map(|(binding, _, value)| (binding, value))
                    .collect(),
                default.hir_id.owner,
                default.span,
            )?;
            if let Some(outputs) = outputs {
                let completed = outputs.unpack(value, default.span)?;
                for (id, domain, value) in completed.writes {
                    writes.insert(id, (domain, value));
                }
                Ok(Some(completed.result))
            } else {
                Ok(Some((value, ty)))
            }
        })
    }
}
