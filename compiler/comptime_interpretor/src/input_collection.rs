//! One invocation-local scheduler for ordered input execution and domain wakeup.
//! Providers retain their own admission and final construction semantics.
use super::*;
use nir::program::{NormalFormArgument as Argument, NormalFormPreparation};
use normal_inputs::StagedNormalInput;

pub(super) trait Source: fmt::Display {
    fn source(&self) -> SourceRange;
    fn preparation(&self, index: usize) -> Option<&NormalFormPreparation>;
    fn slot(&self, position: usize) -> Option<usize>;
    fn input(&self, slot: usize) -> &Argument;
    fn specialize(&mut self, vm: &mut Interpreter<'_>, slot: usize) -> Result<(), Error>;
    fn domain(&self, vm: &mut Interpreter<'_>, slot: usize) -> Result<(Term, bool), Error>;
    fn check_input(
        &self,
        vm: &Interpreter<'_>,
        slot: usize,
        expected: Option<&Term>,
    ) -> Result<(), Error>;
    fn bind(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
        slot: usize,
        value: Value,
    ) -> Result<(), Error>;
    fn complete(&mut self, vm: &mut Interpreter<'_>) -> Result<Progress, Error>;
    fn cleanup(&self, environment: &mut Environment);
}

enum Waiting {
    Preparation(usize),
    Ready { slot: usize, expected: Term },
    Stage(usize),
}

pub(super) enum Request<'a> {
    Preparation(&'a NormalFormPreparation),
    Ready(&'a Argument, &'a Term),
    Stage(&'a Argument),
    Complete,
}

pub(super) enum Completion {
    Prepared,
    Value(Value),
    Staged(StagedNormalInput),
}

pub(super) enum Progress {
    InputsComplete,
    Input(bytecode::InputFrame),
    Complete(Flow),
}

pub(super) trait InputPlan {
    fn cleanup(&self, environment: &mut Environment);
    fn next_input(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
    ) -> Result<Progress, Error>;
    fn accept_input(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
        result: bytecode::InputResult,
    ) -> Result<Option<Flow>, Error>;
}

pub(super) struct Frame<S> {
    pub(super) source: S,
    position: usize,
    preparation: usize,
    waiting: Option<Waiting>,
    pending: BTreeMap<usize, StagedNormalInput>,
    drain: bool,
    terminal: bool,
}

impl<S: Source> fmt::Display for Frame<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(input-collection-frame (position {}) (preparation {}) (terminal {}) (drain {}) {} (waiting",
            self.position, self.preparation, self.terminal, self.drain, self.source
        )?;
        match &self.waiting {
            Some(Waiting::Preparation(index)) => write!(f, " (preparation {index})")?,
            Some(Waiting::Ready { slot, expected }) => write!(f, " (ready {slot} {expected})")?,
            Some(Waiting::Stage(slot)) => write!(f, " (stage {slot})")?,
            None => {}
        }
        f.write_str(") (pending")?;
        for slot in self.pending.keys() {
            write!(f, " (input {slot})")?;
        }
        f.write_str("))")
    }
}

impl<S: Source> Frame<S> {
    pub(super) fn from_source(source: S) -> Self {
        Self {
            source,
            position: 0,
            preparation: 0,
            waiting: None,
            pending: BTreeMap::new(),
            drain: false,
            terminal: false,
        }
    }

    fn drain(&mut self, vm: &mut Interpreter<'_>, env: &mut Environment) -> Result<(), Error> {
        loop {
            vm.integer_operation(Some(self.source.source()), |budget| {
                budget.consume(self.pending.len())
            })?;
            let mut progress = false;
            for slot in self.pending.keys().copied().collect::<Vec<_>>() {
                let (expected, unresolved) = self.source.domain(vm, slot)?;
                if unresolved {
                    continue;
                }
                self.source.specialize(vm, slot)?;
                self.source.check_input(vm, slot, Some(&expected))?;
                let argument = self.source.input(slot);
                let mut literals = BTreeMap::new();
                vm.prepare_normal_argument(argument, &expected, &mut literals, 0)?;
                let staged = self.pending.remove(&slot).expect("pending input");
                let value =
                    vm.finish_staged_normal_argument(staged, &mut literals, argument.source(), 0)?;
                self.source.bind(vm, env, slot, value)?;
                progress = true;
            }
            if !progress {
                return Ok(());
            }
        }
    }

    pub(super) fn advance(
        &mut self,
        vm: &mut Interpreter<'_>,
        env: &mut Environment,
    ) -> Result<Request<'_>, Error> {
        if self.waiting.is_some() || self.terminal {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source.source())));
        }
        // A semantic failure ends this frame. It must never replay a producer.
        self.terminal = true;
        if self.drain {
            self.drain = false;
            self.drain(vm, env)?;
        }
        if let Some(preparation) = self.source.preparation(self.preparation)
            && preparation.before == self.position
        {
            self.waiting = Some(Waiting::Preparation(self.preparation));
        } else if let Some(slot) = self.source.slot(self.position) {
            self.source.specialize(vm, slot)?;
            self.source.check_input(vm, slot, None)?;
            let (expected, unresolved) = self.source.domain(vm, slot)?;
            self.waiting = Some(if unresolved {
                Waiting::Stage(slot)
            } else {
                self.source.check_input(vm, slot, Some(&expected))?;
                Waiting::Ready { slot, expected }
            });
        } else {
            return Ok(Request::Complete);
        }
        self.terminal = false;
        Ok(match self.waiting.as_ref().expect("requested input") {
            Waiting::Preparation(index) => Request::Preparation(
                self.source
                    .preparation(*index)
                    .expect("requested preparation"),
            ),
            Waiting::Ready { slot, expected } => Request::Ready(self.source.input(*slot), expected),
            Waiting::Stage(slot) => Request::Stage(self.source.input(*slot)),
        })
    }

    pub(super) fn receive(
        &mut self,
        vm: &mut Interpreter<'_>,
        env: &mut Environment,
        completion: Completion,
    ) -> Result<(), Error> {
        if !matches!(
            (&self.waiting, &completion),
            (Some(Waiting::Preparation(_)), Completion::Prepared)
                | (Some(Waiting::Ready { .. }), Completion::Value(_))
                | (Some(Waiting::Stage(_)), Completion::Staged(_))
        ) {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source.source())));
        }
        self.terminal = true;
        match (self.waiting.take().expect("completed request"), completion) {
            (Waiting::Preparation(_), Completion::Prepared) => self.preparation += 1,
            (Waiting::Ready { slot, .. }, Completion::Value(value)) => {
                self.source.bind(vm, env, slot, value)?;
                self.position += 1;
                self.drain = true;
            }
            (Waiting::Stage(slot), Completion::Staged(staged)) => {
                self.pending.insert(slot, staged);
                self.position += 1;
                self.drain = true;
            }
            _ => unreachable!("completion kind checked above"),
        }
        self.terminal = false;
        Ok(())
    }

    fn next(&mut self, vm: &mut Interpreter<'_>, env: &mut Environment) -> Result<Progress, Error> {
        loop {
            let completion = match self.advance(vm, env)? {
                Request::Preparation(preparation) => {
                    if let Some(frame) = vm.start_input_preparation(preparation, env)? {
                        return Ok(Progress::Input(frame));
                    }
                    match vm.eval(&preparation.expression, env)? {
                        Flow::Value(_) => Completion::Prepared,
                        flow => return Ok(Progress::Complete(flow)),
                    }
                }
                Request::Ready(argument, expected) => {
                    if let Some(frame) = vm.start_contextual_input(argument, expected, env)? {
                        return Ok(Progress::Input(frame));
                    }
                    match vm.interpret_contextual_input(argument, expected, env)? {
                        Flow::Value(value) => Completion::Value(value),
                        flow => return Ok(Progress::Complete(flow)),
                    }
                }
                Request::Stage(argument) => {
                    if let Some(frame) = vm.start_staged_input(argument, env, 0)? {
                        return Ok(Progress::Input(frame));
                    }
                    match vm.interpret_staged_argument(argument, env, 0)? {
                        std::ops::ControlFlow::Continue(value) => Completion::Staged(value),
                        std::ops::ControlFlow::Break(flow) => return Ok(Progress::Complete(flow)),
                    }
                }
                Request::Complete => return self.source.complete(vm),
            };
            self.receive(vm, env, completion)?;
        }
    }
}

