//! Invocation admission, owned frame state, delivery and caller restoration.
//! These boundaries are shared by the migration interpreter and linear bodies.
use super::*;
mod contracts;
pub(super) mod definition_host;
pub(super) mod host;
pub(super) mod method_host;
mod session;
use session::Session;

pub(super) enum Body<'a> {
    Package(&'a Expr),
    Owned(Arc<Expr>),
}
impl Body<'_> {
    pub(super) fn expression(&self) -> &Expr {
        match self {
            Self::Package(body) => body,
            Self::Owned(body) => body,
        }
    }
}
impl fmt::Display for Body<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Package(body) => write!(f, "(package-body {body})"),
            Self::Owned(body) => write!(f, "(owned-body {body})"),
        }
    }
}

struct InvocationFrame<'a> {
    function: Value,
    environment: Environment,
    source: SourceRange,
    result_type: Term,
    type_scope: Option<nir::FunctionInputBinding>,
    origin: Option<DeclarationId>,
    progress: contracts::Progress<'a>,
    condition: Option<Arc<Expr>>,
}

impl InvocationFrame<'_> {
    /// Only the active contract or the admitted body can supply execution code.
    /// Entry and exit states do not retain a fallback body expression.
    fn parts(&mut self) -> (&Expr, Option<DeclarationId>, &mut Environment) {
        let (expression, identity) = match self.condition.as_deref() {
            Some(condition) => (condition, None),
            None => {
                let (body, identity) = self
                    .progress
                    .body()
                    .expect("invocation step must prepare its execution code");
                (body.expression(), identity)
            }
        };
        (expression, identity, &mut self.environment)
    }
}

// The monotonic effect identity allocator and pending transfers are intentionally
// not restored. They belong to the engine, not the suspended caller activation.
struct SavedInvocation {
    counts_native_depth: bool,
    effect_invocation: u64,
    effect_resume: Option<u64>,
    effect_environment: Vec<effects::Entry>,
    cleanup_frame: Option<iir::InstanceKey>,
    prepared_frames: Vec<cleanup::PreparedFrame>,
    residuals: Vec<(SourceRange, memory::Storage)>,
    pattern_orders: BTreeMap<(u32, u32), Vec<LocalId>>,
    cleanup_context: Vec<iir::cleanup::ContextKey>,
}

struct Activation<'a> {
    frame: InvocationFrame<'a>,
    saved: SavedInvocation,
}

enum Execution<'a> {
    Associated {
        activation: associated_execution::Activation<'a>,
        environment: Environment,
    },
    Contract {
        expression: Arc<Expr>,
        environment: Environment,
    },
    Invocation(Box<Activation<'a>>),
    Default {
        expression: Arc<Expr>,
        environment: Environment,
    },
}

impl Execution<'_> {
    fn parts(&mut self) -> (&Expr, Option<DeclarationId>, &mut Environment) {
        match self {
            Self::Associated {
                activation,
                environment,
                ..
            } => (activation.expression.expression(), None, environment),
            Self::Invocation(activation) => activation.frame.parts(),
            Self::Contract {
                expression,
                environment,
            }
            | Self::Default {
                expression,
                environment,
            } => (expression, None, environment),
        }
    }
}

enum Completion<'a> {
    Continue(Box<Activation<'a>>),
    Ordinary {
        flow: Result<Flow, Error>,
        defaults: Option<Environment>,
    },
    Input(bytecode::InputResult),
}

enum StackContext<'a> {
    Execution(Execution<'a>),
    Input(bytecode::InputFinish),
}

struct PlanActivation<'a> {
    frame: Box<dyn input_collection::InputPlan + 'a>,
    environment: Environment,
}
impl<'a> PlanActivation<'a> {
    fn complete(mut self, flow: Result<Flow, Error>) -> Completion<'a> {
        self.frame.cleanup(&mut self.environment);
        Completion::Ordinary {
            flow,
            defaults: Some(self.environment),
        }
    }
}
enum StackEntry<'a> {
    Code(StackContext<'a>, bytecode::Frame),
    Plan(Box<PlanActivation<'a>>),
}
impl StackEntry<'_> {
    fn code(&mut self) -> Option<&mut bytecode::Frame> {
        match self {
            Self::Code(_, code) => Some(code),
            Self::Plan(_) => None,
        }
    }
}

