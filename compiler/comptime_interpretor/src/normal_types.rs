//! Invocation-local state for sequential normal-form construction.
use super::*;

pub(super) struct Inputs {
    pub head: DeclarationId,
    pub arguments: Vec<Term>,
    parameters: Vec<nir::DeclarationParameter>,
    pub prepared: BTreeMap<usize, Prepared>,
}
impl Inputs {
    pub fn new(head: DeclarationId) -> Self {
        Self {
            head,
            arguments: vec![],
            parameters: vec![],
            prepared: BTreeMap::new(),
        }
    }
}
impl fmt::Display for Inputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(normal-inputs (head {}) (arguments", self.head)?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") (prepared")?;
        for (slot, value) in &self.prepared {
            write!(f, " (%{slot} {value})")?;
        }
        write!(f, "))")
    }
}
impl Interpreter<'_> {
    pub(super) fn prepare_normal_type(
        &mut self,
        head: DeclarationId,
        ty: &Term,
        count: usize,
        diverges: bool,
        source: SourceRange,
    ) -> Result<(), Error> {
        let metadata = self
            .normal_forms
            .get(&head)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let expected = if diverges {
            Primitive::Never
        } else {
            Primitive::Type
        };
        if *ty != Term::Primitive(expected) || count != metadata.parameters.len() {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        self.integer_operation(Some(source), |budget| budget.consume(count))
    }
    pub(super) fn prepare_compiled_normal_input(
        &mut self,
        inputs: &mut Inputs,
        index: usize,
        input: &comptime_compiler::normal_types::Shape,
        source: SourceRange,
    ) -> Result<(), Error> {
        let metadata = *self
            .normal_forms
            .get(&inputs.head)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        if index != inputs.arguments.len() || index >= metadata.parameters.len() {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        let expected = self.instantiate_contextual_domain(
            &metadata.parameters[index].ty,
            &inputs.parameters,
            &inputs.arguments,
            source,
        )?;
        inputs.prepared.clear();
        self.prepare_normal_shape(input, &expected, &mut inputs.prepared, 0)
    }
    pub(super) fn normal_list_domain(
        &mut self,
        expected: &Term,
        count: usize,
        source: SourceRange,
    ) -> Result<Term, Error> {
        if count > 100_000 {
            return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(source)));
        }
        self.integer_operation(Some(source), |budget| {
            let before = budget.steps;
            let mut fuel = before;
            if nir::type_computations::charge_term(expected, &mut fuel).is_err() {
                return budget.consume(before.saturating_add(1));
            }
            budget.consume(before - fuel)
        })?;
        let element = self
            .list_element(expected)
            .cloned()
            .or_else(|| {
                self.is_type_list_input(expected)
                    .then_some(Term::Primitive(Primitive::Type))
            })
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        self.integer_operation(Some(source), |budget| budget.consume(count))?;
        Ok(element)
    }
    fn prepare_normal_shape(
        &mut self,
        shape: &comptime_compiler::normal_types::Shape,
        expected: &Term,
        prepared: &mut BTreeMap<usize, Prepared>,
        depth: usize,
    ) -> Result<(), Error> {
        use comptime_compiler::normal_types::{Input, ShapeKind};
        let source = shape.source;
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        self.tick(Some(source))?;
        let value =
            match &shape.kind {
                ShapeKind::Choice {
                    condition,
                    condition_source,
                    then_input,
                    else_input,
                } => {
                    if !matches!(
                        condition,
                        Term::Primitive(Primitive::Bool | Primitive::Never)
                    ) {
                        return Err(self.failure(ErrorKind::Type, Some(*condition_source)));
                    }
                    self.prepare_normal_shape(then_input, expected, prepared, depth + 1)?;
                    self.prepare_normal_shape(else_input, expected, prepared, depth + 1)?;
                    return Ok(());
                }
                ShapeKind::Block { result } => {
                    if let Some(result) = result {
                        self.prepare_normal_shape(result, expected, prepared, depth + 1)?;
                    } else if !shape.diverges {
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
                ShapeKind::Scalar(Input::Value(ty)) => {
                    self.prepare_normal_value_type(ty, expected, source)?;
                    return Ok(());
                }
                ShapeKind::Scalar(Input::Unit) => {
                    Prepared::Literal(self.normal_unit_value(expected, source)?)
                }
                ShapeKind::Scalar(Input::Integer { spelling, negative }) => Prepared::Literal(
                    self.normal_integer_value(expected, spelling, *negative, source)?,
                ),
                ShapeKind::List(items) => {
                    let element = self.normal_list_domain(expected, items.len(), source)?;
                    for (spread, child) in items {
                        self.prepare_normal_shape(
                            child,
                            if *spread { expected } else { &element },
                            prepared,
                            depth + 1,
                        )?;
                    }
                    Prepared::List {
                        ty: expected.clone(),
                        types: vec![],
                        values: vec![],
                    }
                }
                ShapeKind::Tuple { head, items } => {
                    let type_tuple = *expected == Term::Primitive(Primitive::Type);
                    if type_tuple {
                        for child in items {
                            self.prepare_normal_shape(child, expected, prepared, depth + 1)?;
                        }
                        if let Some(head) = head {
                            if !self.normal_forms.get(head).is_some_and(|metadata| {
                                metadata.role == nir::normal_forms::Role::Tuple
                            }) {
                                return Err(self.failure(ErrorKind::Type, Some(source)));
                            }
                        }
                    } else {
                        let types = self
                            .tuple_input_elements(expected)
                            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                        if types.len() != items.len() {
                            return Err(self.failure(ErrorKind::Arity, Some(source)));
                        }
                        for (child, ty) in items.iter().zip(types) {
                            self.prepare_normal_shape(child, ty, prepared, depth + 1)?;
                        }
                    }
                    Prepared::Tuple {
                        head: type_tuple.then_some(*head),
                        values: vec![],
                    }
                }
            };
        prepared.insert(shape.slot, value);
        Ok(())
    }
    pub(super) fn append_compiled_normal_container(
        &mut self,
        container: &mut Prepared,
        value: Value,
        spread: bool,
        source: SourceRange,
    ) -> Result<(), Error> {
        match container {
            Prepared::List { ty, types, values } => {
                self.append_normal_list_item(ty, types, values, spread, value, source)
            }
            Prepared::Tuple { values, .. } => {
                values.push(value);
                Ok(())
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }
    pub(super) fn finish_compiled_normal_container(
        &mut self,
        container: Prepared,
        source: SourceRange,
    ) -> Result<Value, Error> {
        match container {
            Prepared::List { ty, types, values } => self.finish_normal_list(ty, types, values),
            Prepared::Tuple {
                head: Some(head),
                values,
            } => {
                let types = values
                    .into_iter()
                    .map(|value| self.type_description_term(value, source))
                    .collect::<Result<_, _>>()?;
                self.contextual_tuple_type(head, types, source)
            }
            Prepared::Tuple { head: None, values } => Ok(Value::Tuple(values)),
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }
    pub(super) fn normal_unit_value(
        &mut self,
        expected: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let value = match expected {
            Term::Primitive(Primitive::Type) => Value::Type(Term::Primitive(Primitive::Void)),
            _ => {
                self.check_type_contract(
                    &nir::program::TypePredicate::Equal(
                        Term::Primitive(Primitive::Void),
                        expected.clone(),
                    ),
                    source,
                )?;
                Value::Unit
            }
        };
        Ok(value)
    }
    pub(super) fn normal_integer_value(
        &mut self,
        expected: &Term,
        spelling: &str,
        negative: bool,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let integer = self.integer_literal(spelling, Some(source))?;
        let integer = if negative {
            self.integer_operation(Some(source), |budget| integer.negate(budget))?
        } else {
            integer
        };
        let value = match expected {
            Term::Primitive(Primitive::Integer) => Value::Integer(integer),
            Term::Primitive(Primitive::BitVec(width)) => Value::BitVec(
                self.bit_operation(source, |budget| integer.to_bitvec(*width, budget))?,
            ),
            Term::Primitive(primitive) if self.integer_layout(*primitive).is_some() => {
                let (signed, bits) = self.integer_layout(*primitive).unwrap();
                if signed {
                    Value::SignedInteger(
                        integer
                            .to_signed(bits)
                            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?,
                    )
                } else {
                    Value::UnsignedInteger(
                        integer
                            .to_unsigned(bits)
                            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?,
                    )
                }
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        };
        Ok(value)
    }
    pub(super) fn prepare_normal_value_type(
        &mut self,
        ty: &Term,
        expected: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if ty != expected && *ty != Term::Primitive(Primitive::Never) {
            self.check_type_contract(
                &nir::program::TypePredicate::Equal(ty.clone(), expected.clone()),
                source,
            )?;
        }
        Ok(())
    }
    pub(super) fn bind_compiled_normal_input(
        &mut self,
        inputs: &mut Inputs,
        value: Value,
        source: SourceRange,
    ) -> Result<(), Error> {
        let parameter = self
            .normal_forms
            .get(&inputs.head)
            .and_then(|head| head.parameters.get(inputs.arguments.len()))
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let parameter = nir::DeclarationParameter {
            declaration: parameter.declaration,
            sort: if parameter.kind == nir::normal_forms::Kind::Type {
                nir::Sort::Type
            } else {
                nir::Sort::ComptimeValue
            },
        };
        let term = self.residual_input_term(value, source)?;
        inputs.parameters.push(parameter);
        inputs.arguments.push(term);
        Ok(())
    }
}

/// Prepared domains and accumulators are invocation-local, never code-cache data.
pub(super) enum Prepared {
    Literal(Value),
    List {
        ty: Term,
        types: Vec<Term>,
        values: Vec<Value>,
    },
    Tuple {
        head: Option<Option<DeclarationId>>,
        values: Vec<Value>,
    },
}
impl fmt::Display for Prepared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, values) = match self {
            Self::Literal(value) => return write!(f, "(literal {value})"),
            Self::List { ty, types, values } => {
                write!(f, "(list (type {ty}) (types")?;
                for ty in types {
                    write!(f, " {ty}")?;
                }
                write!(f, ")")?;
                ("values", values)
            }
            Self::Tuple { head, values } => {
                write!(f, "(tuple (type-tuple {}) (head", head.is_some())?;
                if let Some(Some(head)) = head {
                    write!(f, " {head}")?;
                }
                write!(f, ")")?;
                ("values", values)
            }
        };
        write!(f, " ({kind}")?;
        for value in values {
            write!(f, " {value}")?;
        }
        write!(f, "))")
    }
}
