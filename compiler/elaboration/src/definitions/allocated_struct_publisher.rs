//! Explicit invocation ownership of the source declaration frontier.
use super::{CheckedStructPublisher, StructBuildService};
use comptime_interpretor::{Error, ErrorKind, PreparedStructBuild, StructBuildRequest};
use nir::{DeclarationId, program as n};
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};

pub struct AllocatedStructPublisher<'a> {
    checked: CheckedStructPublisher<'a>,
    package: u64,
    next: Cell<u32>,
    fuel: Cell<usize>,
    diagnostic: RefCell<Option<String>>,
}
impl<'a> AllocatedStructPublisher<'a> {
    /// `next` covers the complete source scan and earlier generated identities,
    /// not just the declarations reachable in the checked invocation snapshot.
    pub fn new(baseline: &'a n::Package, package: u64, next: u32, fuel: usize) -> Self {
        Self {
            checked: CheckedStructPublisher::new(baseline),
            package,
            next: Cell::new(next),
            fuel: Cell::new(fuel),
            diagnostic: RefCell::new(None),
        }
    }
    pub fn remaining_structural_fuel(&self) -> usize {
        self.fuel.get()
    }
    pub fn finish(self) -> (Vec<Arc<[n::Struct]>>, u32) {
        (self.checked.into_definitions(), self.next.get())
    }
}
impl StructBuildService for AllocatedStructPublisher<'_> {
    fn prepare<'s>(
        &'s self,
        request: &StructBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        *self.diagnostic.borrow_mut() = None;
        let fail = || Error {
            kind: ErrorKind::StepsExhausted,
            source: Some(request.source),
        };
        let mut fuel = self.fuel.replace(0);
        let reserved = (|| {
            // Reserve descriptor assembly and the later copy into the output
            // package. Unexecuted branches consume no structural allowance.
            fuel = fuel
                .checked_sub(
                    request
                        .name
                        .len()
                        .saturating_add(request.fields.len())
                        .saturating_add(1),
                )
                .ok_or_else(fail)?;
            for field in &request.fields {
                fuel = fuel
                    .checked_sub(field.name.len().saturating_add(1))
                    .ok_or_else(fail)?;
                nir::type_computations::charge_term(&field.ty, &mut fuel).map_err(|_| fail())?;
            }
            Ok::<_, Error>(())
        })();
        self.fuel.set(fuel);
        if let Err(error) = reserved {
            *self.diagnostic.borrow_mut() =
                Some("builder preparation structural budget exhausted".into());
            return Err(error);
        }
        let local = self.next.get();
        let next = local.checked_add(1).ok_or_else(|| {
            *self.diagnostic.borrow_mut() =
                Some("builder declaration identity space exhausted".into());
            Error {
                kind: ErrorKind::InvalidCallPlan,
                source: Some(request.source),
            }
        })?;
        self.next.set(next);
        self.checked.prepare(
            DeclarationId {
                package: self.package,
                local,
            },
            request,
            steps,
        )
    }
    fn last_diagnostic(&self) -> Option<String> {
        self.diagnostic
            .borrow()
            .clone()
            .or_else(|| self.checked.last_diagnostic())
    }
}
impl std::fmt::Display for AllocatedStructPublisher<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(allocated-struct-publisher (package {}) (next {}) (structural-fuel {}) {})",
            self.package,
            self.next.get(),
            self.fuel.get(),
            self.checked
        )
    }
}
