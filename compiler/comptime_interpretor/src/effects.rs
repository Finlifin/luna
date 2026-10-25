//! Reference execution of direct and installation-time value entries.
//! Scheduled direct handlers require independently verified resource contexts.
//! Scalar Copy captures share installer storage. Lexical returns unwind to the
//! installer before checking its result; loop exits resume only in the lexical
//! target resource scope. Other captures and affine installation-time slots
//! need additional ownership/memory proofs.
use super::*;
use nir::effects::{Abi, Operation};
use nir::program::EffectHandler;

#[derive(Clone)]
pub(super) struct Entry {
    operation: Operation,
    activation: Arc<Activation>,
}
struct Activation {
    identity: u64,
    invocation: u64,
    handler: EffectHandler,
    environment: Environment,
    outer: Vec<Entry>,
    provided: std::sync::Mutex<Option<Value>>,
    copy_provided: bool,
    installation: SourceRange,
    handler_index: usize,
    cleanup_frame: Option<iir::InstanceKey>,
    cleanup_context: Vec<iir::cleanup::ContextKey>,
}
#[derive(Default)]
pub(super) struct State {
    pub(super) environment: Vec<Entry>,
    pub(super) invocation: u64,
    pub(super) next_identity: u64,
    pub(super) resume: Option<u64>,
    pub(super) transfer: Option<(u64, Flow)>,
    // Invocation distinguishes recursion; resource context distinguishes
    // suspended loops in nested handlers sharing that invocation.
    loops: Vec<(u64, LocalId, Vec<iir::cleanup::ContextKey>)>,
}
impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(effect-entry (activation {}) {})",
            self.activation.identity, self.operation
        )
    }
}
impl Interpreter<'_> {
    pub(super) fn effect_charge(
        &mut self,
        amount: usize,
        source: Option<SourceRange>,
    ) -> Result<(), Error> {
        self.budget.steps = self
            .budget
            .steps
            .checked_sub(amount as u64)
            .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, source))?;
        Ok(())
    }
    fn effect_operation_work(&self, operation: &Operation) -> Option<usize> {
        operation.arguments.iter().try_fold(1usize, |sum, ty| {
            sum.checked_add(instance_argument_work(ty, &self.normal_forms)?)
        })
    }

    fn effect_cleanup_context_work(
        &self,
        frame: Option<&iir::InstanceKey>,
        context: &[iir::cleanup::ContextKey],
    ) -> Option<usize> {
        let mut work = context.len().checked_add(1)?;
        for key in frame
            .into_iter()
            .chain(context.iter().filter_map(|context| match context {
                iir::cleanup::ContextKey::Function { callee, .. } => Some(callee),
                _ => None,
            }))
        {
            work = work.checked_add(1)?;
            for argument in &key.arguments {
                work = work.checked_add(instance_argument_work(argument, &self.normal_forms)?)?;
            }
        }
        Some(work)
    }

    pub(super) fn check_effect_environment(
        &mut self,
        row: &[Operation],
        source: Option<SourceRange>,
    ) -> Result<(), Error> {
        self.tick(source)?;
        if row.len() != self.effect_state.environment.len() {
            return Err(self.failure(ErrorKind::InvalidEffectEnvironment, source));
        }
        let work = row
            .iter()
            .try_fold(0usize, |sum, op| {
                sum.checked_add(self.effect_operation_work(op)?)
            })
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, source))?;
        self.effect_charge(work, source)?;
        for (operation, entry) in row.iter().zip(&self.effect_state.environment) {
            if operation != &entry.operation {
                return Err(self.failure(ErrorKind::InvalidEffectEnvironment, source));
            }
        }
        Ok(())
    }
    pub(super) fn eval_effect(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        // Concrete IIR and schedule publication validate the supported control
        // and ownership fragment before any scheduled execution begins.
        if self.comptime_only {
            return Err(self.failure(ErrorKind::Unsupported, Some(expr.source)));
        }
        match &expr.kind {
            K::EffectComputation { effects, body } => {
                self.check_effect_environment(effects, Some(expr.source))?;
                self.eval(body, environment)
            }
            K::EffectRun { computation, slots } => {
                let Term::Computation { effects, .. } = &computation.ty else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                if slots.len() != effects.len() {
                    return Err(self.failure(ErrorKind::Unsupported, Some(expr.source)));
                }
                let mut selected = Vec::new();
                let mut used = BTreeSet::new();
                for (index, (slot, operation)) in slots.iter().zip(effects).enumerate() {
                    self.tick(Some(expr.source))?;
                    for (prior_operation, prior_slot) in
                        effects[..index].iter().zip(&slots[..index])
                    {
                        self.tick(Some(expr.source))?;
                        if prior_operation == operation && prior_slot >= slot {
                            return Err(self
                                .failure(ErrorKind::InvalidEffectEnvironment, Some(expr.source)));
                        }
                    }
                    let entry = self.effect_state.environment.get(*slot).ok_or_else(|| {
                        self.failure(ErrorKind::InvalidEffectEnvironment, Some(expr.source))
                    })?;
                    if !used.insert(*slot) || &entry.operation != operation {
                        return Err(
                            self.failure(ErrorKind::InvalidEffectEnvironment, Some(expr.source))
                        );
                    }
                    selected.push(entry.clone());
                }
                self.eval_mapped_computation(computation, environment, selected)
            }
            K::EffectHandle {
                computation,
                handlers,
                outer,
            } => {
                self.check_effect_environment(outer, Some(expr.source))?;
                let previous = self.effect_state.environment.clone();
                // Sharing addressable cells preserves writes to lexical captures
                // while pattern bindings remain private to each request.
                for value in environment.values_mut() {
                    self.tick(Some(expr.source))?;
                    if !matches!(value, Value::Storage(_)) {
                        let owned = std::mem::replace(value, Value::Unit);
                        *value = Value::Storage(Storage::new(owned));
                    }
                }
                let mut installed = Vec::new();
                for (handler_index, handler) in handlers.iter().enumerate() {
                    let work = self
                        .effect_operation_work(&handler.operation)
                        .and_then(|n| {
                            n.checked_add(type_value_work(&handler.signature, &self.normal_forms)?)
                        })
                        .and_then(|n| n.checked_add(environment.len()))
                        .and_then(|n| n.checked_add(previous.len()))
                        .and_then(|n| {
                            n.checked_add(self.effect_cleanup_context_work(
                                self.cleanup_frame.as_ref(),
                                &self.cleanup_context,
                            )?)
                        })
                        .ok_or_else(|| {
                            self.failure(ErrorKind::RepresentationSizeLimit, Some(expr.source))
                        })?;
                    self.effect_charge(work, Some(expr.source))?;
                    for arm in &handler.arms {
                        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                        let before = fuel;
                        let result = iir::visit::expression(&arm.body, &mut fuel, |_, _| Ok(()));
                        self.budget.steps -= (before - fuel) as u64;
                        result.map_err(|_| {
                            self.failure(ErrorKind::StepsExhausted, Some(expr.source))
                        })?;
                    }
                    self.tick(Some(expr.source))?;
                    self.effect_state.next_identity = self
                        .effect_state
                        .next_identity
                        .checked_add(1)
                        .ok_or_else(|| {
                            self.failure(ErrorKind::StepsExhausted, Some(expr.source))
                        })?;
                    let mut copy_provided = false;
                    if handler.abi == Abi::DirectPassing {
                        let Term::Function { result, .. } = &handler.signature else {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        };
                        copy_provided = if let Some(plan) = self.effect_values {
                            let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                            let before = fuel;
                            let decision = plan.delivery(&handler.operation, result, &mut fuel);
                            self.budget.steps -= (before - fuel) as u64;
                            decision.map_err(|_| {
                                self.failure(
                                    if fuel == 0 {
                                        ErrorKind::StepsExhausted
                                    } else {
                                        ErrorKind::InvalidCallPlan
                                    },
                                    Some(expr.source),
                                )
                            })? == iir::effect_values::Delivery::Copy
                        } else {
                            nir::program::intrinsic_pointer_value(result)
                                || **result == Term::Primitive(nir::Primitive::Void)
                        };
                    }
                    let activation = Arc::new(Activation {
                        identity: self.effect_state.next_identity,
                        invocation: self.effect_state.invocation,
                        handler: handler.clone(),
                        environment: environment.clone(),
                        outer: previous.clone(),
                        provided: std::sync::Mutex::new(None),
                        copy_provided,
                        installation: expr.source,
                        handler_index,
                        cleanup_frame: self.cleanup_frame.clone(),
                        cleanup_context: self.cleanup_context.clone(),
                    });
                    let entry = Entry {
                        operation: handler.operation.clone(),
                        activation,
                    };
                    if handler.abi == Abi::DirectPassing {
                        let Term::Function { parameters, result } = &handler.signature else {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        };
                        if !parameters.is_empty() {
                            return Err(self.failure(ErrorKind::Arity, Some(expr.source)));
                        }
                        let provided = match self.effect_dispatch(
                            entry.clone(),
                            Value::Tuple(vec![]),
                            expr.source,
                            true,
                        ) {
                            Ok(value) => Ok(value),
                            Err(error) => {
                                self.cleanup_effect_failure(&error, expr.source, environment)?;
                                Err(error)
                            }
                        };
                        let value = value!(provided);
                        if !self.has_type(&value, result) {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        }
                        *entry.activation.provided.lock().map_err(|_| {
                            self.failure(ErrorKind::InvalidControlFlow, Some(expr.source))
                        })? = Some(value);
                    }
                    installed.push(entry);
                }
                installed.extend(previous.iter().cloned());
                self.effect_state.environment = installed;
                let result = self.eval(computation, environment);
                self.effect_state.environment = previous;
                result
            }
            K::EffectRequest {
                operation,
                abi,
                allowed_index,
                signature,
                arguments,
            } => self.with_prepared_frame(expr.source, |this| {
                if !matches!(abi, Abi::Direct | Abi::DirectPassing) {
                    return Err(this.failure(ErrorKind::Unsupported, Some(expr.source)));
                }
                let Term::Function { parameters, result } = signature.as_ref() else {
                    return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                };
                if arguments.len() != parameters.len() {
                    return Err(this.failure(ErrorKind::Arity, Some(expr.source)));
                }
                let mut values = Vec::new();
                for (slot, (argument, parameter)) in arguments.iter().zip(parameters).enumerate() {
                    let value = value!(this.eval(argument, environment));
                    if !this.has_type(&value, &parameter.ty) {
                        return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                    }
                    this.retain_prepared(slot, argument.source, &value)?;
                    values.push(value);
                }
                let entry = this
                    .effect_state
                    .environment
                    .get(*allowed_index)
                    .cloned()
                    .ok_or_else(|| {
                        this.failure(ErrorKind::InvalidEffectEnvironment, Some(expr.source))
                    })?;
                if entry.operation != *operation || entry.activation.handler.abi != *abi {
                    return Err(
                        this.failure(ErrorKind::InvalidEffectEnvironment, Some(expr.source))
                    );
                }
                this.transfer_prepared();
                let delivered =
                    match this.effect_dispatch(entry, Value::Tuple(values), expr.source, false) {
                        Ok(delivered) => delivered,
                        Err(error) => {
                            this.cleanup_effect_failure(&error, expr.source, environment)?;
                            return Err(error);
                        }
                    };
                if let Flow::Value(value) = &delivered {
                    if !this.has_type(value, result) {
                        return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                    }
                }
                Ok(delivered)
            }),
            K::EffectResume(value) => {
                let target = self.effect_state.resume.ok_or_else(|| {
                    self.failure(ErrorKind::InvalidControlFlow, Some(expr.source))
                })?;
                let value = value!(self.eval(value, environment));
                self.cleanup_site(
                    expr.source,
                    iir::cleanup::Boundary::TemporaryEnd,
                    environment,
                )?;
                self.cleanup_site(
                    expr.source,
                    iir::cleanup::Boundary::EffectResume,
                    environment,
                )?;
                Ok(Flow::EffectResume(target, value))
            }
            _ => unreachable!(),
        }
    }
    /// Effect qualification belongs to the result computation. In particular
    /// the arguments of an effectful call remain caller-side computations.
    fn eval_mapped_computation(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
        selected: Vec<Entry>,
    ) -> Result<Flow, Error> {
        self.tick(Some(expr.source))?;
        self.depth += 1;
        let result = (|| match &expr.kind {
            K::Call { .. } => self.eval_call_with_effects(expr, environment, Some(selected)),
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let existing = environment.snapshot();
                let result = (|| {
                    for statement in statements {
                        value!(self.eval(statement, environment));
                    }
                    let result = result
                        .as_ref()
                        .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                    self.eval_mapped_computation(result, environment, selected)
                })();
                let result = match result {
                    Ok(Flow::Value(value)) => {
                        if matches!(expr.kind, K::CallPreparation { .. }) {
                            self.cleanup_site(
                                expr.source,
                                iir::cleanup::Boundary::ScopeEnd,
                                environment,
                            )?;
                        } else {
                            self.cleanup_block(expr.source, environment)?;
                        }
                        Ok(Flow::Value(value))
                    }
                    other => other,
                };
                environment.retain(|id, _| existing.contains(id));
                result
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let Value::Bool(condition) = value!(self.eval(condition, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let branch = if condition {
                    then_branch.as_ref()
                } else {
                    else_branch
                        .as_deref()
                        .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?
                };
                self.eval_mapped_computation(branch, environment, selected)
            }
            K::Match { value, arms } => {
                let value = value!(self.eval(value, environment));
                let existing = environment.snapshot();
                let result = (|| {
                    for arm in arms {
                        environment.retain(|id, _| existing.contains(id));
                        let (matched, bindings) = self.match_arm_guard_with_bindings(
                            &arm.pattern,
                            arm.guard.as_ref(),
                            &value,
                            environment,
                        )?;
                        match matched {
                            Flow::Value(Value::Bool(false)) => continue,
                            Flow::Value(Value::Bool(true)) => {
                                self.retain_pattern_residual(arm.body.source, &value, &bindings)?;
                                let result =
                                    self.eval_mapped_computation(&arm.body, environment, selected)?;
                                if matches!(result, Flow::Value(_))
                                    && !matches!(
                                        arm.body.kind,
                                        K::Block { .. } | K::CallPreparation { .. }
                                    )
                                {
                                    self.cleanup_site(
                                        arm.body.source,
                                        iir::cleanup::Boundary::ScopeEnd,
                                        environment,
                                    )?;
                                }
                                return Ok(result);
                            }
                            flow => return Ok(flow),
                        }
                    }
                    Err(self.failure(ErrorKind::NonExhaustiveMatch, Some(expr.source)))
                })();
                environment.retain(|id, _| existing.contains(id));
                result
            }
            K::EffectComputation { .. } => {
                let previous = std::mem::replace(&mut self.effect_state.environment, selected);
                let result = self.eval_effect(expr, environment);
                self.effect_state.environment = previous;
                result
            }
            _ => Err(self.failure(ErrorKind::Unsupported, Some(expr.source))),
        })();
        self.depth -= 1;
        self.finish_evaluation(expr, environment, result)
    }

    fn effect_dispatch(
        &mut self,
        mut entry: Entry,
        inputs: Value,
        source: SourceRange,
        mut supplying: bool,
    ) -> Result<Flow, Error> {
        // Partial forwarding is iterative: it must not consume the host stack
        // or remove the inner installation from the resumed subject.
        loop {
            self.tick(Some(source))?;
            let activation = entry.activation.clone();
            if activation.handler.abi == Abi::DirectPassing && !supplying {
                let Term::Function { parameters, .. } = &activation.handler.signature else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                if !parameters.is_empty() {
                    return Err(self.failure(ErrorKind::Arity, Some(source)));
                }
                let mut slot = activation
                    .provided
                    .lock()
                    .map_err(|_| self.failure(ErrorKind::InvalidControlFlow, Some(source)))?;
                let copy = activation.copy_provided;
                if copy {
                    if let Some(value) = slot.as_ref() {
                        self.effect_copy_work(value, source, 0)?;
                    }
                }
                let value = if copy { slot.clone() } else { slot.take() }
                    .ok_or_else(|| self.failure(ErrorKind::EffectValueConsumed, Some(source)))?;
                return Ok(Flow::Value(value));
            }
            self.effect_charge(activation.outer.len(), Some(source))?;
            let previous =
                std::mem::replace(&mut self.effect_state.environment, activation.outer.clone());
            let previous_invocation =
                std::mem::replace(&mut self.effect_state.invocation, activation.invocation);
            let previous_resume = self.effect_state.resume.replace(activation.identity);
            let selected = (|| {
                for (arm_index, arm) in activation.handler.arms.iter().enumerate() {
                    self.tick(Some(source))?;
                    self.effect_charge(activation.environment.len(), Some(source))?;
                    let mut local = activation.environment.clone();
                    let selected = self.with_effect_handler_resources(
                        &activation,
                        arm_index,
                        source,
                        |this| {
                            let (matched, bindings) = this.match_arm_guard_with_bindings(
                                &arm.pattern,
                                arm.guard.as_ref(),
                                &inputs,
                                &mut local,
                            )?;
                            match matched {
                                Flow::Value(Value::Bool(false)) => Ok(None),
                                Flow::Value(Value::Bool(true)) => {
                                    this.retain_pattern_residual(
                                        arm.body.source,
                                        &inputs,
                                        &bindings,
                                    )?;
                                    let result = this.eval(&arm.body, &mut local)?;
                                    if matches!(result, Flow::Value(_)) {
                                        this.cleanup_site(
                                            arm.body.source,
                                            iir::cleanup::Boundary::HandlerEnd,
                                            &mut local,
                                        )?;
                                    }
                                    Ok(Some(result))
                                }
                                other => Ok(Some(other)),
                            }
                        },
                    )?;
                    if selected.is_some() {
                        return Ok(selected);
                    }
                }
                Ok(None)
            })();
            self.effect_state.environment = previous;
            self.effect_state.invocation = previous_invocation;
            self.effect_state.resume = previous_resume;
            match selected? {
                Some(Flow::Value(value)) => return Ok(Flow::Value(value)),
                Some(Flow::EffectResume(target, value)) if target == activation.identity => {
                    return Ok(Flow::Value(value));
                }
                Some(flow) => {
                    self.effect_state.transfer = Some((activation.invocation, flow));
                    return Err(self.failure(ErrorKind::EffectControlTransfer, Some(source)));
                }
                None => {
                    let slot = activation
                        .handler
                        .fallback
                        .ok_or_else(|| self.failure(ErrorKind::NonExhaustiveMatch, Some(source)))?;
                    let next = activation.outer.get(slot).cloned().ok_or_else(|| {
                        self.failure(ErrorKind::InvalidEffectEnvironment, Some(source))
                    })?;
                    if next.operation != entry.operation {
                        return Err(self.failure(ErrorKind::InvalidEffectEnvironment, Some(source)));
                    }
                    entry = next;
                    supplying = false;
                }
            }
        }
    }

    /// A handler executes in its installation's resource context, never in
    /// the suspended callee's. Validate before guards can observe/write state;
    /// restore the suspended frame on every result, including control transfer.
    fn with_effect_handler_resources<T>(
        &mut self,
        activation: &Activation,
        arm: usize,
        source: SourceRange,
        evaluate: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let Some(schedule) = self.cleanup_schedule else {
            return evaluate(self);
        };
        if activation.cleanup_context.len() >= self.budget.depth {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        let work = self
            .effect_cleanup_context_work(
                activation.cleanup_frame.as_ref(),
                &activation.cleanup_context,
            )
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, Some(source)))?;
        self.effect_charge(work, Some(source))?;
        let mut context = activation.cleanup_context.clone();
        context.push(iir::cleanup::ContextKey::EffectHandler {
            installation: activation.installation,
            handler: activation.handler_index,
            arm,
        });
        let frame = activation
            .cleanup_frame
            .as_ref()
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
        let scan_work = work
            .checked_mul(schedule.functions.len())
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, Some(source)))?;
        self.effect_charge(scan_work, Some(source))?;
        schedule
            .exact_function(frame, &context)
            .map_err(|_| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
        let suspended_frame = std::mem::replace(&mut self.cleanup_frame, Some(frame.clone()));
        let suspended_context = std::mem::replace(&mut self.cleanup_context, context);
        let suspended_preparation = std::mem::take(&mut self.prepared_frames);
        let suspended_temporaries = std::mem::take(&mut self.residual_temporaries);
        let suspended_orders = std::mem::take(&mut self.pattern_binding_orders);
        let result = evaluate(self);
        self.pattern_binding_orders = suspended_orders;
        self.residual_temporaries = suspended_temporaries;
        self.prepared_frames = suspended_preparation;
        self.cleanup_context = suspended_context;
        self.cleanup_frame = suspended_frame;
        result
    }

    pub(super) fn with_effect_loop(
        &mut self,
        identity: LocalId,
        source: SourceRange,
        evaluate: impl FnOnce(&mut Self) -> Result<Flow, Error>,
    ) -> Result<Flow, Error> {
        let work = self
            .effect_cleanup_context_work(None, &self.cleanup_context)
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, Some(source)))?;
        self.effect_charge(work, Some(source))?;
        let floor = self.effect_state.loops.len();
        self.effect_state.loops.push((
            self.effect_state.invocation,
            identity,
            self.cleanup_context.clone(),
        ));
        let result = evaluate(self);
        self.effect_state.loops.truncate(floor);
        result
    }

    pub(super) fn effect_transfer_targets_scope(&mut self) -> Result<bool, Error> {
        let Some((target, flow)) = self.effect_state.transfer.as_ref() else {
            return Ok(false);
        };
        if *target != self.effect_state.invocation {
            return Ok(false);
        }
        let identity = match flow {
            Flow::Break(id) | Flow::Continue(id) => *id,
            _ => {
                return Ok(!matches!(
                    self.cleanup_context.last(),
                    Some(iir::cleanup::ContextKey::EffectHandler { .. })
                ));
            }
        };
        let work = self
            .effect_state
            .loops
            .len()
            .checked_mul(self.cleanup_context.len().saturating_add(1))
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, None))?;
        self.effect_charge(work, None)?;
        Ok(self
            .effect_state
            .loops
            .iter()
            .rev()
            .any(|(invocation, target, context)| {
                *invocation == self.effect_state.invocation
                    && *target == identity
                    && *context == self.cleanup_context
            }))
    }

    pub(super) fn cleanup_effect_failure(
        &mut self,
        error: &Error,
        source: SourceRange,
        environment: &mut Environment,
    ) -> Result<(), Error> {
        if self.cleanup_failed {
            return Ok(());
        }
        if error.kind == ErrorKind::EffectControlTransfer
            && !self.effect_transfer_targets_scope()?
        {
            self.cleanup_site(source, iir::cleanup::Boundary::EffectEscape, environment)?;
        } else if error.kind == ErrorKind::ProgramContractRejected {
            self.cleanup_site(
                source,
                iir::cleanup::Boundary::PredicateFailure,
                environment,
            )?;
        }
        Ok(())
    }

    fn effect_copy_work(
        &mut self,
        value: &Value,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        self.tick(Some(source))?;
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        match value {
            Value::BitVec(value) => self.effect_charge(value.words().len(), Some(source))?,
            Value::Integer(value) => {
                self.effect_charge(value.bit_len().div_ceil(32), Some(source))?
            }
            Value::Record {
                arguments, fields, ..
            }
            | Value::Variant {
                arguments, fields, ..
            } => {
                for ty in arguments {
                    let work = instance_argument_work(ty, &self.normal_forms).ok_or_else(|| {
                        self.failure(ErrorKind::RepresentationSizeLimit, Some(source))
                    })?;
                    self.effect_charge(work, Some(source))?;
                }
                for field in fields {
                    self.effect_copy_work(field, source, depth + 1)?;
                }
            }
            Value::Tuple(fields) => {
                for field in fields {
                    self.effect_copy_work(field, source, depth + 1)?;
                }
            }
            Value::AnonymousRecord(fields) => {
                for (name, field) in fields {
                    self.effect_charge(name.len(), Some(source))?;
                    self.effect_copy_work(field, source, depth + 1)?;
                }
            }
            Value::Closure(value)
                if !value.environment.is_empty() || !value.capture_order.is_empty() =>
            {
                return Err(self.failure(ErrorKind::Unsupported, Some(source)));
            }
            Value::RuntimeFunction { arguments, .. } => {
                for ty in arguments {
                    let work = instance_argument_work(ty, &self.normal_forms).ok_or_else(|| {
                        self.failure(ErrorKind::RepresentationSizeLimit, Some(source))
                    })?;
                    self.effect_charge(work, Some(source))?;
                }
            }
            Value::RuntimeTraitMethod(_) => {
                return Err(self.failure(ErrorKind::Unsupported, Some(source)));
            }
            // Unsupported managed/capture representations cannot be made Copy
            // by a host policy that only established their trait classification.
            Value::Storage(_) | Value::String(_) | Value::Type(_) | Value::Object(_) => {
                return Err(self.failure(ErrorKind::Unsupported, Some(source)));
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod resource_context_tests {
    use super::*;
    use iir::cleanup::{ContextKey, FunctionSchedule, Schedule};

    fn key(local: u32) -> iir::InstanceKey {
        iir::InstanceKey {
            declaration: DeclarationId { package: 0, local },
            arguments: vec![],
        }
    }
    fn activation() -> Activation {
        Activation {
            identity: 1,
            invocation: 1,
            handler: EffectHandler {
                operation: Operation {
                    declaration: DeclarationId {
                        package: 0,
                        local: 3,
                    },
                    arguments: vec![],
                },
                abi: Abi::Direct,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(Term::Primitive(Primitive::Bool)),
                },
                arms: vec![],
                fallback: None,
            },
            environment: Environment::new(),
            outer: vec![],
            provided: Default::default(),
            copy_provided: false,
            installation: SourceRange { start: 10, end: 20 },
            handler_index: 2,
            cleanup_frame: Some(key(1)),
            cleanup_context: vec![],
        }
    }
    fn context(activation: &Activation, arm: usize) -> ContextKey {
        ContextKey::EffectHandler {
            installation: activation.installation,
            handler: activation.handler_index,
            arm,
        }
    }
    fn schedule(activation: &Activation) -> Schedule {
        Schedule {
            closure_definitions: vec![],
            borrowed_callable_functions: vec![],
            functions: vec![FunctionSchedule {
                instance: key(1),
                context: vec![context(activation, 0)],
                sites: vec![],
                discards: vec![],
                transfers: vec![],
                copies: vec![],
                pattern_transfers: vec![],
                pattern_orders: vec![],
            }],
            aliases: vec![],
        }
    }

    #[test]
    fn handler_resource_context_restores_suspended_state_on_completion_and_failure() {
        for fail in [false, true] {
            let package = Package::default();
            let activation = activation();
            let schedule = schedule(&activation);
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            // Exercise the context primitive directly; public scheduled effect
            // admission remains closed until handler resource proofs exist.
            vm.cleanup_schedule = Some(&schedule);
            vm.cleanup_frame = Some(key(9));
            let suspended_context = vec![ContextKey::Default {
                construction: SourceRange { start: 30, end: 40 },
                parameter: 4,
            }];
            vm.cleanup_context = suspended_context.clone();
            vm.prepared_frames.push(cleanup::PreparedFrame {
                source: SourceRange { start: 50, end: 60 },
                values: vec![],
            });
            vm.residual_temporaries.push((
                SourceRange { start: 70, end: 80 },
                Storage::new(Value::Bool(true)),
            ));
            vm.pattern_binding_orders
                .insert((90, 100), vec![LocalId { owner: 9, index: 2 }]);
            let result =
                vm.with_effect_handler_resources(&activation, 0, activation.installation, |vm| {
                    assert_eq!(vm.cleanup_frame, Some(key(1)));
                    assert_eq!(vm.cleanup_context, vec![context(&activation, 0)]);
                    assert!(vm.prepared_frames.is_empty());
                    assert!(vm.residual_temporaries.is_empty());
                    assert!(vm.pattern_binding_orders.is_empty());
                    vm.prepared_frames.push(cleanup::PreparedFrame {
                        source: activation.installation,
                        values: vec![],
                    });
                    if fail {
                        Err(vm.failure(
                            ErrorKind::EffectControlTransfer,
                            Some(activation.installation),
                        ))
                    } else {
                        Ok(42)
                    }
                });
            if fail {
                assert_eq!(result.unwrap_err().kind, ErrorKind::EffectControlTransfer);
            } else {
                assert_eq!(result.unwrap(), 42);
            }
            assert_eq!(vm.cleanup_frame, Some(key(9)));
            assert_eq!(vm.cleanup_context, suspended_context);
            assert_eq!(vm.prepared_frames.len(), 1);
            assert_eq!(
                vm.prepared_frames[0].source,
                SourceRange { start: 50, end: 60 }
            );
            assert_eq!(vm.residual_temporaries.len(), 1);
            assert_eq!(
                vm.residual_temporaries[0].0,
                SourceRange { start: 70, end: 80 }
            );
            assert_eq!(
                vm.pattern_binding_orders[&(90, 100)],
                vec![LocalId { owner: 9, index: 2 }]
            );
        }
    }

    #[test]
    fn handler_resource_context_rejects_missing_and_ambiguous_schedules_before_body() {
        for count in [0, 2] {
            let package = Package::default();
            let activation = activation();
            let mut schedule = schedule(&activation);
            let entry = schedule.functions[0].clone();
            schedule.functions = vec![entry; count];
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            vm.cleanup_schedule = Some(&schedule);
            vm.cleanup_frame = Some(key(9));
            let mut evaluated = false;
            let result =
                vm.with_effect_handler_resources(&activation, 0, activation.installation, |_| {
                    evaluated = true;
                    Ok(())
                });
            assert_eq!(result.unwrap_err().kind, ErrorKind::InvalidCallPlan);
            assert!(!evaluated);
            assert_eq!(vm.cleanup_frame, Some(key(9)));
            assert!(vm.cleanup_context.is_empty());
        }
    }

    #[test]
    fn handler_resource_context_identity_distinguishes_installation_handler_and_arm() {
        let activation = activation();
        assert_ne!(context(&activation, 0), context(&activation, 1));
        let mut other = self::activation();
        other.handler_index += 1;
        assert_ne!(context(&activation, 0), context(&other, 0));
        other.handler_index = activation.handler_index;
        other.installation.end += 1;
        assert_ne!(context(&activation, 0), context(&other, 0));
        assert_eq!(
            context(&activation, 0).to_string(),
            "(effect-handler-context (installation (source 10 20)) (handler 2) (arm 0))"
        );
        assert_eq!(
            iir::cleanup::Boundary::HandlerEnd.to_string(),
            "handler-end"
        );
        assert_eq!(
            iir::cleanup::Boundary::EffectResume.to_string(),
            "effect-resume"
        );
    }
}
