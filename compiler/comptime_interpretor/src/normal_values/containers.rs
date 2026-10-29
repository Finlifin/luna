//! Container reconstruction preserves field order and per-child admission.
use super::*;

impl Interpreter<'_> {
    #[inline(never)]
    pub(super) fn decode_optional_normal_value(
        &mut self,
        term: &Term,
        ty: &Term,
        depth: usize,
        reconstruct_callables: bool,
    ) -> Result<Value, Error> {
        let value = match (term, ty) {
            (Term::OptionalValue { ty: actual, value }, Term::Optional(payload))
                if actual.as_ref() == ty =>
            {
                Value::Optional {
                    ty: ty.clone(),
                    value: value
                        .as_ref()
                        .map(|value| {
                            self.normal_pattern_value_at(
                                value,
                                payload,
                                depth + 1,
                                reconstruct_callables,
                            )
                            .map(Box::new)
                        })
                        .transpose()?,
                }
            }
            _ => return Err(self.failure(ErrorKind::Type, None)),
        };
        Ok(value)
    }
    #[inline(never)]
    pub(super) fn decode_error_normal_value(
        &mut self,
        term: &Term,
        ty: &Term,
        depth: usize,
        reconstruct_callables: bool,
    ) -> Result<Value, Error> {
        let value = match (term, ty) {
            (
                Term::ErrorValue {
                    ty: actual,
                    label,
                    value,
                },
                Term::ErrorResult { errors, result },
            ) if actual.as_ref() == ty => {
                let (error, payload) = match label {
                    None => (None, result.as_ref()),
                    Some(label) => (
                        Some(
                            errors
                                .iter()
                                .position(|ty| ty == label.as_ref())
                                .ok_or_else(|| self.failure(ErrorKind::Type, None))?,
                        ),
                        label.as_ref(),
                    ),
                };
                Value::ErrorResult {
                    ty: ty.clone(),
                    error,
                    value: Box::new(self.normal_pattern_value_at(
                        value,
                        payload,
                        depth + 1,
                        reconstruct_callables,
                    )?),
                }
            }
            _ => return Err(self.failure(ErrorKind::Type, None)),
        };
        Ok(value)
    }
    #[inline(never)]
    pub(super) fn decode_object_normal_value(
        &mut self,
        term: &Term,
        ty: &Term,
        depth: usize,
        reconstruct_callables: bool,
    ) -> Result<Value, Error> {
        let value = match (term, ty) {
            (
                Term::NominalValue {
                    ty: actual,
                    variant: None,
                    fields,
                },
                Term::Primitive(Primitive::Object),
            ) if actual.as_ref() == ty => {
                if !self.comptime_only {
                    return Err(self.failure(ErrorKind::PhaseMismatch, None));
                }
                self.integer_operation(None, |budget| budget.consume(fields.len()))?;
                let mut entries = Vec::with_capacity(fields.len());
                for entry in fields {
                    let (key, domain, value) = nir::objects::entry_parts(entry)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    let work = self
                        .type_description_work(domain)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    self.integer_operation(None, |budget| {
                        budget.consume(work.saturating_add(key.map_or(0, str::len)))
                    })?;
                    let value = ObjectValue {
                        ty: domain.clone(),
                        value: self.normal_pattern_value_at(
                            value,
                            domain,
                            depth + 1,
                            reconstruct_callables,
                        )?,
                    };
                    entries.push(match key {
                        Some(key) => ObjectEntry::Property {
                            key: key.to_owned(),
                            value,
                        },
                        None => ObjectEntry::Value(value),
                    });
                }
                Value::Object(Object::from_entries(entries))
            }
            _ => return Err(self.failure(ErrorKind::Type, None)),
        };
        Ok(value)
    }
    #[inline(never)]
    pub(super) fn decode_list_normal_value(
        &mut self,
        term: &Term,
        ty: &Term,
        depth: usize,
        reconstruct_callables: bool,
    ) -> Result<Value, Error> {
        let value = match (term, ty) {
            (
                Term::NominalValue {
                    ty: actual,
                    variant: None,
                    fields,
                },
                Term::NormalForm { .. },
            ) if actual.as_ref() == ty => {
                let element = self
                    .list_element(ty)
                    .cloned()
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                if element == Term::Primitive(Primitive::Type) {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                self.integer_operation(None, |budget| budget.consume(fields.len()))?;
                let values = fields
                    .iter()
                    .map(|value| {
                        self.normal_pattern_value_at(
                            value,
                            &element,
                            depth + 1,
                            reconstruct_callables,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                let value = Value::List {
                    ty: ty.clone(),
                    values,
                };
                self.validate_immutable_input(&value, ty)?;
                value
            }
            _ => return Err(self.failure(ErrorKind::Type, None)),
        };
        Ok(value)
    }
    #[inline(never)]
    pub(super) fn decode_nominal_normal_value(
        &mut self,
        term: &Term,
        ty: &Term,
        depth: usize,
        reconstruct_callables: bool,
    ) -> Result<Value, Error> {
        let value = match (term, ty) {
            (
                Term::NominalValue {
                    ty: actual,
                    variant,
                    fields,
                },
                Term::Nominal {
                    declaration,
                    arguments,
                },
            ) if actual.as_ref() == ty => {
                self.integer_operation(None, |budget| budget.consume(fields.len()))?;
                let mut fuel = usize::try_from(self.budget.steps)
                    .unwrap_or(usize::MAX)
                    .min(100_000);
                let before = fuel;
                let types = self.normal_value_fields(ty, *variant, &mut fuel);
                self.budget.steps -= (before - fuel) as u64;
                let types = types.ok_or_else(|| {
                    self.failure(
                        if fuel == 0 {
                            ErrorKind::StepsExhausted
                        } else {
                            ErrorKind::Type
                        },
                        None,
                    )
                })?;
                if fields.len() != types.len() {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                let values = fields
                    .iter()
                    .zip(&types)
                    .map(|(value, ty)| {
                        self.normal_pattern_value_at(value, ty, depth + 1, reconstruct_callables)
                    })
                    .collect::<Result<_, _>>()?;
                match variant {
                    Some(index) => Value::Variant {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                        index: *index as usize,
                        fields: values,
                    },
                    None if self.graph.map_or_else(
                        || self.newtypes.contains_key(declaration),
                        |graph| {
                            graph.nominals.iter().any(|n| {
                                n.ty == *ty
                                    && matches!(n.definition, iir::NominalDefinition::Newtype(_))
                            })
                        },
                    ) =>
                    {
                        let [value]: [Value; 1] = values
                            .try_into()
                            .map_err(|_| self.failure(ErrorKind::Type, None))?;
                        Value::Newtype {
                            ty: ty.clone(),
                            value: Box::new(value),
                        }
                    }
                    None => Value::Record {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                        fields: values,
                    },
                }
            }
            _ => return Err(self.failure(ErrorKind::Type, None)),
        };
        Ok(value)
    }
    #[inline(never)]
    pub(super) fn decode_record_normal_value(
        &mut self,
        term: &Term,
        ty: &Term,
        depth: usize,
        reconstruct_callables: bool,
    ) -> Result<Value, Error> {
        let value = match (term, ty) {
            (Term::RecordValue(values), Term::Record(types)) if values.len() == types.len() => {
                let mut fields = Vec::new();
                for ((name, value), (expected_name, ty)) in values.iter().zip(types) {
                    self.integer_operation(None, |budget| budget.consume(name.len()))?;
                    if name != expected_name {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                    fields.push((
                        name.clone(),
                        self.normal_pattern_value_at(value, ty, depth + 1, reconstruct_callables)?,
                    ));
                }
                Value::AnonymousRecord(fields)
            }
            _ => return Err(self.failure(ErrorKind::Type, None)),
        };
        Ok(value)
    }
    #[inline(never)]
    pub(super) fn decode_tuple_normal_value(
        &mut self,
        term: &Term,
        ty: &Term,
        depth: usize,
        reconstruct_callables: bool,
    ) -> Result<Value, Error> {
        let value = match (term, ty) {
            (Term::TupleValue(values), ty) => {
                let Some(types) = self.tuple_input_elements(ty) else {
                    return Err(self.failure(ErrorKind::Type, None));
                };
                if values.len() != types.len() {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                Value::Tuple(
                    values
                        .iter()
                        .zip(types)
                        .map(|(value, ty)| {
                            self.normal_pattern_value_at(
                                value,
                                ty,
                                depth + 1,
                                reconstruct_callables,
                            )
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
            _ => return Err(self.failure(ErrorKind::Type, None)),
        };
        Ok(value)
    }
}
