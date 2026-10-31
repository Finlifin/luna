//! One contextual domain preparation algorithm over borrowed source or compiled
//! descriptions. Neither adapter executes operand expressions or unfolds heads.
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
    Integer(&'a str, bool),
    Choice(&'a Term, SourceRange),
    Block(bool),
    Match(usize),
    List(usize),
    Tuple(Option<DeclarationId>, usize),
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
                A::Unit { .. } => Kind::Unit,
                A::Integer {
                    spelling, negative, ..
                } => Kind::Integer(spelling, *negative),
                A::If { condition, .. } => Kind::Choice(&condition.ty, condition.source),
                A::Block { result, .. } => Kind::Block(result.is_some()),
                A::Match { arms, .. } => Kind::Match(arms.len()),
                A::List { items, .. } => Kind::List(items.len()),
                A::Tuple {
                    type_head, items, ..
                } => Kind::Tuple(*type_head, items.len()),
            },
            Self::Compiled(shape) => match &shape.kind {
                ShapeKind::Scalar(Scalar::Value(ty)) => Kind::Value(ty),
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
                ShapeKind::Tuple { head, items } => Kind::Tuple(*head, items.len()),
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
        emit: &mut impl FnMut(InputKey, Prepared),
    ) -> Result<(), Error> {
        let source = node.source();
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        self.tick(Some(source))?;
        let value =
            match node.kind() {
                Kind::Value(ty) => {
                    self.prepare_normal_value_type(ty, expected, source)?;
                    return Ok(());
                }
                Kind::Unit => Prepared::Literal(self.normal_unit_value(expected, source)?),
                Kind::Integer(spelling, negative) => Prepared::Literal(
                    self.normal_integer_value(expected, spelling, negative, source)?,
                ),
                Kind::Choice(condition, condition_source) => {
                    if !matches!(
                        condition,
                        Term::Primitive(Primitive::Bool | Primitive::Never)
                    ) {
                        return Err(self.failure(ErrorKind::Type, Some(condition_source)));
                    }
                    for index in 0..2 {
                        self.prepare_input_domain(
                            node.child(index),
                            expected,
                            node.child_key(index, &key),
                            depth + 1,
                            emit,
                        )?;
                    }
                    return Ok(());
                }
                Kind::Block(has_result) => {
                    if has_result {
                        self.prepare_input_domain(
                            node.child(0),
                            expected,
                            node.child_key(0, &key),
                            depth + 1,
                            emit,
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
                        self.prepare_input_domain(
                            node.child(index),
                            expected,
                            node.child_key(index, &key),
                            depth + 1,
                            emit,
                        )?;
                    }
                    return Ok(());
                }
                Kind::List(count) => {
                    let element = self.normal_list_domain(expected, count, source)?;
                    for index in 0..count {
                        self.prepare_input_domain(
                            node.child(index),
                            if node.spread(index) {
                                expected
                            } else {
                                &element
                            },
                            node.child_key(index, &key),
                            depth + 1,
                            emit,
                        )?;
                    }
                    Prepared::List {
                        ty: expected.clone(),
                        values: vec![],
                    }
                }
                Kind::Tuple(head, count) => {
                    let type_tuple = *expected == Term::Primitive(Primitive::Type);
                    if type_tuple {
                        for index in 0..count {
                            self.prepare_input_domain(
                                node.child(index),
                                expected,
                                node.child_key(index, &key),
                                depth + 1,
                                emit,
                            )?;
                        }
                        if let Some(head) = head {
                            if !self.normal_forms.get(&head).is_some_and(|metadata| {
                                metadata.role == nir::normal_forms::Role::Tuple
                            }) {
                                return Err(self.failure(ErrorKind::Type, Some(source)));
                            }
                        }
                    } else {
                        let types = self
                            .tuple_input_elements(expected)
                            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                        if types.len() != count {
                            return Err(self.failure(ErrorKind::Arity, Some(source)));
                        }
                        for (index, ty) in types.iter().enumerate() {
                            self.prepare_input_domain(
                                node.child(index),
                                ty,
                                node.child_key(index, &key),
                                depth + 1,
                                emit,
                            )?;
                        }
                    }
                    Prepared::Tuple {
                        head: type_tuple.then_some(head),
                        values: vec![],
                    }
                }
            };
        emit(key, value);
        Ok(())
    }
}
