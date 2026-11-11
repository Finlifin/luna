//! Linear definition preparation outside the suspended VM. A response carries
//! an uncommitted receipt; only its originating frame may admit and install it.
use super::*;
use crate::request_identity::Ticket;

pub struct DefinitionHostRequest<'a> {
    pub(super) ticket: Ticket,
    pub(super) world: &'a Package,
    pub(super) request: DefinitionBuildRequest,
    pub(super) resolver: definition_build::Resolver<'a>,
    pub(super) granted: u64,
    pub(super) remaining: u64,
}
pub struct DefinitionHostResponse<'a> {
    pub(super) ticket: Ticket,
    pub(super) world: &'a Package,
    pub(super) request: DefinitionBuildRequest,
    pub(super) remaining: u64,
    pub(super) prepared: Result<PreparedDefinitionBuild<'a>, Error>,
}
pub struct RejectedDefinitionResponse<'a> {
    response: DefinitionHostResponse<'a>,
    error: Error,
}
impl<'a> DefinitionHostRequest<'a> {
    pub fn descriptor(&self) -> &DefinitionBuildRequest {
        &self.request
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
        prepared: Result<PreparedDefinitionBuild<'a>, Error>,
    ) -> DefinitionHostResponse<'a> {
        DefinitionHostResponse {
            ticket: self.ticket,
            world: self.world,
            request: self.request,
            remaining: self.remaining,
            prepared,
        }
    }
    /// Use the configured publisher, outside the VM driver. Callers may instead
    /// retain this request while preparing its receipt through other work.
    pub fn prepare(mut self) -> DefinitionHostResponse<'a> {
        let prepared = self.resolver.prepare(&self.request, &mut self.remaining);
        self.respond(prepared)
    }
}
impl<'a> DefinitionHostResponse<'a> {
    pub fn reject(self) -> RejectedDefinitionResponse<'a> {
        let error = Error {
            kind: ErrorKind::InvalidControlFlow,
            source: Some(self.request.source()),
        };
        RejectedDefinitionResponse {
            response: self,
            error,
        }
    }
    pub(super) fn settle(
        mut self,
        granted: u64,
    ) -> (
        u64,
        DefinitionBuildRequest,
        Result<PreparedDefinitionBuild<'a>, Error>,
    ) {
        if self.remaining > granted {
            self.remaining = 0;
            self.prepared = Err(Error {
                kind: ErrorKind::StepsExhausted,
                source: Some(self.request.source()),
            });
        }
        (self.remaining, self.request, self.prepared)
    }
}
impl<'a> RejectedDefinitionResponse<'a> {
    pub fn error(&self) -> &Error {
        &self.error
    }
    pub fn into_response(self) -> DefinitionHostResponse<'a> {
        self.response
    }
}
impl fmt::Display for DefinitionHostRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(definition-host-request {} (granted {}) (remaining {}) {})",
            self.ticket, self.granted, self.remaining, self.request
        )
    }
}
impl fmt::Display for DefinitionHostResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(definition-host-response {} (remaining {}) {} ",
            self.ticket, self.remaining, self.request
        )?;
        match &self.prepared {
            Ok(PreparedDefinitionBuild::Struct(p)) => write!(
                f,
                "(prepared-struct (result {}) (definitions {}))",
                p.result,
                p.definitions.len()
            )?,
            Ok(PreparedDefinitionBuild::Enum(p)) => write!(
                f,
                "(prepared-enum (result {}) (definitions {}))",
                p.result,
                p.definitions.len()
            )?,
            Ok(PreparedDefinitionBuild::Newtype(p)) => write!(
                f,
                "(prepared-newtype (result {}) (definitions {}))",
                p.result,
                p.definitions.len()
            )?,
            Err(e) => write!(f, "(failed {})", nir::QuotedDisplay(&e.to_string()))?,
        }
        f.write_str(")")
    }
}
impl fmt::Display for RejectedDefinitionResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(rejected-definition-response (error {}) {})",
            nir::QuotedDisplay(&self.error.to_string()),
            self.response
        )
    }
}
impl fmt::Debug for DefinitionHostRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Debug for RejectedDefinitionResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
