//! Ordered domain conversion for retained Type pattern inputs.
use super::*;
use nir::pattern_structure::TypeStructure as S;

pub(super) struct TypePattern {
    shape: S,
    next: usize,
    binders: Vec<nir::DeclarationParameter>,
}
impl fmt::Display for TypePattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(type-pattern {} (next {}) (binders",
            self.shape, self.next
        )?;
        for binder in &self.binders {
            write!(f, " {binder}")?;
        }
        write!(f, "))")
    }
}

impl<'a> Interpreter<'a> {
    fn type_pattern_tuple<'v>(&self, input: &'v Value) -> Result<Option<&'v [Term]>, Error> {
        let Value::Type(ty) = input else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        Ok(Some(match ty {
            Term::Tuple(types) => types,
            Term::NormalForm { head, .. } => self
                .normal_forms
                .get(head)
                .and_then(|metadata| metadata.tuple_elements(ty))
                .ok_or_else(|| self.failure(ErrorKind::TypeContractUnresolved, None))?,
            Term::Primitive(Primitive::Unit) => {
                return Err(self.failure(ErrorKind::TypeContractUnresolved, None));
            }
            Term::Primitive(_)
            | Term::Pointer(_)
            | Term::Function { .. }
            | Term::ComptimeFunction(_) => return Ok(None),
            _ => return Err(self.failure(ErrorKind::TypeContractUnresolved, None)),
        }))
    }

    fn nominal_pattern_metadata(
        &self,
        declaration: DeclarationId,
    ) -> Result<(Vec<nir::DeclarationParameter>, &[Term]), Error> {
        let (binders, domains) = if let Some(def) = self.structs.get(&declaration) {
            (def.parameter_binders(), def.parameter_types.as_slice())
        } else if let Some(def) = self.enums.get(&declaration) {
            (def.parameter_binders(), def.parameter_types.as_slice())
        } else if let Some(def) = self.newtypes.get(&declaration) {
            (def.parameter_binders(), def.parameter_types.as_slice())
        } else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        Ok((
            binders.map_err(|_| self.failure(ErrorKind::Type, None))?,
            domains,
        ))
    }

    pub(super) fn prepare_type_pattern(
        &mut self,
        shape: S,
        input: &Value,
    ) -> Result<Option<TypePattern>, Error> {
        if matches!(input, Value::AbstractValue { .. }) {
            return Err(self.failure(ErrorKind::AbstractValueUnresolved, None));
        }
        if matches!(
            input,
            Value::TypeParameter(_) | Value::Type(Term::Rigid { .. })
        ) {
            return Err(self.failure(ErrorKind::TypeContractUnresolved, None));
        }
        let binders = match shape {
            S::Tuple { fields } => {
                let Some(types) = self.type_pattern_tuple(input)? else {
                    return Ok(None);
                };
                if types.len() != fields {
                    return Ok(None);
                };
                vec![]
            }
            S::Nominal {
                declaration,
                fields,
            } => {
                let Value::Type(Term::Nominal {
                    declaration: actual,
                    arguments,
                }) = input
                else {
                    return Ok(None);
                };
                if *actual != declaration {
                    return Ok(None);
                };
                let (binders, _) = self.nominal_pattern_metadata(declaration)?;
                if arguments.len() != fields || binders.len() != fields {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                binders
            }
            S::Normal { head, fields } => {
                let metadata = self
                    .normal_forms
                    .get(&head)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                if fields != metadata.parameters.len() {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                let Value::Type(ty) = input else {
                    return Err(self.failure(ErrorKind::Type, None));
                };
                let Term::NormalForm {
                    head: actual,
                    arguments,
                    ..
                } = ty
                else {
                    return Ok(None);
                };
                if *actual != head {
                    return Ok(None);
                };
                if arguments.len() != fields {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                let work = self
                    .type_description_work(ty)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                self.integer_operation(None, |budget| budget.consume(work))?;
                self.effect_charge(metadata.parameters.len(), None)?;
                metadata
                    .parameters
                    .iter()
                    .map(|p| nir::DeclarationParameter {
                        declaration: p.declaration,
                        sort: if p.kind == nir::normal_forms::Kind::Type {
                            nir::Sort::Type
                        } else {
                            nir::Sort::ComptimeValue
                        },
                    })
                    .collect()
            }
        };
        Ok(Some(TypePattern {
            shape,
            next: 0,
            binders,
        }))
    }

    fn type_pattern_domain(
        &mut self,
        domain: &Term,
        binders: &[nir::DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Term, Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = nir::type_computations::instantiate_parameters_bounded(
            domain,
            binders,
            arguments,
            &[],
            &mut fuel,
        );
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|e| {
            self.failure(
                if matches!(e, nir::type_computations::Error::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                None,
            )
        })
    }

    pub(super) fn type_pattern_field(
        &mut self,
        state: &mut TypePattern,
        input: &Value,
        index: usize,
        wild: bool,
    ) -> Result<Option<Value>, Error> {
        if index != state.next || index >= state.shape.fields() {
            return Err(self.failure(ErrorKind::InvalidCallPlan, None));
        }
        let value = match state.shape {
            S::Tuple { .. } => {
                let ty = self
                    .type_pattern_tuple(input)?
                    .and_then(|types| types.get(index))
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                let work = self
                    .type_description_work(ty)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                self.integer_operation(None, |budget| budget.consume(work))?;
                if wild {
                    None
                } else {
                    Some(Value::Type(ty.clone()))
                }
            }
            S::Nominal { declaration, .. } => {
                let Value::Type(Term::Nominal { arguments, .. }) = input else {
                    return Err(self.failure(ErrorKind::Type, None));
                };
                self.tick(None)?;
                let struct_definition = self.structs.get(&declaration).cloned();
                let enum_definition = self.enums.get(&declaration).cloned();
                let domains = if let Some(def) = struct_definition.as_ref() {
                    def.parameter_types.as_slice()
                } else if let Some(def) = enum_definition.as_ref() {
                    def.parameter_types.as_slice()
                } else if let Some(def) = self.newtypes.get(&declaration) {
                    def.parameter_types.as_slice()
                } else {
                    return Err(self.failure(ErrorKind::Type, None));
                };
                let domain = domains
                    .get(index)
                    .unwrap_or(&Term::Primitive(Primitive::Type));
                let domain = self.type_pattern_domain(domain, &state.binders, arguments)?;
                let argument = arguments
                    .get(index)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                let value = self.normal_pattern_value(argument, &domain)?;
                if wild { None } else { Some(value) }
            }
            S::Normal { head, .. } => {
                let metadata = self
                    .normal_forms
                    .get(&head)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                let Value::Type(Term::NormalForm { arguments, .. }) = input else {
                    return Err(self.failure(ErrorKind::Type, None));
                };
                let parameter = metadata
                    .parameters
                    .get(index)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                let prefix = arguments
                    .get(..index)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                let domain =
                    self.type_pattern_domain(&parameter.ty, &state.binders[..index], prefix)?;
                // A normal-form wildcard keeps the original opaque-input rule.
                if wild {
                    None
                } else {
                    let argument = arguments
                        .get(index)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    Some(self.normal_pattern_value(argument, &domain)?)
                }
            }
        };
        state.next += 1;
        Ok(value)
    }

    pub(super) fn match_type_structure(
        &mut self,
        shape: S,
        fields: &[nir::program::Pattern],
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        let Some(mut state) = self.prepare_type_pattern(shape, input)? else {
            return Ok(Flow::Value(Value::Bool(false)));
        };
        for (index, pattern) in fields.iter().enumerate() {
            let wild = matches!(pattern, nir::program::Pattern::Wild);
            if let Some(value) = self.type_pattern_field(&mut state, input, index, wild)? {
                match self.match_pattern_field(pattern, &value, environment, undo, None)? {
                    Flow::Value(Value::Bool(true)) => {}
                    flow => return Ok(flow),
                }
            }
        }
        Ok(Flow::Value(Value::Bool(true)))
    }
}
