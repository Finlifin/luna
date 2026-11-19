//! A single source identity frontier for all supported concrete builder requests.
use super::{CheckedDefinitionPublisher, PublishedDefinitions, StructBuildService};
use comptime_interpretor::{
    EnumBuildRequest, Error, ErrorKind, NewtypeBuildRequest, PreparedEnumBuild,
    PreparedNewtypeBuild, PreparedStructBuild, StructBuildRequest,
};
use nir::{DeclarationId, program as n};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Fresh local declaration IDs owned by one source elaboration. Nested source
/// checks and VM publishers reserve immediately; descriptors publish separately.
#[derive(Clone)]
pub struct DefinitionIdentities(Rc<Cell<u32>>);

impl From<u32> for DefinitionIdentities {
    fn from(next: u32) -> Self {
        Self(Rc::new(Cell::new(next)))
    }
}

impl DefinitionIdentities {
    pub fn next(&self) -> u32 {
        self.0.get()
    }

    pub(crate) fn reserve(&self) -> Option<u32> {
        let local = self.next();
        self.0.set(local.checked_add(1)?);
        Some(local)
    }
}

pub struct AllocatedDefinitionPublisher<'a> {
    checked: CheckedDefinitionPublisher<'a>,
    package: u64,
    identities: DefinitionIdentities,
    fuel: Cell<usize>,
    diagnostic: RefCell<Option<String>>,
}
impl<'a> AllocatedDefinitionPublisher<'a> {
    /// The frontier covers all source declarations, including those absent from
    /// the invocation snapshot. Rejected or abandoned reserved IDs are not reused.
    pub fn new(
        baseline: &'a n::Package,
        package: u64,
        identities: impl Into<DefinitionIdentities>,
        fuel: usize,
    ) -> Self {
        Self {
            checked: CheckedDefinitionPublisher::new(baseline),
            package,
            identities: identities.into(),
            fuel: Cell::new(fuel),
            diagnostic: RefCell::new(None),
        }
    }
    pub fn new_with_type_solver(
        baseline: &'a n::Package,
        package: u64,
        identities: impl Into<DefinitionIdentities>,
        fuel: usize,
        solver: &'a comptime_interpretor::BudgetedTypeSolver<'a>,
    ) -> Self {
        Self {
            checked: CheckedDefinitionPublisher::new_with_type_solver(baseline, solver),
            package,
            identities: identities.into(),
            fuel: Cell::new(fuel),
            diagnostic: RefCell::new(None),
        }
    }
    pub fn remaining_structural_fuel(&self) -> usize {
        self.fuel.get()
    }
    pub fn finish(self) -> (PublishedDefinitions, u32) {
        (self.checked.into_definitions(), self.identities.next())
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
    pub fn prepare_newtype<'s>(
        &'s self,
        request: &NewtypeBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedNewtypeBuild<'s>, Error> {
        let identity = self.reserve(request.source, |fuel| request.charge_assembly(fuel))?;
        self.checked
            .prepare_newtype_request(identity, request, steps)
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
        let local = self.identities.reserve().ok_or_else(|| {
            *self.diagnostic.borrow_mut() =
                Some("builder declaration identity space exhausted".into());
            Error {
                kind: ErrorKind::InvalidCallPlan,
                source: Some(source),
            }
        })?;
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
    fn prepare_newtype<'s>(
        &'s self,
        request: &NewtypeBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedNewtypeBuild<'s>, Error> {
        AllocatedDefinitionPublisher::prepare_newtype(self, request, steps)
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
            self.identities.next(),
            self.fuel.get(),
            self.checked
        )
    }
}
