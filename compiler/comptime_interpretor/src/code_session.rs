//! Immutable-world code reuse across independently budgeted invocations.
use super::*;

/// Owns the exact source or graph snapshot whose compiled code may be reused. This is
/// separate from result memoization: no invocation values or proof outcomes are
/// retained. A changed world requires a new session, even with identical IDs.
pub struct CodeSession {
    world: CodeWorld,
    graph_validated: bool,
    indexes: Option<Arc<declaration_index::SourceIndexes>>,
    bytecode: bytecode::CodeCache,
    inputs: input_bodies::Cache,
    defaults: default_code::Cache,
    contextual: contextual_code::Cache,
    instances: function_instances::Cache,
}
enum CodeWorld {
    Source(Arc<Package>),
    Graph(Arc<iir::Package>),
}
impl CodeSession {
    pub fn new(package: Arc<Package>) -> Self {
        Self {
            world: CodeWorld::Source(package),
            graph_validated: false,
            indexes: None,
            bytecode: Default::default(),
            inputs: Default::default(),
            defaults: Default::default(),
            contextual: Default::default(),
            instances: Default::default(),
        }
    }

    /// Code reuse and successful validation are bound to this exact immutable
    /// IIR snapshot, not declaration IDs. Every VM owns fresh invocation state.
    pub fn for_iir(graph: Arc<iir::Package>) -> Self {
        Self {
            world: CodeWorld::Graph(graph),
            graph_validated: false,
            indexes: None,
            bytecode: Default::default(),
            inputs: Default::default(),
            defaults: Default::default(),
            contextual: Default::default(),
            instances: Default::default(),
        }
    }

    fn matches(&self, vm: &Interpreter<'_>) -> bool {
        match &self.world {
            CodeWorld::Source(package) => {
                vm.graph.is_none()
                    && vm
                        .declaration_world
                        .is_some_and(|world| std::ptr::eq(world, package.as_ref()))
            }
            CodeWorld::Graph(graph) => vm
                .graph
                .is_some_and(|world| std::ptr::eq(world, graph.as_ref())),
        }
    }

