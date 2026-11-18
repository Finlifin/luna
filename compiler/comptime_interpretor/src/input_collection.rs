//! One invocation-local scheduler for ordered input execution and domain wakeup.
//! Providers retain their own admission and final construction semantics.
use super::*;
use comptime_compiler::declaration_inputs::PreparationSite;
pub(super) use comptime_compiler::declaration_inputs::{PlanView, Target};
use nir::program::NormalFormArgument as Argument;
use normal_inputs::StagedNormalInput;

#[derive(Clone)]
pub(super) enum Plan<'e> {
    Borrowed(PlanView<'e>),
    Compiled {
        plan: Arc<comptime_compiler::DeclarationInputPlan>,
        register_collision: bool,
    },
}
impl<'e> Plan<'e> {
    pub(super) fn borrowed(expr: &'e Expr) -> Self {
        Self::Borrowed(PlanView::from_expression(expr).expect("declaration inputs"))
    }

    pub(super) fn view(&self) -> PlanView<'_> {
        match self {
            Self::Borrowed(view) => *view,
            Self::Compiled { plan, .. } => plan.view(),
        }
    }

    fn preparation(&self, index: usize) -> PreparationSite<'_> {
        match self {
            Self::Borrowed(view) => PreparationSite::borrowed(&view.schedule.preparations[index]),
            Self::Compiled { plan, .. } => plan.preparation(index),
        }
    }

    pub(super) fn inputs_diverge(&self) -> bool {
        match self {
            Self::Borrowed(view) => view.inputs_diverge(),
            Self::Compiled { plan, .. } => plan.inputs_diverge(),
        }
    }

    /// Acquire only locals absent from the caller, through alias body completion.
    pub(super) fn admit_scope(
        &self,
        vm: &mut Interpreter<'_>,
        environment: &Environment,
    ) -> Result<Arc<[LocalId]>, Error> {
        let source = Some(self.view().source);
        let locals: Arc<[LocalId]> = match self {
            Self::Compiled {
                plan,
                register_collision,
            } => {
                if *register_collision {
                    return Err(vm.failure(ErrorKind::Type, source));
                }
                plan.locals().clone()
            }
            Self::Borrowed(view) => view
                .schedule
                .check(view.arguments.len(), |n| {
                    vm.integer_operation(source, |b| b.consume(n))
                })?
                .ok_or_else(|| vm.failure(ErrorKind::Type, source))?
                .locals
                .into(),
        };
        vm.integer_operation(source, |b| b.consume(locals.len()))?;
        if locals.iter().any(|id| environment.contains_key(id)) {
            return Err(vm.failure(ErrorKind::Type, source));
        }
        Ok(locals)
    }
}

