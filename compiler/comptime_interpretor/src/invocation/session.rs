//! One owner for the complete driver continuation and its exclusively borrowed VM.
//! Poll boundaries are internal progress, not yet asynchronous host requests.
use super::*;
use std::task::Poll;

enum Root<'r> {
    Invocation,
    Fragment(&'r mut bytecode::Frame),
    Plan(&'r mut dyn input_collection::InputPlan, &'r mut Environment),
}

impl Root<'_> {
    fn code(&mut self) -> Option<&mut bytecode::Frame> {
        match self {
            Self::Fragment(frame) => Some(frame),
            _ => None,
        }
    }

    fn plan(&mut self) -> Option<(&mut dyn input_collection::InputPlan, &mut Environment)> {
        match self {
            Self::Plan(frame, environment) => Some((&mut **frame, &mut **environment)),
            _ => None,
        }
    }

    fn cancel(&mut self) {
        match self {
            Self::Invocation => {}
            Self::Fragment(frame) => frame.cancel(),
            Self::Plan(frame, environment) => frame.cancel(environment),
        }
    }
}

pub(super) struct Session<'vm, 'a, 'r> {
    vm: &'vm mut Interpreter<'a>,
    root: Root<'r>,
    execution: Option<Execution<'a>>,
    stack: Vec<StackEntry<'a>>,
    result: Option<Completion>,
    terminal: bool,
}

impl<'vm, 'a, 'r> Session<'vm, 'a, 'r> {
    fn new(vm: &'vm mut Interpreter<'a>, root: Root<'r>) -> Self {
        Self {
            vm,
            root,
            execution: None,
            stack: Vec::new(),
            result: None,
            terminal: false,
        }
    }

    pub(super) fn invocation(vm: &'vm mut Interpreter<'a>, active: Box<Activation<'a>>) -> Self {
        let mut session = Self::new(vm, Root::Invocation);
        session.execution = Some(Execution::Invocation(active));
        session
    }

    pub(super) fn fragment(vm: &'vm mut Interpreter<'a>, frame: &'r mut bytecode::Frame) -> Self {
        Self::new(vm, Root::Fragment(frame))
    }

    pub(super) fn plan(
        vm: &'vm mut Interpreter<'a>,
        frame: &'r mut dyn input_collection::InputPlan,
        environment: &'r mut Environment,
    ) -> Self {
        Self::new(vm, Root::Plan(frame, environment))
    }

    pub(super) fn run(mut self) -> Result<Option<Flow>, Error> {
        loop {
            if let Poll::Ready(flow) = self.poll()? {
                return Ok(flow);
            }
        }
    }

    // A poll is one driver transition, including at most one bytecode slice.
    // Tree migration and synchronous preparation retain their existing budgets;
    // this is not a preemption or wall-clock latency guarantee.
    fn poll(&mut self) -> Result<Poll<Option<Flow>>, Error> {
        if self.terminal {
            return Err(self.vm.failure(ErrorKind::InvalidControlFlow, None));
        }
        let result = self.step();
        if !matches!(result, Ok(Poll::Pending)) {
            debug_assert!(
                self.stack.is_empty() && self.execution.is_none() && self.result.is_none()
            );
            if result.is_err() {
                self.root.cancel();
            }
            self.terminal = true;
        }
        result
    }

