//! Immutable-source code reuse across independently budgeted invocations.
use super::*;

/// Owns the exact source snapshot whose compiled code may be reused. This is
/// separate from result memoization: no invocation values or proof outcomes are
/// retained. A changed package requires a new session, even with identical IDs.
pub struct CodeSession {
    package: Arc<Package>,
    bytecode: bytecode::CodeCache,
    inputs: input_bodies::Cache,
    defaults: default_code::Cache,
    instances: function_instances::Cache,
}
impl CodeSession {
    pub fn new(package: Arc<Package>) -> Self {
        Self {
            package,
            bytecode: Default::default(),
            inputs: Default::default(),
            defaults: Default::default(),
            instances: Default::default(),
        }
    }

    fn matches(&self, vm: &Interpreter<'_>) -> bool {
        vm.graph.is_none()
            && vm
                .declaration_world
                .is_some_and(|world| std::ptr::eq(world, self.package.as_ref()))
    }

    fn exchange(&mut self, vm: &mut Interpreter<'_>) {
        std::mem::swap(&mut self.bytecode, &mut vm.bytecode_cache);
        std::mem::swap(&mut self.inputs, &mut vm.input_body_cache);
        std::mem::swap(&mut self.defaults, &mut vm.default_code_cache);
        std::mem::swap(&mut self.instances, &mut vm.prepared_instance_cache);
    }
}

struct Scope<'session, 'vm, 'program> {
    session: &'session mut CodeSession,
    vm: &'vm mut Interpreter<'program>,
}
impl Drop for Scope<'_, '_, '_> {
    fn drop(&mut self) {
        // A host callback may replace the VM; never publish another world's code.
        if self.session.matches(self.vm) {
            self.session.exchange(self.vm);
        }
    }
}

impl<'program> Interpreter<'program> {
    /// Run with bounded immutable code caches from this exact source snapshot.
    /// A different world simply executes with its own caches. Completed code is
    /// returned on both failure and unwind; failed compilations never publish.
    pub fn with_code_session<T>(
        &mut self,
        session: &mut CodeSession,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        if !session.matches(self) {
            return operation(self);
        }
        session.exchange(self);
        let scope = Scope { session, vm: self };
        operation(scope.vm)
    }
}
