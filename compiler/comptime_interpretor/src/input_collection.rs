//! One invocation-local scheduler for ordered input execution and domain wakeup.
//! Providers retain their own admission and final construction semantics.
use super::*;
use nir::program::{NormalFormArgument as Argument, NormalFormPreparation};
use normal_inputs::StagedNormalInput;

/// Acquire only locals that are absent from the caller. Keep this list until
/// normal completion or cancellation, including while an alias body executes.
pub(super) fn admit_scope(
    vm: &mut Interpreter<'_>,
    expression: &Expr,
    environment: &Environment,
) -> Result<Vec<LocalId>, Error> {
    let (arguments, schedule) = expression
        .kind
        .declaration_inputs()
        .expect("declaration inputs");
    let source = Some(expression.source);
    let checked = schedule
        .check(arguments.len(), |n| {
            vm.integer_operation(source, |b| b.consume(n))
        })?
        .ok_or_else(|| vm.failure(ErrorKind::Type, source))?;
    if checked.locals.iter().any(|id| environment.contains_key(id)) {
        return Err(vm.failure(ErrorKind::Type, source));
    }
    Ok(checked.locals)
}

pub(super) trait Source: fmt::Display {
    fn source(&self) -> SourceRange;
    fn preparation(&self, index: usize) -> Option<&NormalFormPreparation>;
    fn slot(&self, position: usize) -> Option<usize>;
    fn input(&self, slot: usize) -> &Argument;
    fn substitutions(&self) -> Option<&input_bindings::Bindings>;

