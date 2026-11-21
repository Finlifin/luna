//! An exclusively borrowed invocation with owned selection and definition messages.
//! Call admission and other host services are still synchronous. This interface is
//! not a general frontend-region suspension or a cross-process wire protocol.
use super::*;
use crate::request_identity::Ticket;

/// Sent and returned by value. Only the unspent allowance can be changed
/// by the host. The waiting VM retains its ticket and execution continuation.
pub(super) struct HostLease<'a> {
    pub(super) ticket: Ticket,
    pub(super) world: &'a Package,
    pub(super) granted: u64,
    pub(super) remaining: u64,
}

impl<'a> HostLease<'a> {
    pub(super) fn reserve(ticket: Ticket, world: &'a Package, steps: &mut u64) -> Self {
        let granted = std::mem::take(steps);
        Self {
            ticket,
            world,
            granted,
            remaining: granted,
        }
    }

    pub(super) fn settle<T>(
        self,
        steps: &mut u64,
        source: SourceRange,
        result: Result<T, Error>,
    ) -> Result<T, Error> {
        if self.remaining > self.granted {
            *steps = 0;
            return Err(Error {
                kind: ErrorKind::StepsExhausted,
                source: Some(source),
            });
        }
        *steps = self.remaining;
        result
    }
}

/// A source comptime invocation. Dropping it cancels and restores its caller.
pub struct InvocationSession<'vm, 'world> {
    pub(super) session: Session<'vm, 'world, 'static>,
}

/// A normal return after the callee's exit checks. The result type is the
/// admitted signature result with this invocation's ordinary inputs bound.
#[derive(Debug)]
pub struct InvocationResult {
    pub value: Value,
    pub result_type: Term,
}

impl fmt::Display for InvocationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(invocation-result (type {}) (value {}))",
            self.result_type, self.value
        )
    }
}

/// Identifies one dynamic call under preparation, including across nested
/// calls at the same source location. Its lifetime is independent of the
/// frame; retaining the token does not retain the callee or its values.
#[derive(Clone, Debug)]
pub struct OperandCall(Arc<()>);

impl PartialEq for OperandCall {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for OperandCall {}

/// A bytecode call has accepted one ordinary input. The session lends the slot
/// until its next transition. Freezing is optional and uses this VM's scope and
/// remaining work allowance. The value itself remains in its original slot.
pub struct DeliveredOperand<'session, 'world> {
    pub source: SourceRange,
    pub parameter: usize,
    pub(crate) callee: &'session Value,
    pub(crate) value: &'session Value,
    pub(crate) identity: &'session mut Option<OperandCall>,
    pub(crate) vm: &'session mut Interpreter<'world>,
}

impl DeliveredOperand<'_, '_> {
    /// Only consumers correlating several deliveries need to allocate a token.
    pub fn call(&mut self) -> OperandCall {
        self.identity
            .get_or_insert_with(|| OperandCall(Arc::new(())))
            .clone()
    }

    /// The named target when the evaluated callee carries a declaration.
    pub fn named_callee(&self) -> Option<(DeclarationId, &[Term])> {
        match self.callee {
            Value::Function(id) => Some((*id, &[])),
            other => other.declaration_instance(),
        }
    }

    /// Export ordinary data for source checking. Description fields retain
    /// their private telescope. Invocation-local references cannot escape.
    pub fn freeze_data(self) -> Result<nir::described_applications::DataArgument, Error> {
        self.vm.freeze_normal_argument(self.value, self.source)
    }
}

pub enum OperandProgress<'session, 'world> {
    Supplied(DeliveredOperand<'session, 'world>),
    Invocation(InvocationProgress<'world, InvocationResult>),
}

/// Running made internal progress; Waiting has an outstanding owned request.
pub enum InvocationProgress<'world, T = Value> {
    Running,
    Waiting,
    Associated(AssociatedHostRequest<'world>),
    Definition(DefinitionHostRequest<'world>),
    Method(MethodHostRequest<'world>),
    Complete(T),
}

/// Resolve against this immutable source package and the compiler's matching
/// semantic context. The allowance belongs to this request until delivery.
/// Ordinary VM state, local storage and mutable inference are not exposed.
/// Dropping a request leaves its session waiting; cancel or drop that session
/// rather than polling for a response that can no longer arrive.
pub struct AssociatedHostRequest<'world> {
    pub(super) lease: HostLease<'world>,
    pub(super) request: associated_execution::Request,
    pub(super) resolver: Option<&'world AssociatedResolver<'world>>,
}

/// A linear answer, still subject to the originating session's admission.
pub struct AssociatedHostResponse<'world> {
    pub(super) lease: HostLease<'world>,
    pub(super) request: associated_execution::Request,
    pub(super) selection: Result<AssociatedSelection, Error>,
}

/// Invalid delivery does not consume the current waiter or the other answer.
pub struct RejectedHostResponse<'world> {
    error: Error,
    response: AssociatedHostResponse<'world>,
}

impl<'world> AssociatedHostRequest<'world> {
    pub fn access(&self) -> &Expr {
        self.request.access()
    }
    pub fn source_world(&self) -> &'world Package {
        self.lease.world
    }
    pub fn remaining_steps(&self) -> u64 {
        self.lease.remaining
    }
    pub fn remaining_steps_mut(&mut self) -> &mut u64 {
        &mut self.lease.remaining
    }

