//! Acquire explicit computed quote operands before record fields.
use super::*;
use runtime_environment::Acquisition;

impl Checker<'_, '_> {
    /// A computed quote argument belongs to the construction that evaluates it.
    /// Keep its identity in the type and deliver the value through a lexical let.
    pub(super) fn acquire_constructor_inputs(
        &mut self,
        head: &Expr<'_>,
        ty: &Term,
    ) -> Result<Acquisition, Error> {
        let mut statements = Vec::new();
        let H::NFApplication(_, args) = &head.kind else {
            return Ok(Acquisition(statements));
        };
        self.required_steps = self
            .required_steps
            .checked_sub(args.len() as u64)
            .ok_or_else(|| error(head.span, "constructor input inspection budget exhausted"))?;
        let mut captured = BTreeSet::new();
        for arg in *args {
            let (Arg::Positional(value)
            | Arg::Named(_, value)
            | Arg::Expand(value)
            | Arg::Implicit(value)
            | Arg::PostLambda(value)) = arg;
            let Some(Term::Runtime(id)) = self.local_runtime_origins.sources.get(&value.hir_id)
            else {
                continue;
            };
            let id = *id;
            // Stable storage and forwarded environments already have delivery.
            let Some(entry) = self.local_runtime_deliveries.get(&id) else {
                continue;
            };
            if entry.producer_site != Some(local(value.hir_id))
                || matches!(entry.expr.kind, K::Local(_))
            {
                continue;
            }
            let domain = declaration_inputs::copy_input_domain(
                &entry.expr.ty,
                &mut self.required_steps,
                value.span,
            )?;
            let mut operand =
                self.deliver_runtime_dependency(&Term::Runtime(id), &domain, value.span)?;
            statements.push(self.prepare_call_operand(&mut operand, value.span)?);
            let K::Local(binding) = operand.kind else {
                unreachable!("prepared operand")
            };
            self.locals.insert(binding, domain);
            self.immutable.insert(binding);
            self.local_runtime_origins
                .constructor_bindings
                .insert(binding, ());
            self.local_runtime_deliveries.get_mut(&id).unwrap().expr = operand;
            captured.insert(id);
        }
        if !captured.is_empty()
            && let Term::Nominal { arguments, .. } = ty
        {
            self.required_steps = self
                .required_steps
                .checked_sub(arguments.len() as u64)
                .ok_or_else(|| {
                    error(
                        head.span,
                        "constructor projection inspection budget exhausted",
                    )
                })?;
            for arg in arguments {
                let Term::Runtime(id) = arg else { continue };
                let Some(entry) = self.local_runtime_deliveries.get(id) else {
                    continue;
                };
                let Some(path) = entry.projection.as_ref() else {
                    continue;
                };
                if captured.contains(&path.root) {
                    let root = &self.local_runtime_deliveries[&path.root].expr;
                    constants::charge_copy(root, &mut self.required_steps, head.span)?;
                    let base = root.clone();
                    let placed =
                        path.apply(&entry.expr, base, &mut self.required_steps, head.span)?;
                    self.local_runtime_deliveries.get_mut(id).unwrap().expr = placed;
                }
            }
        }
        Ok(Acquisition(statements))
    }

    /// Transfer a constructor's preparations to its consumer. Only lets owned
    /// by the constructor can cross this boundary. User blocks stay lexical.
    pub(super) fn take_constructor_environment(
        &mut self,
        value: &mut n::Expr,
        span: Span,
    ) -> Result<Vec<n::Expr>, Error> {
        let K::Block { statements, result } = &mut value.kind else {
            return Ok(Vec::new());
        };
        if statements.is_empty()
            || !result
                .as_ref()
                .is_some_and(|e| matches!(e.kind, K::Record { .. }))
        {
            return Ok(Vec::new());
        }
        for statement in statements.iter() {
            self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                error(span, "constructor environment inspection budget exhausted")
            })?;
            let K::Let { binding, .. } = statement.kind else {
                return Ok(Vec::new());
            };
            if !self
                .local_runtime_origins
                .constructor_bindings
                .contains_key(&binding)
            {
                return Ok(Vec::new());
            }
        }
        for statement in statements.iter() {
            if let K::Let { binding, value } = &statement.kind {
                let ty = declaration_inputs::copy_input_domain(
                    &value.ty,
                    &mut self.required_steps,
                    span,
                )?;
                self.locals.insert(*binding, ty);
            }
        }
        let prefix = std::mem::take(statements);
        *value = *result.take().unwrap();
        Ok(prefix)
    }
}
