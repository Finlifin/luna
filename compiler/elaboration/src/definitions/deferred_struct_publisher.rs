//! First-use preparation of a mutable publication world for explicit invocation.
use super::*;
use comptime_interpretor::{Error, ErrorKind, PreparedStructBuild, StructBuildRequest};
use std::cell::{Cell, OnceCell, RefCell};

/// Host publication boundary shared by eager and first-use package ownership.
pub trait StructBuildService {
    fn prepare<'s>(
        &'s self,
        request: &StructBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedStructBuild<'s>, Error>;
    fn last_diagnostic(&self) -> Option<String>;
}
impl StructBuildService for StructPublisher<'_> {
    fn prepare<'s>(
        &'s self,
        request: &StructBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        StructPublisher::prepare(self, request, steps)
    }
    fn last_diagnostic(&self) -> Option<String> {
        StructPublisher::last_diagnostic(self)
    }
}

/// Merely configuring an invocation does not fork HIR or consume structural
/// fuel. The first executed request creates one owned publication world; later
/// requests see its committed definitions. Unexecuted branches create no world.
pub struct DeferredStructPublisher<'source, 'hir> {
    arena: &'hir hir::HirArena,
    source: &'source hir::Package<'hir>,
    resolution_package: u32,
    identities: &'source BTreeMap<u32, u64>,
    protocols: &'source Protocols,
    target: Option<nir::target::TargetData>,
    fuel: Cell<usize>,
    publisher: OnceCell<StructPublisher<'hir>>,
    diagnostic: RefCell<Option<String>>,
}
impl<'source, 'hir> DeferredStructPublisher<'source, 'hir> {
    pub fn new(
        arena: &'hir hir::HirArena,
        source: &'source hir::Package<'hir>,
        resolution_package: u32,
        identities: &'source BTreeMap<u32, u64>,
        protocols: &'source Protocols,
        target: Option<nir::target::TargetData>,
        fuel: usize,
    ) -> Self {
        Self {
            arena,
            source,
            resolution_package,
            identities,
            protocols,
            target,
            fuel: Cell::new(fuel),
            publisher: OnceCell::new(),
            diagnostic: RefCell::new(None),
        }
    }
    pub fn is_initialized(&self) -> bool {
        self.publisher.get().is_some()
    }
    pub fn remaining_structural_fuel(&self) -> usize {
        self.publisher.get().map_or_else(
            || self.fuel.get(),
            StructPublisher::remaining_structural_fuel,
        )
    }
    pub fn finish(self) -> Option<nir::program::Package> {
        self.publisher
            .into_inner()
            .and_then(|publisher| publisher.finish().1)
    }
    fn initialize(&self, request: &StructBuildRequest) -> Result<(), Error> {
        if self.is_initialized() {
            return Ok(());
        }
        let mut fuel = self.fuel.replace(0);
        let candidate = hir::definitions::fork_package(self.source, &mut fuel);
        self.fuel.set(fuel);
        let publisher = candidate
            .map_err(|error| format!("builder package preparation failed: {error}"))
            .and_then(|candidate| {
                StructPublisher::new(
                    self.arena,
                    candidate,
                    self.resolution_package,
                    self.identities.clone(),
                    self.protocols.clone(),
                    self.target,
                    fuel,
                )
            });
        match publisher {
            Ok(publisher) => {
                // Initialization does not execute callbacks and cannot reenter.
                assert!(self.publisher.set(publisher).is_ok());
                *self.diagnostic.borrow_mut() = None;
                Ok(())
            }
            Err(message) => {
                *self.diagnostic.borrow_mut() = Some(message);
                Err(Error {
                    kind: ErrorKind::Type,
                    source: Some(request.source),
                })
            }
        }
    }
}
impl StructBuildService for DeferredStructPublisher<'_, '_> {
    fn prepare<'s>(
        &'s self,
        request: &StructBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        self.initialize(request)?;
        self.publisher
            .get()
            .expect("initialized publication world")
            .prepare(request, steps)
    }
    fn last_diagnostic(&self) -> Option<String> {
        self.publisher
            .get()
            .and_then(StructPublisher::last_diagnostic)
            .or_else(|| self.diagnostic.borrow().clone())
    }
}
impl std::fmt::Display for DeferredStructPublisher<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(deferred-struct-publisher (initialized {}) (structural-fuel {}))",
            self.is_initialized(),
            self.remaining_structural_fuel()
        )
    }
}
