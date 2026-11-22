//! Abstract control and local writes within a checked factory invocation.
use super::*;

pub(super) enum Completion<T = Arc<PartialStagedValue>> {
    Next(T),
    Returned,
    Unavailable,
}

impl Completion {
    pub(super) fn unit() -> Self {
        Self::Next(PartialStagedValue::known(Term::Unit))
    }

    fn observed(value: Option<Arc<PartialStagedValue>>) -> Self {
        value.map_or(Self::Unavailable, Self::Next)
    }
}

impl Checker<'_, '_> {
    pub(in crate::program::constants) fn structural_value(
        &mut self,
        expr: &n::Expr,
        scope: &mut ObservationScope,
        span: Span,
        depth: usize,
    ) -> Result<Option<Arc<PartialStagedValue>>, Error> {
        Ok(
            match self.observe_factory_expr(expr, scope, &mut None, span, depth)? {
                Completion::Next(value) => Some(value),
                Completion::Returned | Completion::Unavailable => None,
            },
        )
    }

    pub(super) fn observe_factory_expr(
        &mut self,
        expr: &n::Expr,
        scope: &mut ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Completion, Error> {
        if depth >= 64 {
            return Ok(Completion::Unavailable);
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "structural call observation budget exhausted"))?;
        match &expr.kind {
            K::Block { .. } | K::CallPreparation { .. } => {
                let mut inner = self.schema_fuel(span, |_, fuel| scope.fork(fuel))?;
                let result = self.observe_region(expr, &mut inner, returns, span, depth + 1)?;
                if matches!(result, Completion::Next(_)) {
                    self.schema_fuel(span, |_, fuel| {
                        inner.retain(|id| scope.values.contains_key(id), fuel)
                    })?;
                    *scope = inner;
                }
                return Ok(result);
            }
            K::Return(result) => {
                return self.observe_return_value(
                    result.as_deref(),
                    scope,
                    returns,
                    span,
                    depth + 1,
                );
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                return self.observe_factory_if(
                    condition,
                    then_branch,
                    else_branch.as_deref(),
                    scope,
                    returns,
                    span,
                    depth + 1,
                );
            }
            _ if super::conditions::logical_operand(expr).is_some() => {
                return self.observe_boolean_value(expr, scope, returns, span, depth);
            }
            K::Not(left) if matches!(left.ty, Term::Primitive(P::BitVec(_))) => {
                return self.observe_bit_operands(
                    &expr.kind,
                    left,
                    None,
                    scope,
                    returns,
                    span,
                    depth + 1,
                );
            }
            K::And(left, right)
            | K::Or(left, right)
            | K::BitEqual(left, right)
            | K::BitConcat(left, right)
                if matches!(left.ty, Term::Primitive(P::BitVec(_))) =>
            {
                return self.observe_bit_operands(
                    &expr.kind,
                    left,
                    Some(right),
                    scope,
                    returns,
                    span,
                    depth + 1,
                );
            }
            K::Match { value, arms } => {
                let test = self.observe_factory_expr(value, scope, returns, span, depth + 1)?;
                let Completion::Next(subject) = test else {
                    return Ok(test);
                };
                return self.observe_factory_match(&subject, arms, scope, returns, span, depth + 1);
            }
            K::Let { binding, value } => {
                let result = self.observe_factory_expr(value, scope, returns, span, depth + 1)?;
                let Completion::Next(value) = result else {
                    return Ok(result);
                };
                self.schema_fuel(span, |_, fuel| scope.insert(*binding, value, fuel))?;
                return Ok(Completion::unit());
            }
            K::Assign { place, value } => {
                let result = self.observe_factory_expr(value, scope, returns, span, depth + 1)?;
                let Completion::Next(value) = result else {
                    return Ok(result);
                };
                let updated = self.schema_fuel(span, |this, fuel| {
                    update_place(scope, place, value, this.protocols.meta_list, fuel)
                })?;
                return Ok(if updated {
                    Completion::unit()
                } else {
                    Completion::Unavailable
                });
            }
            K::Discard(value) => {
                let result = self.observe_factory_expr(value, scope, returns, span, depth + 1)?;
                return Ok(match result {
                    Completion::Next(_) => Completion::unit(),
                    other => other,
                });
            }
            K::Field { .. } | K::AnonymousField { .. } => {
                // Keep a long projection chain off the host stack and evaluate
                // its producer once, even when it returns from this factory.
                let mut base = expr;
                let mut fields = Vec::new();
                while let K::Field { value, .. } | K::AnonymousField { value, .. } = &base.kind {
                    self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                        error(span, "structural field observation budget exhausted")
                    })?;
                    fields.push(&base.kind);
                    base = value;
                }
                let producer = self.observe_factory_expr(base, scope, returns, span, depth + 1)?;
                let Completion::Next(mut value) = producer else {
                    return Ok(producer);
                };
                return self.schema_fuel(span, |_, fuel| {
                    for field in fields.into_iter().rev() {
                        value = value.project_field(field, fuel)?;
                    }
                    Ok(Completion::Next(value))
                });
            }
            K::BuildStruct(packet) => {
                return Ok(Completion::observed(
                    self.observe_built_struct(packet, scope, span)?,
                ));
            }
            K::ComptimeFunctionInstance {
                declaration,
                inputs,
            } => {
                return self.observe_factory_instance(
                    *declaration,
                    inputs,
                    scope,
                    returns,
                    span,
                    depth + 1,
                );
            }
            K::Call { .. } => {
                return self.observe_factory_call(expr, Some(scope), returns, span, depth + 1);
            }
            _ => {}
        }
        if let Some(value) = self.schema_fuel(span, |this, fuel| {
            if !structural_operand(expr, Some(scope), fuel)? {
                return Ok(None);
            }
            this.partial_staged_value_in(expr, 0, fuel, scope).map(Some)
        })? {
            return Ok(Completion::Next(value));
        }
        let Some((shape, fields)) =
            self.schema_fuel(span, |this, fuel| this.staged_aggregate_parts(expr, fuel))?
        else {
            return Ok(Completion::Unavailable);
        };
        let mut observed = Vec::new();
        for field in fields {
            let result = self.observe_factory_expr(field, scope, returns, span, depth + 1)?;
            let Completion::Next(value) = result else {
                return Ok(result);
            };
            observed.push(value);
        }
        Ok(Completion::Next(PartialStagedValue::from_fields(
            shape, observed,
        )))
    }

    pub(super) fn merge_factory_paths(
        &mut self,
        scope: &mut ObservationScope,
        left: Completion,
        alternative: ObservationScope,
        right: Completion,
        span: Span,
    ) -> Result<Completion, Error> {
        match (left, right) {
            (_, Completion::Unavailable) | (Completion::Unavailable, _) => {
                Ok(Completion::Unavailable)
            }
            (Completion::Returned, other) => {
                *scope = alternative;
                Ok(other)
            }
            (other, Completion::Returned) => Ok(other),
            (Completion::Next(left), Completion::Next(right)) => {
                self.schema_fuel(span, |this, fuel| {
                    scope.join(&alternative, this.protocols.meta_list, fuel)?;
                    PartialStagedValue::common(&left, &right, this.protocols.meta_list, 0, fuel)
                        .map(Completion::Next)
                })
            }
        }
    }

    pub(super) fn observe_region(
        &mut self,
        mut expr: &n::Expr,
        scope: &mut ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Completion, Error> {
        while let K::Block { statements, result } | K::CallPreparation { statements, result } =
            &expr.kind
        {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "structural call observation budget exhausted"))?;
            for statement in statements {
                let result = self.observe_factory_expr(statement, scope, returns, span, depth)?;
                if !matches!(result, Completion::Next(_)) {
                    return Ok(result);
                }
            }
            let Some(tail) = result else {
                return Ok(Completion::unit());
            };
            expr = tail;
        }
        self.observe_factory_expr(expr, scope, returns, span, depth)
    }
}
