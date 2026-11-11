//! An exclusively borrowed invocation with owned selection and definition messages.
//! Call admission and other host services are still synchronous. This interface is
//! not a general frontend-region suspension or a cross-process wire protocol.
use super::*;
use crate::request_identity::Ticket;
use std::task::Poll;

/// A source comptime invocation. Dropping it cancels and restores its caller.
pub struct InvocationSession<'vm, 'world> {
    session: Session<'vm, 'world, 'static>,
}

/// Running made internal progress; Waiting has an outstanding owned request.
pub enum InvocationProgress<'world> {
    Running,
    Waiting,
    Associated(AssociatedHostRequest<'world>),
    Definition(DefinitionHostRequest<'world>),
    Complete(Value),
}

/// Resolve against this immutable source package and the compiler's matching
/// semantic context. The allowance belongs to this request until delivery.
/// Ordinary VM state, local storage and mutable inference are not exposed.
/// Dropping a request leaves its session waiting; cancel or drop that session
/// rather than polling for a response that can no longer arrive.
pub struct AssociatedHostRequest<'world> {
    ticket: Ticket,
    world: &'world Package,
    request: associated_execution::Request,
    granted: u64,
    remaining: u64,
}

/// A linear answer, still subject to the originating session's admission.
pub struct AssociatedHostResponse<'world> {
    pub(super) ticket: Ticket,
    pub(super) world: &'world Package,
    request: associated_execution::Request,
    remaining: u64,
    selection: Result<AssociatedSelection, Error>,
}

/// Invalid delivery does not consume the current waiter or the other answer.
pub struct RejectedHostResponse<'world> {
    error: Error,
    response: AssociatedHostResponse<'world>,
}

impl<'world> AssociatedHostRequest<'world> {
    pub(super) fn new(
        ticket: Ticket,
        world: &'world Package,
        request: associated_execution::Request,
        granted: u64,
    ) -> Self {
        Self {
            ticket,
            world,
            request,
            granted,
            remaining: granted,
        }
    }

    pub fn access(&self) -> &Expr {
        self.request.access()
    }
    pub fn source_world(&self) -> &'world Package {
        self.world
    }
    pub fn remaining_steps(&self) -> u64 {
        self.remaining
    }
    pub fn remaining_steps_mut(&mut self) -> &mut u64 {
        &mut self.remaining
    }

    pub fn respond(
        self,
        selection: Result<AssociatedSelection, Error>,
    ) -> AssociatedHostResponse<'world> {
        AssociatedHostResponse {
            ticket: self.ticket,
            world: self.world,
            request: self.request,
            remaining: self.remaining,
            selection,
        }
    }

    pub fn resolve(
        mut self,
        resolver: impl FnOnce(&Expr, &mut u64) -> Result<AssociatedSelection, Error>,
    ) -> AssociatedHostResponse<'world> {
        let selection = resolver(self.request.access(), &mut self.remaining);
        self.respond(selection)
    }
}

impl<'world> AssociatedHostResponse<'world> {
    /// Return an undeliverable response intact without touching any VM waiter.
    pub fn reject(self) -> RejectedHostResponse<'world> {
        RejectedHostResponse::new(self)
    }

    pub(super) fn settle(self, granted: u64) -> (u64, associated_execution::Response) {
        let (remaining, selection) = if self.remaining > granted {
            (
                0,
                Err(Error {
                    kind: ErrorKind::StepsExhausted,
                    source: Some(self.request.access().source),
                }),
            )
        } else {
            (self.remaining, self.selection)
        };
        (remaining, self.request.respond(selection))
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
    /// Advance one driver transition. Waiting does not spend steps or call the
    /// host. Running may include synchronous services or tree execution; this
    /// boundary does not promise a wall-clock slice or preemption.
    pub fn poll(&mut self) -> Result<InvocationProgress<'world>, Error> {
        match self.session.poll_external()? {
            Poll::Ready(Some(Flow::Value(value))) => Ok(InvocationProgress::Complete(value)),
            Poll::Ready(_) => Err(Error {
                kind: ErrorKind::InvalidControlFlow,
                source: None,
            }),
            Poll::Pending => {
                if let Some(request) = self.session.take_host_request()? {
                    Ok(InvocationProgress::Associated(request))
                } else if let Some(request) = self.session.take_definition_request()? {
                    Ok(InvocationProgress::Definition(request))
                } else if self.session.waiting_for_host() {
                    Ok(InvocationProgress::Waiting)
                } else {
                    Ok(InvocationProgress::Running)
                }
            }
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
            self.ticket, self.granted, self.remaining, self.request
        )
    }
}
impl fmt::Display for AssociatedHostResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-host-response {} (remaining {}) ",
            self.ticket, self.remaining
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
impl fmt::Display for InvocationProgress<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => f.write_str("(running)"),
            Self::Waiting => f.write_str("(waiting)"),
            Self::Associated(request) => write!(f, "{request}"),
            Self::Definition(request) => write!(f, "{request}"),
            Self::Complete(value) => write!(f, "(complete {value})"),
        }
    }
}

impl fmt::Debug for InvocationSession<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Debug for InvocationProgress<'_> {
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
