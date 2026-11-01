//! Explicit encoding observations share checked declaration selection with structure reflection.
use super::*;
use comptime_runtime::Integer;
use nir::program::EnumBitQuery as Q;

impl Interpreter<'_> {
    pub(super) fn observe_enum_bit_encoding(
        &mut self,
        value: Value,
        output: &Term,
        query: Q,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let fail = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        let Value::Type(subject) = value else {
            return Err(fail());
        };
        let (_, definition) = self
            .reflection_enum_definition(&subject)
            .map_err(|mut error| {
                error.source = Some(source);
                error
            })?;
        let encoding = definition.encoding.as_ref().ok_or_else(fail)?;
        self.integer_operation(Some(source), |budget| budget.consume(1))?;
        let value = match query {
            Q::Width => Value::Integer(Integer::from_u128(encoding.width() as u128)),
            Q::BaseType => Value::Optional {
                ty: output.clone(),
                value: encoding
                    .base_type
                    .map(|base| Box::new(Value::Type(Term::Primitive(base)))),
            },
            Q::Masks | Q::Values => {
                self.integer_operation(Some(source), |budget| {
                    budget.consume(encoding.variants().len())
                })?;
                let mut values = Vec::with_capacity(encoding.variants().len());
                for variant in encoding.variants() {
                    let bits = if query == Q::Masks {
                        variant.fixed_mask()
                    } else {
                        variant.fixed_value()
                    };
                    let integer = self.integer_operation(Some(source), |budget| {
                        Integer::from_bitvec(bits, budget)
                    })?;
                    values.push(Value::Integer(integer));
                }
                Value::List {
                    ty: output.clone(),
                    values: values.into(),
                }
            }
            Q::Offsets => {
                let inner = self.list_element(output).ok_or_else(fail)?.clone();
                self.integer_operation(Some(source), |budget| {
                    budget.consume(encoding.variants().len())
                })?;
                let mut values = Vec::with_capacity(encoding.variants().len());
                for variant in encoding.variants() {
                    self.integer_operation(Some(source), |budget| {
                        budget.consume(variant.fields().len())
                    })?;
                    values.push(Value::List {
                        ty: inner.clone(),
                        values: variant
                            .fields()
                            .iter()
                            .map(|field| Value::Integer(Integer::from_u128(field.start as u128)))
                            .collect(),
                    });
                }
                Value::List {
                    ty: output.clone(),
                    values: values.into(),
                }
            }
        };
        self.validate_immutable_input(&value, output)?;
        Ok(value)
    }
}
