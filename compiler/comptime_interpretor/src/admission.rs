//! Bounded immutable host-data admission. Representation validation is not
//! authorization to duplicate resources or reuse an equality-sensitive query.
use super::*;

impl Interpreter<'_> {
    pub(super) fn admitted_nominal_layout(
        &self,
        ty: &Term,
        variant: Option<u32>,
        fuel: &mut usize,
    ) -> Result<Vec<Term>, nir::NominalAdmissionError> {
        self.normal_value_fields(ty, variant, fuel).ok_or_else(|| {
            if *fuel == 0 {
                nir::NominalAdmissionError::Budget
            } else {
                nir::NominalAdmissionError::Invalid("nominal value layout unavailable".into())
            }
        })
    }

    /// Validate immutable data against a closed domain, including nominal
    /// formation predicates. In a comptime normalization scope, Type descriptions
    /// may mention only that scope's authorized declaration inputs. This does not
    /// authorize closed export. No source Eq/Hash or user function executes here.
    pub fn validate_immutable_input(&mut self, value: &Value, ty: &Term) -> Result<(), Error> {
        self.tick(None)?;
        let work = value
            .scoped_copy_work(&self.abstract_type_parameters)
            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        self.integer_operation(None, |budget| budget.consume(work))?;
        let work = type_value_work(ty, &self.normal_forms)
            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        self.integer_operation(None, |budget| budget.consume(work))?;
        ty.check_scope(&[])
            .map_err(|_| self.failure(ErrorKind::Type, None))?;
        self.admit_type(ty, 0)?;
        if !self.has_type(value, ty) {
            return Err(self.failure(ErrorKind::Type, None));
        }
        self.admit_value_types(value, 0)
    }
    fn admit_value_types(&mut self, value: &Value, depth: usize) -> Result<(), Error> {
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, None));
        }
        self.tick(None)?;
        match value {
            Value::Newtype { ty, value } => {
                self.admit_type(ty, depth + 1)?;
                self.admit_value_types(value, depth + 1)?;
            }
            Value::Record {
                declaration,
                arguments,
                fields,
            }
            | Value::Variant {
                declaration,
                arguments,
                fields,
                ..
            } => {
                self.admit_type(
                    &Term::Nominal {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                    },
                    depth,
                )?;
                for field in fields {
                    self.admit_value_types(field, depth + 1)?;
                }
            }
            Value::Type(ty) => {
                ty.check_scope(&[])
                    .map_err(|_| self.failure(ErrorKind::Type, None))?;
                self.admit_type(ty, depth)?;
            }
            Value::TypeList(types) => {
                for ty in types {
                    ty.check_scope(&[])
                        .map_err(|_| self.failure(ErrorKind::Type, None))?;
                    self.admit_type(ty, depth + 1)?;
                }
            }
            Value::Tuple(fields) => {
                for field in fields {
                    self.admit_value_types(field, depth + 1)?;
                }
            }
            Value::AnonymousRecord(fields) => {
                for (_, field) in fields {
                    self.admit_value_types(field, depth + 1)?;
                }
            }
            _ => {} // immutable_copy_work already rejected storage and callables.
        }
        Ok(())
    }
    fn admit_type(&mut self, ty: &Term, depth: usize) -> Result<(), Error> {
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, None));
        }
        self.tick(None)?;
        let source = SourceRange { start: 0, end: 0 };
        match ty {
            Term::Nominal {
                declaration,
                arguments,
            } => {
                let (parameters, domains, requirements, capabilities) =
                    if let Some(s) = self.structs.get(declaration) {
                        (
                            &s.type_parameters,
                            &s.parameter_types,
                            &s.requirements,
                            &s.capabilities,
                        )
                    } else if let Some(e) = self.enums.get(declaration) {
                        (
                            &e.type_parameters,
                            &e.parameter_types,
                            &e.requirements,
                            &e.capabilities,
                        )
                    } else if let Some(n) = self.newtypes.get(declaration) {
                        (
                            &n.type_parameters,
                            &n.parameter_types,
                            &n.requirements,
                            &n.capabilities,
                        )
                    } else {
                        return Err(self.failure(ErrorKind::Type, None));
                    };
                // Prove formation even for phantom arguments with no fields.
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut remaining = before;
                let valid = if let Some(definition) = self.newtypes.get(declaration) {
                    definition.validate_completed_arguments_with_layouts(
                        arguments,
                        &mut remaining,
                        |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                    )
                } else {
                    nir::validate_declaration_arguments_with_layouts(
                        parameters,
                        domains,
                        arguments,
                        &mut remaining,
                        |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                    )
                };
                self.budget.steps -= (before - remaining) as u64;
                valid.map_err(|error| {
                    self.failure(
                        match error {
                            nir::NominalAdmissionError::Budget => ErrorKind::StepsExhausted,
                            nir::NominalAdmissionError::Invalid(_) => ErrorKind::Type,
                        },
                        None,
                    )
                })?;
                let parameters = nir::declaration_parameters(parameters, domains)
                    .map_err(|_| self.failure(ErrorKind::Type, None))?;
                let requirements = requirements
                    .iter()
                    .map(|p| p.instantiate_parameters(&parameters, arguments))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| self.failure(ErrorKind::Type, None))?;
                let capabilities = capabilities
                    .iter()
                    .map(|p| p.instantiate_parameters(&parameters, arguments))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| self.failure(ErrorKind::Type, None))?;
                for p in requirements {
                    self.tick(None)?;
                    self.check_type_contract(&p, source)?;
                }
                for p in capabilities {
                    self.tick(None)?;
                    self.check_type_contract(&nir::program::TypePredicate::Trait(p), source)?;
                }
                for ty in arguments {
                    self.admit_type(ty, depth + 1)?;
                }
            }
            Term::NormalForm {
                head, arguments, ..
            } => {
                let metadata = *self
                    .normal_forms
                    .get(head)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                let mut fuel = usize::try_from(self.budget.steps)
                    .unwrap_or(usize::MAX)
                    .min(100_000);
                let before = fuel;
                let checked = metadata.check_closed_arguments_budgeted(
                    arguments,
                    |head| {
                        self.normal_forms
                            .get(&head)
                            .is_some_and(|head| head.role == nir::normal_forms::Role::Tuple)
                    },
                    |ty, variant, fuel| self.normal_value_fields(ty, variant, fuel),
                    &mut fuel,
                );
                self.budget.steps -= (before - fuel) as u64;
                checked.map_err(|_| {
                    self.failure(
                        if fuel == 0 {
                            ErrorKind::StepsExhausted
                        } else {
                            ErrorKind::Type
                        },
                        None,
                    )
                })?;
                let parameters: Vec<_> =
                    metadata.parameters.iter().map(|p| p.declaration).collect();
                for p in &metadata.requirements {
                    self.tick(None)?;
                    let predicate = p
                        .instantiate_rigids(&parameters, arguments)
                        .map_err(|_| self.failure(ErrorKind::Type, None))?;
                    self.check_type_contract(&predicate, source)?;
                }
                for ty in arguments {
                    self.admit_type(ty, depth + 1)?;
                }
            }
            Term::Tuple(types)
            | Term::TypeListValue(types)
            | Term::TypeListConcat(types)
            | Term::TupleValue(types) => {
                for ty in types {
                    self.admit_type(ty, depth + 1)?;
                }
            }
            Term::NominalValue { ty, fields, .. } => {
                self.admit_type(ty, depth + 1)?;
                for field in fields {
                    self.admit_type(field, depth + 1)?;
                }
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                for (_, ty) in fields {
                    self.admit_type(ty, depth + 1)?;
                }
            }
            Term::Pointer(ty) | Term::ComptimeFunction(ty) => self.admit_type(ty, depth + 1)?,
            Term::Function { parameters, result } => {
                for p in parameters {
                    self.admit_type(&p.ty, depth + 1)?;
                }
                self.admit_type(result, depth + 1)?;
            }
            Term::Closure {
                environment,
                signature,
            } => {
                for ty in environment {
                    self.admit_type(ty, depth + 1)?;
                }
                self.admit_type(signature, depth + 1)?;
            }
            Term::Computation { effects, result } => {
                // Row entries preserve declaration identity, order and repeated
                // occurrences. Formation does not install a handler or grant an
                // effect invocation capability.
                for operation in effects {
                    self.tick(None)?;
                    let declaration = *self
                        .effect_declarations
                        .get(&operation.declaration)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    let signature = declaration
                        .signature
                        .instantiate_types(&declaration.type_parameters, &operation.arguments)
                        .map_err(|_| self.failure(ErrorKind::Type, None))?;
                    let work = self
                        .type_description_work(&signature)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    self.integer_operation(None, |budget| budget.consume(work))?;
                    signature
                        .check_scope(&[])
                        .map_err(|_| self.failure(ErrorKind::Type, None))?;
                    for predicate in &declaration.requirements {
                        self.tick(None)?;
                        let predicate = predicate
                            .instantiate_types(&declaration.type_parameters, &operation.arguments)
                            .map_err(|_| self.failure(ErrorKind::Type, None))?;
                        self.check_type_contract(&predicate, source)?;
                    }
                    for capability in &declaration.capabilities {
                        self.tick(None)?;
                        let capability = capability
                            .instantiate_types(&declaration.type_parameters, &operation.arguments)
                            .map_err(|_| self.failure(ErrorKind::Type, None))?;
                        self.check_type_contract(
                            &nir::program::TypePredicate::Trait(capability),
                            source,
                        )?;
                    }
                    for argument in &operation.arguments {
                        self.admit_type(argument, depth + 1)?;
                    }
                }
                self.admit_type(result, depth + 1)?;
            }
            Term::Rigid {
                declaration,
                sort: nir::Sort::Type,
            } if self.abstract_type_parameters.contains(declaration) => {}
            Term::AssociatedType { .. }
            | Term::Rigid { .. }
            | Term::Runtime(_)
            | Term::Value { .. }
            | Term::Hole { .. } => return Err(self.failure(ErrorKind::Type, None)),
            _ => {}
        }
        Ok(())
    }
}
