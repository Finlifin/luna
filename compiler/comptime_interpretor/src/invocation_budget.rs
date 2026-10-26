//! Scope a checked comptime world and synchronize its execution allowance.
use crate::{Budget, Error, Interpreter};
use nir::program::{Package, TraitRequirement};

struct Scope<'program, 'budget> {
    interpreter: Interpreter<'program>,
    budget: &'budget mut Budget,
}
impl Drop for Scope<'_, '_> {
    fn drop(&mut self) {
        // Keep the construction reservation even if host code replaced the VM
        // with another one. A scope must never replenish its caller's allowance.
        self.budget.steps = self.budget.steps.min(self.interpreter.remaining_steps());
    }
}

impl<'program> Interpreter<'program> {
    /// Prepare a fresh checked world, execute a host operation, and synchronize
    /// remaining steps on success, error, or unwinding. Construction failure
    /// retains the conservative reservation of for_comptime_in_budget.
    ///
    /// Consecutive scopes can use newly published packages without carrying
    /// function instances or evidence caches from the old world. Inputs and
    /// returned values retain ordinary invocation admission. This does not resume
    /// an active call stack or transfer its local storage/cleanup responsibilities.
    pub fn with_comptime_in_budget<T>(
        package: &'program Package,
        assumptions: &[TraitRequirement],
        budget: &mut Budget,
        operation: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let interpreter = Self::for_comptime_in_budget(package, assumptions, budget)?;
        let mut scope = Scope {
            interpreter,
            budget,
        };
        operation(&mut scope.interpreter)
    }
}
