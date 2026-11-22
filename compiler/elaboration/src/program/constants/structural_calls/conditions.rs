//! Track the store on each normal exit of a checked boolean expression.
use super::*;

#[derive(Default)]
pub(super) struct ConditionPaths {
    pub(super) when_true: Option<ObservationScope>,
    pub(super) when_false: Option<ObservationScope>,
}

impl ConditionPaths {
    pub(super) fn leave_scope(
        &mut self,
        outer: &ObservationScope,
        fuel: &mut usize,
    ) -> Result<(), String> {
        for exit in self.when_true.iter_mut().chain(self.when_false.iter_mut()) {
            exit.retain(|id| outer.values.contains_key(id), fuel)?;
        }
        Ok(())
    }

    fn path(&mut self, value: bool) -> &mut Option<ObservationScope> {
        if value {
            &mut self.when_true
        } else {
            &mut self.when_false
        }
    }
}

pub(super) enum Logical<'a> {
    Not,
    Right { when: bool, expr: &'a n::Expr },
}

pub(super) fn logical_operand(expr: &n::Expr) -> Option<(&n::Expr, Logical<'_>)> {
    let (left, operation) = match &expr.kind {
        K::Not(value) => (value, Logical::Not),
        K::And(left, right) => (
            left,
            Logical::Right {
                when: true,
                expr: right,
            },
        ),
        K::Or(left, right) => (
            left,
            Logical::Right {
                when: false,
                expr: right,
            },
        ),
        _ => return None,
    };
    // Bit-vector operators evaluate both operands.
    (matches!(left.ty, Term::Primitive(P::Bool | P::Never))
        || (matches!(left.ty, Term::AssociatedType { .. }) && expr.ty == Term::Primitive(P::Bool)))
    .then_some((left, operation))
}

enum Pending<'a> {
    Operator(Logical<'a>),
    Skipped {
        value: bool,
        scope: ObservationScope,
    },
}

impl Checker<'_, '_> {
    pub(super) fn merge_observed_scope(
        &mut self,
        destination: &mut Option<ObservationScope>,
        incoming: Option<ObservationScope>,
        span: Span,
    ) -> Result<(), Error> {
        if let Some(frame) = incoming {
            if let Some(existing) = destination {
                self.schema_fuel(span, |this, fuel| {
                    existing.join(&frame, this.protocols.meta_list, fuel)
                })?;
            } else {
                *destination = Some(frame);
            }
        }
        Ok(())
    }

    pub(super) fn observe_boolean_value(
        &mut self,
        expr: &n::Expr,
        scope: &mut ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Completion, Error> {
        let Some(mut paths) =
            self.observe_condition(expr, std::mem::take(scope), returns, span, depth)?
        else {
            return Ok(Completion::Unavailable);
        };
        let value = match (paths.when_true.is_some(), paths.when_false.is_some()) {
            (true, true) => PartialStagedValue::unknown(),
            (false, false) => return Ok(Completion::Returned),
            (yes, _) => PartialStagedValue::known(Term::Bool(yes)),
        };
        self.merge_observed_scope(&mut paths.when_true, paths.when_false, span)?;
        *scope = paths.when_true.unwrap();
        Ok(Completion::Next(value))
    }

    pub(super) fn observe_factory_if(
        &mut self,
        condition: &n::Expr,
        yes: &n::Expr,
        no: Option<&n::Expr>,
        scope: &mut ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Completion, Error> {
        let Some(paths) =
            self.observe_condition(condition, std::mem::take(scope), returns, span, depth)?
        else {
            return Ok(Completion::Unavailable);
        };
        let mut result = Completion::Returned;
        for (reached, branch) in [(paths.when_true, Some(yes)), (paths.when_false, no)] {
            if let Some(mut frame) = reached {
                let exit = match branch {
                    Some(branch) => {
                        self.observe_factory_expr(branch, &mut frame, returns, span, depth)?
                    }
                    None => Completion::unit(),
                };
                result = self.merge_factory_paths(scope, result, frame, exit, span)?;
                if matches!(result, Completion::Unavailable) {
                    break;
                }
            }
        }
        Ok(result)
    }

    pub(super) fn observe_condition(
        &mut self,
        mut expr: &n::Expr,
        mut input: ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Option<ConditionPaths>, Error> {
        if depth >= 64 {
            return Ok(None);
        }
        let mut pending = Vec::new();
        'operand: loop {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "boolean observation budget exhausted"))?;
            while let Some((left, operation)) = logical_operand(expr) {
                self.required_steps = self
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "boolean observation budget exhausted"))?;
                pending.push(Pending::Operator(operation));
                expr = left;
            }
            let Some(mut outcome) =
                self.observe_condition_leaf(expr, input, returns, span, depth)?
            else {
                return Ok(None);
            };
            while let Some(step) = pending.pop() {
                match step {
                    Pending::Operator(Logical::Not) => {
                        std::mem::swap(&mut outcome.when_true, &mut outcome.when_false)
                    }
                    Pending::Operator(Logical::Right { when, expr: right }) => {
                        if let Some(frame) = outcome.path(when).take() {
                            if let Some(skipped) = outcome.path(!when).take() {
                                pending.push(Pending::Skipped {
                                    value: !when,
                                    scope: skipped,
                                });
                            }
                            input = frame;
                            expr = right;
                            continue 'operand;
                        }
                    }
                    Pending::Skipped { value, scope } => {
                        self.merge_observed_scope(outcome.path(value), Some(scope), span)?
                    }
                }
            }
            return Ok(Some(outcome));
        }
    }

