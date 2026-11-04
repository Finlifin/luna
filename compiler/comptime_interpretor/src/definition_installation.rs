//! Tentative definition ownership shared by concrete nominal publishers.
use super::*;

/// Payload adapters only select the catalog and immutable descriptor handles.
/// Callers must validate collisions and prepay insertion/rollback before staging.
pub(super) trait Batch<'world> {
    const KIND: &'static str;
    fn length(&self) -> usize;
    fn declaration(&self, index: usize) -> DeclarationId;
    fn insert(&self, vm: &mut Interpreter<'world>, index: usize);
    fn remove(&self, vm: &mut Interpreter<'world>, index: usize);
}

pub(super) struct Installation<'vm, 'world, B: Batch<'world>> {
    pub(super) vm: &'vm mut Interpreter<'world>,
    definitions: B,
    inserted: usize,
    committed: bool,
}

impl<'vm, 'world, B: Batch<'world>> Installation<'vm, 'world, B> {
    /// Expose the whole checked batch before its recursive formation checks.
    /// The guard owns every tentative insertion, including during unwinding.
    pub(super) fn stage(vm: &'vm mut Interpreter<'world>, definitions: B) -> Self {
        let mut installation = Self {
            vm,
            definitions,
            inserted: 0,
            committed: false,
        };
        for index in 0..installation.definitions.length() {
            installation.inserted += 1;
            installation.definitions.insert(installation.vm, index);
        }
        installation
    }

    pub(super) fn commit_with<T, E>(
        mut self,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        let result = commit()?;
        self.committed = true;
        Ok(result)
    }
}

impl<'world, B: Batch<'world>> Drop for Installation<'_, 'world, B> {
    fn drop(&mut self) {
        if !self.committed {
            for index in 0..self.inserted {
                self.definitions.remove(self.vm, index);
            }
        }
    }
}

impl<'world, B: Batch<'world>> fmt::Display for Installation<'_, 'world, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(definition-installation (kind {}) (committed {}) (inserted",
            B::KIND,
            self.committed
        )?;
        for index in 0..self.inserted {
            write!(f, " {}", self.definitions.declaration(index))?;
        }
        f.write_str("))")
    }
}
