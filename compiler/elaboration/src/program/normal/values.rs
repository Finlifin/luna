//! Scoped normalization of already admitted invocation results.
use super::*;

impl Checker<'_, '_> {
    // Normalization can retain scoped Type descriptions. This is distinct from
    // completed immutable export, which must continue rejecting open values.
    pub(in crate::program) fn required_normal_term(
        &mut self,
        value: comptime_interpretor::Value,
        span: Span,
    ) -> Result<Term, Error> {
        self.optional_normal_term(value, span)?
            .ok_or_else(|| error(span, "normal-form input freezing failed: Unsupported"))
    }

    // Delivery-only values need no symbolic identity. Resource, scope and
    // representation failures still propagate; only absent representation is optional.
    pub(in crate::program) fn optional_normal_term(
        &mut self,
        value: comptime_interpretor::Value,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        self.optional_normal_term_at(value, span, 0)
    }

    fn optional_normal_term_at(
        &mut self,
        value: comptime_interpretor::Value,
        span: Span,
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        if depth >= 128 {
            return Err(error(span, "normal-form input nesting limit exceeded"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "normal-form input freezing budget exhausted"))?;
        use comptime_interpretor::Value;
        let term = match value {
            Value::Type(ty) if ty.sort() == Sort::Type => ty,
            Value::Type(_) => {
                return Err(error(span, "normal-form Type input contains a non-Type"));
            }
            Value::RuntimeFunction {
                declaration,
                arguments,
            } => {
                // The required invocation restored these declaration inputs
                // into the caller's telescope. Closed export deliberately
                // rejects Bound terms; scoped normalization retains them.
                Term::FunctionValue {
                    declaration,
                    arguments,
                }
            }
            Value::Record {
                declaration,
                arguments,
                fields,
            } => {
                let Some(fields) = fields
                    .into_iter()
                    .map(|value| self.optional_normal_term_at(value, span, depth + 1))
                    .collect::<Result<Option<Vec<_>>, _>>()?
                else {
                    return Ok(None);
                };
                Term::NominalValue {
                    ty: Box::new(Term::Nominal {
                        declaration,
                        arguments,
                    }),
                    variant: None,
                    fields,
                }
            }
            Value::Variant {
                declaration,
                arguments,
                index,
                fields,
            } => {
                let Some(fields) = fields
                    .into_iter()
                    .map(|value| self.optional_normal_term_at(value, span, depth + 1))
                    .collect::<Result<Option<Vec<_>>, _>>()?
                else {
                    return Ok(None);
                };
                Term::NominalValue {
                    ty: Box::new(Term::Nominal {
                        declaration,
                        arguments,
                    }),
                    variant: Some(u32::try_from(index).map_err(|_| {
                        error(span, "enum variant index exceeds frozen representation")
                    })?),
                    fields,
                }
            }
            Value::Tuple(values) => {
                let Some(fields) = values
                    .into_iter()
                    .map(|value| self.optional_normal_term_at(value, span, depth + 1))
                    .collect::<Result<Option<Vec<_>>, _>>()?
                else {
                    return Ok(None);
                };
                Term::TupleValue(fields)
            }
            Value::AnonymousRecord(values) => {
                let Some(fields) = values
                    .into_iter()
                    .map(|(name, value)| {
                        self.optional_normal_term_at(value, span, depth + 1)
                            .map(|value| value.map(|value| (name, value)))
                    })
                    .collect::<Result<Option<Vec<_>>, _>>()?
                else {
                    return Ok(None);
                };
                Term::RecordValue(fields)
            }
            Value::Optional { ty, value } => {
                if !matches!(ty, Term::Optional(_)) {
                    return Err(error(span, "optional input has an invalid domain"));
                }
                let value = if let Some(value) = value {
                    let Some(value) = self.optional_normal_term_at(*value, span, depth + 1)? else {
                        return Ok(None);
                    };
                    Some(Box::new(value))
                } else {
                    None
                };
                Term::OptionalValue {
                    ty: Box::new(ty),
                    value,
                }
            }
            Value::ErrorResult {
                ty,
                error: tag,
                value,
            } => {
                let Term::ErrorResult { errors, .. } = &ty else {
                    return Err(error(span, "error input has an invalid domain"));
                };
                let label = tag
                    .map(|index| {
                        errors
                            .get(index)
                            .cloned()
                            .map(Box::new)
                            .ok_or_else(|| error(span, "error input has an invalid branch"))
                    })
                    .transpose()?;
                let Some(value) = self.optional_normal_term_at(*value, span, depth + 1)? else {
                    return Ok(None);
                };
                Term::ErrorValue {
                    ty: Box::new(ty),
                    label,
                    value: Box::new(value),
                }
            }
            Value::Newtype { ty, value } => {
                if !matches!(ty, Term::Nominal { .. }) {
                    return Err(error(span, "newtype input has an invalid domain"));
                }
                let Some(value) = self.optional_normal_term_at(*value, span, depth + 1)? else {
                    return Ok(None);
                };
                Term::NominalValue {
                    ty: Box::new(ty),
                    variant: None,
                    fields: vec![value],
                }
            }
            Value::List { ty, values } => {
                // Invocation results have already been admitted by the VM and
                // restored to this call's telescope. Freeze children in that
                // same scope rather than requiring closed Type descriptions.
                let mut fields = Vec::with_capacity(values.len());
                for value in values {
                    let Some(field) = self.optional_normal_term_at(value, span, depth + 1)? else {
                        return Ok(None);
                    };
                    fields.push(field);
                }
                if self.is_type_list_type(&ty) {
                    Term::TypeListValue(fields)
                } else {
                    Term::NominalValue {
                        ty: Box::new(ty),
                        variant: None,
                        fields,
                    }
                }
            }
            value => {
                let parameters = self.visible_comptime_parameters(span)?;
                let mut value_parameters = BTreeMap::new();
                for id in &parameters {
                    if let Some(domain) = self
                        .trait_input_domain(*id)
                        .filter(|domain| *domain != p(P::Type))
                    {
                        value_parameters.insert(*id, self.substituted_trait_domain(&domain, span)?);
                    }
                }
                let parameters = parameters
                    .into_iter()
                    .filter(|id| !value_parameters.contains_key(id))
                    .collect();
                // Domain substitution consumes preparation fuel before the
                // exporter receives its remaining allowance.
                let steps = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let mut budget = comptime_runtime::Budget {
                    steps,
                    max_limbs: 4096,
                };
                let term = value.into_abstract_invocation_term(
                    &mut budget,
                    &parameters,
                    &value_parameters,
                );
                self.required_steps -= (steps - budget.steps) as u64;
                return term.map(Some).or_else(|failure| {
                    if matches!(failure, comptime_interpretor::ErrorKind::Unsupported) {
                        return Ok(None);
                    }
                    Err(error(
                        span,
                        format!("normal-form input freezing failed: {failure:?}"),
                    ))
                });
            }
        };
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let bounded = nir::type_computations::charge_term(&term, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        bounded.map_err(|e| error(span, e.to_string()))?;
        if let Term::TypeListValue(types) = &term {
            if types.iter().any(|ty| ty.sort() != Sort::Type) {
                return Err(error(
                    span,
                    "normal-form Type-list input contains a non-Type",
                ));
            }
        }
        term.check_scope(&self.call_input_frames())
            .map_err(|e| error(span, e.to_string()))?;
        Ok(Some(term))
    }
}
