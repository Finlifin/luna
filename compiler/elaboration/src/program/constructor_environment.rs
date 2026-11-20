//! Acquire runtime inputs before the payload of a nominal constructor.
use super::*;
use runtime_environment::Acquisition;

impl Checker<'_, '_> {
    pub(super) fn with_constructor_inputs(
        &mut self,
        head: &Expr<'_>,
        ty: Term,
        span: Span,
        build: impl FnOnce(&mut Self, Term) -> Result<n::Expr, Error>,
    ) -> Result<n::Expr, Error> {
        let outer = self.locals.clone();
        let result = (|| {
            let acquisition = self.acquire_constructor_inputs(head, &ty)?;
            let value = build(self, ty)?;
            self.finish_constructor_inputs(acquisition, value, span)
        })();
        self.locals = outer;
        result
    }

    pub(super) fn finish_constructor_inputs(
        &mut self,
        acquisition: Acquisition,
        value: n::Expr,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if acquisition.0.is_empty() {
            return Ok(value);
        }
        let region = acquisition.finish(self, value, span)?;
        self.local_runtime_origins
            .regions
            .insert(region.source, runtime_origins::Region::Constructor);
        Ok(region)
    }

    /// A computed quote argument belongs to the construction that evaluates it.
    /// Keep its identity in the type and deliver the value through a lexical let.
    pub(super) fn acquire_constructor_inputs(
        &mut self,
        head: &Expr<'_>,
        ty: &Term,
    ) -> Result<Acquisition, Error> {
        if self.local_runtime_origins.sources.is_empty()
            && self.local_runtime_origins.defaults.is_empty()
        {
            return Ok(Acquisition(Vec::new()));
        }
        let mut statements = Vec::new();
        // A variant or group qualifies its nominal head with member projections.
        let mut subject = head;
        while let H::Projection(base, _) = subject.kind {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(head.span, "constructor head inspection budget exhausted"))?;
            subject = base;
        }
        let args = match subject.kind {
            H::NFApplication(_, args) => args,
            _ => &[],
        };
        self.required_steps = self
            .required_steps
            .checked_sub(args.len() as u64)
            .ok_or_else(|| error(head.span, "constructor input inspection budget exhausted"))?;

        for arg in args {
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
                .acquired_bindings
                .insert(binding, id);
            self.local_runtime_deliveries.get_mut(&id).unwrap().expr = operand;
        }
        if !self.local_runtime_origins.defaults.is_empty() {
            for id in self.result_dependencies(ty, head.span)? {
                if self.local_runtime_origins.defaults.contains_key(&id) {
                    statements.push(self.acquire_runtime_default(id, |_| None, head.span)?);
                }
            }
        }
        Ok(Acquisition(statements))
    }

    /// Transfer acquired bindings to the value's consumer. Nominal
    /// construction and call preparations have distinct cleanup scopes.
    pub(super) fn take_value_environment(
        &mut self,
        value: &mut n::Expr,
        span: Span,
    ) -> Result<Vec<n::Expr>, Error> {
        if self.local_runtime_origins.acquired_bindings.is_empty() {
            return Ok(Vec::new());
        }
        if self.is_prepared_call_region(value) {
            return self.take_returned_environment(value, span);
        }
        let K::Block { statements, result } = &mut value.kind else {
            return Ok(Vec::new());
        };
        if result.is_none()
            || self.local_runtime_origins.regions.get(&value.source)
                != Some(&runtime_origins::Region::Constructor)
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
                .acquired_bindings
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