    fn exchange(&mut self, vm: &mut Interpreter<'_>) {
        std::mem::swap(&mut self.bytecode, &mut vm.bytecode_cache);
        std::mem::swap(&mut self.inputs, &mut vm.input_body_cache);
        std::mem::swap(&mut self.defaults, &mut vm.default_code_cache);
        std::mem::swap(&mut self.contextual, &mut vm.contextual_code_cache);
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
    /// Reuse structural validation only for the exact graph owned by this session.
    /// Index construction and execution still consume the current allowance;
    /// input contracts and phase permissions are checked by each fresh VM.
    pub fn from_iir_for_comptime_in_session(
        graph: &'program iir::Package,
        session: &mut CodeSession,
        budget: &mut Budget,
    ) -> Result<Self, Error> {
        let matches = matches!(&session.world, CodeWorld::Graph(owner)
            if std::ptr::eq(graph, owner.as_ref()));
        if !matches {
            return Self::from_iir_for_comptime_metered(graph, budget);
        }
        let mut vm = if session.graph_validated {
            Self::from_validated_iir_metered(graph, budget)?
        } else {
            Self::from_iir_metered(graph, budget)?
        };
        session.graph_validated = true;
        vm.comptime_only = true;
        Ok(vm)
    }

    /// Construct a fresh execution state using this snapshot's declaration index.
    /// Phase, trait and formation checks still run with the current allowance.
    pub fn for_comptime_in_session(
        package: &'program Package,
        session: &mut CodeSession,
        budget: &mut Budget,
    ) -> Result<Self, Error> {
        let CodeWorld::Source(source) = &session.world else {
            return Self::for_comptime_in_budget(package, &[], budget);
        };
        let source = source.clone();
        if !std::ptr::eq(package, source.as_ref()) {
            return Self::for_comptime_in_budget(package, &[], budget);
        }
        let reserved = *budget;
        budget.steps = 0;
        let mut vm =
            Self::for_comptime_with_index(package, &[], reserved, session.indexes.clone())?;
        if session.indexes.is_none() {
            vm.charge_preparation(package.structs.len().saturating_add(package.enums.len()))?;
        }
        let indexes = session.indexes.get_or_insert_with(|| {
            Arc::new(declaration_index::SourceIndexes {
                method_contracts: vm.method_contracts.entries.clone(),
                normal_forms: vm.normal_forms.positions.clone(),
                newtypes: vm.newtypes.positions.clone(),
                aliases: vm.alias_contracts.positions.clone(),
                structs: Arc::new(
                    package
                        .structs
                        .iter()
                        .enumerate()
                        .map(|(index, definition)| {
                            (
                                definition.declaration,
                                generated_structs::StructDefinition::Source {
                                    package: source.clone(),
                                    index,
                                },
                            )
                        })
                        .collect(),
                ),
                enums: Arc::new(
                    package
                        .enums
                        .iter()
                        .enumerate()
                        .map(|(index, definition)| {
                            (
                                definition.declaration,
                                generated_enums::EnumDefinition::Source {
                                    package: source.clone(),
                                    index,
                                },
                            )
                        })
                        .collect(),
                ),
                method_parameters: vm.method_parameters.clone(),
                method_origins: vm.method_origins.clone(),
                parameter_domains: vm.declaration_parameter_domains.clone(),
                parameter_sorts: vm.declaration_parameter_sorts.clone(),
                functions: vm.functions.positions.clone(),
                runtime_identities: vm.runtime_function_identities.positions.clone(),
                effects: vm.effect_declarations.positions.clone(),
            })
        });
        vm.structs = layout_catalog::Catalog::from_base(indexes.structs.clone());
        vm.enums = layout_catalog::Catalog::from_base(indexes.enums.clone());
        budget.steps = vm.remaining_steps();
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
        let parameter = DeclarationId {
            package: 1,
            local: 4,
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
                type_parameters: vec![parameter],
                signature: signature.clone(),
            }],
            normal_forms: vec![nir::normal_forms::Head {
                declaration: DeclarationId {
                    package: 1,
                    local: 7,
                },
                collector: None,
                role: Default::default(),
                requirements: vec![],
                parameters: vec![],
            }],
            newtypes: vec![nir::newtypes::Newtype {
                declaration: DeclarationId {
                    package: 1,
                    local: 5,
                },
                name: "Flag".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                capture_count: 0,
                requirements: vec![],
                capabilities: vec![],
                representation: Term::Primitive(Primitive::Bool),
            }],
            alias_contracts: vec![nir::type_aliases::Contract {
                declaration: DeclarationId {
                    package: 1,
                    local: 6,
                },
                parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
            }],
            function_parameters: vec![nir::function_parameters::Declaration {
                declaration: identity,
                parameters: vec![parameter],
                domains: vec![Term::Primitive(Primitive::Bool)],
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
        let heads = first.normal_forms.positions.clone();
        let newtypes = first.newtypes.positions.clone();
        let aliases = first.alias_contracts.positions.clone();
        let domains = first.declaration_parameter_domains.clone();
        let sorts = first.declaration_parameter_sorts.clone();
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
        assert!(Arc::ptr_eq(&domains, &next.declaration_parameter_domains));
        assert!(Arc::ptr_eq(&sorts, &next.declaration_parameter_sorts));
        assert!(Arc::ptr_eq(&heads, &next.normal_forms.positions));
        assert!(std::ptr::eq(
            next.normal_forms
                .get(&source.normal_forms[0].declaration)
                .unwrap(),
            &source.normal_forms[0]
        ));
        assert!(Arc::ptr_eq(&newtypes, &next.newtypes.positions));
        assert!(Arc::ptr_eq(&aliases, &next.alias_contracts.positions));
        assert!(std::ptr::eq(
            next.newtypes.get(&source.newtypes[0].declaration).unwrap(),
            &source.newtypes[0]
        ));
        assert!(std::ptr::eq(
            next.alias_contracts
                .get(&source.alias_contracts[0].declaration)
                .unwrap(),
            &source.alias_contracts[0]
        ));
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
        assert!(next_budget.steps > budget.steps);
        assert_eq!(
            next.declaration_parameter_domains.get(&parameter),
            Some(&Term::Primitive(Primitive::Bool))
        );
        assert_eq!(
            next.declaration_parameter_sorts.get(&parameter),
            Some(&nir::Sort::ComptimeValue)
        );
        let mut changed = source.as_ref().clone();
        changed.function_parameters[0].domains[0] = Term::Primitive(Primitive::Char);
        let other =
            Interpreter::for_comptime_in_session(&changed, &mut session, &mut Budget::default())
                .unwrap();
        assert_eq!(
            other.declaration_parameter_domains.get(&parameter),
            Some(&Term::Primitive(Primitive::Char))
        );
        assert!(!Arc::ptr_eq(&domains, &other.declaration_parameter_domains));
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
    #[test]
    fn layout_catalog_publication_exhaustion_leaves_session_unpublished() {
        let source = Arc::new(Package {
            structs: vec![nir::program::Struct {
                declaration: DeclarationId {
                    package: 1,
                    local: 1,
                },
                name: "Record".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                fields: vec![],
                requirements: vec![],
                capabilities: vec![],
            }],
            ..Default::default()
        });
        let ordinary = Interpreter::for_comptime(&source, Budget::default()).unwrap();
        let construction = Budget::default().steps - ordinary.remaining_steps();
        let mut session = CodeSession::new(source.clone());
        let mut budget = Budget {
            steps: construction,
            depth: 128,
        };
        let error = Interpreter::for_comptime_in_session(&source, &mut session, &mut budget)
            .err()
            .expect("catalog ownership also consumes work");
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(budget.steps, 0);
        assert!(session.indexes.is_none());
        Interpreter::for_comptime_in_session(&source, &mut session, &mut Budget::default())
            .unwrap();
        assert!(session.indexes.is_some());
    }
}
