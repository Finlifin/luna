//! Invocation-local evaluation of contextual inputs before their Type arrives.
//! Structural input paths survive cloning; bytecode registers are rekeyed
//! explicitly before completed operands meet prepared domains.
//! Explicit continuations retain logical depth while avoiding host recursion
//! through contextual containers. Expression and guard evaluation stay shared.
use super::*;
use nir::program::NormalFormArgument as A;
use std::ops::ControlFlow;

/// Identity within one contextual input, never a host allocation identity.
/// Child indices follow source order (then/else, fields, arms, block result).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum InputKey {
    Node(Vec<usize>),
    Register(usize),
}
impl InputKey {
    pub(super) fn root() -> Self {
        Self::Node(vec![])
    }
    pub(super) fn child(&self, index: usize) -> Self {
        let Self::Node(path) = self else {
            unreachable!("child of input node")
        };
        let mut path = path.clone();
        path.push(index);
        Self::Node(path)
    }
}
impl fmt::Display for InputKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Register(slot) => write!(f, "(register {slot})"),
            Self::Node(path) => {
                write!(f, "(input-path")?;
                for index in path {
                    write!(f, " {index}")?;
                }
                write!(f, ")")
            }
        }
    }
}

pub(super) enum StagedNormalInput {
    Value(Value),
    List {
        key: InputKey,
        items: Vec<(bool, Self)>,
    },
    Literal(InputKey),
    Tuple {
        key: InputKey,
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
        if let Some(result) = self.try_stage_bytecode(input, environment, depth)? {
            return Ok(result);
        }
        self.interpret_staged_argument(input, environment, depth)
    }

    pub(super) fn interpret_staged_argument(
        &mut self,
        input: &A,
        environment: &mut Environment,
        depth: usize,
    ) -> Result<ControlFlow<Flow, StagedNormalInput>, Error> {
        enum Work<'a> {
            Enter(&'a A, usize, InputKey),
            Exit(&'a A),
            List(InputKey, &'a [A], usize),
            Tuple(InputKey, usize),
            LeaveScope,
            Next(&'a [A], usize, InputKey, usize),
        }
        let original_depth = self.depth;
        let mut scopes: Vec<BTreeSet<LocalId>> = Vec::new();
        let result = (|| {
            let mut work = vec![Work::Enter(input, depth, InputKey::root())];
            let mut values = Vec::new();
            while let Some(next) = work.pop() {
                match next {
                    Work::Next(items, depth, key, index) => {
                        if let Some((item, rest)) = items.split_first() {
                            work.push(Work::Next(rest, depth, key.clone(), index + 1));
                            work.push(Work::Enter(item, depth, key.child(index)));
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
                    Work::List(key, items, start) => {
                        let children = values.split_off(start);
                        values.push(StagedNormalInput::List {
                            key,
                            items: items
                                .iter()
                                .zip(children)
                                .map(|(item, value)| (matches!(item, A::ListSpread(_)), value))
                                .collect(),
                        });
                    }
                    Work::Tuple(key, start) => {
                        let items = values.split_off(start);
                        values.push(StagedNormalInput::Tuple { key, items });
                    }
                    Work::Enter(input, depth, key) => {
                        if depth >= 128 {
                            return Err(
                                self.failure(ErrorKind::DepthExhausted, Some(input.source()))
                            );
                        }
                        self.tick(Some(input.source()))?;
                        self.depth += 1;
                        work.push(Work::Exit(input));
                        match input {
                            A::ListSpread(value) | A::Value(value) => {
                                values.push(StagedNormalInput::Value(staged_value!(
                                    self.eval(value, environment)
                                )));
                            }
                            A::Integer { .. } | A::Unit { .. } => {
                                values.push(StagedNormalInput::Literal(key))
                            }
                            A::List { items, .. } | A::Tuple { items, .. } => {
                                work.push(if matches!(input, A::List { .. }) {
                                    Work::List(key.clone(), items, values.len())
                                } else {
                                    Work::Tuple(key.clone(), values.len())
                                });
                                work.push(Work::Next(items, depth + 1, key, 0));
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
                                    key.child(usize::from(!flag)),
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
                                    Some(result) => {
                                        work.push(Work::Enter(result, depth + 1, key.child(0)))
                                    }
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
                                for (index, arm) in arms.iter().enumerate() {
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
                                            selected = Some((index, &arm.body));
                                            break;
                                        }
                                        flow => return Ok(ControlFlow::Break(flow)),
                                    }
                                }
                                let (index, selected) = selected.ok_or_else(|| {
                                    self.failure(ErrorKind::NonExhaustiveMatch, Some(*source))
                                })?;
                                work.push(Work::LeaveScope);
                                work.push(Work::Enter(selected, depth + 1, key.child(index)));
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
        literals: &mut BTreeMap<InputKey, PreparedNormalInput>,
        source: SourceRange,
        depth: usize,
    ) -> Result<Value, Error> {
        enum Work {
            Enter(StagedNormalInput, usize),
            List {
                ty: Term,
                items: std::vec::IntoIter<(bool, StagedNormalInput)>,
                values: Vec<Value>,
                depth: usize,
                pending: Option<bool>,
            },
            Tuple {
                key: InputKey,
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
                        StagedNormalInput::List { key, items } => {
                            let Some(PreparedNormalInput::ListType(ty)) = literals.remove(&key)
                            else {
                                return Err(self.failure(ErrorKind::Type, Some(source)));
                            };
                            work.push(Work::List {
                                ty,
                                items: items.into_iter(),
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
                    mut values,
                    depth,
                    pending,
                } => {
                    if let Some(spread) = pending {
                        self.append_normal_list_item(
                            &ty,
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
                            values,
                            depth,
                            pending: Some(spread),
                        });
                        work.push(Work::Enter(item, depth + 1));
                    } else {
                        completed = Some(self.finish_normal_list(ty, values)?);
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
                    } else {
                        let head = match literals.remove(&key) {
                            Some(PreparedNormalInput::TypeTuple(head)) => Some(head),
                            _ => None,
                        };
                        completed = Some(self.finish_contextual_tuple(head, values, source)?);
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
