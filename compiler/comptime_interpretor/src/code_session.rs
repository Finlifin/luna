//! Immutable-source code reuse across independently budgeted invocations.
use super::*;

/// Owns the exact source snapshot whose compiled code may be reused. This is
/// separate from result memoization: no invocation values or proof outcomes are
/// retained. A changed package requires a new session, even with identical IDs.
pub struct CodeSession {
    package: Arc<Package>,
    indexes: Option<Arc<declaration_index::SourceIndexes>>,
    bytecode: bytecode::CodeCache,
    inputs: input_bodies::Cache,
    defaults: default_code::Cache,
    instances: function_instances::Cache,
}
impl CodeSession {
    pub fn new(package: Arc<Package>) -> Self {
        Self {
            package,
            indexes: None,
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
    /// Construct a fresh execution state using this snapshot's declaration index.
    /// Phase, trait and formation checks still run with the current allowance.
    pub fn for_comptime_in_session(
        package: &'program Package,
        session: &mut CodeSession,
        budget: &mut Budget,
    ) -> Result<Self, Error> {
        if !std::ptr::eq(package, session.package.as_ref()) {
            return Self::for_comptime_in_budget(package, &[], budget);
        }
        let reserved = *budget;
        budget.steps = 0;
        let vm = Self::for_comptime_with_index(package, &[], reserved, session.indexes.clone())?;
        budget.steps = vm.remaining_steps();
        session.indexes.get_or_insert_with(|| {
            Arc::new(declaration_index::SourceIndexes {
                functions: vm.functions.positions.clone(),
                runtime_identities: vm.runtime_function_identities.positions.clone(),
                effects: vm.effect_declarations.positions.clone(),
            })
        });
        Ok(vm)
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_index_is_shared_without_reusing_execution_state() {
        let identity = DeclarationId {
            package: 1,
            local: 2,
        };
        let effect = DeclarationId {
            package: 1,
            local: 3,
        };
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let source = Arc::new(Package {
            runtime_function_identities: vec![nir::program::RuntimeFunctionIdentity {
                declaration: identity,
                type_parameters: vec![],
                signature: signature.clone(),
            }],
            effects: vec![nir::effects::Declaration {
                declaration: effect,
                group: None,
                name: "read".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                signature,
                abi: nir::effects::Abi::Direct,
                inline: false,
                requirements: vec![],
                capabilities: vec![],
            }],
            ..Default::default()
        });
        let mut session = CodeSession::new(source.clone());
        let mut budget = Budget::default();
        let mut first =
            Interpreter::for_comptime_in_session(&source, &mut session, &mut budget).unwrap();
        let index = first.functions.positions.clone();
        let identities = first.runtime_function_identities.positions.clone();
        let effects = first.effect_declarations.positions.clone();
        first.events.push(Event::EnterFunction(DeclarationId {
            package: 1,
            local: 1,
        }));
        let mut next_budget = Budget::default();
        let next =
            Interpreter::for_comptime_in_session(&source, &mut session, &mut next_budget).unwrap();
        assert!(Arc::ptr_eq(&index, &next.functions.positions));
        assert!(Arc::ptr_eq(
            &identities,
            &next.runtime_function_identities.positions
        ));
        assert!(Arc::ptr_eq(&effects, &next.effect_declarations.positions));
        assert!(next.events().is_empty());
        assert!(next.effect_state.environment.is_empty());
        assert!(std::ptr::eq(
            next.runtime_function_identities.get(&identity).unwrap(),
            &source.runtime_function_identities[0]
        ));
        assert!(std::ptr::eq(
            next.effect_declarations.get(&effect).unwrap(),
            &source.effects[0]
        ));
        assert_eq!(budget.steps, next_budget.steps);
        let mut exhausted = Budget {
            steps: 0,
            depth: 128,
        };
        assert_eq!(
            Interpreter::for_comptime_in_session(&source, &mut session, &mut exhausted)
                .err()
                .unwrap()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert!(Arc::ptr_eq(
            &index,
            &session.indexes.as_ref().unwrap().functions
        ));
    }
}
