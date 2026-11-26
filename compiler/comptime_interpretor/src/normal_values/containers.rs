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
                    self.check_type_description(domain, None)?;
                    self.integer_operation(None, |budget| budget.consume(key.map_or(0, str::len)))?;
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
        let Term::NormalForm { head, .. } = ty else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        let element = self
            .query_admission(None, |vm, fuel| vm.admitted_collection_domain(ty, fuel))?
            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        let metadata = self
            .normal_forms
            .get(head)
            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        let fields = metadata
            .list_value_elements(ty, term)
            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        if fields.len() > 100_000 {
            return Err(self.failure(ErrorKind::RepresentationSizeLimit, None));
        }
        self.integer_operation(None, |budget| budget.consume(fields.len()))?;
        let values = fields
            .iter()
            .map(|value| {
                self.normal_pattern_value_at(value, &element, depth + 1, reconstruct_callables)
            })
            .collect::<Result<_, _>>()?;
        let value = Value::List {
            ty: ty.clone(),
            values,
        };
        self.validate_immutable_input(&value, ty)?;
        Ok(value)
    }

    #[inline(never)]
    pub(super) fn decode_array_normal_value(
        &mut self,
        term: &Term,
        ty: &Term,
        depth: usize,
        reconstruct_callables: bool,
    ) -> Result<Value, Error> {
        let Term::NominalValue {
            ty: actual,
            variant: None,
            fields,
        } = term
        else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        if actual.as_ref() != ty {
            return Err(self.failure(ErrorKind::Type, None));
        }
        let Some((element, len)) = self.query_admission(None, |vm, fuel| {
            nir::type_computations::charge_term(ty, fuel)?;
            Ok(nir::normal_forms::array_element_and_len_with(ty, &|id| {
                vm.normal_forms
                    .get(&id)
                    .is_some_and(|meta| meta.role == nir::normal_forms::Role::Array)
            }))
        })?
        else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        if fields.len() != len {
            return Err(self.failure(ErrorKind::Type, None));
        }
        if fields.len() > 100_000 {
            return Err(self.failure(ErrorKind::RepresentationSizeLimit, None));
        }
        self.integer_operation(None, |budget| budget.consume(fields.len()))?;
        let values = fields
            .iter()
            .map(|value| {
                self.normal_pattern_value_at(value, &element, depth + 1, reconstruct_callables)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let value = Value::Array {
            ty: ty.clone(),
            values,
        };
        self.validate_immutable_input(&value, ty)?;
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
                let types = self.query_admission(None, |vm, fuel| {
                    vm.query_normal_value_fields(ty, *variant, fuel)
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
                        |_| matches!(self.concrete_nominal(ty), Some(ConcreteNominal::Newtype(_))),
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
                // Field names establish membership; restore declaration order
                // before publishing the positional runtime representation.
                let mut named = BTreeMap::new();
                for (name, value) in values {
                    self.integer_operation(None, |budget| budget.consume(name.len() + 1))?;
                    if named.insert(name.as_str(), value).is_some() {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                }
                let mut fields = Vec::new();
                for (name, ty) in types {
                    self.integer_operation(None, |budget| budget.consume(name.len()))?;
                    let value = named
                        .remove(name.as_str())
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
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
        let Term::TupleValue(values) = term else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        let Some(types) = self.tuple_input_elements(ty) else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        if values.len() != types.len() {
            return Err(self.failure(ErrorKind::Type, None));
        }
        let mut decoded = Vec::with_capacity(values.len());
        for (value, ty) in values.iter().zip(types) {
            decoded.push(self.normal_pattern_value_at(
                value,
                ty,
                depth + 1,
                reconstruct_callables,
            )?);
        }
        Ok(Value::Tuple(decoded))
    }
}
