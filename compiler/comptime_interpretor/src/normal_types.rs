//! Invocation-local state for sequential normal-form construction.
use super::*;

pub(super) struct Inputs {
    pub head: DeclarationId,
    pub arguments: Vec<Term>,
    parameters: Vec<nir::DeclarationParameter>,
    pub literal: Option<Value>,
}
impl Inputs {
    pub fn new(head: DeclarationId) -> Self {
        Self {
            head,
            arguments: vec![],
            parameters: vec![],
            literal: None,
        }
    }
}
impl fmt::Display for Inputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(normal-inputs (head {}) (arguments", self.head)?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") (literal")?;
        if let Some(value) = &self.literal {
            write!(f, " {value}")?;
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
        input: &comptime_compiler::normal_types::Input,
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
        use comptime_compiler::normal_types::Input;
        let literal = match input {
            Input::Value(ty) => {
                self.prepare_normal_value_type(ty, &expected, source)?;
                None
            }
            Input::Unit => Some(self.normal_unit_value(&expected, source)?),
            Input::Integer { spelling, negative } => {
                Some(self.normal_integer_value(&expected, spelling, *negative, source)?)
            }
        };
        inputs.literal = literal;
        Ok(())
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
