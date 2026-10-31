//! Invocation-local staged operands. Contextual literals are never ordinary values.
use super::*;
use comptime_compiler::staged_inputs::Instruction as Op;
use normal_inputs::StagedNormalInput as Input;

#[derive(Default)]
pub(super) struct State {
    values: BTreeMap<usize, Input>,
    containers: BTreeMap<usize, Vec<(bool, Input)>>,
    pub completed: Option<Input>,
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
            for (spread, value) in values {
                write!(f, " (item (spread {spread}) {value})")?;
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
                for (spread, value) in items {
                    write!(f, " (item (spread {spread}) {value})")?;
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
            Op::Enter => vm.depth += 1,
            Op::Leave => {
                vm.depth = vm
                    .depth
                    .checked_sub(1)
                    .ok_or_else(|| vm.failure(ErrorKind::InvalidControlFlow, Some(source)))?
            }
            Op::Unit(slot) => {
                self.values.insert(*slot, Input::Value(Value::Unit));
            }
            Op::Literal(slot) => {
                self.values
                    .insert(*slot, Input::Literal(InputKey::Register(*slot)));
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
                self.containers
                    .entry(*parent)
                    .or_default()
                    .push((*spread, input));
            }
            Op::Container { value, list, .. } => {
                let items = self.containers.remove(value).unwrap_or_default();
                let input = if *list {
                    Input::List {
                        key: InputKey::Register(*value),
                        items,
                    }
                } else {
                    Input::Tuple {
                        key: InputKey::Register(*value),
                        items: items.into_iter().map(|(_, item)| item).collect(),
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

impl Interpreter<'_> {
    /// Register keys belong to immutable bytecode. Only after execution do we
    /// connect them to structural input paths used by the domain finisher.
    pub(super) fn rekey_staged_input(
        &mut self,
        root: &nir::program::NormalFormArgument,
        shape: &comptime_compiler::normal_types::Shape,
        staged: &mut Input,
    ) -> Result<(), Error> {
        use comptime_compiler::normal_types::{Shape, ShapeKind as K};
        use nir::program::NormalFormArgument as A;
        let mut pairs: Vec<(&A, &Shape, InputKey)> = vec![(root, shape, InputKey::root())];
        let mut keys = BTreeMap::new();
        while let Some((input, shape, key)) = pairs.pop() {
            self.tick(Some(input.source()))?;
            keys.insert(shape.slot, key.clone());
            match (input, &shape.kind) {
                (
                    A::Integer { .. } | A::Unit { .. } | A::Value(_) | A::ListSpread(_),
                    K::Scalar(_),
                ) => {}
                (A::List { items, .. }, K::List(children)) => pairs.extend(
                    items
                        .iter()
                        .zip(children.iter().map(|(_, child)| child))
                        .enumerate()
                        .map(|(index, (a, b))| (a, b, key.child(index))),
                ),
                (
                    A::Tuple { items, .. },
                    K::Tuple {
                        items: children, ..
                    },
                ) => pairs.extend(
                    items
                        .iter()
                        .zip(children)
                        .enumerate()
                        .map(|(index, (a, b))| (a, b, key.child(index))),
                ),
                (
                    A::If {
                        then_input,
                        else_input,
                        ..
                    },
                    K::Choice {
                        then_input: yes,
                        else_input: no,
                        ..
                    },
                ) => {
                    pairs.push((then_input, yes, key.child(0)));
                    pairs.push((else_input, no, key.child(1)));
                }
                (A::Block { result, .. }, K::Block { result: child }) => pairs.extend(
                    result
                        .iter()
                        .zip(child)
                        .map(|(a, b)| (a.as_ref(), b.as_ref(), key.child(0))),
                ),
                (A::Match { arms, .. }, K::Match(children)) => pairs.extend(
                    arms.iter()
                        .zip(children)
                        .enumerate()
                        .map(|(index, (a, b))| (&a.body, &b.body, key.child(index))),
                ),
                _ => return Err(self.failure(ErrorKind::Type, Some(input.source()))),
            }
        }
        let mut pending = vec![staged];
        while let Some(input) = pending.pop() {
            self.tick(Some(root.source()))?;
            let key = match input {
                Input::Value(_) => continue,
                Input::Literal(key) => key,
                Input::List { key, items } => {
                    pending.extend(items.iter_mut().map(|(_, child)| child));
                    key
                }
                Input::Tuple { key, items } => {
                    pending.extend(items.iter_mut());
                    key
                }
            };
            let InputKey::Register(slot) = key else {
                return Err(self.failure(ErrorKind::InvalidControlFlow, Some(root.source())));
            };
            *key = keys
                .get(slot)
                .ok_or_else(|| self.failure(ErrorKind::Type, Some(root.source())))?
                .clone();
        }
        Ok(())
    }
}
