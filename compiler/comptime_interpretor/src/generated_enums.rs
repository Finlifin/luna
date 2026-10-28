//! Atomic installation of completed enum descriptions after checked publication.
use super::*;

/// Distinguish VM admission from the publisher's final commit failure.
#[derive(Debug)]
pub enum EnumPublicationError<E> {
    Admission(Error),
    Commit(E),
}
impl<E: fmt::Display> fmt::Display for EnumPublicationError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admission(error) => write!(
                f,
                "(enum-publication (admission {}))",
                nir::QuotedDisplay(&error.to_string())
            ),
            Self::Commit(error) => write!(
                f,
                "(enum-publication (commit {}))",
                nir::QuotedDisplay(&error.to_string())
            ),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for EnumPublicationError<E> {}

// Owned batches let a running invocation retain newly checked descriptors.
// Cloning a handle shares storage; it never copies field trees or old catalogs.
#[derive(Clone)]
pub(super) enum EnumBatch<'a> {
    Borrowed(&'a [nir::program::Enum]),
    Shared(Arc<[nir::program::Enum]>),
}
impl std::ops::Deref for EnumBatch<'_> {
    type Target = [nir::program::Enum];
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(definitions) => definitions,
            Self::Shared(definitions) => definitions,
        }
    }
}
#[derive(Clone)]
pub(super) enum EnumDefinition<'a> {
    Borrowed(&'a nir::program::Enum),
    Shared {
        batch: Arc<[nir::program::Enum]>,
        index: usize,
    },
}
impl std::ops::Deref for EnumDefinition<'_> {
    type Target = nir::program::Enum;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(definition) => definition,
            Self::Shared { batch, index } => &batch[*index],
        }
    }
}
impl<'a> EnumBatch<'a> {
    fn definition(&self, index: usize) -> EnumDefinition<'a> {
        match self {
            Self::Borrowed(definitions) => EnumDefinition::Borrowed(&definitions[index]),
            Self::Shared(batch) => EnumDefinition::Shared {
                batch: Arc::clone(batch),
                index,
            },
        }
    }
}

pub(super) struct Installation<'vm, 'world> {
    vm: &'vm mut Interpreter<'world>,
    definitions: EnumBatch<'world>,
    inserted: usize,
    committed: bool,
}
impl Drop for Installation<'_, '_> {
    fn drop(&mut self) {
        if !self.committed {
            for definition in &self.definitions[..self.inserted] {
                self.vm.enums.remove(&definition.declaration);
            }
        }
    }
}

