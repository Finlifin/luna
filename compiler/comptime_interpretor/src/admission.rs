//! Bounded immutable host-data admission. Representation validation is not
//! authorization to duplicate resources or reuse an equality-sensitive query.
use super::*;

mod formation;

#[cfg(test)]
mod tests;

impl Interpreter<'_> {
    /// Execute a passive representation query against the current world and
    /// return its consumed allowance on every outcome. This grants no new
    /// declaration evidence or permission to execute user code.
    pub(super) fn query_admission<T>(
        &mut self,
        source: Option<SourceRange>,
        query: impl FnOnce(&Self, &mut usize) -> Result<T, nir::NominalAdmissionError>,
    ) -> Result<T, Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = query(self, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|failure| {
            self.failure(
                match failure {
                    nir::NominalAdmissionError::Budget => ErrorKind::StepsExhausted,
                    nir::NominalAdmissionError::Unresolved(_) => ErrorKind::TypeContractUnresolved,
                    nir::NominalAdmissionError::Invalid(_) => ErrorKind::Type,
                },
                source,
            )
        })
    }

    /// Ordinary and NF inputs must use the same receiving-world evidence.
    /// Keep typed query failures intact; budget exhaustion is not missing data.
    pub(super) fn with_admission_queries<R>(
        &self,
        check: impl FnOnce(&mut nir::value_admission::DomainQueries<'_, '_>) -> R,
    ) -> R {
        self.with_scoped_queries(&[], check)
    }

    fn with_scoped_queries<R>(
        &self,
        frames: &[&[nir::Parameter]],
        check: impl FnOnce(&mut nir::value_admission::DomainQueries<'_, '_>) -> R,
    ) -> R {
        check(&mut nir::value_admission::DomainQueries {
            layout: &mut |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
            declared: &mut |value, domain, fuel| {
                type_descriptions::declared_domain_matches(
                    frames,
                    &self.abstract_value_parameters,
                    self.runtime_domains(),
                    value,
                    domain,
                    fuel,
                )
            },
            collection: &mut |domain, fuel| self.admitted_collection_domain(domain, fuel),
            callable: &mut |value, frames, fuel| {
                self.admitted_callable_signature(value, frames, fuel)
            },
            target: self.target,
        })
    }

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
            if let Some(elements) =
                nir::normal_forms::registered_tuple_fields(ty, |id| self.normal_forms.get(&id))
            {
                nir::type_computations::charge_term(ty, fuel)?;
                return Ok(elements.to_vec());
            }
        }
        self.query_normal_value_fields(ty, variant, fuel)
    }

    /// Validate immutable data against a closed domain, including nominal
    /// formation predicates. In a comptime normalization scope, Type descriptions
    /// may mention only that scope's authorized declaration inputs. This does not
    /// authorize closed export. No source Eq/Hash or user function executes here.
    pub fn validate_immutable_input(&mut self, value: &Value, ty: &Term) -> Result<(), Error> {
        self.admitted_immutable_size(value, ty).map(|_| ())
    }
    /// Return the representation measurements from this admission. A caller
    /// retaining the immutable value can use them to budget a copy or comparison.
    pub fn admitted_immutable_size(
        &mut self,
        value: &Value,
        ty: &Term,
    ) -> Result<ImmutableSize, Error> {
        self.tick(None)?;
        let size = self.inspect_value_representation(value, None)?;
        self.admit_inspected_input(value, ty)?;
        Ok(size)
    }

    pub(super) fn inspect_value_representation(
        &mut self,
        value: &Value,
        source: Option<SourceRange>,
    ) -> Result<ImmutableSize, Error> {
        self.query_descriptions(source, |vm, fuel| {
            value.inspect_representation(
                &vm.abstract_type_parameters,
                &vm.abstract_value_parameters,
                vm.runtime_domains(),
                fuel,
            )
        })
    }

    pub(super) fn admit_inspected_input(&mut self, value: &Value, ty: &Term) -> Result<(), Error> {
        self.check_closed_type_description(ty, None)?;
        self.admit_type(ty, 0)?;
        self.check_value_type(value, ty, None)?;
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
            Value::Description(_) | Value::Type(_) | Value::List { .. } => {
                self.admit_value_types(value, depth)?
            }
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
            Value::RuntimeTraitMethod(method) => {
                self.admit_runtime_method_identity(method)?;
                self.admit_type(&method.subject, depth + 1)?;
                for argument in method
                    .interface
                    .arguments
                    .iter()
                    .chain(&method.method_arguments)
                {
                    self.admit_type(argument, depth + 1)?;
                }
            }
            Value::Function(declaration) => {
                self.admit_callable_identity(*declaration, &[], SourceRange { start: 0, end: 0 })?;
            }
            Value::FunctionInstance {
                declaration,
                arguments,
            } => {
                // Concrete handles use the signature checked by has_type in
                // the receiving graph. Source identities additionally need
                // the current declaration's input and formation contracts.
                if self.graph.is_none() {
                    self.admit_callable_identity(
                        *declaration,
                        arguments,
                        SourceRange { start: 0, end: 0 },
                    )?;
                }
                for argument in arguments {
                    self.admit_type(argument, depth + 1)?;
                }
            }
            Value::IirCallable(handle) => {
                self.iir_target(handle)?;
                self.admit_type(&handle.ty, depth + 1)?;
            }
            Value::Closure(_) => return Err(self.failure(ErrorKind::Type, None)),
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
            Value::Description(data) if !self.admits_description(data) => {
                return Err(self.failure(ErrorKind::Type, None));
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
            _ => {} // Representation admission already rejected storage and captures.
        }
        Ok(())
    }
}
