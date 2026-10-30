//! Shared value-shape admission and ordered field observation for both engines.
use super::*;
use nir::pattern_structure::View as S;

impl Interpreter<'_> {
    pub(super) fn prepare_pattern_structure(
        &self,
        shape: S<'_>,
        input: &Value,
    ) -> Result<bool, Error> {
        if matches!(input, Value::AbstractValue { .. }) {
            return Err(self.failure(ErrorKind::AbstractValueUnresolved, None));
        }
        if matches!(
            input,
            Value::TypeParameter(_) | Value::Type(Term::Rigid { .. })
        ) {
            return Err(self.failure(ErrorKind::TypeContractUnresolved, None));
        }
        Ok(match (shape, input) {
            (S::Tuple { fields }, Value::Tuple(values)) => fields == values.len(),
            (
                S::Record {
                    declaration,
                    fields,
                },
                Value::Record {
                    declaration: actual,
                    fields: values,
                    ..
                },
            ) => declaration == *actual && fields == values.len(),
            (
                S::Variant {
                    declaration,
                    index,
                    fields,
                },
                Value::Variant {
                    declaration: actual,
                    index: found,
                    fields: values,
                    ..
                },
            ) => declaration == *actual && index == *found && fields == values.len(),
            (S::AnonymousRecord { names, fields }, Value::AnonymousRecord(values)) => {
                if names.len() != fields || values.len() != fields {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                true
            }
            (S::OptionalPresent, Value::Optional { value: Some(_), .. }) => true,
            (S::ErrorResult { label }, Value::ErrorResult { ty, error, .. }) => {
                let Term::ErrorResult { errors, .. } = ty else {
                    return Err(self.failure(ErrorKind::Type, None));
                };
                match error {
                    None => label.is_none(),
                    Some(index) => {
                        Some(
                            errors
                                .get(*index)
                                .ok_or_else(|| self.failure(ErrorKind::Type, None))?,
                        ) == label
                    }
                }
            }
            _ => false,
        })
    }

    pub(super) fn pattern_structure_field<'v>(
        &mut self,
        shape: S<'_>,
        input: &'v Value,
        index: usize,
    ) -> Result<&'v Value, Error> {
        let value = match (shape, input) {
            (S::Tuple { .. }, Value::Tuple(fields))
            | (S::Record { .. }, Value::Record { fields, .. })
            | (S::Variant { .. }, Value::Variant { fields, .. }) => fields.get(index),
            (S::AnonymousRecord { names, .. }, Value::AnonymousRecord(fields)) => {
                let (actual, value) = fields
                    .get(index)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                let name = names
                    .get(index)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                self.integer_operation(None, |budget| {
                    budget.consume(name.len().saturating_add(actual.len()).saturating_add(1))
                })?;
                if name != actual {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                Some(value)
            }
            (
                S::OptionalPresent,
                Value::Optional {
                    value: Some(value), ..
                },
            ) if index == 0 => Some(value.as_ref()),
            (S::ErrorResult { .. }, Value::ErrorResult { value, .. }) if index == 0 => {
                Some(value.as_ref())
            }
            _ => None,
        };
        value.ok_or_else(|| self.failure(ErrorKind::Type, None))
    }

    pub(super) fn match_value_structure(
        &mut self,
        shape: S<'_>,
        fields: &[nir::program::Pattern],
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        if !self.prepare_pattern_structure(shape, input)? {
            return Ok(Flow::Value(Value::Bool(false)));
        }
        for (index, pattern) in fields.iter().enumerate() {
            let value = self.pattern_structure_field(shape, input, index)?;
            match self.match_pattern_field(pattern, value, environment, undo, Some(index))? {
                Flow::Value(Value::Bool(true)) => {}
                flow => return Ok(flow),
            }
        }
        Ok(Flow::Value(Value::Bool(true)))
    }
}
