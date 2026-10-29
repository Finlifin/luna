//! Invocation-local evaluation of contextual inputs before their Type arrives.
//! Literal keys point into boxed immutable arguments owned by that invocation.
//! Explicit continuations retain logical depth while avoiding host recursion
//! through contextual containers. Expression and guard evaluation stay shared.
use super::*;
use nir::program::NormalFormArgument as A;
use std::ops::ControlFlow;

pub(super) enum StagedNormalInput {
    Value(Value),
    TypeList {
        key: usize,
        items: Vec<(bool, Self)>,
    },
    Literal(usize),
    Tuple {
        key: usize,
        items: Vec<Self>,
    },
}

macro_rules! staged_value {
    ($result:expr) => {
        match $result? {
            Flow::Value(value) => value,
            flow => return Ok(ControlFlow::Break(flow)),
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
        enum Work<'a> {
            Enter(&'a A, usize),
            Exit(&'a A),
            List(&'a A, &'a [A], usize),
            Tuple(&'a A, usize),
            LeaveScope,
            Next(&'a [A], usize),
        }
        let original_depth = self.depth;
        let mut scopes: Vec<BTreeSet<LocalId>> = Vec::new();
        let result = (|| {
            let mut work = vec![Work::Enter(input, depth)];
            let mut values = Vec::new();
            while let Some(next) = work.pop() {
                match next {
                    Work::Next(items, depth) => {
                        if let Some((item, rest)) = items.split_first() {
                            work.push(Work::Next(rest, depth));
                            work.push(Work::Enter(item, depth));
                        }
                    }
                    Work::Exit(input) => {
                        self.depth -= 1;
                        if input.is_diverging() {
                            return Err(self.failure(ErrorKind::Type, Some(input.source())));
                        }
                    }
                    Work::LeaveScope => {
                        let existing = scopes.pop().unwrap();
                        environment.retain(|id, _| existing.contains(id));
                    }
                    Work::List(input, items, start) => {
                        let children = values.split_off(start);
                        values.push(StagedNormalInput::TypeList {
                            key: input as *const A as usize,
                            items: items
                                .iter()
                                .zip(children)
                                .map(|(item, value)| (matches!(item, A::TypeListSpread(_)), value))
                                .collect(),
                        });
                    }
                    Work::Tuple(input, start) => {
                        let items = values.split_off(start);
                        values.push(StagedNormalInput::Tuple {
                            key: input as *const A as usize,
                            items,
                        });
                    }
                    Work::Enter(input, depth) => {
                        if depth >= 128 {
                            return Err(
                                self.failure(ErrorKind::DepthExhausted, Some(input.source()))
                            );
                        }
                        self.tick(Some(input.source()))?;
                        self.depth += 1;
                        work.push(Work::Exit(input));
                        match input {
                            A::TypeListSpread(value) | A::Value(value) => {
                                values.push(StagedNormalInput::Value(staged_value!(
                                    self.eval(value, environment)
                                )));
                            }
                            A::Integer { .. } | A::Unit { .. } => {
                                values.push(StagedNormalInput::Literal(input as *const A as usize))
                            }
                            A::TypeList { items, .. } | A::Tuple { items, .. } => {
                                work.push(if matches!(input, A::TypeList { .. }) {
                                    Work::List(input, items, values.len())
                                } else {
                                    Work::Tuple(input, values.len())
                                });
                                work.push(Work::Next(items, depth + 1));
                            }
                            A::If {
                                condition,
                                then_input,
                                else_input,
                                ..
                            } => {
                                let Value::Bool(flag) =
                                    staged_value!(self.eval(condition, environment))
                                else {
                                    return Err(
                                        self.failure(ErrorKind::Type, Some(condition.source))
                                    );
                                };
                                work.push(Work::Enter(
                                    if flag { then_input } else { else_input },
                                    depth + 1,
                                ));
                            }
                            A::Block {
                                statements, result, ..
                            } => {
                                scopes.push(environment.keys().copied().collect());
                                work.push(Work::LeaveScope);
                                for statement in statements {
                                    staged_value!(self.eval(statement, environment));
                                }
                                match result {
                                    Some(result) => work.push(Work::Enter(result, depth + 1)),
                                    None => values.push(StagedNormalInput::Value(Value::Unit)),
                                }
                            }
                            A::Match {
                                source,
                                value,
                                arms,
                            } => {
                                let value = staged_value!(self.eval(value, environment));
                                scopes.push(environment.keys().copied().collect());
                                let mut selected = None;
                                for arm in arms {
                                    self.tick(Some(*source))?;
                                    match self.match_arm_guard(
                                        &arm.pattern,
                                        arm.guard.as_ref(),
                                        &value,
                                        environment,
                                    )? {
                                        Flow::Value(Value::Bool(false)) => environment
                                            .retain(|id, _| scopes.last().unwrap().contains(id)),
                                        Flow::Value(Value::Bool(true)) => {
                                            selected = Some(&arm.body);
                                            break;
                                        }
                                        flow => return Ok(ControlFlow::Break(flow)),
                                    }
                                }
                                let selected = selected.ok_or_else(|| {
                                    self.failure(ErrorKind::NonExhaustiveMatch, Some(*source))
                                })?;
                                work.push(Work::LeaveScope);
                                work.push(Work::Enter(selected, depth + 1));
                            }
                        }
                    }
                }
            }
            Ok(ControlFlow::Continue(values.pop().unwrap()))
        })();
        // Transfers and errors bypass completion tasks but must still discard
        // temporary bindings. Mutations of pre-existing storage remain visible.
        while let Some(existing) = scopes.pop() {
            environment.retain(|id, _| existing.contains(id));
        }
        self.depth = original_depth;
        result
    }

    pub(super) fn finish_staged_normal_argument(
        &mut self,
        staged: StagedNormalInput,
        literals: &mut BTreeMap<usize, PreparedNormalInput>,
        source: SourceRange,
        depth: usize,
    ) -> Result<Value, Error> {
        enum Work {
            Enter(StagedNormalInput, usize),
            List {
                ty: Term,
                items: std::vec::IntoIter<(bool, StagedNormalInput)>,
                types: Vec<Term>,
                values: Vec<Value>,
                depth: usize,
                pending: Option<bool>,
            },
            Tuple {
                key: usize,
                items: std::vec::IntoIter<StagedNormalInput>,
                values: Vec<Value>,
                depth: usize,
                pending: bool,
            },
        }
        let mut work = vec![Work::Enter(staged, depth)];
        let mut completed = None;
        while let Some(next) = work.pop() {
            match next {
                Work::Enter(staged, depth) => {
                    if depth >= 128 {
                        return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
                    }
                    self.tick(Some(source))?;
                    match staged {
                        StagedNormalInput::Value(value) => completed = Some(value),
                        StagedNormalInput::Literal(key) => match literals.remove(&key) {
                            Some(PreparedNormalInput::Value(value)) => completed = Some(value),
                            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                        },
                        StagedNormalInput::TypeList { key, items } => {
                            let Some(PreparedNormalInput::ListType(ty)) = literals.remove(&key)
                            else {
                                return Err(self.failure(ErrorKind::Type, Some(source)));
                            };
                            work.push(Work::List {
                                ty,
                                items: items.into_iter(),
                                types: vec![],
                                values: vec![],
                                depth,
                                pending: None,
                            });
                        }
                        StagedNormalInput::Tuple { key, items } => work.push(Work::Tuple {
                            key,
                            items: items.into_iter(),
                            values: vec![],
                            depth,
                            pending: false,
                        }),
                    }
                }
                Work::List {
                    ty,
                    mut items,
                    mut types,
                    mut values,
                    depth,
                    pending,
                } => {
                    if let Some(spread) = pending {
                        self.append_normal_list_item(
                            &ty,
                            &mut types,
                            &mut values,
                            spread,
                            completed.take().unwrap(),
                            source,
                        )?;
                    }
                    if let Some((spread, item)) = items.next() {
                        work.push(Work::List {
                            ty,
                            items,
                            types,
                            values,
                            depth,
                            pending: Some(spread),
                        });
                        work.push(Work::Enter(item, depth + 1));
                    } else {
                        completed = Some(self.finish_normal_list(ty, types, values)?);
                    }
                }
                Work::Tuple {
                    key,
                    mut items,
                    mut values,
                    depth,
                    pending,
                } => {
                    if pending {
                        values.push(completed.take().unwrap());
                    }
                    if let Some(item) = items.next() {
                        work.push(Work::Tuple {
                            key,
                            items,
                            values,
                            depth,
                            pending: true,
                        });
                        work.push(Work::Enter(item, depth + 1));
                    } else if let Some(PreparedNormalInput::TypeTuple(head)) = literals.remove(&key)
                    {
                        let types = values
                            .into_iter()
                            .map(|value| match value {
                                Value::Type(ty) => Ok(ty),
                                _ => Err(self.failure(ErrorKind::Type, Some(source))),
                            })
                            .collect::<Result<_, _>>()?;
                        completed = Some(self.contextual_tuple_type(head, types, source)?);
                    } else {
                        completed = Some(Value::Tuple(values));
                    }
                }
            }
        }
        Ok(completed.unwrap())
    }
}

#[cfg(test)]
#[path = "normal_inputs_tests.rs"]
mod tests;
