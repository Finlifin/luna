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
    /// Resolve bytecode slots from compiled shape metadata, without retaining
    /// the source input tree. Validation precedes mutation so failure leaves
    /// the completed operands usable for diagnosis or a fresh-budget retry.
    pub(super) fn rekey_staged_input(
        &mut self,
        shape: &comptime_compiler::normal_types::Shape,
        staged: &mut Input,
    ) -> Result<(), Error> {
        use comptime_compiler::normal_types::ShapeKind as K;
        let source = shape.source;
        let mut pairs = vec![(shape, InputKey::root(), 0)];
        let mut keys = BTreeMap::new();
        while let Some((shape, key, depth)) = pairs.pop() {
            self.tick(Some(shape.source))?;
            if depth >= 128 {
                return Err(self.failure(ErrorKind::DepthExhausted, Some(shape.source)));
            }
            if keys.insert(shape.slot, key.clone()).is_some() {
                return Err(self.failure(ErrorKind::InvalidControlFlow, Some(shape.source)));
            }
            match &shape.kind {
                K::Scalar(_) => {}
                K::List(items) => pairs.extend(
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, (_, child))| (child, key.child(index), depth + 1)),
                ),
                K::Tuple { items, .. } => pairs.extend(
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, child)| (child, key.child(index), depth + 1)),
                ),
                K::Choice {
                    then_input,
                    else_input,
                    ..
                } => {
                    pairs.push((then_input, key.child(0), depth + 1));
                    pairs.push((else_input, key.child(1), depth + 1));
                }
                K::Block { result } => {
                    if let Some(child) = result {
                        pairs.push((child, key.child(0), depth + 1));
                    }
                }
                K::Match(arms) => pairs.extend(
                    arms.iter()
                        .enumerate()
                        .map(|(index, arm)| (&arm.body, key.child(index), depth + 1)),
                ),
            }
        }
        // Validate and reserve the complete rewrite before touching any key.
        let mut replacements = Vec::new();
        let mut pending = vec![&*staged];
        while let Some(input) = pending.pop() {
            self.tick(Some(source))?;
            let key = match input {
                Input::Value(_) => continue,
                Input::Literal(key) => key,
                Input::List { key, items } => {
                    pending.extend(items.iter().map(|(_, child)| child));
                    key
                }
                Input::Tuple { key, items } => {
                    pending.extend(items.iter());
                    key
                }
            };
            let InputKey::Register(slot) = key else {
                return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
            };
            let replacement = keys
                .get(slot)
                .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
            replacements.push(replacement.clone());
        }
        let mut replacements = replacements.into_iter();
        let mut pending = vec![staged];
        while let Some(input) = pending.pop() {
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
            *key = replacements.next().expect("validated staged key");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use comptime_compiler::normal_types::{Input as Scalar, Shape, ShapeKind};

    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };

    fn shape() -> Shape {
        Shape {
            slot: 0,
            source: SOURCE,
            diverges: false,
            kind: ShapeKind::Tuple {
                head: None,
                items: [1, 2]
                    .map(|slot| Shape {
                        slot,
                        source: SOURCE,
                        diverges: false,
                        kind: ShapeKind::Scalar(Scalar::Integer {
                            spelling: slot.to_string(),
                            negative: false,
                        }),
                    })
                    .into(),
            },
        }
    }

    fn operands(last: usize) -> Input {
        Input::Tuple {
            key: InputKey::Register(0),
            items: vec![
                Input::Literal(InputKey::Register(1)),
                Input::Literal(InputKey::Register(last)),
            ],
        }
    }

    #[test]
    fn compiled_shapes_rekey_without_source_trees_and_fail_atomically() {
        let package = Package::default();
        let shape = shape();
        for steps in 0..=6 {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut staged = operands(2);
            let before = staged.to_string();
            vm.budget.steps = steps;
            let result = vm.rekey_staged_input(&shape, &mut staged);
            if steps < 6 {
                assert_eq!(result.unwrap_err().kind, ErrorKind::StepsExhausted);
                assert_eq!(staged.to_string(), before);
                vm.budget = Budget::default();
                vm.rekey_staged_input(&shape, &mut staged).unwrap();
            } else {
                result.unwrap();
            }
            assert_eq!(
                staged.to_string(),
                "(staged-tuple (input-path) (staged-literal (input-path 0)) (staged-literal (input-path 1)))"
            );
        }
    }

    #[test]
    fn invalid_registers_and_duplicate_shape_slots_do_not_rewrite_operands() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut shape = shape();
        let mut staged = operands(99);
        let before = staged.to_string();
        assert_eq!(
            vm.rekey_staged_input(&shape, &mut staged).unwrap_err().kind,
            ErrorKind::Type
        );
        assert_eq!(staged.to_string(), before);
        let ShapeKind::Tuple { items, .. } = &mut shape.kind else {
            unreachable!()
        };
        items[1].slot = 1;
        let mut staged = operands(2);
        let before = staged.to_string();
        assert_eq!(
            vm.rekey_staged_input(&shape, &mut staged).unwrap_err().kind,
            ErrorKind::InvalidControlFlow
        );
        assert_eq!(staged.to_string(), before);
    }
}
