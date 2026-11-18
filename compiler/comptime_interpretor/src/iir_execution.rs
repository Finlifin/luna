//! Execution from retained concrete instances; no source templates are consulted.
use super::*;
mod indexes;
mod nominals;
pub(super) use indexes::GraphIndexes;
pub(super) use nominals::ConcreteNominal;

impl<'a> Interpreter<'a> {
    pub(super) fn eval_template_equal(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::TemplateEqual(left, right) = &expr.kind else {
            unreachable!()
        };
        if self.graph.is_none()
            || self.comptime_only
            || left.ty != right.ty
            || expr.ty != nir::Term::Primitive(nir::Primitive::Bool)
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let a = match self.eval(left, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        let b = match self.eval(right, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        if !self.has_type(&a, &left.ty) || !self.has_type(&b, &right.ty) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        if [&a, &b].iter().any(|value| {
            !matches!(
                value,
                Value::FunctionInstance { .. } | Value::RuntimeTraitMethod(_)
            )
        }) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.charge_copy_work(Some(expr.source), |fuel| {
            for value in [&a, &b] {
                match value {
                    Value::FunctionInstance { arguments, .. } => {
                        for term in arguments {
                            nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
                        }
                    }
                    Value::RuntimeTraitMethod(identity) => {
                        for term in std::iter::once(&identity.subject)
                            .chain(&identity.interface.arguments)
                            .chain(&identity.method_arguments)
                        {
                            nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Ok(())
        })?;
        let equal = match (&a, &b) {
            (
                Value::FunctionInstance {
                    declaration: ad,
                    arguments: aa,
                },
                Value::FunctionInstance {
                    declaration: bd,
                    arguments: ba,
                },
            ) => ad == bd && aa == ba,
            (Value::RuntimeTraitMethod(a), Value::RuntimeTraitMethod(b)) => a == b,
            (Value::FunctionInstance { .. }, Value::RuntimeTraitMethod(_))
            | (Value::RuntimeTraitMethod(_), Value::FunctionInstance { .. }) => false,
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        };
        Ok(Flow::Value(Value::Bool(equal)))
    }
    /// Execute concrete staged instances without granting runtime functions
    /// permission to run during comptime invocation.
    pub fn from_iir_for_comptime(
        graph: &'a iir::Package,
        mut budget: Budget,
    ) -> Result<Self, Error> {
        Self::from_iir_for_comptime_metered(graph, &mut budget)
    }

    /// Reference execution for differential checks of concrete comptime graphs.
    pub fn from_iir_for_comptime_tree(
        graph: &'a iir::Package,
        budget: Budget,
    ) -> Result<Self, Error> {
        Self::from_iir_for_comptime(graph, budget).map(Self::with_reference_execution)
    }

    /// Return preparation work even if graph validation rejects the input.
    pub fn from_iir_for_comptime_metered(
        graph: &'a iir::Package,
        budget: &mut Budget,
    ) -> Result<Self, Error> {
        let mut interpreter = Self::from_iir_metered(graph, budget)?;
        interpreter.comptime_only = true;
        Ok(interpreter)
    }

    /// Execute a previously materialized graph using the reference evaluator.
    /// This is not native code generation or a replacement for resource checks.
    pub fn from_iir(graph: &'a iir::Package, mut budget: Budget) -> Result<Self, Error> {
        Self::from_iir_metered(graph, &mut budget)
    }

    /// Charge validation to the caller's allowance before publishing a VM.
    /// After success, the VM owns the remaining execution allowance.
    pub fn from_iir_metered(graph: &'a iir::Package, budget: &mut Budget) -> Result<Self, Error> {
        let mut fuel = usize::try_from(budget.steps).unwrap_or(usize::MAX);
        let validation = graph.validate(&mut fuel);
        budget.steps = fuel as u64;
        validation.map_err(|_| Error {
            kind: ErrorKind::InvalidCallPlan,
            source: None,
        })?;
        Self::from_validated_iir_metered(graph, budget)
    }

    // Only an exact immutable graph owner may retain the validation prerequisite.
    pub(super) fn from_validated_iir_metered(
        graph: &'a iir::Package,
        budget: &mut Budget,
    ) -> Result<Self, Error> {
        let indexes = Arc::new(GraphIndexes::new(graph, budget)?);
        Self::from_indexed_iir_metered(graph, budget, indexes)
    }

    // The caller must own the same immutable, validated graph as these indexes.
    pub(super) fn from_indexed_iir_metered(
        graph: &'a iir::Package,
        budget: &mut Budget,
        indexes: Arc<GraphIndexes>,
    ) -> Result<Self, Error> {
        budget.steps = budget.steps.checked_sub(1).ok_or(Error {
            kind: ErrorKind::StepsExhausted,
            source: None,
        })?;
        Ok(Self {
            graph_indexes: Some(indexes.clone()),
            bytecode_invocations: 0,
            bytecode_default_evaluations: 0,
            bytecode_staged_inputs: 0,
            bytecode_contextual_inputs: 0,
            bytecode_input_preparations: 0,
            bytecode_compilations: 0,
            bytecode_compilation_steps: 0,
            compilation_session: comptime_compiler::CompilationSession::for_concrete_graph(),
            bytecode_cache: Default::default(),
            borrowed_code_cache: Default::default(),
            input_body_cache: Default::default(),
            default_code_cache: Default::default(),
            contextual_code_cache: Default::default(),
            prepared_instance_cache: Default::default(),
            declaration_world: None,
            phase_inspection: None,
            reserved_source_declarations: None,
            // IIR retains occurrence environments, not the source effect world.
            effect_declarations: Default::default(),
            effect_state: super::effects::State::default(),
            effect_values: None,

            graph: Some(graph),
            target: graph.target,
            cleanup_schedule: None,
            cleanup_scopes: HashMap::new(),
            cleanup_sites: HashMap::new(),
            cleanup_effect_context: HashMap::new(),
            cleanup_frame: None,
            cleanup_failed: false,
            prepared_frames: vec![],
            residual_temporaries: vec![],
            pattern_binding_orders: BTreeMap::new(),
            cleanup_context: vec![],
            comptime_only: false,
            bytecode_enabled: true,
            abstract_type_parameters: BTreeSet::new(),
            abstract_value_parameters: BTreeMap::new(),
            struct_builder: None,
            enum_builder: None,
            newtype_builder: None,
            type_solver: None,
            method_resolver: None,
            associated_resolver: None,
            pointer_copy: None,
            declaration_parameter_sorts: Default::default(),
            declaration_parameter_domains: Default::default(),
            method_parameters: Default::default(),
            method_contracts: Default::default(),
            method_origins: Default::default(),
            assumed_interfaces: HashSet::new(),
            trait_declarations: Default::default(),
            implemented_interfaces: indexes.implementations.clone(),
            method_bodies: Arc::new(HashMap::new()),
            resolved_method_bodies: HashMap::new(),
            associated_values: Default::default(),
            associated_initializers: Default::default(),
            active_associated_initializers: HashSet::new(),
            instances: Default::default(),
            iir_callables: Default::default(),
            functions: Default::default(),
            normal_forms: declaration_index::NormalForms::new(
                &graph.normal_forms,
                Some(indexes.normal_forms.clone()),
                |head| head.declaration,
            )?,
            structs: Default::default(),
            newtypes: Default::default(),
            alias_contracts: Default::default(),
            enums: Default::default(),
            budget: Budget {
                steps: budget.steps,
                depth: budget.depth.min(256),
            },
            depth: 0,
            invocation_depth: 0,
            invocation_depth_limit: budget.depth,
            events: vec![],
        })
    }

    /// Install graph-bound Copy/Move decisions supplied by resource elaboration.
    /// This authorizes value delivery only, not effect cleanup or unwinding.
    pub fn with_effect_values(
        mut self,
        plan: &'a iir::effect_values::Plan<'a>,
    ) -> Result<Self, Error> {
        if !self.graph.is_some_and(|graph| plan.belongs_to(graph))
            || self.comptime_only
            || self.cleanup_schedule.is_some()
        {
            return Err(self.failure(ErrorKind::InvalidCallPlan, None));
        }
        self.effect_values = Some(plan);
        Ok(self)
    }

    pub fn invoke_instance(
        &mut self,
        key: &iir::InstanceKey,
        arguments: Vec<Value>,
    ) -> Result<Value, Error> {
        let result = self
            .iir_callable(key)
            .and_then(|callable| self.invoke_value(callable, arguments));
        // Generated ranges identify cleanup sites during execution. Resolve
        // them only after invocation has finished unwinding, for diagnostics.
        result.map_err(|mut error| {
            if let Some(graph) = self.graph {
                error.source = error
                    .source
                    .and_then(|source| source.original(&graph.expansion_sources));
            }
            error
        })
    }

    pub(super) fn iir_method(
        &self,
        interface: DeclarationId,
        arguments: &[Term],
        subject: &Term,
        member: DeclarationId,
        method_arguments: &[Term],
    ) -> Result<Option<&'a nir::program::MethodContract>, Error> {
        let Some(graph) = self.graph else {
            return Ok(None);
        };
        self.graph_indexes
            .as_ref()
            .and_then(|indexes| {
                indexes
                    .contracts
                    .get(interface, arguments, subject, member, method_arguments)
            })
            .map(|(outer, inner)| &graph.interfaces[outer].methods[inner])
            .map(Some)
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, None))
    }

    fn iir_callable(&mut self, key: &iir::InstanceKey) -> Result<Value, Error> {
        self.iir_callable_handle(key).map(Value::IirCallable)
    }

    pub(super) fn iir_callable_handle(
        &mut self,
        key: &iir::InstanceKey,
    ) -> Result<Arc<IirCallable>, Error> {
        // External keys and warm cache queries must be bounded before any
        // structural hash, including the graph's definition lookup.
        self.charge_copy_work(None, |fuel| {
            *fuel = fuel
                .checked_sub(1)
                .ok_or(nir::copy_budget::CopyBudgetExhausted::Steps)?;
            for argument in key.arguments.iter() {
                nir::copy_budget::charge_type_substitution(argument, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        let position = self
            .iir_function_position(key.declaration, &key.arguments)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
        if let Some(handle) = self.iir_callables.get(&position) {
            return Ok(handle.clone());
        }
        let instance = &self.graph.expect("indexed graph").functions[position];
        let declaration = &instance.function.interface;
        self.charge_copy_work(Some(declaration.source), |fuel| {
            nir::copy_budget::charge_type_substitution(&declaration.signature, &[], &[], fuel)
        })?;
        let handle = Arc::new(IirCallable {
            position,
            key: instance.key.clone(),
            ty: declaration.callable_type(),
        });
        self.iir_callables.insert(position, handle.clone());
        Ok(handle)
    }

    pub(super) fn iir_target(
        &self,
        handle: &Arc<IirCallable>,
    ) -> Result<&'a nir::program::Function, Error> {
        if !self
            .iir_callables
            .get(&handle.position)
            .is_some_and(|local| Arc::ptr_eq(local, handle))
        {
            return Err(self.failure(ErrorKind::Type, None));
        }
        Ok(&self.graph.expect("local graph callable").functions[handle.position].function)
    }

    pub(super) fn eval_iir_callable(&mut self, expr: &Expr) -> Result<Flow, Error> {
        let value = match &expr.kind {
            K::Function(declaration) => {
                self.iir_function_value(*declaration, &[], &expr.ty, expr.source)?
            }
            K::FunctionInstance {
                declaration,
                arguments,
            } => self.iir_function_value(*declaration, arguments, &expr.ty, expr.source)?,
            K::TraitMethod { .. } => {
                self.iir_method_value(comptime_compiler::methods::View::expression(expr).unwrap())?
            }
            _ => return Err(self.failure(ErrorKind::InvalidCallPlan, Some(expr.source))),
        };
        Ok(Flow::Value(value))
    }

    pub(super) fn iir_function_value(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
        output: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        self.charge_copy_work(Some(source), |fuel| {
            for term in arguments {
                nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        self.iir_named_value(
            iir::InstanceKey {
                declaration,
                arguments: arguments.to_vec().into(),
            },
            None,
            output,
            source,
        )
    }

    pub(super) fn iir_method_value(
        &mut self,
        method: comptime_compiler::methods::View<'_>,
    ) -> Result<Value, Error> {
        self.charge_copy_work(Some(method.source), |fuel| {
            for term in std::iter::once(method.subject)
                .chain(method.interface_arguments)
                .chain(method.method_arguments)
            {
                nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        let graph = self
            .graph
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(method.source)))?;
        let selected = self
            .graph_indexes
            .as_ref()
            .and_then(|indexes| {
                indexes.dispatch.get(
                    method.interface,
                    method.interface_arguments,
                    method.subject,
                    method.member,
                    method.method_arguments,
                )
            })
            .map(|(outer, inner)| &graph.dispatch[outer].methods[inner])
            .ok_or_else(|| self.failure(ErrorKind::UnmaterializedDispatch, Some(method.source)))?;
        self.charge_copy_work(Some(method.source), |fuel| {
            for term in &selected.arguments {
                nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        self.iir_named_value(
            iir::InstanceKey {
                declaration: selected.body,
                arguments: selected.arguments.clone().into(),
            },
            Some(method),
            method.output,
            method.source,
        )
    }

    fn iir_named_value(
        &mut self,
        key: iir::InstanceKey,
        method: Option<comptime_compiler::methods::View<'_>>,
        output: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        // A runtime interface member transported through comptime keeps the
        // public member identity even when its selected body is executable.
        let transport_method =
            method.is_some() && self.comptime_only && !output.is_comptime_callable();
        let value = if transport_method
            || self
                .iir_template_type(key.declaration, &key.arguments)
                .is_some()
        {
            if let Some(method) = method {
                Value::RuntimeTraitMethod(Box::new(TraitMethodIdentity {
                    interface: InterfaceInstance {
                        declaration: method.interface,
                        arguments: method.interface_arguments.to_vec(),
                    },
                    subject: method.subject.clone(),
                    member: method.member,
                    method_arguments: method.method_arguments.to_vec(),
                }))
            } else {
                Value::FunctionInstance {
                    declaration: key.declaration,
                    arguments: key.arguments.to_vec(),
                }
            }
        } else {
            self.iir_callable(&key)?
        };
        self.check_value_type(&value, output, Some(source))?;
        Ok(value)
    }
}

#[cfg(test)]
mod callable_budget_tests {
    use super::*;

    fn graph(name: String) -> iir::Package {
        let declaration = DeclarationId {
            package: 97,
            local: 1,
        };
        iir::Package {
            functions: vec![iir::FunctionInstance {
                key: iir::InstanceKey {
                    declaration,
                    arguments: vec![Term::Primitive(Primitive::Bool)].into(),
                },
                function: nir::program::Function {
                    interface: nir::program::FunctionDeclaration {
                        source: nir::program::SourceRange { start: 0, end: 0 },
                        declaration,
                        name,
                        phase: nir::program::FunctionPhase::Runtime,
                        inline: false,
                        pure: false,
                        is_unsafe: false,
                        type_parameters: vec![],
                        parameters: vec![],
                        signature: Term::Function {
                            parameters: vec![],
                            result: Box::new(Term::Primitive(Primitive::Bool)),
                        },
                        defaults: vec![],
                        preconditions: vec![],
                        postconditions: vec![],
                        requirements: vec![],
                        capabilities: vec![],
                        result_binding: None,
                        entry_snapshots: vec![],
                    },
                    body: Expr {
                        source: SourceRange { start: 2, end: 9 },
                        ty: Term::Primitive(Primitive::Bool),
                        kind: K::Literal(Literal::Bool(true)),
                    },
                },
            }],
            ..Default::default()
        }
    }

    #[test]
    fn cached_iir_callables_still_charge_keys_before_lookup() {
        let graph = graph("run".into());
        let key = &graph.functions[0].key;
        let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        let Value::IirCallable(first) = vm.iir_callable(key).unwrap() else {
            panic!("missing graph callable")
        };
        vm.budget.steps = 0;
        assert_eq!(
            vm.iir_callable(key).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        vm.budget.steps = 1000;
        let Value::IirCallable(second) = vm.iir_callable(key).unwrap() else {
            panic!("missing cached graph callable")
        };
        assert!(Arc::ptr_eq(&first, &second));
        assert!(vm.budget.steps < 1000);
        let unknown = iir::InstanceKey {
            declaration: key.declaration,
            arguments: vec![Term::Record(
                (0..64)
                    .map(|i| (format!("field{i}"), Term::Primitive(Primitive::Bool)))
                    .collect(),
            )]
            .into(),
        };
        vm.budget.steps = 10;
        assert_eq!(
            vm.iir_callable(&unknown).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        vm.budget.steps = 10_000;
        assert_eq!(
            vm.iir_callable(&unknown).unwrap_err().kind,
            ErrorKind::UnknownFunction
        );
        let mut deep = Term::Primitive(Primitive::Bool);
        for _ in 0..160 {
            deep = Term::Pointer(Box::new(deep));
        }
        let deep_key = iir::InstanceKey {
            declaration: key.declaration,
            arguments: vec![deep].into(),
        };
        assert_eq!(
            vm.iir_callable(&deep_key).unwrap_err().kind,
            ErrorKind::DepthExhausted
        );
        assert_eq!(vm.iir_callables.len(), 1);
        assert!(matches!(
            vm.iir_callable(key).unwrap(),
            Value::IirCallable(_)
        ));
    }

    #[test]
    fn shared_code_does_not_admit_another_vms_callable() {
        let mut source = graph("run".into());
        let input = LocalId { owner: 1, index: 0 };
        source.functions[0].function.parameters.push(input);
        let Term::Function { parameters, .. } = &mut source.functions[0].function.signature else {
            panic!("expected function signature")
        };
        parameters.push(nir::Parameter {
            sort: nir::Sort::Runtime,
            ty: Term::Primitive(Primitive::Bool),
        });
        source.functions[0].function.body.kind = K::Local(input);
        let graph = Arc::new(source);
        let key = &graph.functions[0].key;
        let domain = graph.functions[0].function.callable_type();
        let mut cache = CodeSession::for_iir(graph.clone());
        let mut first = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        let foreign = first
            .with_code_session(&mut cache, |vm| vm.iir_callable(key))
            .unwrap();
        first.validate_immutable_input(&foreign, &domain).unwrap();
        let mut second = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        let local = second
            .with_code_session(&mut cache, |vm| vm.iir_callable(key))
            .unwrap();
        second.validate_immutable_input(&local, &domain).unwrap();
        assert_eq!(
            second
                .validate_immutable_input(&foreign, &domain)
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        let (Value::IirCallable(foreign), Value::IirCallable(local)) = (&foreign, &local) else {
            panic!("concrete IIR calls must retain their local callable")
        };
        assert!(!Arc::ptr_eq(foreign, local));
        let frozen = Value::IirCallable(foreign.clone())
            .into_scoped_invocation_term(&mut comptime_runtime::Budget::default(), &BTreeSet::new())
            .unwrap();
        let Value::IirCallable(restored) = second.normal_pattern_value(&frozen, &domain).unwrap()
        else {
            panic!("frozen instance must resolve in the receiving graph")
        };
        assert!(Arc::ptr_eq(local, &restored));
        assert!(std::ptr::eq(
            first.iir_target(foreign).unwrap(),
            second.iir_target(local).unwrap()
        ));
        assert_eq!(
            second
                .invoke_value(Value::IirCallable(foreign.clone()), vec![Value::Bool(true)])
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        assert_eq!(
            second
                .invoke_value(Value::IirCallable(local.clone()), vec![Value::Bool(false)])
                .unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            first
                .invoke_value(Value::IirCallable(foreign.clone()), vec![Value::Bool(true)])
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn copying_named_callables_preserves_local_admission_without_importing_handles() {
        let graph = graph("run".into());
        let key = &graph.functions[0].key;
        let domain = graph.functions[0].function.callable_type();
        let source = graph.functions[0].function.body.source;
        let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        let local = vm.iir_callable(key).unwrap();
        let mut other = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        let foreign = other.iir_callable(key).unwrap();
        let copied = vm.copy_representation(&local, source, 0).unwrap();
        let checkpoint = vm.checkpoint_representation(&local, source).unwrap();
        let container = Value::Tuple(vec![local.clone()]);
        let nested = vm.copy_representation(&container, source, 0).unwrap();
        let Value::Tuple(nested) = nested else {
            panic!("tuple copy lost structure")
        };
        for copy in [copied, checkpoint, nested[0].clone()] {
            vm.validate_immutable_input(&copy, &domain).unwrap();
            let (Value::IirCallable(original), Value::IirCallable(copied)) = (&local, &copy) else {
                panic!("expected concrete callable")
            };
            assert!(Arc::ptr_eq(original, copied));
            assert_eq!(vm.invoke_value(copy, vec![]).unwrap(), Value::Bool(true));
        }
        // Copying an imported handle cannot turn it into this VM's own handle.
        for preserve in [false, true] {
            let copied = if preserve {
                vm.checkpoint_representation(&foreign, source).unwrap()
            } else {
                vm.copy_representation(&foreign, source, 0).unwrap()
            };
            assert_eq!(
                vm.validate_immutable_input(&copied, &domain)
                    .unwrap_err()
                    .kind,
                ErrorKind::Type
            );
        }
        vm.budget.steps = 0;
        assert_eq!(
            vm.copy_representation(&local, source, 0).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
    }

    #[test]
    fn forming_graph_handles_charges_the_signature_in_each_vm() {
        let mut source = graph("wide".into());
        let definition = &mut source.functions[0].function;
        definition.parameters = (0..256).map(|index| LocalId { owner: 1, index }).collect();
        let Term::Function { parameters, .. } = &mut definition.signature else {
            panic!("expected function signature")
        };
        parameters.extend((0..256).map(|_| nir::Parameter {
            sort: nir::Sort::Runtime,
            ty: Term::Primitive(Primitive::Bool),
        }));
        let graph = Arc::new(source);
        let key = &graph.functions[0].key;
        let mut cache = CodeSession::for_iir(graph.clone());
        let mut first = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        let Value::IirCallable(initial) = first
            .with_code_session(&mut cache, |vm| vm.iir_callable(key))
            .unwrap()
        else {
            panic!("missing initial callable")
        };
        let mut second = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        second.budget.steps = 64;
        assert_eq!(
            second
                .with_code_session(&mut cache, |vm| vm.iir_callable(key))
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert!(second.iir_callables.is_empty());
        second.budget.steps = 100_000;
        let Value::IirCallable(recovered) = second
            .with_code_session(&mut cache, |vm| vm.iir_callable(key))
            .unwrap()
        else {
            panic!("missing recovered callable")
        };
        assert!(!Arc::ptr_eq(&initial, &recovered));
        assert_eq!(initial.key(), recovered.key());
        assert_eq!(second.iir_callables.len(), 1);
    }

    #[test]
    fn rejected_graph_entry_is_independent_of_body_width() {
        let mut costs = Vec::new();
        for width in [0, 2048] {
            let mut graph = graph("guarded".into());
            let function = &mut graph.functions[0].function;
            function.phase = nir::program::FunctionPhase::Comptime;
            let mut condition = function.body.clone();
            condition.kind = K::Literal(Literal::Bool(false));
            function.preconditions.push(condition.clone());
            function.body.kind = K::Block {
                statements: vec![condition.clone(); width],
                result: Some(Box::new(condition)),
            };
            for tree in [false, true] {
                let mut vm = if tree {
                    Interpreter::from_iir_for_comptime_tree(&graph, Budget::default())
                } else {
                    Interpreter::from_iir_for_comptime(&graph, Budget::default())
                }
                .unwrap();
                vm.budget.steps = 500;
                let key = &graph.functions[0].key;
                let handle = vm.iir_callable(key).unwrap();
                let after_formation = vm.budget.steps;
                assert_eq!(
                    handle
                        .clone()
                        .into_scoped_invocation_term(
                            &mut comptime_runtime::Budget::default(),
                            &BTreeSet::new()
                        )
                        .unwrap(),
                    Term::FunctionValue {
                        declaration: key.declaration,
                        arguments: key.arguments.to_vec()
                    }
                );
                assert_eq!(
                    vm.invoke_value(handle, vec![]).unwrap_err().kind,
                    ErrorKind::ProgramContractRejected
                );
                assert!(!vm.events().contains(&Event::EnterFunction(key.declaration)));
                costs.push((after_formation, vm.budget.steps));
            }
        }
        assert_eq!(costs[..2], costs[2..]);
    }

    #[test]
    fn graph_callable_formation_ignores_declaration_names() {
        let graph = graph("unused".repeat(1000));
        let key = &graph.functions[0].key;
        let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        vm.budget.steps = 500;
        assert!(matches!(
            vm.iir_callable(key).unwrap(),
            Value::IirCallable(_)
        ));
        assert_eq!(vm.invoke_instance(key, vec![]).unwrap(), Value::Bool(true));
    }
}
