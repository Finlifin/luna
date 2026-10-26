//! Atomic installation of completed struct descriptions after checked publication.
use super::*;
mod admission_session;
pub use admission_session::{
    DefinitionAdmissionSession, PreparedEnumAdmission, PreparedStructAdmission,
    StructAdmissionSession,
};

/// Distinguish VM admission from the publisher's final commit failure.
#[derive(Debug)]
pub enum StructPublicationError<E> {
    Admission(Error),
    Commit(E),
}
impl<E: fmt::Display> fmt::Display for StructPublicationError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admission(error) => write!(
                f,
                "(struct-publication (admission {}))",
                nir::QuotedDisplay(&error.to_string())
            ),
            Self::Commit(error) => write!(
                f,
                "(struct-publication (commit {}))",
                nir::QuotedDisplay(&error.to_string())
            ),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for StructPublicationError<E> {}

// Owned batches let a running invocation retain newly checked descriptors.
// Cloning a handle shares storage; it never copies field trees or old catalogs.
#[derive(Clone)]
pub(super) enum StructBatch<'a> {
    Borrowed(&'a [nir::program::Struct]),
    Shared(Arc<[nir::program::Struct]>),
}
impl std::ops::Deref for StructBatch<'_> {
    type Target = [nir::program::Struct];
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(definitions) => definitions,
            Self::Shared(definitions) => definitions,
        }
    }
}
#[derive(Clone)]
pub(super) enum StructDefinition<'a> {
    Borrowed(&'a nir::program::Struct),
    Shared {
        batch: Arc<[nir::program::Struct]>,
        index: usize,
    },
}
impl std::ops::Deref for StructDefinition<'_> {
    type Target = nir::program::Struct;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(definition) => definition,
            Self::Shared { batch, index } => &batch[*index],
        }
    }
}
impl<'a> StructBatch<'a> {
    fn definition(&self, index: usize) -> StructDefinition<'a> {
        match self {
            Self::Borrowed(definitions) => StructDefinition::Borrowed(&definitions[index]),
            Self::Shared(batch) => StructDefinition::Shared {
                batch: Arc::clone(batch),
                index,
            },
        }
    }
}

struct Installation<'vm, 'world> {
    vm: &'vm mut Interpreter<'world>,
    definitions: StructBatch<'world>,
    inserted: usize,
    committed: bool,
}
impl Drop for Installation<'_, '_> {
    fn drop(&mut self) {
        if !self.committed {
            for definition in &self.definitions[..self.inserted] {
                self.vm.structs.remove(&definition.declaration);
            }
        }
    }
}

impl Installation<'_, '_> {
    fn commit_with<T, E>(
        mut self,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, StructPublicationError<E>> {
        let result = commit().map_err(StructPublicationError::Commit)?;
        self.committed = true;
        Ok(result)
    }
}

impl<'world> Interpreter<'world> {
    /// Install a checked batch of concrete builder results into this comptime VM.
    /// The publisher reserves global identities and performs ordinary declaration
    /// checking first. This boundary additionally checks collisions, closed field
    /// descriptions and formation in the combined world before retaining it.
    ///
    /// Existing declarations, values, invocation state and caches are untouched.
    /// Failure/unwind removes the entire batch but does not refund work or roll
    /// back arbitrary host solver effects. The caller owns the immutable structs
    /// for the VM's lifetime. Generic declarations, functions and impls require a
    /// separate publication path; this is not a source builder dispatch hook.
    pub fn install_generated_structs(
        &mut self,
        definitions: &'world [nir::program::Struct],
    ) -> Result<(), Error> {
        match self
            .install_generated_structs_with(definitions, || Ok::<_, std::convert::Infallible>(()))
        {
            Ok(()) => Ok(()),
            Err(StructPublicationError::Admission(error)) => Err(error),
            Err(StructPublicationError::Commit(impossible)) => match impossible {},
        }
    }

