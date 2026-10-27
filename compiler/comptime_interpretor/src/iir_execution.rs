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
                Value::RuntimeFunction { .. } | Value::RuntimeTraitMethod(_)
            )
        }) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.charge_copy_work(Some(expr.source), |fuel| {
            for value in [&a, &b] {
                match value {
                    Value::RuntimeFunction { arguments, .. } => {
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
                Value::RuntimeFunction {
                    declaration: ad,
                    arguments: aa,
                },
                Value::RuntimeFunction {
                    declaration: bd,
                    arguments: ba,
                },
            ) => ad == bd && aa == ba,
            (Value::RuntimeTraitMethod(a), Value::RuntimeTraitMethod(b)) => a == b,
            (Value::RuntimeFunction { .. }, Value::RuntimeTraitMethod(_))
            | (Value::RuntimeTraitMethod(_), Value::RuntimeFunction { .. }) => false,
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
        Ok(Self {
            declaration_world: None,
            // IIR retains occurrence environments, not the source effect world.
            effect_declarations: BTreeMap::new(),
            effect_state: super::effects::State::default(),
            effect_values: None,
            runtime_function_identities: BTreeMap::new(),
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
            abstract_type_parameters: BTreeSet::new(),
            abstract_value_parameters: BTreeMap::new(),
            struct_builder: None,
            enum_builder: None,
            type_solver: None,
            pointer_copy: None,
            declaration_parameter_sorts: BTreeMap::new(),
            declaration_parameter_domains: BTreeMap::new(),
            method_parameters: HashMap::new(),
            method_contracts: HashMap::new(),
            method_origins: HashMap::new(),
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
            instances: HashMap::new(),
            functions: BTreeMap::new(),
            normal_forms: graph
                .normal_forms
                .iter()
                .map(|h| (h.declaration, h))
                .collect(),
            structs: BTreeMap::new(),
            newtypes: BTreeMap::new(),
            alias_contracts: BTreeMap::new(),
            enums: BTreeMap::new(),
            budget: Budget {
                steps: fuel as u64,
                depth: budget.depth.min(256),
            },
            depth: 0,
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
                _ => vec![],
            };
            graph
                .function(&iir::InstanceKey {
                    declaration: id,
                    arguments,
                })
                .map(|f| &f.function)
        } else {
            self.functions.get(&id).copied()
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
        if let Some(callable) = self.instances.get(&cache_key) {
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
        self.charge_copy_work(Some(expr.source), |fuel| {
            match &expr.kind {
                K::FunctionInstance { arguments, .. } => {
                    for term in arguments {
                        nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
                    }
                }
                K::TraitMethod {
                    subject,
                    interface_arguments,
                    method_arguments,
                    ..
                } => {
                    for term in std::iter::once(subject)
                        .chain(interface_arguments)
                        .chain(method_arguments)
                    {
                        nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
        let key = match &expr.kind {
            K::Function(declaration) => iir::InstanceKey {
                declaration: *declaration,
                arguments: vec![],
            },
            K::FunctionInstance {
                declaration,
                arguments,
            } => iir::InstanceKey {
                declaration: *declaration,
                arguments: arguments.clone(),
            },
            K::TraitMethod {
                method_arguments,
                interface,
                interface_arguments,
                member,
                subject,
            } => {
                let method = self
                    .graph
                    .unwrap()
                    .dispatch
                    .iter()
                    .find(|d| {
                        d.interface == *interface
                            && d.interface_arguments == *interface_arguments
                            && d.subject == *subject
                    })
                    .and_then(|d| {
                        d.methods.iter().find(|m| {
                            m.member == *member && m.method_arguments == *method_arguments
                        })
                    })
                    .ok_or_else(|| {
                        self.failure(ErrorKind::UnmaterializedDispatch, Some(expr.source))
                    })?;
                self.charge_copy_work(Some(expr.source), |fuel| {
                    for term in &method.arguments {
                        nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
                    }
                    Ok(())
                })?;
                iir::InstanceKey {
                    declaration: method.body,
                    arguments: method.arguments.clone(),
                }
            }
            _ => return Err(self.failure(ErrorKind::InvalidCallPlan, Some(expr.source))),
        };
        let value = if self.graph.unwrap().template_type(&key).is_some() {
            match &expr.kind {
                K::TraitMethod {
                    interface,
                    interface_arguments,
                    subject,
                    member,
                    method_arguments,
                } => Value::RuntimeTraitMethod(Box::new(TraitMethodIdentity {
                    interface: InterfaceInstance {
                        declaration: *interface,
                        arguments: interface_arguments.clone(),
                    },
                    subject: subject.clone(),
                    member: *member,
                    method_arguments: method_arguments.clone(),
                })),
                _ => Value::RuntimeFunction {
                    declaration: key.declaration,
                    arguments: key.arguments,
                },
            }
        } else {
            self.iir_callable(&key)?
        };
        if !self.has_type(&value, &expr.ty) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        Ok(Flow::Value(value))
    }
}
