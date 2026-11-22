//! Abstract control within one checked factory invocation. Only the return
//! value and its reference receiver can be observed by the calling frame.
use super::*;

pub(super) struct FactoryExit {
    pub(super) value: Arc<PartialStagedValue>,
    pub(super) receiver: Option<Arc<PartialStagedValue>>,
}

pub(super) struct Returns {
    receiver: Option<n::LocalId>,
    exit: Option<FactoryExit>,
}

pub(super) enum Completion {
    Next(Arc<PartialStagedValue>),
    Returned,
    Unavailable,
}

impl Completion {
    fn unit() -> Self {
        Self::Next(PartialStagedValue::known(Term::Unit))
    }

    fn observed(value: Option<Arc<PartialStagedValue>>) -> Self {
        value.map_or(Self::Unavailable, Self::Next)
    }
}

impl Returns {
    fn record(
        &mut self,
        value: Arc<PartialStagedValue>,
        scope: &ObservationScope,
        meta_list: Option<DeclarationId>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        let receiver = self.receiver.map(|id| {
            scope
                .values
                .get(&id)
                .cloned()
                .unwrap_or_else(PartialStagedValue::unknown)
        });
        if let Some(exit) = &mut self.exit {
            exit.value = PartialStagedValue::common(&exit.value, &value, meta_list, 0, fuel)?;
            if let (Some(previous), Some(current)) = (&mut exit.receiver, receiver) {
                *previous = PartialStagedValue::common(previous, &current, meta_list, 0, fuel)?;
            }
        } else {
            self.exit = Some(FactoryExit { value, receiver });
        }
        Ok(())
    }
}

impl Checker<'_, '_> {
    pub(super) fn observe_factory_body(
        &mut self,
        body: &n::Expr,
        frame: &mut ObservationScope,
        receiver: Option<n::LocalId>,
        span: Span,
        depth: usize,
    ) -> Result<Option<FactoryExit>, Error> {
        let mut returns = Some(Returns {
            receiver,
            exit: None,
        });
        match self.observe_region(body, frame, &mut returns, span, depth)? {
            Completion::Next(value) => self.schema_fuel(span, |this, fuel| {
                returns
                    .as_mut()
                    .unwrap()
                    .record(value, frame, this.protocols.meta_list, fuel)
            })?,
            Completion::Returned => {}
            Completion::Unavailable => return Ok(None),
        }
        Ok(returns.and_then(|returns| returns.exit))
    }

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
                if returns.is_none() {
                    return Ok(Completion::Unavailable);
                }
                let result = match result {
                    Some(expr) => {
                        self.observe_factory_expr(expr, scope, returns, span, depth + 1)?
                    }
                    None => Completion::unit(),
                };
                if let Completion::Next(value) = result {
                    self.schema_fuel(span, |this, fuel| {
                        returns.as_mut().unwrap().record(
                            value,
                            scope,
                            this.protocols.meta_list,
                            fuel,
                        )
                    })?;
                    return Ok(Completion::Returned);
                }
                return Ok(result);
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let test = self.observe_factory_expr(condition, scope, returns, span, depth + 1)?;
                let Completion::Next(test) = test else {
                    return Ok(test);
                };
                return self.observe_factory_branches(
                    test.known_term(),
                    then_branch,
                    else_branch.as_deref(),
                    scope,
                    returns,
                    span,
                    depth + 1,
                );
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

    fn observe_factory_branches(
        &mut self,
        test: Option<&Term>,
        yes: &n::Expr,
        no: Option<&n::Expr>,
        scope: &mut ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Completion, Error> {
        let otherwise = |this: &mut Self,
                         frame: &mut ObservationScope,
                         returns: &mut Option<Returns>| match no {
            Some(no) => this.observe_factory_expr(no, frame, returns, span, depth),
            None => Ok(Completion::unit()),
        };
        match test {
            Some(Term::Bool(true)) => {
                return self.observe_factory_expr(yes, scope, returns, span, depth);
            }
            Some(Term::Bool(false)) => return otherwise(self, scope, returns),
            _ => {}
        }
        let mut alternative = self.schema_fuel(span, |_, fuel| scope.fork(fuel))?;
        let left = self.observe_factory_expr(yes, scope, returns, span, depth)?;
        if matches!(left, Completion::Unavailable) {
            return Ok(left);
        }
        let right = otherwise(self, &mut alternative, returns)?;
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

    fn observe_region(
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
