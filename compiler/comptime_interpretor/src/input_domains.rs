//! One contextual domain preparation algorithm over borrowed source or compiled
//! descriptions. Neither adapter executes operand expressions or unfolds heads.
//! Domain checking owns its draft; execution receives only a complete result.
use super::*;
use comptime_compiler::normal_types::{Input as Scalar, Shape, ShapeKind};
use nir::program::NormalFormArgument as A;
use normal_types::Prepared;

#[derive(Clone, Copy)]
pub(super) enum Node<'a> {
    Source(&'a A),
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
            Self::Source(input) => input.source(),
            Self::Compiled(shape) => shape.source,
        }
    }
    fn diverges(self) -> bool {
        match self {
            Self::Source(input) => input.is_diverging(),
            Self::Compiled(shape) => shape.diverges,
        }
    }
    fn kind(self) -> Kind<'a> {
        match self {
            Self::Source(input) => match input {
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
            Self::Source(input) => Self::Source(match input {
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
            }),
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
            Self::Source(A::List { items, .. }) => matches!(items[index], A::ListSpread(_)),
            Self::Compiled(Shape {
                kind: ShapeKind::List(items),
                ..
            }) => items[index].0,
            _ => unreachable!("list spread flag"),
        }
    }
    fn guard(self, index: usize) -> Option<(&'a Term, SourceRange)> {
        match self {
            Self::Source(A::Match { arms, .. }) => arms[index]
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
    fn child_key(self, index: usize, parent: &InputKey) -> InputKey {
        match self.child(index) {
            Self::Compiled(shape) => InputKey::Register(shape.slot),
            Self::Source(_) => parent.child(index),
        }
    }
}

impl Interpreter<'_> {
    pub(super) fn prepare_input_domain(
        &mut self,
        node: Node<'_>,
        expected: &Term,
        key: InputKey,
        depth: usize,
    ) -> Result<Preparation, Error> {
        let mut entries = Vec::new();
        self.check_input_domain(node, expected, key, depth, &mut entries)?;
        Ok(Preparation { entries })
    }

    fn check_input_domain(
        &mut self,
        node: Node<'_>,
        expected: &Term,
        key: InputKey,
        depth: usize,
        entries: &mut Vec<(InputKey, Prepared)>,
    ) -> Result<(), Error> {
        let source = node.source();
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        self.tick(Some(source))?;
        let value = match node.kind() {
            Kind::Value(ty) => {
                self.prepare_normal_value_type(ty, expected, source)?;
                return Ok(());
            }
            Kind::Null => {
                let ty = self.prepare_optional(expected, None, source)?;
                Prepared::Literal(self.complete_optional(&ty, None, source)?)
            }
            Kind::Unit => Prepared::Literal(self.normal_unit_value(expected, source)?),
            Kind::Integer(spelling, negative) => {
                Prepared::Literal(self.normal_integer_value(expected, spelling, negative, source)?)
            }
            Kind::Choice(condition, condition_source) => {
                if !matches!(
                    condition,
                    Term::Primitive(Primitive::Bool | Primitive::Never)
                ) {
                    return Err(self.failure(ErrorKind::Type, Some(condition_source)));
                }
                for index in 0..2 {
                    self.check_input_domain(
                        node.child(index),
                        expected,
                        node.child_key(index, &key),
                        depth + 1,
                        entries,
                    )?;
                }
                return Ok(());
            }
            Kind::Block(has_result) => {
                if has_result {
                    self.check_input_domain(
                        node.child(0),
                        expected,
                        node.child_key(0, &key),
                        depth + 1,
                        entries,
                    )?;
                } else if !node.diverges() {
                    self.check_type_contract(
                        &nir::program::TypePredicate::Equal(
                            Term::Primitive(Primitive::Void),
                            expected.clone(),
                        ),
                        source,
                    )?;
                }
                return Ok(());
            }
            Kind::Match(count) => {
                for index in 0..count {
                    if let Some((ty, source)) = node.guard(index) {
                        if !matches!(ty, Term::Primitive(Primitive::Bool | Primitive::Never)) {
                            return Err(self.failure(ErrorKind::Type, Some(source)));
                        }
                    }
                    self.check_input_domain(
                        node.child(index),
                        expected,
                        node.child_key(index, &key),
                        depth + 1,
                        entries,
                    )?;
                }
                return Ok(());
            }
            Kind::List(count) => {
                let element = self.normal_list_domain(expected, count, source)?;
                for index in 0..count {
                    self.check_input_domain(
                        node.child(index),
                        if node.spread(index) {
                            expected
                        } else {
                            &element
                        },
                        node.child_key(index, &key),
                        depth + 1,
                        entries,
                    )?;
                }
                Prepared::List {
                    ty: expected.clone(),
                    values: vec![],
                }
            }
            Kind::Tuple(count) => {
                let types = self
                    .tuple_input_elements(expected)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                if types.len() != count {
                    return Err(self.failure(ErrorKind::Arity, Some(source)));
                }
                for (index, ty) in types.iter().enumerate() {
                    self.check_input_domain(
                        node.child(index),
                        ty,
                        node.child_key(index, &key),
                        depth + 1,
                        entries,
                    )?;
                }
                Prepared::Tuple { values: vec![] }
            }
        };
        entries.push((key, value));
        Ok(())
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
                Node::Source(&input),
                &Term::Tuple(vec![Term::Primitive(Primitive::U8)]),
                InputKey::root(),
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
