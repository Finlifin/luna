//! Exclusive execution of one ready Type producer. Its output is unpublished
//! until the invocation completes; cancellation is terminal and cannot replay it.
use super::*;
use comptime_interpretor::{
    AssociatedHostRequest, AssociatedHostResponse, DefinitionHostRequest, DefinitionHostResponse,
    InvocationProgress, InvocationSession, RejectedDefinitionResponse, RejectedHostResponse,
};

/// Dependency waits and terminal results do not create an execution driver.
pub enum Start<'session, 'world> {
    Settled(&'session State),
    Invocation(Invocation<'session, 'world>),
}

/// Waiting owns an outstanding host request; polling it never reissues work.
pub enum Progress<'session, 'world> {
    Running,
    Waiting,
    Associated(AssociatedHostRequest<'world>),
    Definition(DefinitionHostRequest<'world>),
    Settled(&'session State),
}

/// Owns one output's publication permit, without borrowing inference storage
/// while the VM is suspended. Unrelated inference can progress between polls.
pub struct Invocation<'session, 'world> {
    driver: Option<InvocationSession<'session, 'world>>,
    context: u64,
    permit: type_resolution::AssignmentPermit,
    pending: &'session mut Pending,
    failed: &'session mut BTreeSet<MetaId>,
    output: MetaId,
    terminal: bool,
}

impl Invocation<'_, '_> {
    fn settle(&mut self, result: Result<Term, Error>) {
        self.pending.state = match result {
            Ok(ty) => State::Complete(ty),
            Err(error) => {
                self.failed.insert(self.output);
                State::Failed(error)
            }
        };
        self.terminal = true;
        self.driver.take();
    }

    fn publish(&mut self, context: &mut InferenceContext, value: Value) -> Result<Term, Error> {
        let driver = self.driver.as_mut().expect("active Type invocation");
        publish(
            context,
            &self.permit,
            value,
            driver.remaining_steps(),
            |work| driver.charge_preparation(work),
        )
    }

    pub fn cancel(&mut self) {
        if !self.terminal {
            self.settle(Err(Error::Cancelled));
        }
    }
}

impl<'world> Invocation<'_, 'world> {
    /// Wrong-context polls leave the execution and its outstanding request
    /// untouched. A committed candidate may change unrelated inference inputs.
    pub fn poll(&mut self, context: &mut InferenceContext) -> Result<Progress<'_, 'world>, Error> {
        if context.is_trial() {
            return Err(Error::SpeculativeContext);
        }
        if context.identity() != self.context {
            return Err(Error::ForeignContext);
        }
        if self.terminal {
            return Ok(Progress::Settled(&self.pending.state));
        }
        match self.driver.as_mut().expect("active Type invocation").poll() {
            Ok(InvocationProgress::Running) => return Ok(Progress::Running),
            Ok(InvocationProgress::Waiting) => return Ok(Progress::Waiting),
            Ok(InvocationProgress::Associated(request)) => {
                return Ok(Progress::Associated(request));
            }
            Ok(InvocationProgress::Definition(request)) => {
                return Ok(Progress::Definition(request));
            }
            Ok(InvocationProgress::Complete(value)) => {
                let result = self.publish(context, value);
                self.settle(result);
            }
            Err(error) => self.settle(Err(Error::Invocation(error))),
        }
        Ok(Progress::Settled(&self.pending.state))
    }

    pub fn resume_definition(
        &mut self,
        response: DefinitionHostResponse<'world>,
    ) -> Result<(), RejectedDefinitionResponse<'world>> {
        if let Some(driver) = &mut self.driver {
            driver.resume_definition(response)
        } else {
            Err(response.reject())
        }
    }

    pub fn resume(
        &mut self,
        response: AssociatedHostResponse<'world>,
    ) -> Result<(), RejectedHostResponse<'world>> {
        // A terminal wrapper no longer owns a waiter. Return late responses
        // intact instead of accepting them or refunding their reserved fuel.
        if let Some(driver) = &mut self.driver {
            driver.resume(response)
        } else {
            Err(response.reject())
        }
    }
}

impl Drop for Invocation<'_, '_> {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl fmt::Display for Invocation<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(type-invocation (output {}) {} ",
            self.output, self.permit
        )?;
        match &self.driver {
            Some(driver) => write!(f, "{driver}")?,
            None => write!(f, "{}", self.pending.state)?,
        }
        f.write_str(")")
    }
}

