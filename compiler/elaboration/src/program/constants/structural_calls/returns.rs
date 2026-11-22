//! Observe a callee return together with its reference-receiver snapshot.
use super::conditions::ConditionPaths;
use super::*;

pub(super) struct FactoryExit {
    pub(super) value: Arc<PartialStagedValue>,
    pub(super) receiver: Option<Arc<PartialStagedValue>>,
}

pub(super) struct Returns {
    receiver: Option<n::LocalId>,
    values: ReturnValues,
}

pub(super) enum ReturnValues {
    Value(Option<FactoryExit>),
    Predicate {
        when_true: Option<FactoryExit>,
        when_false: Option<FactoryExit>,
    },
}

impl FactoryExit {
    fn merge(
        destination: &mut Option<Self>,
        receiver_id: Option<n::LocalId>,
        value: Arc<PartialStagedValue>,
        scope: &ObservationScope,
        meta_list: Option<DeclarationId>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        let receiver = receiver_id.map(|id| {
            scope
                .values
                .get(&id)
                .cloned()
                .unwrap_or_else(PartialStagedValue::unknown)
        });
        if let Some(exit) = destination {
            exit.value = PartialStagedValue::common(&exit.value, &value, meta_list, 0, fuel)?;
            if let (Some(previous), Some(current)) = (&mut exit.receiver, receiver) {
                *previous = PartialStagedValue::common(previous, &current, meta_list, 0, fuel)?;
            }
        } else {
            *destination = Some(FactoryExit { value, receiver });
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
        demand: ReturnValues,
        span: Span,
        depth: usize,
    ) -> Result<Option<ReturnValues>, Error> {
        let mut returns = Some(Returns {
            receiver,
            values: demand,
        });
        let completion = self.observe_return_value(Some(body), frame, &mut returns, span, depth)?;
        Ok(match completion {
            Completion::Returned => returns.map(|r| r.values),
            _ => None,
        })
    }

    // The implicit tail and every explicit return deliver to the same caller.
    pub(super) fn observe_return_value(
        &mut self,
        expr: Option<&n::Expr>,
        scope: &mut ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Completion, Error> {
        let Some(active) = returns.as_ref() else {
            return Ok(Completion::Unavailable);
        };
        if matches!(active.values, ReturnValues::Predicate { .. }) {
            let Some(expr) = expr else {
                return Ok(Completion::Unavailable);
            };
            let Some(ConditionPaths {
                when_true,
                when_false,
            }) = self.observe_condition(expr, std::mem::take(scope), returns, span, depth)?
            else {
                return Ok(Completion::Unavailable);
            };
            let returned = returns.as_mut().expect("active predicate invocation");
            let ReturnValues::Predicate {
                when_true: yes,
                when_false: no,
            } = &mut returned.values
            else {
                unreachable!()
            };
            self.schema_fuel(span, |this, fuel| {
                for (exit, value, scope) in [(yes, true, when_true), (no, false, when_false)] {
                    if let Some(scope) = scope {
                        FactoryExit::merge(
                            exit,
                            returned.receiver,
                            PartialStagedValue::known(Term::Bool(value)),
                            &scope,
                            this.protocols.meta_list,
                            fuel,
                        )?;
                    }
                }
                Ok(())
            })?;
        } else {
            let step = match expr {
                Some(expr) => self.observe_region(expr, scope, returns, span, depth)?,
                None => Completion::unit(),
            };
            let Completion::Next(value) = step else {
                return Ok(step);
            };
            let Some(Returns {
                receiver,
                values: ReturnValues::Value(exit),
            }) = returns
            else {
                return Ok(Completion::Unavailable);
            };
            self.schema_fuel(span, |this, fuel| {
                FactoryExit::merge(
                    exit,
                    *receiver,
                    value,
                    scope,
                    this.protocols.meta_list,
                    fuel,
                )
            })?;
        }
        Ok(Completion::Returned)
    }
}
