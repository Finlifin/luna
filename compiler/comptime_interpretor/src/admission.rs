//! Bounded immutable host-data admission. Representation validation is not
//! authorization to duplicate resources or reuse an equality-sensitive query.
use super::*;

impl Interpreter<'_> {
    /// Registered head metadata supplies collection authority; payload shape
    /// alone cannot establish an element domain.
    pub(super) fn admitted_collection_domain(
        &self,
        domain: &Term,
        fuel: &mut usize,
    ) -> Result<Option<Term>, nir::NominalAdmissionError> {
        let Term::NormalForm { head, .. } = domain else {
            return Ok(None);
        };
        *fuel = fuel
            .checked_sub(1)
            .ok_or(nir::NominalAdmissionError::Budget)?;
        let Some(element) = self
            .normal_forms
            .get(head)
            .and_then(|head| head.list_element(domain))
        else {
            return Ok(None);
        };
        nir::type_computations::charge_term(element, fuel)?;
        Ok(Some(element.clone()))
    }

    /// Decode immutable host data in this VM's type world without executing
    /// closure factories or reconstructing callable values. Nested aggregates
    /// follow the same policy. Admission proves representation and domain, not
    /// resource duplication permission or portability to a different world.
    pub fn decode_immutable_input(&mut self, term: &Term, ty: &Term) -> Result<Value, Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        for input in [term, ty] {
            let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let charged = nir::type_computations::charge_term(input, &mut fuel);
            self.budget.steps -= (before - fuel) as u64;
            charged.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
            input
                .check_scope(&[])
                .map_err(|_| self.failure(ErrorKind::Type, None))?;
        }
        let value = self.normal_pattern_value_at(term, ty, 0, false)?;
        self.validate_immutable_input(&value, ty)?;
        Ok(value)
    }

    pub(super) fn admitted_nominal_layout(
        &self,
        ty: &Term,
        variant: Option<u32>,
        fuel: &mut usize,
    ) -> Result<Vec<Term>, nir::NominalAdmissionError> {
        if variant.is_none() {
            if let Term::NormalForm { head, .. } = ty {
                if let Some(elements) = self
                    .normal_forms
                    .get(head)
                    .and_then(|head| head.tuple_elements(ty))
                {
                    nir::type_computations::charge_term(ty, fuel)
                        .map_err(nir::NominalAdmissionError::from)?;
                    return Ok(elements.to_vec());
                }
            }
        }
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
        let (_, work) = value
            .scoped_representation_work(
                &self.abstract_type_parameters,
                &self.abstract_value_parameters,
            )
            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        self.integer_operation(None, |budget| budget.consume(work))?;
        let work = self
            .closed_type_description_work(ty)
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
    /// Host inputs can hide Type descriptions inside immutable aggregates.
    /// Check their formation before invocation; this does not read pointers,
    /// copy storage, or invoke user code.
    pub(super) fn admit_embedded_type_descriptions(
        &mut self,
        value: &Value,
        depth: usize,
    ) -> Result<(), Error> {
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, None));
        }
        self.tick(None)?;
        match value {
            Value::Type(_) | Value::List { .. } => self.admit_value_types(value, depth)?,
            Value::Optional { value, .. } => {
                if let Some(value) = value {
                    self.admit_embedded_type_descriptions(value, depth + 1)?;
                }
            }
            Value::ErrorResult { value, .. } | Value::Newtype { value, .. } => {
                self.admit_embedded_type_descriptions(value, depth + 1)?
            }
            Value::Tuple(fields) | Value::Record { fields, .. } | Value::Variant { fields, .. } => {
                for field in fields {
                    self.admit_embedded_type_descriptions(field, depth + 1)?;
                }
            }
            Value::AnonymousRecord(fields) => {
                for (_, field) in fields {
                    self.admit_embedded_type_descriptions(field, depth + 1)?;
                }
            }
            Value::Object(object) => {
                for entry in object.entries() {
                    self.admit_type(&entry.value().ty, depth + 1)?;
                    self.admit_embedded_type_descriptions(&entry.value().value, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn admit_value_types(&mut self, value: &Value, depth: usize) -> Result<(), Error> {
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, None));
        }
        self.tick(None)?;
        match value {
            Value::List { ty, values } => {
                self.admit_type(ty, depth + 1)?;
                // Uniform primitive leaves contain no embedded type descriptions.
                // The list's own formation was still checked in the current world.
                if values
                    .summary()
                    .is_some_and(|summary| summary.uniform_scalar.is_some())
                {
                    return Ok(());
                }
                for value in values {
                    self.admit_value_types(value, depth + 1)?;
                }
            }
            Value::Optional { ty, value } => {
                self.admit_type(ty, depth + 1)?;
                if let Some(value) = value {
                    self.admit_value_types(value, depth + 1)?;
                }
            }
            Value::ErrorResult { ty, value, .. } | Value::Newtype { ty, value } => {
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
            Value::Object(object) => {
                for entry in object.entries() {
                    self.admit_type(&entry.value().ty, depth + 1)?;
                    self.admit_value_types(&entry.value().value, depth + 1)?;
                }
            }
            _ => {} // immutable_copy_work already rejected storage and callables.
        }
        Ok(())
    }
    pub(super) fn admit_type(&mut self, ty: &Term, depth: usize) -> Result<(), Error> {
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
                if let Some(graph) = self.graph {
                    let nominal = graph
                        .nominals
                        .iter()
                        .find(|entry| &entry.ty == ty)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    // Concrete descriptors already substituted their declaration telescope.
                    let (requirements, capabilities) = match &nominal.definition {
                        iir::NominalDefinition::Struct(d) => (&d.requirements, &d.capabilities),
                        iir::NominalDefinition::Enum(d) => (&d.requirements, &d.capabilities),
                        iir::NominalDefinition::Newtype(d) => (&d.requirements, &d.capabilities),
                    };
                    for predicate in requirements {
                        self.tick(None)?;
                        self.check_type_contract(predicate, source)?;
                    }
                    for predicate in capabilities {
                        self.tick(None)?;
                        self.check_type_contract(
                            &nir::program::TypePredicate::Trait(predicate.clone()),
                            source,
                        )?;
                    }
                    for argument in arguments {
                        self.admit_type(argument, depth + 1)?;
                    }
                    return Ok(());
                }
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
                    definition.validate_completed_arguments_with_collections_in(
                        arguments,
                        &mut remaining,
                        |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                        |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)),
                        |domain, fuel| self.admitted_collection_domain(domain, fuel),
                        self.target,
                    )
                } else {
                    nir::validate_declaration_arguments_with_collections_in(
                        parameters,
                        domains,
                        arguments,
                        &mut remaining,
                        |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                        |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)),
                        |domain, fuel| self.admitted_collection_domain(domain, fuel),
                        self.target,
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
                let metadata = self
                    .normal_forms
                    .get(head)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                let mut fuel = usize::try_from(self.budget.steps)
                    .unwrap_or(usize::MAX)
                    .min(100_000);
                let before = fuel;
                let checked = metadata.check_arguments_in_scope_with_callables_budgeted_in(
                    arguments,
                    |head| {
                        self.normal_forms
                            .get(&head)
                            .is_some_and(|head| head.role == nir::normal_forms::Role::Tuple)
                    },
                    |head| self.normal_forms.get(&head)
                        .is_some_and(|head| head.role == nir::normal_forms::Role::TypeList),
                    |ty, variant, fuel| self.normal_value_fields(ty, variant, fuel),
                    |value, domain| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)),
                    |id, args, frames, fuel| self.normal_callable_signature(id, args, frames, fuel),
                    &[], &mut fuel, self.target);
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
                self.check_normal_requirements(metadata, arguments, source)?;
                for ty in arguments {
                    self.admit_type(ty, depth + 1)?;
                }
            }
            Term::FunctionValue {
                declaration,
                arguments,
            } => {
                self.admit_callable_identity(*declaration, arguments, source)?;
                for argument in arguments {
                    self.admit_type(argument, depth + 1)?;
                }
            }
            Term::Tuple(types) | Term::ClosureContextValue(types) | Term::TupleValue(types) => {
                for ty in types {
                    self.admit_type(ty, depth + 1)?;
                }
            }
            Term::OptionalValue { ty, value } => {
                self.admit_type(ty, depth + 1)?;
                if let Some(value) = value {
                    self.admit_type(value, depth + 1)?;
                }
            }
            Term::ErrorValue { ty, label, value } => {
                self.admit_type(ty, depth + 1)?;
                if let Some(label) = label {
                    self.admit_type(label, depth + 1)?;
                }
                self.admit_type(value, depth + 1)?;
            }
            Term::ListConcat { ty, parts: fields } | Term::NominalValue { ty, fields, .. } => {
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
            Term::Optional(ty)
            | Term::Pointer(ty)
            | Term::ComptimeFunction(ty)
            | Term::NormalFormFunction(ty, _)
            | Term::UnsafeFunction(ty) => self.admit_type(ty, depth + 1)?,
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
            Term::ContextualClosure { context, signature } => {
                self.admit_type(context, depth + 1)?;
                self.admit_type(signature, depth + 1)?;
            }
            Term::ErrorResult { errors, result } => {
                for label in errors {
                    self.admit_type(label, depth + 1)?;
                }
                self.admit_type(result, depth + 1)?;
            }
            Term::Computation { effects, result } => {
                // Row entries preserve declaration identity, order and repeated
                // occurrences. Formation does not install a handler or grant an
                // effect invocation capability.
                for operation in effects {
                    self.tick(None)?;
                    let declaration = self
                        .effect_declarations
                        .get(&operation.declaration)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    let mut fuel = usize::try_from(self.budget.steps)
                        .unwrap_or(usize::MAX)
                        .min(100_000);
                    let before = fuel;
                    let checked = nir::validate_declaration_arguments_with_collections_in(
                        &declaration.type_parameters,
                        &declaration.parameter_types,
                        &operation.arguments,
                        &mut fuel,
                        |ty, variant, fuel| {
                            self.normal_value_fields(ty, variant, fuel).ok_or_else(|| {
                                nir::NominalAdmissionError::from(
                                    "effect value layout is unavailable",
                                )
                            })
                        },
                        |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)),
                        |domain, fuel| self.admitted_collection_domain(domain, fuel),
                        self.target,
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
                    let signature = declaration
                        .signature
                        .instantiate_parameters(
                            &declaration
                                .parameter_binders()
                                .map_err(|_| self.failure(ErrorKind::Type, None))?,
                            &operation.arguments,
                        )
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
                            .instantiate_parameters(
                                &declaration
                                    .parameter_binders()
                                    .map_err(|_| self.failure(ErrorKind::Type, None))?,
                                &operation.arguments,
                            )
                            .map_err(|_| self.failure(ErrorKind::Type, None))?;
                        self.check_type_contract(&predicate, source)?;
                    }
                    for capability in &declaration.capabilities {
                        self.tick(None)?;
                        let capability = capability
                            .instantiate_parameters(
                                &declaration
                                    .parameter_binders()
                                    .map_err(|_| self.failure(ErrorKind::Type, None))?,
                                &operation.arguments,
                            )
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
            Term::Rigid {
                declaration,
                sort: nir::Sort::ComptimeValue,
            } if self.abstract_value_parameters.contains_key(declaration) => {}
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