    fn step(&mut self) -> Result<Poll<Option<Flow>>, Error> {
        let Self {
            vm,
            root,
            execution,
            stack,
            result,
            ..
        } = self;
        if let Some(mut active) = execution.take() {
            let owned = match &active {
                Execution::Invocation(activation) if activation.frame.stable_code.is_none() => {
                    match &activation.frame.body {
                        Body::Owned(body) => Some(body.clone()),
                        Body::Package(_) => None,
                    }
                }
                Execution::Default { expression, .. } | Execution::Contract { expression, .. } => {
                    Some(expression.clone())
                }
                _ => None,
            };
            let code = if let Execution::Associated { expression, .. } = &active {
                match expression {
                    Body::Package(expression) => vm.borrowed_invocation_code(expression),
                    Body::Owned(expression) => vm.owned_invocation_code(expression),
                }
            } else {
                let (expression, identity, _) = active.parts();
                match owned {
                    Some(body) => vm.owned_invocation_code(&body),
                    None => vm.invocation_code(expression, identity),
                }
            };
            *result = match code {
                Ok(Some(code)) => {
                    // Suspended bytecode activations live on the explicit stack.
                    // Keep the native guard for tree evaluation and preparation.
                    if let Execution::Invocation(activation) = &mut active {
                        activation.saved.counts_native_depth = false;
                        vm.depth -= 1;
                    }
                    match vm.start_frame(&code, active.parts().2) {
                        Ok(code_frame) => {
                            match &active {
                                Execution::Invocation(_) => vm.bytecode_invocations += 1,
                                Execution::Default { .. } => vm.bytecode_default_evaluations += 1,
                                Execution::Contract { .. } | Execution::Associated { .. } => {}
                            }
                            stack.push(StackEntry::Code(
                                StackContext::Execution(active),
                                code_frame,
                            ));
                            None
                        }
                        Err(error) => Some(vm.complete_execution(active, Err(error))),
                    }
                }
                Ok(None) => {
                    let (expression, _, environment) = active.parts();
                    let flow = vm.eval(expression, environment);
                    Some(vm.complete_execution(active, flow))
                }
                Err(error) => Some(vm.complete_execution(active, Err(error))),
            };
        }
        if let Some(delivered) = result.take() {
            let (flow, defaults) = match delivered {
                Completion::Input(input) => {
                    if let Some(StackEntry::Plan(active)) = stack.last_mut() {
                        let outcome = active
                            .frame
                            .accept_input(vm, &mut active.environment, input);
                        if !matches!(&outcome, Ok(None)) {
                            let Some(StackEntry::Plan(active)) = stack.pop() else {
                                unreachable!()
                            };
                            *result = Some(active.complete(outcome.map(|flow| flow.unwrap())));
                        }
                    } else {
                        let (frame, environment) = root.plan().expect("input plan root");
                        if let Some(flow) = frame.accept_input(vm, environment, input)? {
                            return Ok(Poll::Ready(Some(flow)));
                        }
                    }
                    return Ok(Poll::Pending);
                }
                Completion::Ordinary { flow, defaults } => (flow, defaults),
            };
            let Some(parent) = stack
                .last_mut()
                .and_then(StackEntry::code)
                .or_else(|| root.code())
            else {
                return flow.map(|flow| Poll::Ready(Some(flow)));
            };
            let propagation = match parent.receive(flow, defaults) {
                Ok(None) => None,
                Ok(Some(flow)) => Some(Ok(flow)),
                Err(error) => Some(Err(error)),
            };
            if let Some(flow) = propagation {
                // Defaults may return from their enclosing invocation.
                // Restore each activation before propagating a transfer/error.
                if let Some(StackEntry::Code(parent, code)) = stack.pop() {
                    *result = Some(vm.complete_stack(parent, code, flow));
                    return Ok(Poll::Pending);
                }
                return flow.map(|flow| Poll::Ready(Some(flow)));
            }
        }
        if let Some(StackEntry::Plan(active)) = stack.last_mut() {
            match active.frame.next_input(vm, &mut active.environment) {
                Ok(input_collection::Progress::Input(bytecode::InputFrame { code, finish })) => {
                    stack.push(StackEntry::Code(StackContext::Input(finish), code));
                }
                outcome => {
                    let Some(StackEntry::Plan(active)) = stack.pop() else {
                        unreachable!()
                    };
                    let flow = outcome.map(|progress| match progress {
                        input_collection::Progress::Complete(flow) => flow,
                        _ => unreachable!(),
                    });
                    *result = Some(active.complete(flow));
                }
            }
            return Ok(Poll::Pending);
        }
        if stack.is_empty() {
            if let Some((frame, environment)) = root.plan() {
                match frame.next_input(vm, environment)? {
                    input_collection::Progress::Input(bytecode::InputFrame { code, finish }) => {
                        stack.push(StackEntry::Code(StackContext::Input(finish), code));
                    }
                    input_collection::Progress::Complete(flow) => {
                        return Ok(Poll::Ready(Some(flow)));
                    }
                    input_collection::Progress::InputsComplete => return Ok(Poll::Ready(None)),
                }
                return Ok(Poll::Pending);
            }
        }
        let code = stack
            .last_mut()
            .and_then(StackEntry::code)
            .or_else(|| root.code())
            .expect("active execution");
        match vm.resume_frame(code, 1024) {
            Ok(bytecode::Progress::TypeInputPlan {
                expression,
                environment,
            }) => match vm.start_type_plan(expression, &environment) {
                Ok(frame) => stack.push(StackEntry::Plan(Box::new(PlanActivation {
                    frame,
                    environment,
                }))),
                Err(error) => {
                    *result = Some(Completion::Ordinary {
                        flow: Err(error),
                        defaults: Some(environment),
                    })
                }
            },
            Ok(bytecode::Progress::Associated { access, source }) => {
                match vm.prepare_associated_value(
                    &access.interface,
                    &access.subject,
                    access.member,
                    &access.ty,
                    source,
                ) {
                    Ok((expression, key, result_type)) => {
                        *execution = Some(Execution::Associated {
                            expression,
                            result_type,
                            key,
                            source,
                            environment: Environment::default(),
                        })
                    }
                    Err(error) => {
                        *result = Some(Completion::Ordinary {
                            flow: Err(error),
                            defaults: None,
                        })
                    }
                }
            }
            Ok(bytecode::Progress::Contract {
                expression,
                environment,
            }) => {
                *execution = Some(Execution::Contract {
                    expression,
                    environment,
                });
            }
            Ok(bytecode::Progress::Suspended) => {}
            Ok(bytecode::Progress::Default {
                expression,
                environment,
            }) => {
                *execution = Some(Execution::Default {
                    expression,
                    environment,
                });
            }
            Ok(bytecode::Progress::Call { callee, arguments }) => {
                match vm.begin_invocation(callee, arguments, None) {
                    Ok(child) => *execution = Some(Execution::Invocation(child)),
                    Err(error) => {
                        *result = Some(Completion::Ordinary {
                            flow: Err(error),
                            defaults: None,
                        })
                    }
                }
            }
            outcome => {
                let flow = match outcome {
                    Ok(bytecode::Progress::Complete(flow)) => Ok(flow),
                    Err(error) => Err(error),
                    _ => unreachable!("handled suspension"),
                };
                if let Some(StackEntry::Code(completed, code)) = stack.pop() {
                    *result = Some(vm.complete_stack(completed, code, flow));
                } else {
                    return flow.map(|flow| Poll::Ready(Some(flow)));
                }
            }
        }
        Ok(Poll::Pending)
    }

