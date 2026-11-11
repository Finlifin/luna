//! A method choice is a response to one checked call prefix, not permission to
//! replay that prefix or a reusable callable value from another VM.
use super::*;
use crate::request_identity::Ticket;

pub struct MethodHostRequest<'a> {
    pub(super) ticket: Ticket,
    pub(super) world: &'a Package,
    pub(super) request: trait_methods::Request,
    pub(super) resolver: Option<&'a MethodResolver<'a>>,
    pub(super) granted: u64,
    pub(super) remaining: u64,
}
pub struct MethodHostResponse<'a> {
    pub(super) ticket: Ticket,
    pub(super) world: &'a Package,
    pub(super) request: trait_methods::Request,
    pub(super) remaining: u64,
    pub(super) selected: Result<nir::program::MethodInstance, Error>,
}
pub struct RejectedMethodResponse<'a> {
    response: MethodHostResponse<'a>,
    error: Error,
}
impl<'a> MethodHostRequest<'a> {
    pub fn access(&self) -> &Expr {
        self.request.access()
    }
    pub fn source_world(&self) -> &'a Package {
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
        selected: Result<nir::program::MethodInstance, Error>,
    ) -> MethodHostResponse<'a> {
        MethodHostResponse {
            ticket: self.ticket,
            world: self.world,
            request: self.request,
            remaining: self.remaining,
            selected,
        }
    }
    pub fn resolve(
        mut self,
        resolver: impl FnOnce(&Expr, &mut u64) -> Result<nir::program::MethodInstance, Error>,
    ) -> MethodHostResponse<'a> {
        let selected = resolver(self.request.access(), &mut self.remaining);
        self.respond(selected)
    }
    pub fn prepare(self) -> MethodHostResponse<'a> {
        let resolver = self.resolver;
        self.resolve(|access, fuel| {
            resolver.ok_or(Error {
                kind: ErrorKind::UnmaterializedDispatch,
                source: Some(access.source),
            })?(access, fuel)
        })
    }
}
impl<'a> MethodHostResponse<'a> {
    pub fn reject(self) -> RejectedMethodResponse<'a> {
        let error = Error {
            kind: ErrorKind::InvalidControlFlow,
            source: Some(self.request.access().source),
        };
        RejectedMethodResponse {
            response: self,
            error,
        }
    }
    pub(super) fn settle(
        mut self,
        granted: u64,
    ) -> (
        u64,
        trait_methods::Request,
        Result<nir::program::MethodInstance, Error>,
    ) {
        if self.remaining > granted {
            self.remaining = 0;
            self.selected = Err(Error {
                kind: ErrorKind::StepsExhausted,
                source: Some(self.request.access().source),
            });
        }
        (self.remaining, self.request, self.selected)
    }
}
impl<'a> RejectedMethodResponse<'a> {
    pub fn error(&self) -> &Error {
        &self.error
    }
    pub fn into_response(self) -> MethodHostResponse<'a> {
        self.response
    }
}
impl fmt::Display for MethodHostRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(method-host-request {} (granted {}) (remaining {}) {})",
            self.ticket, self.granted, self.remaining, self.request
        )
    }
}
impl fmt::Display for MethodHostResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(method-host-response {} (remaining {}) {} ",
            self.ticket, self.remaining, self.request
        )?;
        match &self.selected {
            Ok(selected) => write!(f, "{selected}")?,
            Err(error) => write!(f, "(failed {})", nir::QuotedDisplay(&error.to_string()))?,
        }
        f.write_str(")")
    }
}
impl fmt::Display for RejectedMethodResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(rejected-method-response (error {}) {})",
            nir::QuotedDisplay(&self.error.to_string()),
            self.response
        )
    }
}
impl fmt::Debug for RejectedMethodResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
