//! One contextual domain preparation algorithm over borrowed source or compiled
//! descriptions. Neither adapter executes operand expressions or unfolds heads.
//! Domain checking owns its draft; execution receives only a complete result.
use super::*;
use comptime_compiler::normal_types::{Input as Scalar, Shape, ShapeKind};
use input_bindings::Bindings;
use nir::program::NormalFormArgument as A;
use normal_types::Prepared;
use std::borrow::Cow;

/// Source annotations belong to the original declaration template. Substitute
/// each observed annotation once with the current activation's complete prefix.
pub(super) fn annotation<'a>(
    vm: &mut Interpreter<'_>,
    bindings: Option<&Bindings>,
    ty: &'a Term,
    source: SourceRange,
) -> Result<Cow<'a, Term>, Error> {
    match bindings {
        Some(bindings) => bindings.instantiate(vm, ty, source).map(Cow::Owned),
        None => Ok(Cow::Borrowed(ty)),
    }
}

#[derive(Clone, Copy)]
pub(super) enum Node<'a> {
    Source(&'a A, Option<&'a Bindings>),
    Compiled(&'a Shape),
}

enum Kind<'a> {
    Value(&'a Term),
    Unit,
    Null,
    Integer(&'a str, bool),
    Choice(&'a Term, SourceRange),
    Block(bool),
    Match(usize),
    List(usize),
    Tuple(usize),
}

/// A completed preparation, owned by one invocation. No callback can publish a
/// prefix while later branches or fields still have unchecked domains.
pub(super) struct Preparation {
    entries: Vec<(InputKey, Prepared)>,
}

impl Preparation {
    pub(super) fn deliver_literals(self, literals: &mut BTreeMap<InputKey, PreparedNormalInput>) {
        for (key, value) in self.entries {
            let value = match value {
                Prepared::Literal(value) => PreparedNormalInput::Value(value),
                Prepared::List { ty, .. } => PreparedNormalInput::ListType(ty),
                Prepared::Tuple { .. } => continue,
            };
            literals.insert(key, value);
        }
    }
    pub(super) fn into_entries(self) -> impl Iterator<Item = (InputKey, Prepared)> {
        self.entries.into_iter()
    }
}

impl fmt::Display for Preparation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(input-domain-preparation")?;
        for (key, value) in &self.entries {
            write!(f, " (entry {key} {value})")?;
        }
        f.write_str(")")
    }
}

impl<'a> Node<'a> {
    fn source(self) -> SourceRange {
        match self {
            Self::Source(input, _) => input.source(),
            Self::Compiled(shape) => shape.source,
        }
    }
    fn bindings(self) -> Option<&'a Bindings> {
        match self {
            Self::Source(_, bindings) => bindings,
            Self::Compiled(_) => None,
        }
    }
    fn empty_block_diverges(self, vm: &mut Interpreter<'_>) -> Result<bool, Error> {
        match self {
            Self::Compiled(shape) => Ok(shape.diverges),
            Self::Source(
                A::Block {
                    statements,
                    result: None,
                    ..
                },
                bindings,
            ) => {
                for statement in statements {
                    vm.tick(Some(statement.source))?;
                    if *annotation(vm, bindings, &statement.ty, statement.source)?
                        == Term::Primitive(Primitive::Never)
                    {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => unreachable!("block without a result"),
        }
    }
    fn kind(self) -> Kind<'a> {
        match self {
            Self::Source(input, _) => match input {
                A::Value(value) | A::ListSpread(value) => Kind::Value(&value.ty),
                A::Null { .. } => Kind::Null,
                A::Unit { .. } => Kind::Unit,
                A::Integer {
                    spelling, negative, ..
                } => Kind::Integer(spelling, *negative),
                A::If { condition, .. } => Kind::Choice(&condition.ty, condition.source),
                A::Block { result, .. } => Kind::Block(result.is_some()),
                A::Match { arms, .. } => Kind::Match(arms.len()),
                A::List { items, .. } => Kind::List(items.len()),
                A::Tuple { items, .. } => Kind::Tuple(items.len()),
            },
            Self::Compiled(shape) => match &shape.kind {
                ShapeKind::Scalar(Scalar::Value(ty)) => Kind::Value(ty),
                ShapeKind::Scalar(Scalar::Null) => Kind::Null,
                ShapeKind::Scalar(Scalar::Unit) => Kind::Unit,
                ShapeKind::Scalar(Scalar::Integer { spelling, negative }) => {
                    Kind::Integer(spelling, *negative)
                }
                ShapeKind::Choice {
                    condition,
                    condition_source,
                    ..
                } => Kind::Choice(condition, *condition_source),
                ShapeKind::Block { result } => Kind::Block(result.is_some()),
                ShapeKind::Match(arms) => Kind::Match(arms.len()),
                ShapeKind::List(items) => Kind::List(items.len()),
                ShapeKind::Tuple { items } => Kind::Tuple(items.len()),
            },
        }
    }
    fn child(self, index: usize) -> Self {
        match self {
            Self::Source(input, bindings) => Self::Source(
                match input {
                    A::List { items, .. } | A::Tuple { items, .. } => &items[index],
                    A::Match { arms, .. } => &arms[index].body,
                    A::If {
                        then_input,
                        else_input,
                        ..
                    } => {
                        if index == 0 {
                            then_input
                        } else {
                            else_input
                        }
                    }
                    A::Block { result, .. } => result.as_ref().unwrap(),
                    _ => unreachable!("contextual container child"),
                },
                bindings,
            ),
            Self::Compiled(shape) => Self::Compiled(match &shape.kind {
                ShapeKind::List(items) => &items[index].1,
                ShapeKind::Tuple { items, .. } => &items[index],
                ShapeKind::Match(arms) => &arms[index].body,
                ShapeKind::Choice {
                    then_input,
                    else_input,
                    ..
                } => {
                    if index == 0 {
                        then_input
                    } else {
                        else_input
                    }
                }
                ShapeKind::Block { result } => result.as_ref().unwrap(),
                _ => unreachable!("compiled container child"),
            }),
        }
    }
    fn spread(self, index: usize) -> bool {
        match self {
            Self::Source(A::List { items, .. }, _) => matches!(items[index], A::ListSpread(_)),
            Self::Compiled(Shape {
                kind: ShapeKind::List(items),
                ..
            }) => items[index].0,
            _ => unreachable!("list spread flag"),
        }
    }
    fn guard(self, index: usize) -> Option<(&'a Term, SourceRange)> {
        match self {
            Self::Source(A::Match { arms, .. }, _) => arms[index]
                .guard
                .as_ref()
                .map(|guard| (&guard.ty, guard.source)),
            Self::Compiled(Shape {
                kind: ShapeKind::Match(arms),
                ..
            }) => arms[index].guard.as_ref().map(|(ty, source)| (ty, *source)),
            _ => unreachable!("match guard"),
        }
    }
    fn key(self, source_key: impl FnOnce() -> InputKey) -> InputKey {
        match self {
            Self::Compiled(shape) => InputKey::Register(shape.slot),
            Self::Source(..) => source_key(),
        }
    }
}

/// A frame retains only the parent and its next field. In particular a wide
/// tuple cannot enqueue all its siblings before the VM charges the first one.
enum Children<'a> {
    Same,
    Match,
    List { element: &'a Term },
    Tuple { fields: &'a [Term] },
}

struct Cursor<'a> {
    node: Node<'a>,
    expected: &'a Term,
    key: InputKey,
    children: Children<'a>,
    index: usize,
    count: usize,
}

impl<'a> Cursor<'a> {
    fn next(&mut self, vm: &mut Interpreter<'_>) -> Result<(Node<'a>, &'a Term, InputKey), Error> {
        let index = self.index;
        self.index += 1;
        let expected = match self.children {
            Children::Match => {
                if let Some((ty, source)) = self.node.guard(index) {
                    vm.check_input_condition(self.node, ty, source)?;
                }
                self.expected
            }
            Children::List { element } if !self.node.spread(index) => element,
            Children::Tuple { fields } => &fields[index],
            Children::Same | Children::List { .. } => self.expected,
        };
        let child = self.node.child(index);
        Ok((child, expected, child.key(|| self.key.child(index))))
    }

    fn finish(self) -> Option<(InputKey, Prepared)> {
        let container = match self.children {
            Children::List { .. } => Prepared::List {
                ty: self.expected.clone(),
                values: vec![],
            },
            Children::Tuple { .. } => Prepared::Tuple { values: vec![] },
            Children::Match | Children::Same => return None,
        };
        Some((self.key, container))
    }
}

impl Interpreter<'_> {
    pub(super) fn prepare_input_domain<'a>(
        &mut self,
        node: Node<'a>,
        expected: &'a Term,
        depth: usize,
    ) -> Result<Preparation, Error> {
        let mut entries = Vec::new();
        let mut parents = Vec::<Cursor<'_>>::new();
        let mut current = (node, expected, node.key(InputKey::root));
        loop {
            let (node, expected, key) = current;
            if depth.saturating_add(parents.len()) >= 128 {
                return Err(self.failure(ErrorKind::DepthExhausted, Some(node.source())));
            }
            if let Some(parent) = self.enter_input_domain(node, expected, key, &mut entries)? {
                parents.push(parent);
            }

            loop {
                let Some(parent) = parents.last_mut() else {
                    return Ok(Preparation { entries });
                };
                if parent.index < parent.count {
                    current = parent.next(self)?;
                    break;
                }
                let parent = parents.pop().expect("completed domain cursor");
                if let Some(entry) = parent.finish() {
                    entries.push(entry);
                }
            }
        }
    }

    fn enter_input_domain<'a>(
        &mut self,
        node: Node<'a>,
        expected: &'a Term,
        key: InputKey,
        entries: &mut Vec<(InputKey, Prepared)>,
    ) -> Result<Option<Cursor<'a>>, Error> {
        let source = node.source();
        self.tick(Some(source))?;
        let (children, count) = match node.kind() {
            Kind::Value(ty) => {
                let ty = annotation(self, node.bindings(), ty, source)?;
                self.prepare_normal_value_type(&ty, expected, source)?;
                return Ok(None);
            }
            Kind::Null => {
                let ty = self.prepare_optional(expected, None, source)?;
                let value = self.complete_optional(&ty, None, source)?;
                entries.push((key, Prepared::Literal(value)));
                return Ok(None);
            }
            Kind::Unit => {
                entries.push((
                    key,
                    Prepared::Literal(self.normal_unit_value(expected, source)?),
                ));
                return Ok(None);
            }
            Kind::Integer(spelling, negative) => {
                let value = self.normal_integer_value(expected, spelling, negative, source)?;
                entries.push((key, Prepared::Literal(value)));
                return Ok(None);
            }
            Kind::Choice(ty, source) => {
                self.check_input_condition(node, ty, source)?;
                (Children::Same, 2)
            }
            Kind::Block(true) => (Children::Same, 1),
            Kind::Block(false) => {
                if !node.empty_block_diverges(self)? {
                    self.check_type_contract(
                        &nir::program::TypePredicate::Equal(
                            Term::Primitive(Primitive::Void),
                            expected.clone(),
                        ),
                        source,
                    )?;
                }
                return Ok(None);
            }
            Kind::Match(count) => (Children::Match, count),
            Kind::List(count) => {
                let element = self.normal_list_domain(expected, count, source)?;
                (Children::List { element }, count)
            }
            Kind::Tuple(count) => {
                let fields = self
                    .tuple_input_elements(expected)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                if fields.len() != count {
                    return Err(self.failure(ErrorKind::Arity, Some(source)));
                }
                (Children::Tuple { fields }, count)
            }
        };
        Ok(Some(Cursor {
            node,
            expected,
            key,
            children,
            index: 0,
            count,
        }))
    }

    fn check_input_condition(
        &mut self,
        node: Node<'_>,
        ty: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        let ty = annotation(self, node.bindings(), ty, source)?;
        if matches!(*ty, Term::Primitive(Primitive::Bool | Primitive::Never)) {
            Ok(())
        } else {
            Err(self.failure(ErrorKind::Type, Some(source)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_domain_dump_keeps_paths_and_literal_values() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let input = A::Tuple {
            source: SourceRange { start: 3, end: 8 },
            items: vec![A::Integer {
                source: SourceRange { start: 4, end: 5 },
                spelling: "7".into(),
                negative: false,
            }],
        };
        let complete = vm
            .prepare_input_domain(
                Node::Source(&input, None),
                &Term::Tuple(vec![Term::Primitive(Primitive::U8)]),
                0,
            )
            .unwrap();
        assert_eq!(
            complete.to_string(),
            format!(
                "(input-domain-preparation (entry (input-path 0) (literal {})) (entry (input-path) (tuple (values))))",
                Value::UnsignedInteger(7)
            )
        );
        assert_eq!(complete.into_entries().count(), 2);
    }
}