impl<'a> Interpreter<'a> {
    #[inline(never)]
    pub(super) fn run_associated_preparation(
        &mut self,
        prepared: associated_execution::Preparation<'a>,
    ) -> Result<Flow, Error> {
        Session::associated(self, prepared)
            .run()?
            .ok_or_else(|| self.failure(ErrorKind::InvalidControlFlow, None))
    }

    pub(super) fn invoke_value_scoped(
        &mut self,
        function: Value,
        arguments: Vec<Value>,
        context: Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
    ) -> Result<Value, Error> {
        let mut activation = self.begin_invocation(function, arguments, context)?;
        if !self.bytecode_invocation_available() {
            // Tree execution must not retain the large linear VM dispatcher
            // frame at every native recursive call. Admission, completion and
            // restoration still use the shared invocation boundaries.
            let (expression, _, environment) = activation.frame.parts();
            let result = self.eval(expression, environment);
            return self.complete_invocation(activation, result);
        }
        match Session::invocation(self, activation).run()? {
            Some(Flow::Value(value)) => Ok(value),
            _ => Err(self.failure(ErrorKind::InvalidControlFlow, None)),
        }
    }

    #[cfg(test)]
    pub(super) fn run_fragment(&mut self, frame: &mut bytecode::Frame) -> Result<Flow, Error> {
        Session::fragment(self, frame)
            .run()?
            .ok_or_else(|| self.failure(ErrorKind::InvalidControlFlow, None))
    }

    pub(super) fn run_normal_plan(
        &mut self,
        frame: &mut normal_plan::Frame<'_>,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.run_type_plan(frame, environment)
    }