    fn observe_condition_leaf(
        &mut self,
        expr: &n::Expr,
        mut frame: ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Option<ConditionPaths>, Error> {
        match &expr.kind {
            K::Match { value, arms } => {
                match self.observe_factory_expr(value, &mut frame, returns, span, depth + 1)? {
                    Completion::Next(subject) => self.observe_predicate_match(
                        &subject,
                        arms,
                        &frame,
                        returns,
                        span,
                        depth + 1,
                    ),
                    Completion::Returned => Ok(Some(ConditionPaths::default())),
                    Completion::Unavailable => Ok(None),
                }
            }
            K::Call { .. } => self.observe_call_condition(expr, frame, returns, span, depth + 1),
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let mut inner = self.schema_fuel(span, |_, fuel| frame.fork(fuel))?;
                for statement in statements {
                    match self.observe_factory_expr(
                        statement,
                        &mut inner,
                        returns,
                        span,
                        depth + 1,
                    )? {
                        Completion::Next(_) => {}
                        Completion::Returned => return Ok(Some(ConditionPaths::default())),
                        Completion::Unavailable => return Ok(None),
                    }
                }
                let Some(tail) = result else { return Ok(None) };
                let Some(mut paths) =
                    self.observe_condition(tail, inner, returns, span, depth + 1)?
                else {
                    return Ok(None);
                };
                self.schema_fuel(span, |_, fuel| paths.leave_scope(&frame, fuel))?;
                Ok(Some(paths))
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let Some(paths) =
                    self.observe_condition(condition, frame, returns, span, depth + 1)?
                else {
                    return Ok(None);
                };
                let mut result = ConditionPaths::default();
                for (path, branch) in [
                    (paths.when_true, Some(then_branch.as_ref())),
                    (paths.when_false, else_branch.as_deref()),
                ] {
                    let Some(scope) = path else { continue };
                    let Some(branch) = branch else {
                        return Ok(None);
                    };
                    let Some(branches) =
                        self.observe_condition(branch, scope, returns, span, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    self.merge_observed_scope(&mut result.when_true, branches.when_true, span)?;
                    self.merge_observed_scope(&mut result.when_false, branches.when_false, span)?;
                }
                Ok(Some(result))
            }
            _ => {
                let mut paths = ConditionPaths::default();
                match self.observe_factory_expr(expr, &mut frame, returns, span, depth)? {
                    Completion::Unavailable => return Ok(None),
                    Completion::Returned => {}
                    Completion::Next(value) => {
                        if let Some(Term::Bool(known)) = value.known_term() {
                            *paths.path(*known) = Some(frame);
                        } else {
                            let mut other = self.schema_fuel(span, |_, fuel| frame.fork(fuel))?;
                            self.schema_fuel(span, |this, fuel| {
                                if let Some(place) = structural_place(expr, fuel)? {
                                    for (path, truth) in [(&mut frame, true), (&mut other, false)] {
                                        update_place(
                                            path,
                                            &place,
                                            PartialStagedValue::known(Term::Bool(truth)),
                                            this.protocols.meta_list,
                                            fuel,
                                        )?;
                                    }
                                }
                                Ok(())
                            })?;
                            paths.when_false = Some(other);
                            paths.when_true = Some(frame);
                        }
                    }
                }
                Ok(Some(paths))
            }
        }
    }
}
