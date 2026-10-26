//! Reusable, non-executing admission of concrete nominal descriptions.
use super::*;

/// Owns a checked baseline and committed additions, but exposes no execution or
/// arbitrary VM mutation. A receipt is valid only for this session's revision.
pub struct DefinitionAdmissionSession<'world> {
    vm: Interpreter<'world>,
    identity: Arc<()>,
    revision: u64,
}

/// Compatibility name for existing concrete struct publishers.
pub type StructAdmissionSession<'world> = DefinitionAdmissionSession<'world>;

/// An enum admission receipt shares the session revision with struct receipts.
pub struct PreparedEnumAdmission {
    definitions: Arc<[nir::program::Enum]>,
    identity: Arc<()>,
    revision: u64,
}

/// Completed admission with prepaid installation work. Dropping it publishes
/// nothing. Private fields prevent replacing the descriptions after checking.
pub struct PreparedStructAdmission {
    definitions: Arc<[nir::program::Struct]>,
    identity: Arc<()>,
    revision: u64,
}

struct Allowance<'vm, 'world, 'budget> {
    vm: &'vm mut Interpreter<'world>,
    budget: &'budget mut Budget,
    reserved: u64,
}
impl Drop for Allowance<'_, '_, '_> {
    fn drop(&mut self) {
        self.budget.steps = self.reserved.min(self.vm.remaining_steps());
    }
}

impl<'world> DefinitionAdmissionSession<'world> {
    pub fn new(package: &'world nir::program::Package, budget: &mut Budget) -> Result<Self, Error> {
        Ok(Self {
            vm: Interpreter::for_comptime_in_budget(package, &[], budget)?,
            identity: Arc::new(()),
            revision: 0,
        })
    }

    /// Check in the combined world, then roll back the tentative entries. Only
    /// completed descriptors are retained; no execution frames or live inference
    /// state can enter this session. Every attempt borrows its caller's budget,
    /// including after exhaustion, and returns unused fuel even on unwinding.
    pub fn prepare(
        &mut self,
        definitions: Arc<[nir::program::Struct]>,
        budget: &mut Budget,
    ) -> Result<PreparedStructAdmission, Error> {
        let reserved = std::mem::take(&mut budget.steps);
        self.vm.budget = Budget {
            steps: reserved,
            depth: budget.depth,
        };
        let allowance = Allowance {
            vm: &mut self.vm,
            budget,
            reserved,
        };
        // The commit is allowed after the caller's budget has changed. Reserve
        // its journal and insertion work now, before returning the receipt.
        allowance
            .vm
            .charge_preparation(definitions.len().saturating_mul(2))?;
        drop(
            allowance
                .vm
                .prepare_generated_structs(StructBatch::Shared(Arc::clone(&definitions)))?,
        );
        Ok(PreparedStructAdmission {
            definitions,
            identity: Arc::clone(&self.identity),
            revision: self.revision,
        })
    }

    /// Prepare an enum batch without exposing tentative entries to later calls.
    pub fn prepare_enums(
        &mut self,
        definitions: Arc<[nir::program::Enum]>,
        budget: &mut Budget,
    ) -> Result<PreparedEnumAdmission, Error> {
        let reserved = std::mem::take(&mut budget.steps);
        self.vm.budget = Budget {
            steps: reserved,
            depth: budget.depth,
        };
        let allowance = Allowance {
            vm: &mut self.vm,
            budget,
            reserved,
        };
        allowance
            .vm
            .charge_preparation(definitions.len().saturating_mul(2))?;
        drop(
            allowance
                .vm
                .prepare_generated_enums(crate::generated_enums::EnumBatch::Shared(Arc::clone(
                    &definitions,
                )))?,
        );
        Ok(PreparedEnumAdmission {
            definitions,
            identity: Arc::clone(&self.identity),
            revision: self.revision,
        })
    }

    /// A commit of either definition kind invalidates all competing receipts.
    /// Insertion has already been paid for; failed callbacks preserve revision.
    pub fn commit_enums_with<T, E>(
        &mut self,
        prepared: PreparedEnumAdmission,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, crate::EnumPublicationError<E>> {
        let next = self.revision.checked_add(1);
        if !Arc::ptr_eq(&self.identity, &prepared.identity)
            || self.revision != prepared.revision
            || next.is_none()
        {
            return Err(crate::EnumPublicationError::Admission(Error {
                kind: ErrorKind::InvalidCallPlan,
                source: None,
            }));
        }
        let result = self
            .vm
            .commit_admitted_enums_with(prepared.definitions, commit)?;
        self.revision = next.unwrap();
        Ok(result)
    }

    /// Install an admitted receipt and commit the enclosing publisher atomically.
    /// A stale or foreign receipt never invokes the callback. Callback failure or
    /// unwind removes the tentative entries and leaves the revision unchanged.
    pub fn commit_with<T, E>(
        &mut self,
        prepared: PreparedStructAdmission,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, StructPublicationError<E>> {
        let next = self.revision.checked_add(1);
        if !Arc::ptr_eq(&self.identity, &prepared.identity)
            || self.revision != prepared.revision
            || next.is_none()
        {
            return Err(StructPublicationError::Admission(Error {
                kind: ErrorKind::InvalidCallPlan,
                source: None,
            }));
        }
        let definitions = StructBatch::Shared(prepared.definitions);
        let mut installation = Installation {
            vm: &mut self.vm,
            definitions,
            inserted: 0,
            committed: false,
        };
        // No mutation is possible between admission and this revision check.
        // The same immutable batch was already collision- and scope-checked.
        for (index, definition) in installation.definitions.iter().enumerate() {
            installation.inserted += 1;
            installation.vm.structs.insert(
                definition.declaration,
                installation.definitions.definition(index),
            );
        }
        let result = installation.commit_with(commit)?;
        self.revision = next.unwrap();
        Ok(result)
    }
}

impl fmt::Display for DefinitionAdmissionSession<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(definition-admission-session (revision {}))",
            self.revision
        )
    }
}
impl fmt::Display for PreparedStructAdmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(prepared-struct-admission (revision {}) (definitions",
            self.revision
        )?;
        for definition in self.definitions.iter() {
            write!(f, " {}", definition.declaration)?;
        }
        f.write_str("))")
    }
}

impl fmt::Display for PreparedEnumAdmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(prepared-enum-admission (revision {}) (definitions",
            self.revision
        )?;
        for definition in self.definitions.iter() {
            write!(f, " {}", definition.declaration)?;
        }
        f.write_str("))")
    }
}
