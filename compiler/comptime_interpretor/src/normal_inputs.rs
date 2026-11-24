//! Invocation-local evaluation of contextual inputs before their Type arrives.
//! Tree and bytecode producers retain the same structural path at execution.
//! Explicit continuations retain logical depth while avoiding host recursion
//! through contextual containers. Expression and guard evaluation stay shared.
use super::*;
use nir::program::NormalFormArgument as A;
use std::ops::ControlFlow;

/// Identity within one contextual input, never a host allocation identity.
/// Child indices follow source order (then/else, fields, arms, block result).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum InputKey {
    Node(nir::program::InputPath),
    Register(usize),
}

impl InputKey {
    pub(super) fn root() -> Self {
        Self::Node(nir::program::InputPath::default())
    }

    pub(super) fn child(&self, index: usize) -> Self {
        let Self::Node(path) = self else {
            unreachable!("child of input node")
        };
        Self::Node(path.child(index))
    }
}

impl fmt::Display for InputKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Register(slot) => write!(f, "(register {slot})"),
            Self::Node(path) => path.fmt(f),
        }
    }
}

pub(super) enum StagedNormalInput {
    Value(Value),
    List {
        key: InputKey,
        items: Vec<ListField>,
    },
    Literal(InputKey),
    Tuple {
        key: InputKey,
        items: Vec<Self>,
    },
}

/// Admission of a saved spread or element belongs to its original field.
pub(super) struct ListField {
    pub(super) spread: bool,
    pub(super) source: SourceRange,
    pub(super) value: StagedNormalInput,
}

impl fmt::Display for ListField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(item {} (spread {}) {})",
            self.source, self.spread, self.value
        )
    }
}

mod delivery;
mod execution;

impl Interpreter<'_> {
    pub(super) fn finish_staged_normal_argument(
        &mut self,
        staged: StagedNormalInput,
        literals: &mut BTreeMap<InputKey, PreparedNormalInput>,
        source: SourceRange,
        depth: usize,
    ) -> Result<Value, Error> {
        use delivery::{Delivery, Ready};

        enum Work {
            Enter(StagedNormalInput, usize),
            List {
                list: (Term, Vec<Value>),
                items: std::vec::IntoIter<ListField>,
                depth: usize,
                pending: Option<(bool, SourceRange)>,
            },
            Tuple {
                key: InputKey,
                items: std::vec::IntoIter<StagedNormalInput>,
                values: Vec<Value>,
                depth: usize,
                pending: bool,
            },
        }
        let mut delivery = Ready(literals);
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
                        StagedNormalInput::Literal(key) => {
                            completed = Some(delivery.literal(key, source)?);
                        }
                        StagedNormalInput::List { key, items } => work.push(Work::List {
                            list: delivery.list(&key, source)?,
                            items: items.into_iter(),
                            depth,
                            pending: None,
                        }),
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
                    mut list,
                    mut items,
                    depth,
                    pending,
                } => {
                    if let Some((spread, field_source)) = pending {
                        let value = completed.take().ok_or_else(|| {
                            self.failure(ErrorKind::InvalidControlFlow, Some(source))
                        })?;
                        Ready::append(&mut list, self, value, spread, field_source)?;
                    }
                    if let Some(field) = items.next() {
                        work.push(Work::List {
                            list,
                            items,
                            depth,
                            pending: Some((field.spread, field.source)),
                        });
                        work.push(Work::Enter(field.value, depth + 1));
                    } else {
                        completed = Some(Ready::finish_list(list, self)?);
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
                        let value = completed.take().ok_or_else(|| {
                            self.failure(ErrorKind::InvalidControlFlow, Some(source))
                        })?;
                        values.push(value);
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
                        completed = Some(Ready::tuple(key, values));
                    }
                }
            }
        }
        completed.ok_or_else(|| self.failure(ErrorKind::InvalidControlFlow, Some(source)))
    }
}

#[cfg(test)]
#[path = "normal_inputs_tests.rs"]
mod tests;