    /// Admit the batch, then run the publisher's commit exactly once. An
    /// admission failure never calls commit; a commit error or unwind removes
    /// the VM entries. The callback must itself be transactional on failure;
    /// arbitrary external side effects cannot be undone by this interpreter.
    pub fn install_generated_structs_with<T, E>(
        &mut self,
        definitions: &'world [nir::program::Struct],
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, StructPublicationError<E>> {
        self.prepare_generated_structs(StructBatch::Borrowed(definitions))
            .map_err(StructPublicationError::Admission)?
            .commit_with(commit)
    }

    /// Retain shared checked descriptors created after this VM was started.
    /// Uses the same admission and transactional callback as borrowed batches.
    /// No field trees are cloned; rollback releases this VM's ownership.
    pub fn install_owned_generated_structs_with<T, E>(
        &mut self,
        definitions: Arc<[nir::program::Struct]>,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, StructPublicationError<E>> {
        self.prepare_generated_structs(StructBatch::Shared(definitions))
            .map_err(StructPublicationError::Admission)?
            .commit_with(commit)
    }

    fn prepare_generated_structs<'vm>(
        &'vm mut self,
        definitions: StructBatch<'world>,
    ) -> Result<Installation<'vm, 'world>, Error> {
        if !self.comptime_only || self.graph.is_some() || self.cleanup_schedule.is_some() {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        if !self.abstract_type_parameters.is_empty() || !self.abstract_value_parameters.is_empty() {
            return Err(self.failure(ErrorKind::Type, None));
        }
        self.integer_operation(None, |budget| {
            budget.consume(definitions.len().saturating_add(1))
        })?;
        let mut identities = BTreeSet::new();
        for definition in definitions.iter() {
            if !definition.type_parameters.is_empty()
                || !definition.parameter_types.is_empty()
                || !identities.insert(definition.declaration)
                || self.known_definition(definition.declaration)?
            {
                return Err(self.failure(ErrorKind::InvalidCallPlan, None));
            }
            self.integer_operation(None, |budget| {
                budget.consume(
                    definition
                        .name
                        .len()
                        .saturating_add(definition.fields.len()),
                )
            })?;
            for requirement in &definition.requirements {
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut remaining = before;
                let checked = nir::copy_budget::charge_predicate_substitution(
                    requirement,
                    &[],
                    &[],
                    &mut remaining,
                );
                self.budget.steps -= (before - remaining) as u64;
                checked.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
            }
            for capability in &definition.capabilities {
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut remaining = before;
                let checked = (|| {
                    nir::type_computations::charge_term(&capability.subject, &mut remaining)?;
                    for argument in &capability.interface_arguments {
                        nir::type_computations::charge_term(argument, &mut remaining)?;
                    }
                    Ok::<_, nir::type_computations::Error>(())
                })();
                self.budget.steps -= (before - remaining) as u64;
                checked.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
            }
            let mut names = BTreeSet::new();
            for (name, ty) in &definition.fields {
                self.integer_operation(None, |budget| {
                    budget.consume(name.len().saturating_add(1))
                })?;
                if !names.insert(name) || ty.sort() != nir::Sort::Type {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut remaining = before;
                let checked = nir::type_computations::charge_term(ty, &mut remaining);
                self.budget.steps -= (before - remaining) as u64;
                checked.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
                ty.check_scope(&[])
                    .map_err(|_| self.failure(ErrorKind::Type, None))?;
            }
        }
        // Every identity was checked before mutation. Reserve insertion and
        // rollback work up front; unwinding must remove only this batch even if
        // admission consumes the rest of the invocation's execution allowance.
        self.integer_operation(None, |budget| {
            budget.consume(definitions.len().saturating_mul(2))
        })?;
        let mut installation = Installation {
            vm: self,
            definitions: definitions.clone(),
            inserted: 0,
            committed: false,
        };
        for (index, definition) in definitions.iter().enumerate() {
            installation.inserted += 1;
            installation
                .vm
                .structs
                .insert(definition.declaration, definitions.definition(index));
        }
        for definition in definitions.iter() {
            let ty = Term::Nominal {
                declaration: definition.declaration,
                arguments: vec![],
            };
            installation.vm.admit_type(&ty, 0)?;
            for (_, field) in &definition.fields {
                installation.vm.admit_type(field, 0)?;
            }
        }
        Ok(installation)
    }

    pub(super) fn known_definition(&mut self, id: DeclarationId) -> Result<bool, Error> {
        self.tick(None)?;
        if self.structs.contains_key(&id)
            || self.enums.contains_key(&id)
            || self.newtypes.contains_key(&id)
            || self.normal_forms.contains_key(&id)
            || self.functions.contains_key(&id)
            || self.runtime_function_identities.contains_key(&id)
            || self.alias_contracts.contains_key(&id)
            || self.effect_declarations.contains_key(&id)
            || self.declaration_parameter_sorts.contains_key(&id)
        {
            return Ok(true);
        }
        // Trait/impl and body-free telescope identities are not all represented
        // by executable function tables. They still occupy declaration space.
        let world = self
            .declaration_world
            .ok_or_else(|| self.failure(ErrorKind::PhaseMismatch, None))?;
        for interface in &world.traits {
            self.integer_operation(None, |budget| {
                budget.consume(
                    interface
                        .methods
                        .len()
                        .saturating_add(interface.associated.len())
                        .saturating_add(1),
                )
            })?;
            if interface.declaration == id
                || interface.self_parameter == id
                || interface
                    .methods
                    .iter()
                    .any(|method| method.declaration == id)
                || interface
                    .associated
                    .iter()
                    .any(|member| member.declaration == id)
            {
                return Ok(true);
            }
        }
        for implementation in &world.implementations {
            self.tick(None)?;
            if implementation.declaration == id {
                return Ok(true);
            }
        }
        for parameters in &world.function_parameters {
            self.tick(None)?;
            if parameters.declaration == id {
                return Ok(true);
            }
        }
        for template in &world.quote_templates {
            self.tick(None)?;
            if template.declaration == id {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests;
