//! Invocation-local staged operands. Contextual literals are never ordinary values.
use super::*;
use comptime_compiler::staged_inputs::Instruction as Op;
use normal_inputs::{ListField, StagedNormalInput as Input};

#[derive(Default)]
pub(super) struct State {
    values: BTreeMap<usize, Input>,
    containers: BTreeMap<usize, Vec<ListField>>,
    pub completed: Option<Input>,
    initial_depth: usize,
    caller_depth: usize,
}

impl State {
    pub(super) fn at_depth(initial_depth: usize, caller_depth: usize) -> Self {
        Self {
            initial_depth,
            caller_depth,
            ..Self::default()
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(staged-inputs (values")?;
        for (slot, value) in &self.values {
            write!(f, " (%{slot} {value})")?;
        }
        write!(f, ") (containers")?;
        for (slot, values) in &self.containers {
            write!(f, " (%{slot}")?;
            for field in values {
                write!(f, " {field}")?;
            }
            write!(f, ")")?;
        }
        write!(f, ") (completed")?;
        if let Some(value) = &self.completed {
            write!(f, " {value}")?;
        }
        write!(f, "))")
    }
}

impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(value) => write!(f, "(staged-value {value})"),
            Self::Literal(key) => write!(f, "(staged-literal {key})"),
            Self::List { key, items } => {
                write!(f, "(staged-list {key}")?;
                for field in items {
                    write!(f, " {field}")?;
                }
                write!(f, ")")
            }
            Self::Tuple { key, items } => {
                write!(f, "(staged-tuple {key}")?;
                for value in items {
                    write!(f, " {value}")?;
                }
                write!(f, ")")
            }
        }
    }
}

impl State {
    fn take(
        &mut self,
        slot: usize,
        slots: &mut [Option<Value>],
        source: SourceRange,
    ) -> Result<Input, Error> {
        self.values
            .remove(&slot)
            .or_else(|| slots.get_mut(slot)?.take().map(Input::Value))
            .ok_or(Error {
                kind: ErrorKind::MissingBinding,
                source: Some(source),
            })
    }

    pub fn step(
        &mut self,
        op: &Op,
        slots: &mut [Option<Value>],
        vm: &mut Interpreter<'_>,
        source: SourceRange,
    ) -> Result<Option<Flow>, Error> {
        let diverges = match op {
            Op::Append { diverges, .. }
            | Op::Container { diverges, .. }
            | Op::Transfer { diverges, .. }
            | Op::Finish { diverges, .. } => *diverges,
            _ => false,
        };
        if diverges {
            return Err(vm.failure(ErrorKind::Type, Some(source)));
        }
        match op {
            Op::Enter => {
                if self
                    .initial_depth
                    .saturating_add(vm.depth.saturating_sub(self.caller_depth))
                    >= 128
                {
                    return Err(vm.failure(ErrorKind::DepthExhausted, Some(source)));
                }
                vm.depth += 1;
            }
            Op::Leave => {
                vm.depth = vm
                    .depth
                    .checked_sub(1)
                    .ok_or_else(|| vm.failure(ErrorKind::InvalidControlFlow, Some(source)))?
            }
            Op::Unit(slot) => {
                self.values.insert(*slot, Input::Value(Value::Unit));
            }
            Op::Literal { value: slot, path } => {
                self.values
                    .insert(*slot, Input::Literal(InputKey::Node(path.clone())));
            }
            Op::Transfer {
                destination, value, ..
            } => {
                let input = self.take(*value, slots, source)?;
                self.values.insert(*destination, input);
            }
            Op::Append {
                parent,
                value,
                spread,
                ..
            } => {
                let input = self.take(*value, slots, source)?;
                self.containers.entry(*parent).or_default().push(ListField {
                    spread: *spread,
                    source,
                    value: input,
                });
            }
            Op::Container {
                value, list, path, ..
            } => {
                let items = self.containers.remove(value).unwrap_or_default();
                let input = if *list {
                    Input::List {
                        key: InputKey::Node(path.clone()),
                        items,
                    }
                } else {
                    Input::Tuple {
                        key: InputKey::Node(path.clone()),
                        items: items.into_iter().map(|field| field.value).collect(),
                    }
                };
                self.values.insert(*value, input);
            }
            Op::Finish { value, .. } => {
                self.completed = Some(self.take(*value, slots, source)?);
                return Ok(Some(Flow::Value(Value::Unit)));
            }
        }
        Ok(None)
    }
}
