//! Native recursion scopes restore their caller's depth on every exit.
//! Invocation activations and explicit VM stacks retain their own ownership.
use super::*;

struct Scope<'vm, 'program> {
    vm: &'vm mut Interpreter<'program>,
    depth: usize,
}

impl Drop for Scope<'_, '_> {
    fn drop(&mut self) {
        self.vm.depth = self.depth;
    }
}

impl Interpreter<'_> {
    pub(super) fn with_depth_checkpoint<T>(&mut self, operation: impl FnOnce(&mut Self) -> T) -> T {
        let scope = Scope {
            depth: self.depth,
            vm: self,
        };
        operation(&mut *scope.vm)
    }

    /// The caller must charge and check depth before entering this scope.
    /// Keeping that check outside preserves its original failure boundary.
    pub(super) fn with_native_depth<T>(&mut self, operation: impl FnOnce(&mut Self) -> T) -> T {
        self.with_depth_checkpoint(|vm| {
            vm.depth += 1;
            operation(vm)
        })
    }
}