    pub fn respond(
        self,
        selection: Result<AssociatedSelection, Error>,
    ) -> AssociatedHostResponse<'world> {
        AssociatedHostResponse {
            lease: self.lease,
            request: self.request,
            selection,
        }
    }

    /// Inspect the access and debit this request while retaining the ticket.
    pub fn query<T>(&mut self, query: impl FnOnce(&Expr, &mut u64) -> T) -> T {
        query(self.request.access(), &mut self.lease.remaining)
    }

    pub fn resolve(
        mut self,
        resolver: impl FnOnce(&Expr, &mut u64) -> Result<AssociatedSelection, Error>,
    ) -> AssociatedHostResponse<'world> {
        let selection = self.query(resolver);
        self.respond(selection)
    }

    /// Resolve through the originating world's configured service. Preparing
    /// does not deliver the answer or publish a value in the waiting VM.
    pub fn prepare(self) -> AssociatedHostResponse<'world> {
        let resolver = self.resolver;
        self.resolve(|access, fuel| {
            resolver.ok_or(Error {
                kind: ErrorKind::UnmaterializedDispatch,
                source: Some(access.source),
            })?(access, fuel)
        })
    }
}

impl<'world> AssociatedHostResponse<'world> {
    /// Return an undeliverable response intact without touching any VM waiter.
    pub fn reject(self) -> RejectedHostResponse<'world> {
        RejectedHostResponse::new(self)
    }
}

impl<'world> RejectedHostResponse<'world> {
    pub(super) fn new(response: AssociatedHostResponse<'world>) -> Self {
        Self {
            error: Error {
                kind: ErrorKind::InvalidControlFlow,
                source: Some(response.request.access().source),
            },
            response,
        }
    }
    pub fn error(&self) -> &Error {
        &self.error
    }
    pub fn into_response(self) -> AssociatedHostResponse<'world> {
        self.response
    }
}

impl<'world> Interpreter<'world> {
    /// Start with the same domain admission and graph transport as synchronous
    /// prepared invocation. Argument factories and admission are still synchronous.
    pub fn start_prepared_invocation<'vm>(
        &'vm mut self,
        function: DeclarationId,
        arguments: Vec<(Term, InvocationValue)>,
    ) -> Result<InvocationSession<'vm, 'world>, Error> {
        if self.graph.is_some() || !self.bytecode_invocation_available() {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        let values = self.prepare_invocation_arguments(arguments)?;
        self.start_invocation(Value::Function(function), values)
    }

    /// Begin ordinary source comptime invocation admission, then return its
    /// owned driver. Admission may use configured synchronous semantic services.
    /// Associated selections in declaration contracts and linear bodies can
    /// leave the driver without losing the invocation's current stage.
    pub fn start_invocation<'vm>(
        &'vm mut self,
        function: Value,
        arguments: Vec<Value>,
    ) -> Result<InvocationSession<'vm, 'world>, Error> {
        if self.graph.is_some() || !self.bytecode_invocation_available() {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        let active = self.begin_invocation(function, arguments, None)?;
        Ok(InvocationSession {
            session: Session::invocation(self, active),
        })
    }
}