pub(super) fn publish(
    context: &mut InferenceContext,
    permit: &type_resolution::AssignmentPermit,
    value: Value,
    remaining: u64,
    charge: impl FnOnce(usize) -> Result<(), comptime_interpretor::Error>,
) -> Result<Term, Error> {
    let ty = match value {
        Value::Type(ty) => ty,
        Value::TypeParameter(declaration) => Term::Rigid {
            declaration,
            sort: Sort::Type,
        },
        _ => return Err(Error::NotAType),
    };
    let mut fuel = usize::try_from(remaining).unwrap_or(usize::MAX);
    let before = fuel;
    let bounded = nir::type_computations::charge_term(&ty, &mut fuel);
    charge(before - fuel).map_err(Error::Invocation)?;
    bounded.map_err(|_| Error::Inference(InferenceError::Budget))?;
    context.assign_reserved(permit, InferenceTerm::Stable(ty.clone()))?;
    Ok(ty)
}

pub(super) fn arguments(
    vm: &mut Interpreter<'_>,
    pending: &mut Pending,
    context: &InferenceContext,
) -> Result<Vec<(Term, InvocationValue)>, Error> {
    let mut fuel = usize::try_from(vm.remaining_steps()).unwrap_or(usize::MAX);
    let before = fuel;
    match &mut pending.inputs {
        PendingInputs::Symbolic(inputs) => {
            let arguments = inputs
                .iter()
                .map(|input| {
                    Ok((
                        context.freeze_metered(&input.ty, &mut fuel)?,
                        InvocationValue::Normal(context.freeze_metered(&input.value, &mut fuel)?),
                    ))
                })
                .collect::<Result<Vec<_>, InferenceError>>();
            vm.charge_preparation(before - fuel)
                .map_err(Error::Invocation)?;
            Ok(arguments?)
        }
        PendingInputs::Prepared { domains, values } => {
            let domains = domains
                .iter()
                .map(|ty| context.freeze_metered(ty, &mut fuel))
                .collect::<Result<Vec<_>, _>>();
            vm.charge_preparation(before - fuel)
                .map_err(Error::Invocation)?;
            let domains = domains?;
            let values = values.take().ok_or(Error::ConsumedInputs)?;
            Ok(domains.into_iter().zip(values).collect())
        }
    }
}

pub(super) fn begin<'s, 'w>(
    vm: &'s mut Interpreter<'w>,
    context: &mut InferenceContext,
    pending: &'s mut Pending,
    failed: &'s mut BTreeSet<MetaId>,
    output: MetaId,
) -> Start<'s, 'w> {
    let arguments = (|| {
        let permit = context.reserve_assignment(output)?;
        Ok((permit, arguments(vm, pending, context)?))
    })();
    let prepared = arguments.and_then(|(permit, arguments)| {
        vm.start_prepared_invocation(pending.target, arguments)
            .map(|driver| (permit, driver))
            .map_err(Error::Invocation)
    });
    match prepared {
        Ok((permit, driver)) => Start::Invocation(Invocation {
            driver: Some(driver),
            context: context.identity(),
            permit,
            pending,
            failed,
            output,
            terminal: false,
        }),
        Err(error) => {
            pending.state = State::Failed(error);
            failed.insert(output);
            Start::Settled(&pending.state)
        }
    }
}

pub(super) fn drive(
    vm: &mut Interpreter<'_>,
    context: &mut InferenceContext,
    pending: &mut Pending,
    failed: &mut BTreeSet<MetaId>,
    output: MetaId,
    resolve: &mut dyn FnMut(
        &nir::program::Expr,
        &mut u64,
    ) -> Result<
        comptime_interpretor::AssociatedSelection,
        comptime_interpretor::Error,
    >,
) {
    let Start::Invocation(mut execution) = begin(vm, context, pending, failed, output) else {
        return;
    };
    loop {
        let progress = match execution.poll(context) {
            Ok(progress) => progress,
            Err(error) => {
                execution.settle(Err(error));
                return;
            }
        };
        match progress {
            Progress::Running => {}
            Progress::Associated(request) => {
                let response = request.resolve(&mut *resolve);
                if let Err(rejected) = execution.resume(response) {
                    execution.settle(Err(Error::Invocation(rejected.error().clone())));
                }
            }
            Progress::Definition(request) => {
                if let Err(rejected) = execution.resume_definition(request.prepare()) {
                    execution.settle(Err(Error::Invocation(rejected.error().clone())));
                }
            }
            Progress::Waiting => {
                execution.settle(Err(Error::Invocation(comptime_interpretor::Error {
                    kind: comptime_interpretor::ErrorKind::InvalidControlFlow,
                    source: None,
                })));
            }
            Progress::Settled(_) => return,
        }
    }
}