    // Specialize executable code for its source position. Waking the input
    // later only observes domain annotations from the original template.
    fn operand(
        &self,
        vm: &mut Interpreter<'_>,
        slot: usize,
    ) -> Result<std::borrow::Cow<'_, Argument>, Error> {
        let template = self.input(slot);
        match self.substitutions() {
            Some(bindings) => bindings
                .specialized(vm, template)
                .map(std::borrow::Cow::Owned),
            None => Ok(std::borrow::Cow::Borrowed(template)),
        }
    }
    fn pending_inputs(
        &self,
        vm: &mut Interpreter<'_>,
        slot: usize,
    ) -> Result<BTreeSet<usize>, Error>;
    fn expected(&self, vm: &mut Interpreter<'_>, slot: usize) -> Result<Term, Error>;
    fn check_input(
        &self,
        vm: &mut Interpreter<'_>,
        slot: usize,
        operand: &Argument,
        bindings: Option<&input_bindings::Bindings>,
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

struct Pending {
    staged: StagedNormalInput,
    awaiting: BTreeSet<usize>,
}

enum Waiting {
    Preparation(usize),
    Ready(usize),
    Stage {
        slot: usize,
        awaiting: BTreeSet<usize>,
    },
}

pub(super) enum Request<'a> {
    Preparation(&'a NormalFormPreparation),
    Ready(std::borrow::Cow<'a, Argument>, Term),
    Stage(std::borrow::Cow<'a, Argument>),
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

pub(super) trait InputPlan: fmt::Display {
    fn cleanup(&self, environment: &mut Environment);
    fn cancel(&mut self, environment: &mut Environment);
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
    pending: BTreeMap<usize, Pending>,
    dependents: BTreeMap<usize, BTreeSet<usize>>,
    ready: BTreeSet<usize>,
    terminal: bool,
}

impl<S: Source> fmt::Display for Frame<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(input-collection-frame (position {}) (preparation {}) (terminal {}) {} (waiting",
            self.position, self.preparation, self.terminal, self.source
        )?;
        match &self.waiting {
            Some(Waiting::Preparation(index)) => write!(f, " (preparation {index})")?,
            Some(Waiting::Ready(slot)) => write!(f, " (ready {slot})")?,
            Some(Waiting::Stage { slot, awaiting }) => {
                write!(f, " (stage {slot} (awaiting")?;
                for dependency in awaiting {
                    write!(f, " {dependency}")?;
                }
                f.write_str("))")?;
            }
            None => {}
        }
        f.write_str(") (ready")?;
        for slot in &self.ready {
            write!(f, " {slot}")?;
        }
        f.write_str(") (pending")?;
        for (slot, pending) in &self.pending {
            write!(f, " (input {slot} (awaiting")?;
            for dependency in &pending.awaiting {
                write!(f, " {dependency}")?;
            }
            f.write_str("))")?;
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
            dependents: BTreeMap::new(),
            ready: BTreeSet::new(),
            terminal: false,
        }
    }

    fn watch(
        &mut self,
        vm: &mut Interpreter<'_>,
        slot: usize,
        pending: Pending,
    ) -> Result<(), Error> {
        vm.integer_operation(Some(self.source.source()), |budget| {
            budget.consume(pending.awaiting.len())
        })?;
        for dependency in &pending.awaiting {
            self.dependents.entry(*dependency).or_default().insert(slot);
        }
        self.pending.insert(slot, pending);
        Ok(())
    }

    fn completed(&mut self, vm: &mut Interpreter<'_>, slot: usize) -> Result<(), Error> {
        if let Some(dependents) = self.dependents.remove(&slot) {
            vm.integer_operation(Some(self.source.source()), |budget| {
                budget.consume(dependents.len())
            })?;
            for dependent in dependents {
                let pending = self
                    .pending
                    .get_mut(&dependent)
                    .expect("registered dependent");
                pending.awaiting.remove(&slot);
                if pending.awaiting.is_empty() {
                    self.ready.insert(dependent);
                }
            }
        }
        Ok(())
    }

    fn drain(&mut self, vm: &mut Interpreter<'_>, env: &mut Environment) -> Result<(), Error> {
        while !self.ready.is_empty() {
            // Keep declaration-order rounds. A binding completed in this round
            // can wake an earlier slot for the next round, without replaying
            // producers or changing the source-order execution cursor.
            let mut round = std::mem::take(&mut self.ready);
            while let Some(slot) = round.pop_first() {
                let pending = self.pending.remove(&slot).expect("awakened input");
                vm.tick(Some(self.source.source()))?;
                let expected = self.source.expected(vm, slot)?;
                let argument = self.source.input(slot);
                let bindings = self.source.substitutions();
                self.source
                    .check_input(vm, slot, argument, bindings, Some(&expected))?;
                let prepared = vm.prepare_input_domain(
                    input_domains::Node::Source(argument, bindings),
                    &expected,
                    0,
                )?;
                let mut literals = BTreeMap::new();
                prepared.deliver_literals(&mut literals);
                let value = vm.finish_staged_normal_argument(
                    pending.staged,
                    &mut literals,
                    argument.source(),
                    0,
                )?;
                self.source.bind(vm, env, slot, value)?;
                self.completed(vm, slot)?;
                // Newly affected later slots belong to this ordered round;
                // earlier slots are revisited in the next one.
                round.append(&mut self.ready.split_off(&slot.saturating_add(1)));
            }
        }
        Ok(())
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
        self.drain(vm, env)?;
        let request = if let Some(preparation) = self.source.preparation(self.preparation)
            && preparation.before == self.position
        {
            self.waiting = Some(Waiting::Preparation(self.preparation));
            Request::Preparation(preparation)
        } else if let Some(slot) = self.source.slot(self.position) {
            let operand = self.source.operand(vm, slot)?;
            self.source.check_input(vm, slot, &operand, None, None)?;
            let awaiting = self.source.pending_inputs(vm, slot)?;
            if awaiting.is_empty() {
                let expected = self.source.expected(vm, slot)?;
                self.source
                    .check_input(vm, slot, &operand, None, Some(&expected))?;
                self.waiting = Some(Waiting::Ready(slot));
                Request::Ready(operand, expected)
            } else {
                self.waiting = Some(Waiting::Stage { slot, awaiting });
                Request::Stage(operand)
            }
        } else {
            return Ok(Request::Complete);
        };
        self.terminal = false;
        Ok(request)
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
                | (Some(Waiting::Ready(_)), Completion::Value(_))
                | (Some(Waiting::Stage { .. }), Completion::Staged(_))
        ) {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source.source())));
        }
        self.terminal = true;
        match (self.waiting.take().expect("completed request"), completion) {
            (Waiting::Preparation(_), Completion::Prepared) => self.preparation += 1,
            (Waiting::Ready(slot), Completion::Value(value)) => {
                self.source.bind(vm, env, slot, value)?;
                self.completed(vm, slot)?;
                self.position += 1;
            }
            (Waiting::Stage { slot, awaiting }, Completion::Staged(staged)) => {
                self.watch(vm, slot, Pending { staged, awaiting })?;
                self.position += 1;
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
                    if let Some(frame) = vm.start_contextual_input(&argument, &expected, env)? {
                        return Ok(Progress::Input(frame));
                    }
                    match vm.interpret_contextual_input(&argument, &expected, env)? {
                        Flow::Value(value) => Completion::Value(value),
                        flow => return Ok(Progress::Complete(flow)),
                    }
                }
                Request::Stage(argument) => {
                    if let Some(frame) = vm.start_staged_input(&argument, env, 0)? {
                        return Ok(Progress::Input(frame));
                    }
                    match vm.interpret_staged_argument(&argument, env, 0)? {
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

#[cfg(test)]
mod tests;

impl<S: Source> InputPlan for Frame<S> {
    fn cancel(&mut self, environment: &mut Environment) {
        self.terminal = true;
        self.source.cleanup(environment);
    }

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
                Some(Waiting::Ready(_)) => Completion::Value(value),
                Some(Waiting::Stage { .. }) => {
                    let staged = result.staged.ok_or_else(|| {
                        vm.failure(ErrorKind::InvalidControlFlow, Some(self.source.source()))
                    })?;
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
