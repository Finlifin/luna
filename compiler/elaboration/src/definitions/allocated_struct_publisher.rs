//! Compatibility adapter for callers that only request struct generation.
use super::{AllocatedDefinitionPublisher, StructBuildService};
use comptime_interpretor::{Error, PreparedStructBuild, StructBuildRequest};
use nir::program as n;
use std::sync::Arc;

pub struct AllocatedStructPublisher<'a> {
    inner: AllocatedDefinitionPublisher<'a>,
}
impl<'a> AllocatedStructPublisher<'a> {
    /// `next` covers the complete source scan and earlier generated identities.
    pub fn new(baseline: &'a n::Package, package: u64, next: u32, fuel: usize) -> Self {
        Self {
            inner: AllocatedDefinitionPublisher::new(baseline, package, next, fuel),
        }
    }
    pub fn remaining_structural_fuel(&self) -> usize {
        self.inner.remaining_structural_fuel()
    }
    pub fn finish(self) -> (Vec<Arc<[n::Struct]>>, u32) {
        let (definitions, next) = self.inner.finish();
        (definitions.structs, next)
    }
}
impl StructBuildService for AllocatedStructPublisher<'_> {
    fn prepare<'s>(
        &'s self,
        request: &StructBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        self.inner.prepare_struct(request, steps)
    }
    fn last_diagnostic(&self) -> Option<String> {
        self.inner.last_diagnostic()
    }
}
impl std::fmt::Display for AllocatedStructPublisher<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(allocated-struct-publisher {})", self.inner)
    }
}