impl<'world> InvocationSession<'_, 'world> {
    /// Complete a fresh invocation using the semantic services of its source
    /// world. Each request owns its allowance until the response is admitted.
    pub fn run_to_completion(self) -> Result<Value, Error> {
        self.run_with_associated(AssociatedHostRequest::prepare)
    }

    pub fn run_with_associated(
        self,
        associated: impl FnMut(AssociatedHostRequest<'world>) -> AssociatedHostResponse<'world>,
    ) -> Result<Value, Error> {
        self.drive(associated).map(|result| result.value)
    }

    /// Drive this session and deliver the root's bound result type with its value.
    pub fn run_with_result_type(self) -> Result<InvocationResult, Error> {
        self.drive(AssociatedHostRequest::prepare)
    }

    fn drive(
        mut self,
        mut associated: impl FnMut(AssociatedHostRequest<'world>) -> AssociatedHostResponse<'world>,
    ) -> Result<InvocationResult, Error> {
        loop {
            match self.poll_with_result_type()? {
                InvocationProgress::Complete(value) => return Ok(value),
                InvocationProgress::Running => {}
                InvocationProgress::Associated(request) => self
                    .resume(associated(request))
                    .map_err(|rejected| rejected.error().clone())?,
                InvocationProgress::Definition(request) => self
                    .resume_definition(request.prepare())
                    .map_err(|rejected| rejected.error().clone())?,
                InvocationProgress::Method(request) => self
                    .resume_method(request.prepare())
                    .map_err(|rejected| rejected.error().clone())?,
                // An outstanding request taken by an earlier external poll
                // cannot be serviced here. Its owner must deliver it first.
                InvocationProgress::Waiting => {
                    return Err(Error {
                        kind: ErrorKind::InvalidControlFlow,
                        source: None,
                    });
                }
            }
        }
    }

    /// Advance one driver transition. Waiting does not spend steps or call the
    /// host. Running may include synchronous services or tree execution; this
    /// boundary does not promise a wall-clock slice or preemption.
    pub fn poll(&mut self) -> Result<InvocationProgress<'world>, Error> {
        self.session
            .poll_external(&mut bytecode::OperandPoll::Skip, |result| result.value)
    }

    /// Like `poll`, preserving the result type on the successful terminal transition.
    pub fn poll_with_result_type(
        &mut self,
    ) -> Result<InvocationProgress<'world, InvocationResult>, Error> {
        self.session
            .poll_external(&mut bytecode::OperandPoll::Skip, std::convert::identity)
    }

    /// Advance one transition, stopping after the next ordinary bytecode call
    /// slot is filled, before evaluating later operands or admitting the callee.
    /// An escaping producer never delivers its unfinished slot. Subsequent
    /// polls continue after the store, including when switching to plain poll.
    pub fn poll_with_operands(&mut self) -> Result<OperandProgress<'_, 'world>, Error> {
        let mut delivered = bytecode::OperandPoll::Next;
        let progress = self
            .session
            .poll_external(&mut delivered, std::convert::identity)?;
        if let bytecode::OperandPoll::Supplied(site) = delivered {
            Ok(OperandProgress::Supplied(
                self.session.delivered_operand(site),
            ))
        } else {
            Ok(OperandProgress::Invocation(progress))
        }
    }

    /// Deliver an answer for later admission. A rejected answer is returned
    /// intact, without changing the current waiter or its allowance.
    pub fn resume(
        &mut self,
        response: AssociatedHostResponse<'world>,
    ) -> Result<(), RejectedHostResponse<'world>> {
        self.session.receive_host_response(response)
    }
    pub fn resume_definition(
        &mut self,
        response: DefinitionHostResponse<'world>,
    ) -> Result<(), RejectedDefinitionResponse<'world>> {
        self.session.receive_definition_response(response)
    }
    pub fn resume_method(
        &mut self,
        response: MethodHostResponse<'world>,
    ) -> Result<(), RejectedMethodResponse<'world>> {
        self.session.receive_method_response(response)
    }
    pub fn cancel(&mut self) {
        self.session.cancel();
    }
    pub fn remaining_steps(&self) -> u64 {
        self.session.remaining_steps()
    }
    /// Charge bounded consumer work before publishing an invocation result.
    pub fn charge_preparation(&mut self, steps: usize) -> Result<(), Error> {
        self.session.charge_preparation(steps)
    }
}

impl fmt::Display for InvocationSession<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.session.fmt(f)
    }
}
impl fmt::Display for AssociatedHostRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-host-request {} (granted {}) (remaining {}) {})",
            self.lease.ticket, self.lease.granted, self.lease.remaining, self.request
        )
    }
}
impl fmt::Display for AssociatedHostResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-host-response {} (remaining {}) ",
            self.lease.ticket, self.lease.remaining
        )?;
        match &self.selection {
            Ok(selection) => write!(f, "{selection}")?,
            Err(error) => write!(f, "(failed {})", nir::QuotedDisplay(&error.to_string()))?,
        }
        f.write_str(")")
    }
}
impl fmt::Display for RejectedHostResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(rejected-host-response (error {}) {})",
            nir::QuotedDisplay(&self.error.to_string()),
            self.response
        )
    }
}
impl<T: fmt::Display> fmt::Display for InvocationProgress<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => f.write_str("(running)"),
            Self::Waiting => f.write_str("(waiting)"),
            Self::Associated(request) => write!(f, "{request}"),
            Self::Definition(request) => write!(f, "{request}"),
            Self::Method(request) => write!(f, "{request}"),
            Self::Complete(value) => write!(f, "(complete {value})"),
        }
    }
}

impl fmt::Debug for InvocationSession<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<T: fmt::Display> fmt::Debug for InvocationProgress<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Debug for AssociatedHostRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Debug for AssociatedHostResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Debug for RejectedHostResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests;
