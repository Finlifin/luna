//! Execution from retained concrete instances; no source templates are consulted.
use super::*;

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
        let mut interpreter = Self::from_iir_for_comptime(graph, budget)?;
        interpreter.bytecode_enabled = false;
        Ok(interpreter)
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
        let mut retained_callable_signatures = HashMap::new();
        for identity in &graph.callable_identities {
            let indexed = (|| {
                fuel = fuel
                    .checked_sub(1)
                    .ok_or(nir::type_computations::Error::Budget)?;
                for argument in identity.key.arguments.iter() {
                    nir::type_computations::charge_term(argument, &mut fuel)?;
                }
                let signature = identity
                    .closed_signature()
                    .expect("validated callable signature");
                retained_callable_signatures.insert(
                    (identity.key.declaration, identity.key.arguments.as_ref()),
                    signature,
                );
                Ok::<_, nir::type_computations::Error>(())
            })();
            budget.steps = fuel as u64;
            indexed.map_err(|_| Error {
                kind: ErrorKind::StepsExhausted,
                source: None,
            })?;
        }
        Ok(Self {
            retained_callable_signatures,
            bytecode_invocations: 0,
            bytecode_default_evaluations: 0,
            bytecode_staged_inputs: 0,
            bytecode_contextual_inputs: 0,
            bytecode_input_preparations: 0,
            bytecode_compilations: 0,
            bytecode_compilation_steps: 0,
            bytecode_cache: Default::default(),
            borrowed_code_cache: Default::default(),
            input_body_cache: Default::default(),
            default_code_cache: Default::default(),
            contextual_code_cache: Default::default(),
            prepared_instance_cache: Default::default(),
            declaration_world: None,
            // IIR retains occurrence environments, not the source effect world.
            effect_declarations: Default::default(),
            effect_state: super::effects::State::default(),
            effect_values: None,
            runtime_function_identities: Default::default(),
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
            type_solver: None,
            pointer_copy: None,
            declaration_parameter_sorts: Default::default(),
            declaration_parameter_domains: Default::default(),
            method_parameters: Default::default(),
            method_contracts: Default::default(),
            method_origins: Default::default(),
            assumed_interfaces: HashSet::new(),
            implemented_interfaces: graph
                .dispatch
                .iter()
                .map(|d| {
                    (
                        (d.interface_instance(), d.subject.clone()),
                        d.implementation,
                    )
                })
                .collect(),
            method_bodies: HashMap::new(),
            associated_values: HashMap::new(),
            associated_initializers: HashMap::new(),
            active_associated_initializers: HashSet::new(),
            instances: Default::default(),
            functions: Default::default(),
            normal_forms: declaration_index::NormalForms::new(&graph.normal_forms, None, |head| {
                head.declaration
            })?,
            structs: Default::default(),
            newtypes: Default::default(),
            alias_contracts: Default::default(),
            enums: Default::default(),
            budget: Budget {
                steps: fuel as u64,
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

    pub(super) fn invocation_definition(
        &self,
        id: DeclarationId,
        value: &Value,
    ) -> Option<&'a nir::program::Function> {
        if let Some(graph) = self.graph {
            let arguments = match value {
                Value::Closure(c) => c.instance_arguments.clone().unwrap_or_default(),
                _ => Arc::from([]),
            };
            graph
                .function(&iir::InstanceKey {
                    declaration: id,
                    arguments: arguments.into(),
                })
                .map(|f| &f.function)
        } else {
            self.functions.get(&id)
        }
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
        graph
            .interfaces
            .iter()
            .find(|i| {
                i.interface.declaration == interface
                    && i.interface.arguments == arguments
                    && &i.subject == subject
            })
            .and_then(|i| {
                i.methods
                    .iter()
                    .find(|m| m.declaration == member && m.type_arguments == method_arguments)
            })
            .map(Some)
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, None))
    }

    fn iir_callable(&mut self, key: &iir::InstanceKey) -> Result<Value, Error> {
        let graph = self
            .graph
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, None))?;
        let definition = &graph
            .function(key)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?
            .function;
        let cache_key = (key.declaration, key.arguments.clone());
        if let Some(callable) = self.instances.lookup(key.declaration, &key.arguments) {
            return Ok(Value::Closure(callable.clone()));
        }
        let callable = Arc::new(Closure {
            definition_source: None,
            origin: Some(key.declaration),
            instance_arguments: Some(key.arguments.clone()),
            parameters: definition.parameters.clone(),
            environment: Environment::new(),
            capture_order: vec![],
            body: Arc::new(definition.body.clone()),
            ty: definition.callable_type(),
        });
        self.instances.insert(cache_key, callable.clone());
        Ok(Value::Closure(callable))
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
        let selected = graph
            .dispatch
            .iter()
            .find(|d| {
                d.interface == method.interface
                    && d.interface_arguments == method.interface_arguments
                    && d.subject == *method.subject
            })
            .and_then(|d| {
                d.methods.iter().find(|m| {
                    m.member == method.member && m.method_arguments == method.method_arguments
                })
            })
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
        let graph = self
            .graph
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
        let value = if graph.template_type(&key).is_some() {
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
        if !self.has_type(&value, output) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(value)
    }
}
