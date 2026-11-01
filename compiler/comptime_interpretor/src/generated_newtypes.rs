//! Atomic installation of completed newtype descriptions after checked publication.
use super::*;

/// Distinguish VM admission from the publisher's final commit failure.
#[derive(Debug)]
pub enum NewtypePublicationError<E> {
    Admission(Error),
    Commit(E),
}
impl<E: fmt::Display> fmt::Display for NewtypePublicationError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admission(error) => write!(
                f,
                "(newtype-publication (admission {}))",
                nir::QuotedDisplay(&error.to_string())
            ),
            Self::Commit(error) => write!(
                f,
                "(newtype-publication (commit {}))",
                nir::QuotedDisplay(&error.to_string())
            ),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for NewtypePublicationError<E> {}

// Owned batches let a running invocation retain newly checked descriptors.
// Cloning a handle shares storage; it never copies representation trees or old catalogs.
#[derive(Clone)]
pub(super) enum NewtypeBatch<'a> {
    Borrowed(&'a [nir::newtypes::Newtype]),
    Shared(Arc<[nir::newtypes::Newtype]>),
}
impl std::ops::Deref for NewtypeBatch<'_> {
    type Target = [nir::newtypes::Newtype];
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(definitions) => definitions,
            Self::Shared(definitions) => definitions,
        }
    }
}
#[derive(Clone)]
pub(super) enum NewtypeDefinition<'a> {
    Borrowed(&'a nir::newtypes::Newtype),
    Shared {
        batch: Arc<[nir::newtypes::Newtype]>,
        index: usize,
    },
}
impl std::ops::Deref for NewtypeDefinition<'_> {
    type Target = nir::newtypes::Newtype;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(definition) => definition,
            Self::Shared { batch, index } => &batch[*index],
        }
    }
}
impl<'a> NewtypeBatch<'a> {
    fn definition(&self, index: usize) -> NewtypeDefinition<'a> {
        match self {
            Self::Borrowed(definitions) => NewtypeDefinition::Borrowed(&definitions[index]),
            Self::Shared(batch) => NewtypeDefinition::Shared {
                batch: Arc::clone(batch),
                index,
            },
        }
    }
}

pub(super) struct Installation<'vm, 'world> {
    vm: &'vm mut Interpreter<'world>,
    definitions: NewtypeBatch<'world>,
    inserted: usize,
    committed: bool,
}
impl Drop for Installation<'_, '_> {
    fn drop(&mut self) {
        if !self.committed {
            for definition in &self.definitions[..self.inserted] {
                self.vm.newtypes.remove(&definition.declaration);
            }
        }
    }
}

impl Installation<'_, '_> {
    fn commit_with<T, E>(
        mut self,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, NewtypePublicationError<E>> {
        let result = commit().map_err(NewtypePublicationError::Commit)?;
        self.committed = true;
        Ok(result)
    }
}

impl<'world> Interpreter<'world> {
    /// Install a checked batch of concrete builder results into this comptime VM.
    /// The publisher reserves global identities and performs ordinary declaration
    /// checking first. This boundary additionally checks collisions, closed
    /// representation descriptions and formation in the combined world.
    ///
    /// Existing declarations, values, invocation state and caches are untouched.
    /// Failure/unwind removes the entire batch but does not refund work or roll
    /// back arbitrary host solver effects. The caller owns the immutable newtypes
    /// for the VM's lifetime. This entry accepts ordinary concrete newtypes;
    /// generic or captured newtypes need additional publication contracts.
    /// This is not a source builder dispatch hook or a representation-cycle check.
    pub fn install_generated_newtypes(
        &mut self,
        definitions: &'world [nir::newtypes::Newtype],
    ) -> Result<(), Error> {
        match self
            .install_generated_newtypes_with(definitions, || Ok::<_, std::convert::Infallible>(()))
        {
            Ok(()) => Ok(()),
            Err(NewtypePublicationError::Admission(error)) => Err(error),
            Err(NewtypePublicationError::Commit(impossible)) => match impossible {},
        }
    }

    /// Admit the batch, then run the publisher's commit exactly once. An
    /// admission failure never calls commit; a commit error or unwind removes
    /// the VM entries. The callback must itself be transactional on failure;
    /// arbitrary external side effects cannot be undone by this interpreter.
    pub fn install_generated_newtypes_with<T, E>(
        &mut self,
        definitions: &'world [nir::newtypes::Newtype],
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, NewtypePublicationError<E>> {
        self.prepare_generated_newtypes(NewtypeBatch::Borrowed(definitions))
            .map_err(NewtypePublicationError::Admission)?
            .commit_with(commit)
    }

    /// Retain shared checked descriptors created after this VM was started.
    /// Uses the same admission and transactional callback as borrowed batches.
    /// No representation trees are cloned; rollback releases this VM's ownership.
    pub fn install_owned_generated_newtypes_with<T, E>(
        &mut self,
        definitions: Arc<[nir::newtypes::Newtype]>,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, NewtypePublicationError<E>> {
        self.prepare_generated_newtypes(NewtypeBatch::Shared(definitions))
            .map_err(NewtypePublicationError::Admission)?
            .commit_with(commit)
    }

    // Only the private admission session may commit this exact immutable batch
    // after checking its world identity/revision and prepaying insertion work.
    pub(super) fn commit_admitted_newtypes_with<T, E>(
        &mut self,
        definitions: Arc<[nir::newtypes::Newtype]>,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, NewtypePublicationError<E>> {
        let mut installation = Installation {
            vm: self,
            definitions: NewtypeBatch::Shared(definitions),
            inserted: 0,
            committed: false,
        };
        for (index, definition) in installation.definitions.iter().enumerate() {
            installation.inserted += 1;
            installation.vm.newtypes.insert(
                definition.declaration,
                installation.definitions.definition(index),
            );
        }
        installation.commit_with(commit)
    }

    pub(super) fn prepare_generated_newtypes<'vm>(
        &'vm mut self,
        definitions: NewtypeBatch<'world>,
    ) -> Result<Installation<'vm, 'world>, Error> {
        if !self.comptime_only || self.cleanup_schedule.is_some() {
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
                || definition.capture_count != 0
                || !identities.insert(definition.declaration)
                || self.known_definition(definition.declaration)?
            {
                return Err(self.failure(ErrorKind::InvalidCallPlan, None));
            }
            self.integer_operation(None, |budget| {
                budget.consume(definition.name.len().saturating_add(1))
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
            let ty = &definition.representation;
            let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let charged = nir::type_computations::charge_term(ty, &mut fuel);
            self.budget.steps -= (before - fuel) as u64;
            charged.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
            if ty.sort() != nir::Sort::Type {
                return Err(self.failure(ErrorKind::Type, None));
            }
            ty.check_scope(&[])
                .map_err(|_| self.failure(ErrorKind::Type, None))?;
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
                .newtypes
                .insert(definition.declaration, definitions.definition(index));
        }
        for definition in definitions.iter() {
            let ty = Term::Nominal {
                declaration: definition.declaration,
                arguments: vec![],
            };
            installation.vm.admit_type(&ty, 0)?;
            installation.vm.admit_type(&definition.representation, 0)?;
        }
        Ok(installation)
    }
}

#[cfg(test)]
mod tests;
