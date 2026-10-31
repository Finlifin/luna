//! Source struct requests adapted to the shared concrete definition publisher.
use super::CheckedDefinitionPublisher;
use comptime_interpretor::{Error, PreparedStructBuild, StructBuildRequest};
use nir::{DeclarationId, program as n};
use std::sync::Arc;

/// Concrete struct request adapter. Identity reservation belongs to the host.
pub struct CheckedStructPublisher<'a> {
    inner: CheckedDefinitionPublisher<'a>,
}
impl<'a> CheckedStructPublisher<'a> {
    pub fn new(baseline: &'a n::Package) -> Self {
        Self {
            inner: CheckedDefinitionPublisher::new(baseline),
        }
    }
    pub fn published_definition_count(&self) -> usize {
        self.inner.published_definition_count()
    }
    pub fn last_diagnostic(&self) -> Option<String> {
        self.inner.last_diagnostic()
    }
    pub fn into_definitions(self) -> Vec<Arc<[n::Struct]>> {
        self.inner.into_definitions().structs
    }
    pub fn prepare<'s>(
        &'s self,
        identity: DeclarationId,
        request: &StructBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        self.inner.prepare_struct_request(identity, request, steps)
    }
}

impl std::fmt::Display for CheckedStructPublisher<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(checked-struct-publisher (definitions {}))",
            self.published_definition_count()
        )
    }
}