    pub(super) fn run_type_plan(
        &mut self,
        frame: &mut dyn input_collection::InputPlan,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        if !self.bytecode_enabled || !self.comptime_only {
            // In a fixed tree mode the common scheduler completes its own
            // inputs. Do not retain the large bytecode driver frame at each
            // recursively constructed type. Dynamically disabled bytecode still
            // uses the resumable driver below.
            return match frame.next_input(self, environment)? {
                input_collection::Progress::Complete(flow) => Ok(flow),
                _ => Err(self.failure(ErrorKind::InvalidControlFlow, None)),
            };
        }
        Session::plan(self, frame, environment)
            .run()?
            .ok_or_else(|| self.failure(ErrorKind::InvalidControlFlow, None))
    }
    fn start_type_plan(
        &mut self,
        expression: Arc<Expr>,
        environment: &Environment,
    ) -> Result<Box<dyn input_collection::InputPlan + 'a>, Error> {
        match &expression.kind {
            K::NormalFormType { .. } => Ok(Box::new(normal_plan::Frame::owned(
                self,
                expression,
                environment,
            )?)),
            K::NominalType { .. } | K::CheckTypeAlias { .. } => {
                Ok(Box::new(nominal_plan::Frame::new(
                    self,
                    nominal_plan::Expression::Owned(expression),
                    environment,
                )?))
            }
            _ => Err(self.failure(ErrorKind::Type, Some(expression.source))),
        }
    }

    /// None means all inputs are ready for declaration admission. A flow is an
    /// early transfer, never a fabricated value standing in for the input list.
    #[cfg(test)]
    pub(super) fn run_nominal_inputs(
        &mut self,
        frame: &mut nominal_inputs::Frame<'_>,
        environment: &mut Environment,
    ) -> Result<Option<Flow>, Error> {
        Session::plan(self, frame, environment).run()
    }

    fn complete_stack(
        &mut self,
        context: StackContext<'a>,
        code: bytecode::Frame,
        flow: Result<Flow, Error>,
    ) -> Completion<'a> {
        match context {
            StackContext::Execution(mut execution) => {
                *execution.parts().2 = code.restore_environment();
                self.complete_execution(execution, flow)
            }
            StackContext::Input(finish) => Completion::Input(finish.complete(self, code, flow)),
        }
    }

    fn complete_execution(
        &mut self,
        execution: Execution<'a>,
        flow: Result<Flow, Error>,
    ) -> Completion<'a> {
        match execution {
            Execution::Associated { activation, .. } => Completion::Ordinary {
                flow: activation.complete(self, flow),
                defaults: None,
            },
            Execution::Invocation(mut activation) => {
                match flow.and_then(|flow| self.accept_invocation_step(&mut activation.frame, flow))
                {
                    Ok(None) => Completion::Continue(activation),
                    result => {
                        self.leave_invocation(activation.saved);
                        Completion::Ordinary {
                            flow: result
                                .map(|value| Flow::Value(value.expect("completed invocation"))),
                            defaults: None,
                        }
                    }
                }
            }
            Execution::Default { environment, .. } | Execution::Contract { environment, .. } => {
                Completion::Ordinary {
                    flow,
                    defaults: Some(environment),
                }
            }
        }
    }

    fn begin_invocation(
        &mut self,
        function: Value,
        arguments: Vec<Value>,
        context: Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
    ) -> Result<Box<Activation<'a>>, Error> {
        let function = self.resolve_invocation_identity(function)?;
        let origin = self.admit_invocation(&function, context.as_ref())?;
        let saved = self.enter_invocation(&function, origin, context)?;
        let prepared = self
            .prepare_invocation(function, arguments, origin)
            .and_then(|mut frame| {
                self.prepare_invocation_step(&mut frame)?;
                Ok(frame)
            });
        match prepared {
            Ok(frame) => Ok(Box::new(Activation { frame, saved })),
            Err(error) => {
                self.leave_invocation(saved);
                Err(error)
            }
        }
    }

    /// A transported name has no executable environment. Resolve it against
    /// this VM's authority and specialize through the ordinary formation path.
    pub(super) fn resolve_invocation_identity(&mut self, function: Value) -> Result<Value, Error> {
        let Value::FunctionInstance {
            declaration,
            arguments,
        } = function
        else {
            return Ok(function);
        };
        // Runtime identities and retained IIR evidence are transport-only.
        // A signature, including a forged comptime qualifier, is not authority.
        let definition = self
            .functions
            .get(&declaration)
            .filter(|definition| definition.phase == nir::program::FunctionPhase::Comptime)
            .ok_or_else(|| self.failure(ErrorKind::PhaseMismatch, None))?;
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let signature = self.normal_callable_signature(declaration, &arguments, &[], &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        let signature = signature.ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        let expression = Expr {
            source: definition.body.source,
            ty: signature,
            kind: K::FunctionInstance {
                declaration,
                arguments,
            },
        };
        match self.eval_function_instance(&expression, &mut Environment::new())? {
            Flow::Value(Value::Closure(closure)) => Ok(Value::Closure(closure)),
            _ => Err(self.failure(ErrorKind::PhaseMismatch, Some(expression.source))),
        }
    }

    fn complete_invocation(
        &mut self,
        mut activation: Box<Activation<'a>>,
        result: Result<Flow, Error>,
    ) -> Result<Value, Error> {
        let mut result = result;
        let completed = loop {
            match result.and_then(|flow| self.accept_invocation_step(&mut activation.frame, flow)) {
                Ok(None) => {
                    let (expression, _, environment) = activation.frame.parts();
                    result = self.eval(expression, environment);
                }
                outcome => break outcome.map(|value| value.expect("completed invocation")),
            }
        };
        self.leave_invocation(activation.saved);
        completed
    }

    fn admit_invocation(
        &mut self,
        function: &Value,
        context: Option<&(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
    ) -> Result<Option<DeclarationId>, Error> {
        if matches!(
            function,
            Value::FunctionInstance { .. } | Value::RuntimeTraitMethod(_)
        ) {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        let origin = match &function {
            Value::Function(id) => Some(*id),
            Value::Closure(closure) => closure.origin,
            _ => None,
        };
        if origin.is_some_and(|id| self.normal_forms.contains_key(&id)) {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        if self.comptime_only {
            let declared_comptime = |id| {
                self.invocation_definition(id, &function)
                    .is_some_and(|f| f.phase == nir::program::FunctionPhase::Comptime)
            };
            let authorized = match &function {
                Value::Function(id) => declared_comptime(*id),
                Value::Closure(closure) => {
                    closure.ty.is_comptime_callable()
                        && closure.origin.is_none_or(declared_comptime)
                }
                _ => false,
            };
            if !authorized {
                return Err(self.failure(ErrorKind::PhaseMismatch, None));
            }
        }
        if self.cleanup_schedule.is_some() && context.is_none() {
            let named_instance = match &function {
                Value::Function(declaration) => Some((*declaration, &[][..])),
                Value::Closure(closure) => closure.origin.map(|declaration| {
                    (
                        declaration,
                        closure.instance_arguments.as_deref().unwrap_or(&[]),
                    )
                }),
                _ => None,
            };
            if let Some((declaration, arguments)) = named_instance {
                for key in &self.cleanup_schedule.unwrap().borrowed_callable_functions {
                    self.tick(None)?;
                    if key.declaration == declaration && key.arguments.as_ref() == arguments {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                    }
                }
            }
            if let Some(declaration) = origin {
                let definition = self
                    .invocation_definition(declaration, &function)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
                if matches!(definition.signature.callable_signature(), Term::Function {parameters,..} if parameters.iter().any(|p|matches!(p.ty.callable_signature(),Term::Function {..})))
                {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                }
            }
        }
        Ok(origin)
    }

    fn enter_invocation(
        &mut self,
        function: &Value,
        origin: Option<DeclarationId>,
        context: Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
    ) -> Result<SavedInvocation, Error> {
        self.tick(None)?;
        if self.invocation_depth >= self.invocation_depth_limit {
            return Err(self.failure(ErrorKind::DepthExhausted, None));
        }
        self.effect_charge(self.effect_state.environment.len(), None)?;
        let next_effect_identity = self
            .effect_state
            .next_identity
            .checked_add(1)
            .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, None))?;
        let previous_effect_invocation = self.effect_state.invocation;
        let previous_effect_resume = self.effect_state.resume.take();
        let previous_effect_environment = self.effect_state.environment.clone();
        self.effect_state.next_identity = next_effect_identity;
        self.effect_state.invocation = next_effect_identity;
        if self.depth == 0 && self.invocation_depth == 0 {
            self.cleanup_failed = false;
            self.effect_state.transfer = None;
        }
        self.depth += 1;
        self.invocation_depth += 1;
        let previous_cleanup_context = std::mem::take(&mut self.cleanup_context);
        let previous_residuals = std::mem::take(&mut self.residual_temporaries);
        let previous_pattern_orders = std::mem::take(&mut self.pattern_binding_orders);
        let previous_prepared_frames = std::mem::take(&mut self.prepared_frames);
        let previous_cleanup_frame = self.cleanup_frame.take();
        self.cleanup_frame = if self.cleanup_schedule.is_some() {
            origin.map(|declaration| iir::InstanceKey {
                declaration,
                arguments: (match &function {
                    Value::Closure(closure) => {
                        closure.instance_arguments.clone().unwrap_or_default()
                    }
                    _ => Arc::from([]),
                })
                .into(),
            })
        } else {
            None
        };
        if let Some((instance, context)) = context {
            self.cleanup_frame = Some(instance);
            self.cleanup_context = context;
        }
        Ok(SavedInvocation {
            counts_native_depth: true,
            effect_invocation: previous_effect_invocation,
            effect_resume: previous_effect_resume,
            effect_environment: previous_effect_environment,
            cleanup_frame: previous_cleanup_frame,
            prepared_frames: previous_prepared_frames,
            residuals: previous_residuals,
            pattern_orders: previous_pattern_orders,
            cleanup_context: previous_cleanup_context,
        })
    }

    fn leave_invocation(&mut self, saved: SavedInvocation) {
        self.effect_state.invocation = saved.effect_invocation;
        self.effect_state.resume = saved.effect_resume;
        self.effect_state.environment = saved.effect_environment;
        self.cleanup_frame = saved.cleanup_frame;
        self.prepared_frames = saved.prepared_frames;
        self.residual_temporaries = saved.residuals;
        self.pattern_binding_orders = saved.pattern_orders;
        self.cleanup_context = saved.cleanup_context;
        self.invocation_depth -= 1;
        if saved.counts_native_depth {
            self.depth -= 1;
        }
    }

    /// Entry contracts have succeeded. Complete executable body preparation
    /// once, without replacing the activation or its argument storage.
    fn prepare_invocation_body(
        &mut self,
        frame: &InvocationFrame<'a>,
    ) -> Result<(Body<'a>, Option<DeclarationId>), Error> {
        let mut body = match &frame.function {
            Value::Function(id) => Body::Package(
                &self
                    .functions
                    .get(id)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(frame.source)))?
                    .body,
            ),
            Value::Closure(closure) => Body::Owned(closure.body.clone()),
            _ => return Err(self.failure(ErrorKind::Type, Some(frame.source))),
        };
        let independent = if let Some(scope) = &frame.type_scope {
            let (prepared, independent) = match &body {
                Body::Package(body) => {
                    let declaration = match &frame.function {
                        Value::Function(id) => Some(*id),
                        _ => None,
                    };
                    self.prepare_input_body(body, scope, declaration)?
                }
                Body::Owned(body) => self.prepare_owned_input_body(body, scope)?,
            };
            body = Body::Owned(prepared);
            independent
        } else {
            true
        };
        let stable_code = match &frame.function {
            Value::Function(id) if independent => Some(*id),
            _ => None,
        };
        Ok((body, stable_code))
    }

    fn prepare_invocation(
        &mut self,
        function: Value,
        arguments: Vec<Value>,
        origin: Option<DeclarationId>,
    ) -> Result<InvocationFrame<'a>, Error> {
        let (parameters, mut environment, source, signature) = match &function {
            Value::Function(id) => {
                let definition = self
                    .functions
                    .get(id)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
                if !definition.type_parameters.is_empty() {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                // A residual runtime body can retain closed formation clauses.
                // Prove them against this VM's dispatch world rather than
                // rejecting every constrained direct function or erasing them.
                if !self.comptime_only
                    && (!definition.requirements.is_empty() || !definition.capabilities.is_empty())
                {
                    self.admit_callable_identity(*id, &[], definition.body.source)?;
                }
                (
                    definition.parameters.as_slice(),
                    Environment::with_capacity(definition.parameters.len()),
                    definition.body.source,
                    &definition.signature,
                )
            }
            Value::Closure(closure) => {
                let signature = match &closure.ty {
                    Term::Closure { signature, .. } => signature.as_ref(),
                    ty => ty,
                };
                (
                    closure.parameters.as_slice(),
                    self.clone_invocation_environment(&closure.environment, closure.body.source)?,
                    closure.body.source,
                    signature,
                )
            }
            _ => return Err(self.failure(ErrorKind::Type, None)),
        };
        let Term::Function {
            parameters: types,
            result: result_type,
        } = signature.callable_signature()
        else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        let type_scope = self.bind_invocation_inputs(
            parameters,
            arguments,
            signature,
            &mut environment,
            source,
        )?;
        if self.comptime_only || matches!(function, Value::Function(_)) {
            if let Some(id) = origin {
                let definition = self
                    .invocation_definition(id, &function)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
                let generic_arguments = match &function {
                    _ if self.graph.is_some() => &[][..],
                    Value::Closure(closure) => closure.instance_arguments.as_deref().unwrap_or(&[]),
                    _ => &[],
                };
                for requirement in &definition.requirements {
                    let requirement = self.specialize_function_predicate(
                        definition,
                        generic_arguments,
                        requirement,
                        source,
                    )?;
                    let requirement = self
                        .bind_invocation_contract(&requirement, type_scope.as_ref(), source, 0)?
                        .ok_or_else(|| {
                            self.failure(ErrorKind::TypeContractUnresolved, Some(source))
                        })?;
                    self.check_type_contract(&requirement, source)?;
                }
                self.check_invocation_capabilities(
                    definition,
                    generic_arguments,
                    type_scope.as_ref(),
                    source,
                )?;
            }
        }
        let result_type = match &type_scope {
            Some(scope) => scope.rebind(result_type),
            None => result_type.substitute_partial(&vec![None; types.len()]),
        }
        .map_err(|_| self.failure(ErrorKind::Type, None))?;
        result_type
            .check_scope(&[])
            .map_err(|_| self.failure(ErrorKind::Type, None))?;
        let result_type = self.selected_type_view(&result_type, Some(source))?;
        self.check_comptime_domain(&result_type, source)?;
        if let Term::Computation { effects, .. } = &result_type {
            self.check_effect_environment(effects, None)?;
        } else {
            self.effect_state.environment.clear();
        }
        let origin = match &function {
            Value::Function(id) => Some(*id),
            Value::Closure(closure) => closure.origin,
            _ => None,
        };
        if let Some(id) = origin {
            let definition = self
                .invocation_definition(id, &function)
                .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
            for snapshot in &definition.entry_snapshots {
                let value = self.local_value(&environment, snapshot.parameter, None)?;
                environment.insert(snapshot.binding, value);
            }
        }
        // Entry has no prepared execution body. The callable still retains its
        // source; Entry -> Body acquires execution code only after success.
        Ok(InvocationFrame {
            function,
            environment,
            source,
            result_type,
            type_scope,
            origin,
            progress: contracts::Progress::Entry(0),
            condition: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_calls_unwind_nested_activations_and_preserve_the_root_environment() {
        use nir::program::Place;
        let source = SourceRange { start: 1, end: 2 };
        let local = |index| LocalId { owner: 83, index };
        let boolean = Term::Primitive(Primitive::Bool);
        let signature = Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(boolean.clone()),
        }));
        let expr = |ty, kind| Expr { ty, source, kind };
        let call = || {
            expr(
                boolean.clone(),
                K::Call {
                    callee: Box::new(expr(signature.clone(), K::Local(local(0)))),
                    arguments: vec![],
                    parameter_bindings: vec![],
                },
            )
        };
        let closure = |body, captured: Option<Value>| {
            let capture_order = captured
                .as_ref()
                .map(|_| vec![local(0)])
                .unwrap_or_default();
            Value::Closure(Arc::new(Closure {
                definition_source: None,
                origin: None,
                instance_arguments: None,
                parameters: vec![],
                environment: captured
                    .map(|value| Environment::from([(local(0), value)]))
                    .unwrap_or_default(),
                capture_order,
                body: Arc::new(body),
                ty: signature.clone(),
            }))
        };
        let body = expr(
            boolean.clone(),
            K::Block {
                statements: vec![expr(
                    Term::Primitive(Primitive::Void),
                    K::Assign {
                        place: Place {
                            root: local(1),
                            fields: vec![],
                        },
                        value: Box::new(expr(boolean.clone(), K::Literal(Literal::Bool(true)))),
                    },
                )],
                result: Some(Box::new(call())),
            },
        );
        let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.effect_state.invocation = 71;
        vm.effect_state.resume = Some(19);
        vm.pattern_binding_orders.insert((1, 2), vec![local(1)]);
        for fail in [true, false, true, false] {
            let leaf = expr(
                boolean.clone(),
                if fail {
                    K::Unit
                } else {
                    K::Literal(Literal::Bool(true))
                },
            );
            let mut value = closure(leaf, None);
            for _ in 0..8 {
                value = closure(call(), Some(value));
            }
            let mut environment =
                Environment::from([(local(0), value), (local(1), Value::Bool(false))]);
            let mut frame = vm.start_frame(&code, &mut environment).unwrap();
            let before = vm.bytecode_invocations();
            let result = vm.run_fragment(&mut frame);
            environment = frame.restore_environment();
            if fail {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            } else {
                assert!(matches!(result.unwrap(), Flow::Value(Value::Bool(true))));
            }
            assert_eq!(vm.bytecode_invocations() - before, 9);
            assert_eq!(environment.get(&local(1)), Some(&Value::Bool(true)));
            assert_eq!(environment.len(), 2);
            assert_eq!(vm.depth, 0);
            assert_eq!(vm.invocation_depth, 0);
            assert_eq!(vm.effect_state.invocation, 71);
            assert_eq!(vm.effect_state.resume, Some(19));
            assert_eq!(
                vm.pattern_binding_orders.get(&(1, 2)),
                Some(&vec![local(1)])
            );
            assert!(vm.prepared_frames.is_empty());
        }
    }

    #[test]
    fn nested_activations_restore_callers_without_reusing_effect_identities() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let id = DeclarationId {
            package: 1,
            local: 0,
        };
        let key = iir::InstanceKey {
            declaration: id,
            arguments: vec![].into(),
        };
        let local = LocalId { owner: 1, index: 0 };
        vm.depth = 1;
        vm.effect_state.invocation = 7;
        vm.effect_state.next_identity = 10;
        vm.effect_state.resume = Some(3);
        vm.cleanup_frame = Some(key.clone());
        vm.pattern_binding_orders.insert((1, 2), vec![local]);
        vm.prepared_frames.push(cleanup::PreparedFrame {
            source: SourceRange { start: 1, end: 2 },
            values: vec![],
        });
        let callable = Value::Function(id);
        let caller = vm.enter_invocation(&callable, Some(id), None).unwrap();
        assert_eq!(vm.depth, 2);
        assert_eq!(vm.effect_state.invocation, 11);
        assert_eq!(vm.effect_state.resume, None);
        assert!(vm.pattern_binding_orders.is_empty());
        assert!(vm.prepared_frames.is_empty());
        assert!(vm.cleanup_frame.is_none());
        vm.effect_state.resume = Some(4);
        vm.pattern_binding_orders.insert((3, 4), vec![local]);
        let nested = vm.enter_invocation(&callable, Some(id), None).unwrap();
        assert_eq!(vm.effect_state.invocation, 12);
        vm.leave_invocation(nested);
        assert_eq!(vm.depth, 2);
        assert_eq!(vm.effect_state.invocation, 11);
        assert_eq!(vm.effect_state.resume, Some(4));
        assert_eq!(vm.pattern_binding_orders.get(&(3, 4)), Some(&vec![local]));
        vm.leave_invocation(caller);
        assert_eq!(vm.depth, 1);
        assert_eq!(vm.effect_state.invocation, 7);
        assert_eq!(vm.effect_state.resume, Some(3));
        assert_eq!(vm.effect_state.next_identity, 12);
        assert_eq!(vm.cleanup_frame, Some(key));
        assert_eq!(vm.prepared_frames.len(), 1);
        assert_eq!(vm.pattern_binding_orders.get(&(1, 2)), Some(&vec![local]));
        assert!(!vm.pattern_binding_orders.contains_key(&(3, 4)));
    }

    #[test]
    fn failed_entry_and_failed_preparation_restore_caller_state() {
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let id = DeclarationId {
            package: 1,
            local: 0,
        };
        vm.effect_state.resume = Some(9);
        vm.effect_state.invocation = 5;
        vm.effect_state.next_identity = u64::MAX;
        let error = vm
            .invoke_value_scoped(Value::Function(id), vec![], None)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.effect_state.resume, Some(9));
        assert_eq!(vm.effect_state.invocation, 5);
        vm.effect_state.next_identity = 10;
        // Admission accepts the runtime carrier, but preparation cannot resolve
        // this declaration. The entered activation must still be unwound.
        let error = vm
            .invoke_value_scoped(Value::Function(id), vec![], None)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnknownFunction);
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.effect_state.resume, Some(9));
        assert_eq!(vm.effect_state.invocation, 5);
        assert_eq!(vm.effect_state.next_identity, 11);
    }
    #[test]
    fn prepared_closure_survives_nested_failed_delivery() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let captured = LocalId { owner: 1, index: 0 };
        let callable = |body| {
            let mut environment = Environment::new();
            environment.insert(captured, Value::Bool(true));
            Value::Closure(Arc::new(Closure {
                definition_source: None,
                origin: None,
                instance_arguments: None,
                parameters: vec![],
                environment,
                capture_order: vec![captured],
                body: Arc::new(Expr {
                    source: SourceRange { start: 1, end: 2 },
                    ty: Term::Primitive(Primitive::Bool),
                    kind: body,
                }),
                ty: Term::ComptimeFunction(Box::new(Term::Function {
                    parameters: vec![],
                    result: Box::new(Term::Primitive(Primitive::Bool)),
                })),
            }))
        };
        let parent = vm
            .begin_invocation(callable(K::Local(captured)), vec![], None)
            .unwrap();
        let parent_identity = vm.effect_state.invocation;
        let child = vm
            .begin_invocation(callable(K::Unit), vec![], None)
            .unwrap();
        // A delivery type error must unwind the child activation while leaving
        // the suspended parent's owned closure and captured environment usable.
        assert_eq!(
            vm.complete_invocation(child, Ok(Flow::Value(Value::Unit)))
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        assert_eq!(vm.depth, 1);
        assert_eq!(vm.effect_state.invocation, parent_identity);
        assert!(matches!(
            Session::invocation(&mut vm, parent).run().unwrap(),
            Some(Flow::Value(Value::Bool(true)))
        ));
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.effect_state.invocation, 0);
    }
}
