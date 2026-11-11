//! Exclusive execution of one ready Type producer. Its output is unpublished
//! until the invocation completes; cancellation is terminal and cannot replay it.
use super::*;
use comptime_interpretor::{
    AssociatedHostRequest, AssociatedHostResponse, InvocationProgress, InvocationSession,
    RejectedHostResponse,
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
    Settled(&'session State),
}

/// Owns this producer's publication authority until completion or cancellation.
/// The inference context is exclusively borrowed; this is not yet a frontend
/// region that permits other inference work while an invocation is suspended.
pub struct Invocation<'session, 'world> {
    driver: Option<InvocationSession<'session, 'world>>,
    context: &'session mut InferenceContext,
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

    fn publish(&mut self, value: Value) -> Result<Term, Error> {
        let driver = self.driver.as_mut().expect("active Type invocation");
        publish(
            self.context,
            self.output,
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
    pub fn poll(&mut self) -> Progress<'_, 'world> {
        if self.terminal {
            return Progress::Settled(&self.pending.state);
        }
        match self.driver.as_mut().expect("active Type invocation").poll() {
            Ok(InvocationProgress::Running) => return Progress::Running,
            Ok(InvocationProgress::Waiting) => return Progress::Waiting,
            Ok(InvocationProgress::Associated(request)) => return Progress::Associated(request),
            Ok(InvocationProgress::Complete(value)) => {
                let result = self.publish(value);
                self.settle(result);
            }
            Err(error) => self.settle(Err(Error::Invocation(error))),
        }
        Progress::Settled(&self.pending.state)
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
        write!(f, "(type-invocation (output {}) ", self.output)?;
        match &self.driver {
            Some(driver) => write!(f, "{driver}")?,
            None => write!(f, "{}", self.pending.state)?,
        }
        f.write_str(")")
    }
}

pub(super) fn publish(
    context: &mut InferenceContext,
    output: MetaId,
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
    context.assign(output, InferenceTerm::Stable(ty.clone()))?;
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
    context: &'s mut InferenceContext,
    pending: &'s mut Pending,
    failed: &'s mut BTreeSet<MetaId>,
    output: MetaId,
) -> Start<'s, 'w> {
    let arguments = (|| {
        if context.is_assigned(output)? {
            return Err(Error::Inference(InferenceError::AlreadyAssigned(output)));
        }
        arguments(vm, pending, context)
    })();
    let prepared = arguments.and_then(|arguments| {
        vm.start_prepared_invocation(pending.target, arguments)
            .map_err(Error::Invocation)
    });
    match prepared {
        Ok(driver) => Start::Invocation(Invocation {
            driver: Some(driver),
            context,
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
        match execution.poll() {
            Progress::Running => {}
            Progress::Associated(request) => {
                let response = request.resolve(&mut *resolve);
                if let Err(rejected) = execution.resume(response) {
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
