//! A single source identity frontier for all supported concrete builder requests.
use super::{CheckedDefinitionPublisher, PublishedDefinitions, StructBuildService};
use comptime_interpretor::{
    EnumBuildRequest, Error, ErrorKind, PreparedEnumBuild, PreparedStructBuild, StructBuildRequest,
};
use nir::{DeclarationId, program as n};
use std::cell::{Cell, RefCell};

pub struct AllocatedDefinitionPublisher<'a> {
    checked: CheckedDefinitionPublisher<'a>,
    package: u64,
    next: Cell<u32>,
    fuel: Cell<usize>,
    diagnostic: RefCell<Option<String>>,
}
impl<'a> AllocatedDefinitionPublisher<'a> {
    /// The frontier covers all source declarations, including those absent from
    /// the invocation snapshot. Rejected or abandoned reserved IDs are not reused.
    pub fn new(baseline: &'a n::Package, package: u64, next: u32, fuel: usize) -> Self {
        Self {
            checked: CheckedDefinitionPublisher::new(baseline),
            package,
            next: Cell::new(next),
            fuel: Cell::new(fuel),
            diagnostic: RefCell::new(None),
        }
    }
    pub fn remaining_structural_fuel(&self) -> usize {
        self.fuel.get()
    }
    pub fn finish(self) -> (PublishedDefinitions, u32) {
        (self.checked.into_definitions(), self.next.get())
    }
    pub fn last_diagnostic(&self) -> Option<String> {
        self.diagnostic
            .borrow()
            .clone()
            .or_else(|| self.checked.last_diagnostic())
    }
    pub fn prepare_struct<'s>(
        &'s self,
        request: &StructBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        let identity = self.reserve(request.source, |fuel| request.charge_assembly(fuel))?;
        self.checked
            .prepare_struct_request(identity, request, steps)
    }
    pub fn prepare_enum<'s>(
        &'s self,
        request: &EnumBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedEnumBuild<'s>, Error> {
        let identity = self.reserve(request.source, |fuel| request.charge_assembly(fuel))?;
        self.checked.prepare_enum_request(identity, request, steps)
    }
    fn reserve(
        &self,
        source: n::SourceRange,
        charge: impl FnOnce(&mut usize) -> Result<(), ()>,
    ) -> Result<DeclarationId, Error> {
        *self.diagnostic.borrow_mut() = None;
        // Prepay assembly and transfer into the output package before allocating
        // an identity. Actual invocation checks consume their separate allowance.
        let mut fuel = self.fuel.replace(0);
        let result = charge(&mut fuel);
        self.fuel.set(fuel);
        if result.is_err() {
            *self.diagnostic.borrow_mut() =
                Some("builder preparation structural budget exhausted".into());
            return Err(Error {
                kind: ErrorKind::StepsExhausted,
                source: Some(source),
            });
        }
        let local = self.next.get();
        let next = local.checked_add(1).ok_or_else(|| {
            *self.diagnostic.borrow_mut() =
                Some("builder declaration identity space exhausted".into());
            Error {
                kind: ErrorKind::InvalidCallPlan,
                source: Some(source),
            }
        })?;
        self.next.set(next);
        Ok(DeclarationId {
            package: self.package,
            local,
        })
    }
}
impl StructBuildService for AllocatedDefinitionPublisher<'_> {
    fn prepare<'s>(
        &'s self,
        request: &StructBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        self.prepare_struct(request, steps)
    }
    fn prepare_enum<'s>(
        &'s self,
        request: &EnumBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedEnumBuild<'s>, Error> {
        AllocatedDefinitionPublisher::prepare_enum(self, request, steps)
    }
    fn last_diagnostic(&self) -> Option<String> {
        self.last_diagnostic()
    }
}
impl std::fmt::Display for AllocatedDefinitionPublisher<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(allocated-definition-publisher (package {}) (next {}) (structural-fuel {}) {})",
            self.package,
            self.next.get(),
            self.fuel.get(),
            self.checked
        )
    }
}