pub(super) trait Source: fmt::Display {
    fn plan(&self) -> &Plan<'_>;
    fn source(&self) -> SourceRange {
        self.plan().view().source
    }
    fn inputs(&self) -> (&[Argument], nir::program::InputSchedule<'_>) {
        let view = self.plan().view();
        (view.arguments, view.schedule)
    }
    fn input(&self, slot: usize) -> &Argument {
        &self.inputs().0[slot]
    }
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
    fn bind(&mut self, vm: &mut Interpreter<'_>, slot: usize, value: Value) -> Result<(), Error>;
    fn complete(&mut self, vm: &mut Interpreter<'_>) -> Result<Progress, Error>;
    fn cleanup(&self, environment: &mut Environment);
}

struct Pending {
    staged: StagedNormalInput,
    awaiting: BTreeSet<usize>,
}

enum State {
    Ready,
    Waiting(Waiting),
    Stopped,
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
    DomainDelivered,
    Preparation(PreparationSite<'a>),
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
    /// One previously staged operand has received its completed domain.
    DomainDelivered,
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
    state: State,
    pending: BTreeMap<usize, Pending>,
    dependents: BTreeMap<usize, BTreeSet<usize>>,
    ready: BTreeSet<usize>,
    last_delivery: Option<usize>,
}

impl<S: Source> fmt::Display for Frame<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(input-collection-frame (position {}) (preparation {}) (terminal {}) {} (waiting",
            self.position,
            self.preparation,
            matches!(self.state, State::Stopped),
            self.source
        )?;
        match &self.state {
            State::Waiting(Waiting::Preparation(index)) => write!(f, " (preparation {index})")?,
            State::Waiting(Waiting::Ready(slot)) => write!(f, " (ready {slot})")?,
            State::Waiting(Waiting::Stage { slot, awaiting }) => {
                write!(f, " (stage {slot} (awaiting")?;
                for dependency in awaiting {
                    write!(f, " {dependency}")?;
                }
                f.write_str("))")?;
            }
            State::Ready | State::Stopped => {}
        }
        f.write_str(") (ready")?;
        for slot in &self.ready {
            write!(f, " {slot}")?;
        }
        f.write_str(") (last-delivery")?;
        if let Some(slot) = self.last_delivery {
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
            state: State::Ready,
            pending: BTreeMap::new(),
            dependents: BTreeMap::new(),
            ready: BTreeSet::new(),
            last_delivery: None,
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

    // The declaration policy admits the actual operand. Only after admission
    // can a later producer read its lexical snapshot through the slot binder.
    fn bind(
        &mut self,
        vm: &mut Interpreter<'_>,
        env: &mut Environment,
        slot: usize,
        value: Value,
    ) -> Result<(), Error> {
        let exported = match self.source.inputs().1.bindings {
            Some(ids) => Some((
                ids[slot],
                vm.checkpoint_representation(&value, self.source.input(slot).source())?,
            )),
            None => None,
        };
        self.source.bind(vm, slot, value)?;
        if let Some((id, snapshot)) = exported {
            env.insert(id, snapshot);
        }
        self.completed(vm, slot)
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

    // Newly ready higher slots join the current declaration-order round.
    // Wrap only after it is exhausted, without moving any of the other entries.
    fn next_ready(&mut self) -> Option<usize> {
        use std::ops::Bound::{Excluded, Unbounded};
        let next = self
            .last_delivery
            .and_then(|last| self.ready.range((Excluded(last), Unbounded)).next())
            .or_else(|| self.ready.first())
            .copied();
        if let Some(slot) = next {
            self.ready.remove(&slot);
        }
        self.last_delivery = next;
        next
    }

    /// Resume the selected input against this activation's declaration bindings.
    /// Its source effects have already run. Publish its saved value only after
    /// all domain annotations and literal ranges have passed checking.
    fn deliver_domain(
        &mut self,
        vm: &mut Interpreter<'_>,
        env: &mut Environment,
        slot: usize,
    ) -> Result<(), Error> {
        let expected = self.source.expected(vm, slot)?;
        let operand = self.source.input(slot);
        let bindings = self.source.substitutions();
        self.source
            .check_input(vm, slot, operand, bindings, Some(&expected))?;
        let checked =
            vm.prepare_input_domain(input_domains::Node::Source(operand, bindings), &expected, 0)?;
        let mut literals = BTreeMap::new();
        checked.deliver_literals(&mut literals);
        let pending = self.pending.remove(&slot).expect("awakened input");
        let value =
            vm.finish_staged_normal_argument(pending.staged, &mut literals, operand.source(), 0)?;
        self.bind(vm, env, slot, value)?;
        Ok(())
    }

    pub(super) fn advance(
        &mut self,
        vm: &mut Interpreter<'_>,
        env: &mut Environment,
    ) -> Result<Request<'_>, Error> {
        if !matches!(self.state, State::Ready) {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source.source())));
        }
        // A semantic failure ends this frame. It must never replay a producer.
        self.state = State::Stopped;
        if let Some(slot) = self.next_ready() {
            vm.tick(Some(self.source.source()))?;
            self.deliver_domain(vm, env, slot)?;
            self.state = State::Ready;
            return Ok(Request::DomainDelivered);
        }
        let (arguments, schedule) = self.source.inputs();
        let request = if let Some(preparation) = schedule.preparations.get(self.preparation)
            && preparation.before == self.position
        {
            self.state = State::Waiting(Waiting::Preparation(self.preparation));
            Request::Preparation(self.source.plan().preparation(self.preparation))
        } else if self.position < arguments.len() {
            let slot = schedule
                .order
                .map_or(self.position, |order| order[self.position]);
            let operand = self.source.operand(vm, slot)?;
            self.source.check_input(vm, slot, &operand, None, None)?;
            let awaiting = self.source.pending_inputs(vm, slot)?;
            if awaiting.is_empty() {
                let expected = self.source.expected(vm, slot)?;
                self.source
                    .check_input(vm, slot, &operand, None, Some(&expected))?;
                self.state = State::Waiting(Waiting::Ready(slot));
                Request::Ready(operand, expected)
            } else {
                self.state = State::Waiting(Waiting::Stage { slot, awaiting });
                Request::Stage(operand)
            }
        } else {
            return Ok(Request::Complete);
        };
        Ok(request)
    }

    pub(super) fn receive(
        &mut self,
        vm: &mut Interpreter<'_>,
        env: &mut Environment,
        completion: Completion,
    ) -> Result<(), Error> {
        let state = std::mem::replace(&mut self.state, State::Stopped);
        match (state, completion) {
            (State::Waiting(Waiting::Preparation(_)), Completion::Prepared) => {
                self.preparation += 1
            }
            (State::Waiting(Waiting::Ready(slot)), Completion::Value(value)) => {
                self.bind(vm, env, slot, value)?;
                self.position += 1;
            }
            (State::Waiting(Waiting::Stage { slot, awaiting }), Completion::Staged(staged)) => {
                self.watch(vm, slot, Pending { staged, awaiting })?;
                self.position += 1;
            }
            (state, _) => {
                self.state = state;
                return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source.source())));
            }
        }
        self.state = State::Ready;
        Ok(())
    }

    fn next(&mut self, vm: &mut Interpreter<'_>, env: &mut Environment) -> Result<Progress, Error> {
        loop {
            let completion = match self.advance(vm, env)? {
                Request::DomainDelivered => return Ok(Progress::DomainDelivered),
                Request::Preparation(preparation) => {
                    if let Some(frame) = vm.start_input_preparation(preparation, env)? {
                        return Ok(Progress::Input(frame));
                    }
                    match vm.eval(preparation.expression(), env)? {
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
        self.state = State::Stopped;
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
        if !matches!(&result, Ok(Progress::Input(_) | Progress::DomainDelivered)) {
            self.state = State::Stopped;
        }
        result
    }

    fn accept_input(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
        result: bytecode::InputResult,
    ) -> Result<Option<Flow>, Error> {
        let State::Waiting(waiting) = &self.state else {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source.source())));
        };
        *environment = result.environment;
        let value = match result.flow {
            Ok(Flow::Value(value)) => value,
            other => {
                self.state = State::Stopped;
                return other.map(Some);
            }
        };
        let completion = match waiting {
            Waiting::Preparation(_) => Completion::Prepared,
            Waiting::Ready(_) => Completion::Value(value),
            Waiting::Stage { .. } => {
                let Some(staged) = result.staged else {
                    self.state = State::Stopped;
                    return Err(
                        vm.failure(ErrorKind::InvalidControlFlow, Some(self.source.source()))
                    );
                };
                Completion::Staged(staged)
            }
        };
        self.receive(vm, environment, completion)?;
        Ok(None)
    }
}