impl<S: Source> InputPlan for Frame<S> {
    fn cleanup(&self, environment: &mut Environment) {
        self.source.cleanup(environment);
    }

    fn next_input(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
    ) -> Result<Progress, Error> {
        let result = self.next(vm, environment);
        if !matches!(&result, Ok(Progress::Input(_))) {
            self.terminal = true;
        }
        result
    }

    fn accept_input(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
        result: bytecode::InputResult,
    ) -> Result<Option<Flow>, Error> {
        if self.terminal || self.waiting.is_none() {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source.source())));
        }
        *environment = result.environment;
        self.terminal = true;
        let completion = match result.flow? {
            Flow::Value(value) => match &self.waiting {
                Some(Waiting::Preparation(_)) => Completion::Prepared,
                Some(Waiting::Ready { .. }) => Completion::Value(value),
                Some(Waiting::Stage(_)) => {
                    let (shape, mut staged) = result.staged.ok_or_else(|| {
                        vm.failure(ErrorKind::InvalidControlFlow, Some(self.source.source()))
                    })?;
                    vm.rekey_staged_input(&shape, &mut staged)?;
                    Completion::Staged(staged)
                }
                None => unreachable!("waiting request checked above"),
            },
            flow => return Ok(Some(flow)),
        };
        self.receive(vm, environment, completion)?;
        Ok(None)
    }
}
