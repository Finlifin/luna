//! A method choice is a response to one checked call prefix, not permission to
//! replay that prefix or a reusable callable value from another VM.
use super::host::HostLease;
use super::*;

pub struct MethodHostRequest<'a> {
    pub(super) lease: HostLease<'a>,
    pub(super) request: trait_methods::Request,
    pub(super) resolver: Option<&'a MethodResolver<'a>>,
}
pub struct MethodHostResponse<'a> {
    pub(super) lease: HostLease<'a>,
    pub(super) request: trait_methods::Request,
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
        selected: Result<nir::program::MethodInstance, Error>,
    ) -> MethodHostResponse<'a> {
        MethodHostResponse {
            lease: self.lease,
            request: self.request,
            selected,
        }
    }
    pub fn resolve(
        mut self,
        resolver: impl FnOnce(&Expr, &mut u64) -> Result<nir::program::MethodInstance, Error>,
    ) -> MethodHostResponse<'a> {
        let selected = resolver(self.request.access(), &mut self.lease.remaining);
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
            self.lease.ticket, self.lease.granted, self.lease.remaining, self.request
        )
    }
}
impl fmt::Display for MethodHostResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(method-host-response {} (remaining {}) {} ",
            self.lease.ticket, self.lease.remaining, self.request
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