    fn cancel(&mut self) {
        if self.terminal {
            return;
        }
        let error = self.vm.failure(ErrorKind::InvalidControlFlow, None);
        self.result = if let Some(active) = self.execution.take() {
            Some(self.vm.complete_execution(active, Err(error)))
        } else if let Some(completion) = self.result.take() {
            Some(match completion {
                Completion::Ordinary { defaults, .. } => Completion::Ordinary {
                    flow: Err(error),
                    defaults,
                },
                Completion::Input(mut input) => {
                    input.flow = Err(error);
                    Completion::Input(input)
                }
            })
        } else {
            self.stack.pop().map(|entry| match entry {
                StackEntry::Code(context, code) => {
                    self.vm.complete_stack(context, code, Err(error))
                }
                StackEntry::Plan(active) => active.complete(Err(error)),
            })
        };
        // Deliver failure through the same restoration boundaries. No source
        // code, default or contract is executed while unwinding this error.
        if self.result.is_some() {
            while matches!(self.step(), Ok(Poll::Pending)) {}
        }
        debug_assert!(self.stack.is_empty() && self.execution.is_none() && self.result.is_none());
        self.root.cancel();
        self.terminal = true;
    }
}

impl Drop for Session<'_, '_, '_> {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl fmt::Display for Session<'_, '_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn execution(f: &mut fmt::Formatter<'_>, value: &Execution<'_>) -> fmt::Result {
            match value {
                Execution::Invocation(active) => {
                    write!(f, "(invocation {})", active.frame.function)
                }
                Execution::Associated { source, .. } => write!(f, "(associated {source})"),
                Execution::Contract { .. } => f.write_str("(contract)"),
                Execution::Default { .. } => f.write_str("(default)"),
            }
        }
        write!(f, "(execution-session (terminal {}) (root ", self.terminal)?;
        match &self.root {
            Root::Invocation => f.write_str("invocation")?,
            Root::Fragment(code) => write!(f, "(fragment {code})")?,
            Root::Plan(plan, _) => write!(f, "(plan {plan})")?,
        }
        f.write_str(") (queued")?;
        if let Some(value) = &self.execution {
            f.write_str(" ")?;
            execution(f, value)?;
        }
        f.write_str(") (completion")?;
        if let Some(completion) = &self.result {
            f.write_str(match completion {
                Completion::Ordinary { .. } => " ordinary",
                Completion::Input(_) => " input",
            })?;
        }
        f.write_str(") (stack")?;
        for entry in &self.stack {
            match entry {
                StackEntry::Code(context, code) => {
                    f.write_str(" (code ")?;
                    match context {
                        StackContext::Execution(active) => execution(f, active)?,
                        StackContext::Input(_) => f.write_str("(input)")?,
                    }
                    write!(f, " {code})")?;
                }
                StackEntry::Plan(active) => write!(f, " (plan {})", active.frame)?,
            }
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use input_collection::InputPlan;
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    use nir::program::{NormalFormArgument, NormalFormInputPlan, NormalFormPreparation, Place};

    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
    fn local(index: u32) -> LocalId {
        LocalId { owner: 95, index }
    }
    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 95, local }
    }
    fn ty(primitive: Primitive) -> Term {
        Term::Primitive(primitive)
    }
    fn expr(ty: Term, kind: K) -> Expr {
        Expr {
            source: SOURCE,
            ty,
            kind,
        }
    }
    fn mark() -> Expr {
        expr(
            ty(Primitive::Void),
            K::Assign {
                place: Place {
                    root: local(1),
                    fields: vec![],
                },
                value: Box::new(expr(ty(Primitive::Bool), K::Literal(Literal::Bool(true)))),
            },
        )
    }

    fn nested_fragment(fail: bool) -> (Arc<comptime_compiler::Program>, Environment) {
        let signature = Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(ty(Primitive::Bool)),
        }));
        let call = || {
            expr(
                ty(Primitive::Bool),
                K::Call {
                    callee: Box::new(expr(signature.clone(), K::Local(local(0)))),
                    arguments: vec![],
                    parameter_bindings: vec![],
                },
            )
        };
        let closure = |body, child: Option<Value>| {
            Value::Closure(Arc::new(Closure {
                definition_source: None,
                origin: None,
                instance_arguments: None,
                parameters: vec![],
                capture_order: child.as_ref().map(|_| vec![local(0)]).unwrap_or_default(),
                environment: child
                    .map(|value| Environment::from([(local(0), value)]))
                    .unwrap_or_default(),
                body: Arc::new(body),
                ty: signature.clone(),
            }))
        };
        let leaf = expr(
            ty(Primitive::Bool),
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
        let body = expr(
            ty(Primitive::Bool),
            K::Block {
                statements: vec![mark()],
                result: Some(Box::new(call())),
            },
        );
        (
            Arc::new(comptime_compiler::compile(&body, &mut 100_000).unwrap()),
            Environment::from([(local(0), value), (local(1), Value::Bool(false))]),
        )
    }

    #[test]
    fn sessions_move_between_polls_and_cancel_at_every_call_boundary() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.effect_state.invocation = 71;
        vm.effect_state.resume = Some(19);
        vm.pattern_binding_orders.insert((1, 2), vec![local(1)]);
        let mut completed_polls = None;
        // First measure the complete run; then abandon every strict prefix.
        for limit in std::iter::once(usize::MAX).chain(0..40) {
            if completed_polls.is_some_and(|count| limit >= count) {
                break;
            }
            let (code, mut environment) = nested_fragment(false);
            let mut frame = vm.start_frame(&code, &mut environment).unwrap();
            let before = vm.bytecode_invocations();
            let mut session = Box::new(Session::fragment(&mut vm, &mut frame));
            let mut polls = 0;
            while polls < limit {
                polls += 1;
                if let Poll::Ready(flow) = session.poll().unwrap() {
                    assert!(matches!(flow, Some(Flow::Value(Value::Bool(true)))));
                    assert_eq!(session.vm.bytecode_invocations() - before, 9);
                    assert_eq!(
                        session.poll().err().unwrap().kind,
                        ErrorKind::InvalidControlFlow
                    );
                    completed_polls = Some(polls);
                    break;
                }
                session = Box::new(*session);
            }
            let calls = session.vm.bytecode_invocations();
            let events = session.vm.events().len();
            let dump = session.to_string();
            assert!(dump.starts_with("(execution-session (terminal "));
            assert_eq!(
                dump.chars().filter(|c| *c == '(').count(),
                dump.chars().filter(|c| *c == ')').count()
            );
            drop(session);
            assert_eq!(vm.bytecode_invocations(), calls);
            assert_eq!(vm.events().len(), events);
            assert_eq!(
                vm.resume_frame(&mut frame, 1).err().unwrap().kind,
                ErrorKind::InvalidControlFlow
            );
            environment = frame.restore_environment();
            assert_eq!(environment.get(&local(1)), Some(&Value::Bool(polls > 0)));
            assert_eq!(environment.len(), 2);
            assert_eq!(vm.depth, 0);
            assert_eq!(vm.invocation_depth, 0);
            assert_eq!(vm.effect_state.invocation, 71);
            assert_eq!(vm.effect_state.resume, Some(19));
            assert_eq!(
                vm.pattern_binding_orders.get(&(1, 2)),
                Some(&vec![local(1)])
            );
        }
        assert!(completed_polls.is_some_and(|count| count > 9 && count < 40));
        // A terminal semantic failure restores the same VM before a fresh run.
        for fail in [true, false] {
            let (code, mut environment) = nested_fragment(fail);
            let mut frame = vm.start_frame(&code, &mut environment).unwrap();
            let result = Session::fragment(&mut vm, &mut frame).run();
            if fail {
                assert_eq!(result.err().unwrap().kind, ErrorKind::Type);
            } else {
                assert!(matches!(
                    result.unwrap(),
                    Some(Flow::Value(Value::Bool(true)))
                ));
            }
            assert_eq!(vm.invocation_depth, 0);
            assert_eq!(vm.depth, 0);
        }
    }

    #[test]
    fn cancelling_invocation_root_restores_admission_and_can_restart() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.effect_state.invocation = 23;
        for polls in [0, 1, 4, 12] {
            let (_, mut environment) = nested_fragment(false);
            let callable = environment.remove(&local(0)).unwrap();
            let active = vm.begin_invocation(callable.clone(), vec![], None).unwrap();
            let mut session = Session::invocation(&mut vm, active);
            for _ in 0..polls {
                assert!(matches!(session.poll().unwrap(), Poll::Pending));
            }
            let events = session.vm.events().len();
            drop(session);
            assert_eq!(vm.events().len(), events);
            assert_eq!(vm.depth, 0);
            assert_eq!(vm.invocation_depth, 0);
            assert_eq!(vm.effect_state.invocation, 23);
            assert_eq!(
                vm.invoke_value(callable, vec![]).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(vm.effect_state.invocation, 23);
        }
    }

    fn dependent_plan() -> (Package, Expr) {
        let package = Package {
            normal_forms: vec![Head {
                declaration: id(0),
                collector: None,
                role: Role::Opaque,
                requirements: vec![],
                parameters: vec![
                    Parameter {
                        declaration: id(1),
                        binding: local(10),
                        kind: Kind::Type,
                        ty: ty(Primitive::Type),
                    },
                    Parameter {
                        declaration: id(2),
                        binding: local(11),
                        kind: Kind::Value,
                        ty: Term::Rigid {
                            declaration: id(1),
                            sort: nir::Sort::Type,
                        },
                    },
                ],
            }],
            ..Default::default()
        };
        let expression = expr(
            ty(Primitive::Type),
            K::NormalFormType {
                head: id(0),
                arguments: vec![
                    NormalFormArgument::Value(expr(
                        ty(Primitive::Type),
                        K::TypeValue(ty(Primitive::U8)),
                    )),
                    NormalFormArgument::Integer {
                        spelling: "7".into(),
                        negative: false,
                        source: SOURCE,
                    },
                ],
                plan: Some(NormalFormInputPlan {
                    order: vec![1, 0],
                    bindings: vec![local(10), local(11)],
                    preparations: vec![NormalFormPreparation {
                        before: 0,
                        expression: expr(
                            ty(Primitive::Void),
                            K::LetPattern {
                                pattern: nir::program::Pattern::Tuple(vec![]),
                                guard: None,
                                otherwise: None,
                                value: Box::new(expr(
                                    Term::Tuple(vec![]),
                                    K::Block {
                                        statements: vec![mark()],
                                        result: Some(Box::new(expr(
                                            Term::Tuple(vec![]),
                                            K::Tuple(vec![]),
                                        ))),
                                    },
                                )),
                            },
                        ),
                    }],
                }),
            },
        );
        (package, expression)
    }

    #[test]
    fn cancelling_input_sessions_restores_staging_depth_and_owned_bindings() {
        let (package, expression) = dependent_plan();
        let code = Arc::new(comptime_compiler::compile(&expression, &mut 100_000).unwrap());
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for borrowed_plan in [false, true] {
            let mut complete_polls = None;
            for limit in std::iter::once(usize::MAX).chain(0..30) {
                if complete_polls.is_some_and(|count| limit >= count) {
                    break;
                }
                let mut environment = Environment::from([(local(1), Value::Bool(false))]);
                let mut plan =
                    normal_plan::Frame::borrowed(&mut vm, &expression, &environment).unwrap();
                let mut frame = if borrowed_plan {
                    None
                } else {
                    Some(vm.start_frame(&code, &mut environment).unwrap())
                };
                let mut session = if let Some(frame) = &mut frame {
                    Session::fragment(&mut vm, frame)
                } else {
                    Session::plan(&mut vm, &mut plan, &mut environment)
                };
                let mut polls = 0;
                while polls < limit {
                    polls += 1;
                    if let Poll::Ready(flow) = session.poll().unwrap() {
                        assert!(matches!(
                            flow,
                            Some(Flow::Value(Value::Type(Term::NormalForm { .. })))
                        ));
                        complete_polls = Some(polls);
                        break;
                    }
                }
                let steps = session.vm.budget.steps;
                drop(session);
                assert_eq!(
                    vm.budget.steps, steps,
                    "cancellation must not execute producers"
                );
                if let Some(frame) = frame {
                    environment = frame.restore_environment();
                } else {
                    plan.cleanup(&mut environment);
                }
                assert_eq!(environment.len(), 1, "only caller binding survives");
                assert!(environment.contains_key(&local(1)));
                if limit == usize::MAX {
                    assert_eq!(environment.get(&local(1)), Some(&Value::Bool(true)));
                }
                assert_eq!(vm.depth, 0);
                assert_eq!(vm.invocation_depth, 0);
                if borrowed_plan && limit != usize::MAX {
                    assert_eq!(
                        plan.next_input(&mut vm, &mut environment)
                            .err()
                            .unwrap()
                            .kind,
                        ErrorKind::InvalidControlFlow
                    );
                }
            }
            assert!(complete_polls.is_some_and(|count| count > 3 && count < 30));
        }
    }
}
