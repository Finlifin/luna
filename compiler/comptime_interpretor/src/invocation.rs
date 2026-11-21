//! Invocation admission, owned frame state, delivery and caller restoration.
//! These boundaries are shared by the migration interpreter and linear bodies.
use super::*;

mod admitted;
mod source_prefix;
pub use source_prefix::{SourceCallPrefix, SourceOperandScope, SourceOperandStep};

pub use admitted::AdmittedCall;

mod contracts;
pub(super) mod definition_host;
pub(super) mod host;
pub(super) mod method_host;
mod session;
mod target;

use session::Session;
use target::Target;

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
    target: Target<'a>,
    environment: Environment,
    source: SourceRange,
    result_type: Term,
    type_scope: Option<nir::FunctionInputBinding>,
    deferred_inputs: Vec<(usize, LocalId)>,
    progress: contracts::Progress<'a>,
    condition: Option<Arc<Expr>>,
}

impl InvocationFrame<'_> {
    /// Only the active contract or the admitted body can supply execution code.
    /// Entry and exit states do not retain a fallback body expression.
    fn parts(&mut self) -> (&Expr, Option<CodeOrigin>, &mut Environment) {
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
    fn parts(&mut self) -> (&Expr, Option<CodeOrigin>, &mut Environment) {
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

/// The stage that lent the bindings determines who may restore them and
/// whether a lexical loop transfer can resume at the suspension site.
pub(super) enum ReturnedEnvironment {
    Private,
    Declaration(Environment),
    Lexical(Environment),
}

enum Completion<'a> {
    Returned(InvocationResult),
    Continue(Box<Activation<'a>>),
    Ordinary {
        flow: Result<Flow, Error>,
        bindings: ReturnedEnvironment,
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
            bindings: ReturnedEnvironment::Lexical(self.environment),
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
        let activation = self.begin_invocation(function, arguments, context)?;
        self.run_activation(activation)
    }

    fn run_activation(&mut self, activation: Box<Activation<'a>>) -> Result<Value, Error> {
        if !self.bytecode_invocation_available() {
            // Tree execution must not retain the large linear VM dispatcher
            // frame at every native recursive call. Admission, completion and
            // restoration still use the shared invocation boundaries.
            return self.run_tree_invocation(activation);
        }
        match Session::invocation(self, activation).run()? {
            Some(Flow::Value(value)) => Ok(value),
            _ => Err(self.failure(ErrorKind::InvalidControlFlow, None)),
        }
    }

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
            loop {
                match frame.next_input(self, environment)? {
                    input_collection::Progress::DomainDelivered => continue,
                    input_collection::Progress::Complete(flow) => return Ok(flow),
                    _ => return Err(self.failure(ErrorKind::InvalidControlFlow, None)),
                }
            }
        }
        Session::plan(self, frame, environment)
            .run()?
            .ok_or_else(|| self.failure(ErrorKind::InvalidControlFlow, None))
    }

    fn start_type_plan(
        &mut self,
        plan: input_collection::Plan<'a>,
        environment: &Environment,
        selected: Option<Value>,
    ) -> Result<Box<dyn input_collection::InputPlan + 'a>, Error> {
        use input_collection::Target;
        match plan.view().target {
            Target::Normal(_) | Target::Selected { .. } => Ok(Box::new(
                normal_plan::Frame::application(self, plan, environment, selected)?,
            )),
            Target::Callable(_) | Target::Nominal(_) | Target::Alias { .. } => Ok(Box::new(
                declaration_plan::Frame::new(self, plan, environment)?,
            )),
        }
    }

    /// None means all inputs are ready for declaration admission. A flow is an
    /// early transfer, never a fabricated value standing in for the input list.
    #[cfg(test)]
    pub(super) fn run_declaration_inputs(
        &mut self,
        frame: &mut declaration_inputs::Frame<'_>,
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
                // Only a continuation that observes locals needs them moved
                // back out of the bytecode registers. Returned values and
                // shared receiver cells already own their storage.
                let destination = match &mut execution {
                    Execution::Default { environment, .. }
                    | Execution::Contract { environment, .. } => Some(environment),
                    Execution::Associated { .. } => None,
                    Execution::Invocation(active) => {
                        let frame = &mut active.frame;
                        let body_finishes_here =
                            matches!(frame.progress, contracts::Progress::Body { .. })
                                && self.cleanup_schedule.is_none()
                                && frame
                                    .target
                                    .declaration()
                                    .is_none_or(|f| f.postconditions.is_empty());
                        (flow.is_ok() && !body_finishes_here).then_some(&mut frame.environment)
                    }
                };
                if let Some(environment) = destination {
                    *environment = code.restore_environment();
                } else {
                    drop(code);
                }
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
                bindings: ReturnedEnvironment::Private,
            },
            Execution::Invocation(mut activation) => {
                match flow.and_then(|flow| self.accept_invocation_step(&mut activation.frame, flow))
                {
                    Ok(None) => Completion::Continue(activation),
                    Ok(Some(value)) => {
                        self.leave_invocation(activation.saved);
                        Completion::Returned(InvocationResult {
                            value,
                            result_type: activation.frame.result_type,
                        })
                    }
                    Err(error) => {
                        self.leave_invocation(activation.saved);
                        Completion::Ordinary {
                            flow: Err(error),
                            bindings: ReturnedEnvironment::Private,
                        }
                    }
                }
            }
            Execution::Default { environment, .. } | Execution::Contract { environment, .. } => {
                Completion::Ordinary {
                    flow,
                    bindings: ReturnedEnvironment::Declaration(environment),
                }
            }
        }
    }

    /// Invoke explicit declaration inputs through the same specialization and
    /// contract path used by source FunctionInstance expressions. No binder or
    /// obligation is erased to make a template look monomorphic.
    pub fn invoke_instantiated(
        &mut self,
        function: DeclarationId,
        types: Vec<Term>,
        arguments: Vec<Value>,
    ) -> Result<Value, Error> {
        if self.graph.is_some() {
            return self.invoke_instance(
                &iir::InstanceKey {
                    declaration: function,
                    arguments: (types).into(),
                },
                arguments,
            );
        }
        let activation = self.begin_named_invocation(function, types, arguments, None)?;
        self.run_activation(activation)
    }

    fn begin_invocation(
        &mut self,
        function: Value,
        arguments: Vec<Value>,
        context: Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
    ) -> Result<Box<Activation<'a>>, Error> {
        let target = self.invocation_target(function)?;
        self.begin_target_invocation(target, arguments, context, None)
    }

    fn begin_named_invocation(
        &mut self,
        declaration: DeclarationId,
        inputs: Vec<Term>,
        arguments: Vec<Value>,
        domains: Option<&mut Vec<Term>>,
    ) -> Result<Box<Activation<'a>>, Error> {
        let target = self.named_entry_target(declaration, inputs)?;
        self.begin_target_invocation(target, arguments, None, domains)
    }

    fn begin_target_invocation(
        &mut self,
        target: Target<'a>,
        arguments: Vec<Value>,
        context: Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
        domains: Option<&mut Vec<Term>>,
    ) -> Result<Box<Activation<'a>>, Error> {
        self.check_invocation_cleanup(&target, context.is_some())?;
        let cleanup_frame = if self.cleanup_schedule.is_some() && context.is_none() {
            target.cleanup_key()
        } else {
            None
        };
        let saved = self.enter_invocation(cleanup_frame, context)?;
        let prepared = self
            .prepare_invocation(target, arguments, domains)
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
    pub(super) fn check_transported_instance(&mut self, function: &Value) -> Result<(), Error> {
        if let Value::FunctionInstance {
            declaration,
            arguments,
        } = function
        {
            let definition = self
                .functions
                .declaration(declaration)
                .ok_or_else(|| self.failure(ErrorKind::PhaseMismatch, None))?;
            self.check_instance_phase(definition)?;
            self.check_source_formation(definition.into(), arguments, definition.source)?;
        }
        Ok(())
    }

    fn run_tree_invocation(&mut self, mut activation: Box<Activation<'a>>) -> Result<Value, Error> {
        let completed = (|| loop {
            if matches!(activation.frame.progress, contracts::Progress::NeedBody) {
                self.install_invocation_body(&mut activation.frame)?;
            }
            let (expression, _, environment) = activation.frame.parts();
            let flow = self.eval(expression, environment)?;
            if let Some(value) = self.accept_invocation_step(&mut activation.frame, flow)? {
                return Ok(value);
            }
        })();
        self.leave_invocation(activation.saved);
        completed
    }

    fn enter_invocation(
        &mut self,
        cleanup_frame: Option<iir::InstanceKey>,
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
        self.cleanup_frame = cleanup_frame;
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
    ) -> Result<(Body<'a>, Option<CodeOrigin>), Error> {
        let (body, origin) = match &frame.target {
            Target::Named(f) => (
                Body::Package(&f.body),
                Some(CodeOrigin::Declaration(f.declaration)),
            ),
            Target::Instance {
                function,
                substitution,
                ..
            } => {
                let body =
                    self.prepare_function_instance_body(function, substitution, frame.source)?;
                (Body::Owned(body), None)
            }
            Target::Graph(f, handle) => (
                Body::Package(&f.body),
                Some(CodeOrigin::GraphInstance(handle.position)),
            ),
            Target::Closure(closure) => (Body::Owned(closure.body.clone()), None),
        };
        let Some(scope) = &frame.type_scope else {
            return Ok((body, origin));
        };
        let key = match &body {
            Body::Package(_) => input_bodies::SourceKey::Named(origin.expect("named body origin")),
            Body::Owned(body) => input_bodies::SourceKey::Owned(body),
        };
        let (prepared, independent) = self.prepare_input_body_key(
            body.expression(),
            scope,
            key,
            &frame.deferred_inputs,
            &frame.environment,
        )?;
        Ok((Body::Owned(prepared), origin.filter(|_| independent)))
    }

    // A named definition lends the same immutable tree for the VM's lifetime.
    // Only its structural cost is reusable. With insufficient fuel the walker
    // must still report how far it got, including any unspent allowance.
    fn charge_named_signature(
        &mut self,
        signature: &'a Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if let Some((previous, cost)) = self.last_named_signature_work
            && std::ptr::eq(signature, previous)
            && self.budget.steps >= cost
        {
            self.budget.steps -= cost;
            return Ok(());
        }
        let before = self.budget.steps;
        self.charge_copy_work(Some(source), |fuel| {
            nir::copy_budget::charge_term_copy(signature, fuel, 128)
        })?;
        self.last_named_signature_work = Some((signature, before - self.budget.steps));
        Ok(())
    }

    fn prepare_invocation(
        &mut self,
        target: Target<'a>,
        arguments: Vec<Value>,
        domains: Option<&mut Vec<Term>>,
    ) -> Result<InvocationFrame<'a>, Error> {
        let declaration = target.declaration();
        let (parameters, mut environment, source, signature) = match &target {
            Target::Named(definition) => {
                if !definition.type_parameters.is_empty() {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                (
                    definition.parameters.as_slice(),
                    Environment::with_capacity(definition.parameters.len()),
                    definition.source,
                    &definition.signature,
                )
            }
            Target::Instance {
                function,
                signature,
                ..
            } => (
                function.parameters.as_slice(),
                Environment::with_capacity(function.parameters.len()),
                function.source,
                signature,
            ),
            Target::Graph(target, handle) => (
                target.parameters.as_slice(),
                Environment::with_capacity(target.parameters.len()),
                target.body.source,
                &handle.ty,
            ),
            Target::Closure(closure) => {
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
        };
        if let Target::Named(definition) = &target {
            self.charge_named_signature(&definition.signature, source)?;
        } else {
            self.charge_copy_work(Some(source), |fuel| {
                nir::copy_budget::charge_term_copy(signature, fuel, 128)
            })?;
        }
        let pending = if let Some(definition) = declaration
            && (self.comptime_only || matches!(target, Target::Named(_) | Target::Instance { .. }))
            && (!definition.requirements.is_empty() || !definition.capabilities.is_empty())
        {
            let frames;
            let empty;
            let substitution = match target.substitution() {
                Some(inputs) => inputs,
                None => {
                    frames = [self.ordinary_input_frame(&definition.signature, source)?];
                    empty = self
                        .admit_function_inputs(
                            &definition.type_parameters,
                            &[],
                            nir::DeclarationInputScope::ClosedInBody(&frames),
                            source,
                        )?
                        .into_substitution();
                    &empty
                }
            };
            self.pending_callable_requirements(definition.into(), substitution, source)?
        } else {
            Vec::new()
        };
        let mut deferred_inputs = Vec::new();
        let entry_slots;
        let defer = if self.comptime_only
            && pending.is_empty()
            && declaration.is_none_or(|f| {
                f.preconditions.is_empty()
                    && f.postconditions.is_empty()
                    && f.result_guarantees.is_empty()
            })
            && let Term::Function { parameters, result } = signature.callable_signature()
            && parameters.iter().any(|p| p.sort != nir::Sort::Runtime)
        {
            entry_slots =
                nir::copy_budget::signature_input_slots(parameters, result, &mut self.budget.steps)
                    .map_err(|reason| {
                        self.failure(
                            match reason {
                                nir::copy_budget::CopyBudgetExhausted::Steps => {
                                    ErrorKind::StepsExhausted
                                }
                                nir::copy_budget::CopyBudgetExhausted::Depth => {
                                    ErrorKind::DepthExhausted
                                }
                            },
                            Some(source),
                        )
                    })?;
            Some((&entry_slots, &mut deferred_inputs))
        } else {
            None
        };
        let type_scope = self.bind_invocation_inputs_for_body(
            parameters,
            arguments,
            signature,
            &mut environment,
            source,
            domains,
            defer,
        )?;
        for mut predicate in pending {
            let scope = type_scope
                .as_ref()
                .ok_or_else(|| self.failure(ErrorKind::TypeContractUnresolved, Some(source)))?;
            self.bind_invocation_contract(&mut predicate, scope, source, 0)?;
            self.check_type_contract(&predicate, source)?;
        }
        let bound_signature = type_scope
            .as_ref()
            .map_or(signature, |scope| scope.signature());
        let Term::Function { result, .. } = bound_signature.callable_signature() else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        self.require_closed_input_domain(result, source)?;
        let result_type = self.selected_type_view(result, Some(source))?;
        self.check_comptime_domain(&result_type, source)?;
        if let Term::Computation { effects, .. } = &result_type {
            self.check_effect_environment(effects, None)?;
        } else {
            self.effect_state.environment.clear();
        }
        if let Some(definition) = declaration {
            for snapshot in &definition.entry_snapshots {
                let value = self.local_value(&environment, snapshot.parameter, None)?;
                environment.insert(snapshot.binding, value);
            }
        }
        // Entry has no prepared execution body. The callable still retains its
        // source; Entry -> Body acquires execution code only after success.
        Ok(InvocationFrame {
            target,
            environment,
            source,
            result_type,
            type_scope,
            deferred_inputs,
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
                    env_arguments: vec![],
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
    fn warm_named_signature_preserves_cold_budget_failures() {
        let package = Package::default();
        let signature = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Runtime,
                ty: Term::Record(vec![("field".into(), Term::Primitive(Primitive::Bool))]),
            }],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let other = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let source = SourceRange { start: 4, end: 9 };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let before = vm.remaining_steps();
        vm.charge_named_signature(&signature, source).unwrap();
        let cost = before - vm.remaining_steps();
        for allowance in 0..=cost + 1 {
            vm.budget.steps = allowance;
            let warm = vm.charge_named_signature(&signature, source);
            let remaining = vm.remaining_steps();
            let mut fuel = allowance as usize;
            let cold = nir::copy_budget::charge_term_copy(&signature, &mut fuel, 128);
            assert_eq!(warm.is_ok(), cold.is_ok());
            if let Err(error) = warm {
                assert_eq!(error.kind, ErrorKind::StepsExhausted);
                assert_eq!(error.source, Some(source));
            }
            assert_eq!(remaining, fuel as u64);
        }
        vm.budget.steps = 2;
        vm.charge_named_signature(&other, source).unwrap();
        assert_eq!(vm.remaining_steps(), 0);
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
        let caller = vm.enter_invocation(None, None).unwrap();
        assert_eq!(vm.depth, 2);
        assert_eq!(vm.effect_state.invocation, 11);
        assert_eq!(vm.effect_state.resume, None);
        assert!(vm.pattern_binding_orders.is_empty());
        assert!(vm.prepared_frames.is_empty());
        assert!(vm.cleanup_frame.is_none());
        vm.effect_state.resume = Some(4);
        vm.pattern_binding_orders.insert((3, 4), vec![local]);
        let nested = vm.enter_invocation(None, None).unwrap();
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
        let source = SourceRange { start: 1, end: 2 };
        let callable = Value::Closure(Arc::new(Closure {
            definition_source: None,
            parameters: vec![],
            environment: Environment::new(),
            capture_order: vec![],
            body: Arc::new(Expr {
                source,
                ty: Term::Primitive(Primitive::Bool),
                kind: K::Literal(Literal::Bool(true)),
            }),
            ty: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            },
        }));
        vm.effect_state.resume = Some(9);
        vm.effect_state.invocation = 5;
        vm.effect_state.next_identity = 10;
        let error = vm
            .invoke_value_scoped(Value::Function(id), vec![], None)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnknownFunction);
        assert_eq!(vm.effect_state.next_identity, 10);
        vm.effect_state.next_identity = u64::MAX;
        let error = vm
            .invoke_value_scoped(callable.clone(), vec![], None)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.effect_state.resume, Some(9));
        assert_eq!(vm.effect_state.invocation, 5);
        vm.effect_state.next_identity = 10;
        let error = vm
            .invoke_value_scoped(callable, vec![Value::Unit], None)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Arity);
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
            vm.run_tree_invocation(child).unwrap_err().kind,
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
