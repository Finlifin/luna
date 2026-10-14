//! Invocation-local evaluation of contextual inputs before their Type arrives.
//! Literal keys point into boxed immutable arguments owned by that invocation.
use super::*;
use nir::program::NormalFormArgument as A;
use std::ops::ControlFlow;

pub(super) enum StagedNormalInput {
    Value(Value),
    TypeList(Vec<(bool, Self)>),
    Literal(usize),
    Tuple { key: usize, items: Vec<Self> },
}

macro_rules! staged_value {
    ($result:expr) => {
        match $result? {
            Flow::Value(value) => value,
            flow => return Ok(ControlFlow::Break(flow)),
        }
    };
}
macro_rules! staged_input {
    ($result:expr) => {
        match $result? {
            ControlFlow::Continue(value) => value,
            ControlFlow::Break(flow) => return Ok(ControlFlow::Break(flow)),
        }
    };
}

impl Interpreter<'_> {
    pub(super) fn stage_normal_argument(
        &mut self,
        input: &A,
        environment: &mut Environment,
        depth: usize,
    ) -> Result<ControlFlow<Flow, StagedNormalInput>, Error> {
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(input.source())));
        }
        self.tick(Some(input.source()))?;
        self.depth += 1;
        let result = (|| {
            let staged = match input {
                A::TypeListSpread(value) | A::Value(value) => {
                    StagedNormalInput::Value(staged_value!(self.eval(value, environment)))
                }
                A::Integer { .. } | A::Unit { .. } => {
                    StagedNormalInput::Literal(input as *const A as usize)
                }
                A::TypeList { items, .. } => {
                    let mut values = Vec::new();
                    for item in items {
                        values.push((matches!(item, A::TypeListSpread(_)), staged_input!(self.stage_normal_argument(item, environment, depth + 1))));
                    }
                    StagedNormalInput::TypeList(values)
                }
                A::Tuple { items, .. } => {
                    let mut values = Vec::new();
                    for item in items {
                        values.push(staged_input!(self.stage_normal_argument(
                            item,
                            environment,
                            depth + 1
                        )));
                    }
                    StagedNormalInput::Tuple {
                        key: input as *const A as usize,
                        items: values,
                    }
                }
                A::If {
                    condition,
                    then_input,
                    else_input,
                    ..
                } => {
                    let Value::Bool(flag) = staged_value!(self.eval(condition, environment)) else {
                        return Err(self.failure(ErrorKind::Type, Some(condition.source)));
                    };
                    staged_input!(self.stage_normal_argument(
                        if flag { then_input } else { else_input },
                        environment,
                        depth + 1
                    ))
                }
                A::Block {
                    statements, result, ..
                } => {
                    let existing: BTreeSet<_> = environment.keys().copied().collect();
                    let evaluated = (|| {
                        for statement in statements {
                            staged_value!(self.eval(statement, environment));
                        }
                        if let Some(result) = result {
                            self.stage_normal_argument(result, environment, depth + 1)
                        } else {
                            Ok(ControlFlow::Continue(StagedNormalInput::Value(Value::Unit)))
                        }
                    })();
                    environment.retain(|id, _| existing.contains(id));
                    staged_input!(evaluated)
                }
                A::Match {
                    source,
                    value,
                    arms,
                } => {
                    let value = staged_value!(self.eval(value, environment));
                    let existing: BTreeSet<_> = environment.keys().copied().collect();
                    let mut selected = None;
                    for arm in arms {
                        let result = (|| {
                            self.tick(Some(*source))?;
                            match self.match_arm_guard(
                                &arm.pattern,
                                arm.guard.as_ref(),
                                &value,
                                environment,
                            )? {
                                Flow::Value(Value::Bool(false)) => Ok(None),
                                Flow::Value(Value::Bool(true)) => self
                                    .stage_normal_argument(&arm.body, environment, depth + 1)
                                    .map(Some),
                                flow => Ok(Some(ControlFlow::Break(flow))),
                            }
                        })();
                        environment.retain(|id, _| existing.contains(id));
                        if let Some(result) = result? {
                            selected = Some(result);
                            break;
                        }
                    }
                    staged_input!(
                        selected.ok_or_else(
                            || self.failure(ErrorKind::NonExhaustiveMatch, Some(*source))
                        )
                    )
                }
            };
            Ok(ControlFlow::Continue(staged))
        })();
        self.depth -= 1;
        if input.is_diverging() && matches!(&result, Ok(ControlFlow::Continue(_))) {
            return Err(self.failure(ErrorKind::Type, Some(input.source())));
        }
        result
    }

    pub(super) fn finish_staged_normal_argument(
        &mut self,
        staged: StagedNormalInput,
        literals: &mut BTreeMap<usize, PreparedNormalInput>,
        source: SourceRange,
        depth: usize,
    ) -> Result<Value, Error> {
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        self.tick(Some(source))?;
        match staged {
            StagedNormalInput::Value(value) => Ok(value),
            StagedNormalInput::Literal(key) => match literals.remove(&key) {
                Some(PreparedNormalInput::Value(value)) => Ok(value),
                _ => Err(self.failure(ErrorKind::Type, Some(source))),
            },
            StagedNormalInput::TypeList(items) => {
                let mut types = Vec::new();
                for (spread, item) in items {
                    let value = self.finish_staged_normal_argument(item, literals, source, depth + 1)?;
                    match (spread, value) {
                        (true, Value::TypeList(elements)) => {
                            self.integer_operation(Some(source), |budget| budget.consume(elements.len()))?;
                            types.extend(elements);
                        }
                        (false, Value::Type(ty)) => types.push(ty),
                        _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                    }
                }
                Ok(Value::TypeList(types))
            }
            StagedNormalInput::Tuple { key, items } => {
                let values = items
                    .into_iter()
                    .map(|item| {
                        self.finish_staged_normal_argument(item, literals, source, depth + 1)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(PreparedNormalInput::TypeTuple(head)) = literals.remove(&key) {
                    let types = values
                        .into_iter()
                        .map(|value| match value {
                            Value::Type(ty) => Ok(ty),
                            _ => Err(self.failure(ErrorKind::Type, Some(source))),
                        })
                        .collect::<Result<_, _>>()?;
                    self.contextual_tuple_type(head, types, source)
                } else {
                    Ok(Value::Tuple(values))
                }
            }
        }
    }
}