impl Installation<'_, '_> {
    fn commit_with<T, E>(
        mut self,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, EnumPublicationError<E>> {
        let result = commit().map_err(EnumPublicationError::Commit)?;
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
    /// back arbitrary host solver effects. The caller owns the immutable enums
    /// for the VM's lifetime. This entry accepts ordinary concrete enums;
    /// encoded/intrinsic or generic enums need additional publication contracts.
    /// This is not a source builder dispatch hook or a representation-cycle check.
    pub fn install_generated_enums(
        &mut self,
        definitions: &'world [nir::program::Enum],
    ) -> Result<(), Error> {
        match self
            .install_generated_enums_with(definitions, || Ok::<_, std::convert::Infallible>(()))
        {
            Ok(()) => Ok(()),
            Err(EnumPublicationError::Admission(error)) => Err(error),
            Err(EnumPublicationError::Commit(impossible)) => match impossible {},
        }
    }

    /// Admit the batch, then run the publisher's commit exactly once. An
    /// admission failure never calls commit; a commit error or unwind removes
    /// the VM entries. The callback must itself be transactional on failure;
    /// arbitrary external side effects cannot be undone by this interpreter.
    pub fn install_generated_enums_with<T, E>(
        &mut self,
        definitions: &'world [nir::program::Enum],
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, EnumPublicationError<E>> {
        self.prepare_generated_enums(EnumBatch::Borrowed(definitions))
            .map_err(EnumPublicationError::Admission)?
            .commit_with(commit)
    }

    /// Retain shared checked descriptors created after this VM was started.
    /// Uses the same admission and transactional callback as borrowed batches.
    /// No field trees are cloned; rollback releases this VM's ownership.
    pub fn install_owned_generated_enums_with<T, E>(
        &mut self,
        definitions: Arc<[nir::program::Enum]>,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, EnumPublicationError<E>> {
        self.prepare_generated_enums(EnumBatch::Shared(definitions))
            .map_err(EnumPublicationError::Admission)?
            .commit_with(commit)
    }

    // Only the private admission session may commit this exact immutable batch
    // after checking its world identity/revision and prepaying insertion work.
    pub(super) fn commit_admitted_enums_with<T, E>(
        &mut self,
        definitions: Arc<[nir::program::Enum]>,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, EnumPublicationError<E>> {
        let mut installation = Installation {
            vm: self,
            definitions: EnumBatch::Shared(definitions),
            inserted: 0,
            committed: false,
        };
        for (index, definition) in installation.definitions.iter().enumerate() {
            installation.inserted += 1;
            installation.vm.enums.insert(
                definition.declaration,
                installation.definitions.definition(index),
            );
        }
        installation.commit_with(commit)
    }

    pub(super) fn prepare_generated_enums<'vm>(
        &'vm mut self,
        definitions: EnumBatch<'world>,
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
            if definition.is_intrinsic
                || !definition.type_parameters.is_empty()
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
                        .saturating_add(definition.variants.len())
                        .saturating_add(definition.groups.len()),
                )
            })?;
            let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let charged =
                crate::enum_build::charge_fixed_encoding(definition.encoding.as_ref(), &mut fuel)
                    .and_then(|()| {
                        crate::enum_build::charge_symbol_encoding(
                            definition.symbol_encoding.as_ref(),
                            &mut fuel,
                        )
                    });
            self.budget.steps -= (before - fuel) as u64;
            charged.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
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
            for group in &definition.groups {
                self.integer_operation(None, |budget| {
                    budget.consume(group.len().saturating_add(1))
                })?;
            }
            for variant in &definition.variants {
                self.integer_operation(None, |budget| {
                    budget.consume(
                        variant
                            .name
                            .len()
                            .saturating_add(variant.fields.len())
                            .saturating_add(1),
                    )
                })?;
                if variant.name.is_empty() || variant.name.split('.').any(str::is_empty) {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                match &variant.shape {
                    nir::program::VariantShape::Unit if !variant.fields.is_empty() => {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                    nir::program::VariantShape::Named(names) => {
                        self.integer_operation(None, |budget| budget.consume(names.len()))?;
                        if names.len() != variant.fields.len() {
                            return Err(self.failure(ErrorKind::Type, None));
                        }
                        let mut seen = BTreeSet::new();
                        for name in names {
                            self.integer_operation(None, |budget| {
                                budget.consume(name.len().saturating_add(1))
                            })?;
                            if name.is_empty() || !seen.insert(name) {
                                return Err(self.failure(ErrorKind::Type, None));
                            }
                        }
                    }
                    _ => {}
                }
                for ty in &variant.fields {
                    let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                    let mut remaining = before;
                    let checked = nir::type_computations::charge_term(ty, &mut remaining);
                    self.budget.steps -= (before - remaining) as u64;
                    checked.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
                    if ty.sort() != nir::Sort::Type {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                    ty.check_scope(&[])
                        .map_err(|_| self.failure(ErrorKind::Type, None))?;
                }
            }
            definition
                .validate_encoding_in(self.target)
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
                .enums
                .insert(definition.declaration, definitions.definition(index));
        }
        for definition in definitions.iter() {
            let ty = Term::Nominal {
                declaration: definition.declaration,
                arguments: vec![],
            };
            installation.vm.admit_type(&ty, 0)?;
            for variant in &definition.variants {
                for field in &variant.fields {
                    installation.vm.admit_type(field, 0)?;
                }
            }
        }
        Ok(installation)
    }
}

#[cfg(test)]
mod tests;
