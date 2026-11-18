//! Host work crosses the driver boundary once. The outstanding ticket and
//! bytecode continuation stay in the session until delivery or cancellation.
use super::*;
use crate::invocation::host::HostLease;

pub(super) enum Service {
    Associated(associated_execution::Request),
    Definition {
        destination: usize,
        request: DefinitionBuildRequest,
    },
    Method(bytecode::MethodWait),
}

pub(super) enum HostContinuation {
    Associated,
    Definition { destination: usize },
    Method(bytecode::MethodContinuation),
}

impl Service {
    /// Local drivers also operate on IIR, which has no source-world envelope.
    /// They share the receipt admission step with externally driven sessions.
    pub(super) fn resolve<'a>(self, vm: &mut Interpreter<'a>) -> Pending<'a> {
        match self {
            Self::Associated(request) => {
                Pending::AssociatedResponse(vm.resolve_associated_request(request))
            }
            Self::Definition {
                destination,
                request,
            } => {
                let resolver = vm.definition_resolver(&request);
                let prepared = host_budget::clamped(&mut vm.budget.steps, |fuel| {
                    resolver.prepare(&request, fuel)
                });
                Pending::DefinitionResponse {
                    destination,
                    request,
                    prepared,
                }
            }
            Self::Method(wait) => {
                let selected = vm.resolve_method_request(&wait.request);
                Pending::MethodResponse { wait, selected }
            }
        }
    }

    fn send<'a>(
        self,
        vm: &Interpreter<'a>,
        lease: HostLease<'a>,
    ) -> (InvocationProgress<'a>, HostContinuation) {
        match self {
            Self::Associated(request) => (
                InvocationProgress::Associated(AssociatedHostRequest {
                    lease,
                    request,
                    resolver: vm.associated_resolver,
                }),
                HostContinuation::Associated,
            ),
            Self::Definition {
                destination,
                request,
            } => {
                let resolver = vm.definition_resolver(&request);
                (
                    InvocationProgress::Definition(DefinitionHostRequest {
                        lease,
                        request,
                        resolver,
                    }),
                    HostContinuation::Definition { destination },
                )
            }
            Self::Method(wait) => (
                InvocationProgress::Method(MethodHostRequest {
                    lease,
                    request: wait.request,
                    resolver: vm.method_resolver,
                }),
                HostContinuation::Method(wait.continuation),
            ),
        }
    }
}

impl fmt::Display for Service {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Associated(request) => request.fmt(f),
            Self::Method(wait) => wait.fmt(f),
            Self::Definition {
                destination,
                request,
            } => write!(
                f,
                "(prepare-definition (destination {destination}) {request})"
            ),
        }
    }
}

impl fmt::Display for HostContinuation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Associated => f.write_str("(associated)"),
            Self::Method(continuation) => write!(f, "(method {continuation})"),
            Self::Definition { destination } => {
                write!(f, "(definition (destination {destination}))")
            }
        }
    }
}

impl<'a> Session<'_, 'a, '_> {
    pub(super) fn take_service(&mut self) -> Option<Service> {
        match self.pending.take_if(|p| matches!(p, Pending::Request(_))) {
            Some(Pending::Request(request)) => Some(request),
            _ => None,
        }
    }

    pub(in crate::invocation) fn poll_external(&mut self) -> Result<InvocationProgress<'a>, Error> {
        if matches!(self.pending, Some(Pending::Waiting { .. })) {
            return Ok(InvocationProgress::Waiting);
        }
        if !matches!(self.pending, Some(Pending::Request(_))) {
            match self.advance(true)? {
                Poll::Ready(Some(Flow::Value(value))) => {
                    return Ok(InvocationProgress::Complete(value));
                }
                Poll::Ready(_) => return Err(self.vm.failure(ErrorKind::InvalidControlFlow, None)),
                Poll::Pending => {}
            }
        }
        let Some(request) = self.take_service() else {
            return Ok(InvocationProgress::Running);
        };
        let Some(world) = self.vm.declaration_world else {
            self.cancel();
            return Err(self.vm.failure(ErrorKind::InvalidControlFlow, None));
        };
        let ticket = self.requests.issue().inspect_err(|_| self.cancel())?;
        let lease = HostLease::reserve(ticket.clone(), world, &mut self.vm.budget.steps);
        let (progress, continuation) = request.send(self.vm, lease);
        self.pending = Some(Pending::Waiting {
            ticket,
            continuation,
        });
        Ok(progress)
    }

    fn claim(
        &mut self,
        lease: &HostLease<'a>,
        accepts: impl FnOnce(&HostContinuation) -> bool,
    ) -> Option<HostContinuation> {
        if !self
            .vm
            .declaration_world
            .is_some_and(|world| std::ptr::eq(world, lease.world))
        {
            return None;
        }
        match self.pending.take_if(|state| {
            matches!(state, Pending::Waiting { ticket, continuation }
            if ticket.matches(&lease.ticket) && accepts(continuation))
        }) {
            Some(Pending::Waiting { continuation, .. }) => Some(continuation),
            _ => None,
        }
    }

    pub(in crate::invocation) fn receive_host_response(
        &mut self,
        response: AssociatedHostResponse<'a>,
    ) -> Result<(), RejectedHostResponse<'a>> {
        if self
            .claim(&response.lease, |c| {
                matches!(c, HostContinuation::Associated)
            })
            .is_none()
        {
            return Err(response.reject());
        }
        let selection = response.lease.settle(
            &mut self.vm.budget.steps,
            response.request.access().source,
            response.selection,
        );
        self.pending = Some(Pending::AssociatedResponse(
            response.request.respond(selection),
        ));
        Ok(())
    }

    pub(in crate::invocation) fn receive_definition_response(
        &mut self,
        response: DefinitionHostResponse<'a>,
    ) -> Result<(), RejectedDefinitionResponse<'a>> {
        let Some(HostContinuation::Definition { destination }) = self.claim(&response.lease, |c| {
            matches!(c, HostContinuation::Definition { .. })
        }) else {
            return Err(response.reject());
        };
        let prepared = response.lease.settle(
            &mut self.vm.budget.steps,
            response.request.source(),
            response.prepared,
        );
        self.pending = Some(Pending::DefinitionResponse {
            destination,
            request: response.request,
            prepared,
        });
        Ok(())
    }

    pub(in crate::invocation) fn receive_method_response(
        &mut self,
        response: MethodHostResponse<'a>,
    ) -> Result<(), RejectedMethodResponse<'a>> {
        let Some(HostContinuation::Method(continuation)) = self.claim(&response.lease, |c| {
            matches!(c, HostContinuation::Method(_))
        }) else {
            return Err(response.reject());
        };
        let selected = response.lease.settle(
            &mut self.vm.budget.steps,
            response.request.access().source,
            response.selected,
        );
        self.pending = Some(Pending::MethodResponse {
            wait: bytecode::MethodWait {
                request: response.request,
                continuation,
            },
            selected,
        });
        Ok(())
    }
}
