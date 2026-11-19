//! One declaration scope for all three nonexecuting VM query services.
use super::*;
use std::cell::RefCell;

#[cfg(test)]
mod tests;

/// Declaration queries owned by one invocation in one immutable source scope.
/// Construct lazily: an unused service incurs no index work. Completed
/// metadata and type views can be reused. Selection obligations are checked on
/// every demand; executable call contracts remain the VM's responsibility.
pub struct SourceInvocationQueries<'a> {
    source: DeclarationSource<'a>,
    context: &'a Context,
    assumptions: &'a [n::TraitRequirement],
    world: RefCell<Option<DeclarationWorld<'a>>>,
}

impl<'a> SourceInvocationQueries<'a> {
    pub fn new(
        package: &'a n::Package,
        context: &'a Context,
        assumptions: &'a [n::TraitRequirement],
    ) -> Self {
        Self::in_source(DeclarationSource::Borrowed(package), context, assumptions)
    }

    pub fn in_closed(declarations: &'a ClosedDeclarations) -> Self {
        Self::in_source(
            DeclarationSource::Snapshot(declarations.source()),
            declarations.context(),
            &[],
        )
    }

    pub fn in_source(
        source: DeclarationSource<'a>,
        context: &'a Context,
        assumptions: &'a [n::TraitRequirement],
    ) -> Self {
        Self {
            source,
            context,
            assumptions,
            world: RefCell::new(None),
        }
    }

    fn query<T>(
        &self,
        budget: &mut Budget,
        query: impl FnOnce(
            &mut DeclarationWorld<'a>,
            &mut Budget,
        ) -> Result<T, nir::NominalAdmissionError>,
    ) -> Result<T, nir::NominalAdmissionError> {
        let mut world = self.world.borrow_mut();
        if world.is_none() {
            // Publish only a complete index. A failed first demand is retryable.
            *world = Some(
                self.source
                    .world(budget, Some((self.context, self.assumptions)))?,
            );
        }
        query(world.as_mut().unwrap(), budget)
    }

    pub fn identity(
        &self,
        callee: &n::Expr,
        fuel: &mut usize,
    ) -> Result<n::MethodInstance, nir::NominalAdmissionError> {
        query_budget::run(fuel, Budget::default(), |budget| {
            self.query(budget, |world, budget| {
                world.method_identity(callee, budget)
            })
        })
    }

    pub fn resolve(
        &self,
        callee: &n::Expr,
        remaining: &mut u64,
    ) -> Result<n::MethodInstance, comptime_interpretor::Error> {
        query_budget::invocation(remaining, unlimited_steps(), |budget| {
            self.query(budget, |world, budget| {
                world.method_identity(callee, budget)
            })
        })
        .map_err(|failure| invocation_failure(failure, callee.source))
    }

    pub fn associated(
        &self,
        access: &n::Expr,
        remaining: &mut u64,
    ) -> Result<comptime_interpretor::AssociatedSelection, comptime_interpretor::Error> {
        query_budget::invocation(remaining, unlimited_steps(), |budget| {
            associated_selection::finish(
                self.select_associated(access, budget)?
                    .read_in_source(budget),
                access.source,
            )
        })
    }

    fn read_associated(
        &self,
        access: &n::Expr,
        budget: &mut Budget,
    ) -> Result<
        associated::AssociatedRead<associated::PendingAssociatedSource<'a>>,
        comptime_interpretor::Error,
    > {
        self.select_associated(access, budget)?
            .read(budget)
            .map_err(|failure| invocation_failure(failure, access.source))
    }

    /// Select in the request's declaration world, then release the query cache.
    /// A missing source keeps the VM ticket and the original selection together.
    pub fn associated_request<'vm>(
        &self,
        mut request: comptime_interpretor::AssociatedHostRequest<'vm>,
    ) -> AssociatedAnswer<'vm, 'a> {
        if !std::ptr::eq(request.source_world(), self.source.package()) {
            let error = comptime_interpretor::Error {
                kind: comptime_interpretor::ErrorKind::InvalidCallPlan,
                source: Some(request.access().source),
            };
            return AssociatedAnswer::Ready(request.respond(Err(error)));
        }
        let read = request.query(|access, remaining| {
            query_budget::invocation(remaining, unlimited_steps(), |budget| {
                self.read_associated(access, budget)
            })
        });
        match read {
            Ok(associated::AssociatedRead::NeedsSource(selection)) => {
                AssociatedAnswer::NeedsSource(SelectedSourceRequest { request, selection })
            }
            Ok(associated::AssociatedRead::Ready(value)) => {
                AssociatedAnswer::Ready(request.respond(Ok(value.supply)))
            }
            Err(error) => AssociatedAnswer::Ready(request.respond(Err(error))),
        }
    }

    /// Release the mutable method cache before handing the selected declaration
    /// to the invoker. The retained scope fixes the source of later admission.
    pub fn select_associated(
        &self,
        access: &n::Expr,
        budget: &mut Budget,
    ) -> Result<associated::PreparedAssociatedMember<'a>, comptime_interpretor::Error> {
        let (interface, subject, member) = associated_selection::operands(access)?;
        let scope = self
            .query(budget, |world, _| Ok(world.scope().clone()))
            .map_err(|failure| invocation_failure(failure, access.source))?;
        scope
            .prepare_associated_member(interface, member, subject, budget)
            .map_err(|failure| invocation_failure(failure, access.source))
    }

    pub fn check(
        &self,
        predicate: &n::TypePredicate,
        limits: Budget,
        remaining: &mut u64,
    ) -> comptime_interpretor::ContractQueryResult {
        self.check_in_context(predicate, self.context, limits, remaining)
    }

    /// A restored binder scope changes only this proof's context. It cannot
    /// replace the source, mutate the default scope, or populate method caches.
    pub fn check_in_context(
        &self,
        predicate: &n::TypePredicate,
        context: &Context,
        limits: Budget,
        remaining: &mut u64,
    ) -> comptime_interpretor::ContractQueryResult {
        let result = query_budget::invocation(remaining, limits, |budget| {
            capabilities::check_predicate_with(context, predicate, budget, |demand, budget| {
                self.query(budget, |world, budget| {
                    demand.resolve(&world.capabilities(), budget)
                })
            })
        });
        contract_result(result)
    }
}

pub enum AssociatedAnswer<'vm, 'source> {
    Ready(comptime_interpretor::AssociatedHostResponse<'vm>),
    NeedsSource(SelectedSourceRequest<'vm, 'source>),
}

pub struct SelectedSourceRequest<'vm, 'source> {
    request: comptime_interpretor::AssociatedHostRequest<'vm>,
    selection: associated::PendingAssociatedSource<'source>,
}

impl<'vm> SelectedSourceRequest<'vm, '_> {
    pub fn source_slot(&self) -> (Id, Id) {
        self.selection.source_slot()
    }

    pub fn remaining_steps_mut(&mut self) -> &mut u64 {
        self.request.remaining_steps_mut()
    }

    /// Read only the template captured by the source of the original query.
    pub fn read_source(self) -> comptime_interpretor::AssociatedHostResponse<'vm> {
        self.request.resolve(|access, remaining| {
            query_budget::invocation(remaining, unlimited_steps(), |budget| {
                associated_selection::finish(self.selection.read_source(budget), access.source)
            })
        })
    }
}

impl std::fmt::Display for SelectedSourceRequest<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(selected-source-request {} {})",
            self.request, self.selection
        )
    }
}

/// Use the same proof and accounting boundary for a driver-owned world.
pub fn check_world_predicate(
    world: &capabilities::World<'_>,
    predicate: &n::TypePredicate,
    limits: Budget,
    remaining: &mut u64,
) -> comptime_interpretor::ContractQueryResult {
    contract_result(query_budget::invocation(remaining, limits, |budget| {
        world.check_predicate_metered(predicate, budget)
    }))
}

fn contract_result(
    result: Result<ptc::Outcome, nir::NominalAdmissionError>,
) -> comptime_interpretor::ContractQueryResult {
    use comptime_interpretor::{ContractOutcome, ErrorKind};
    use nir::NominalAdmissionError as Failure;
    match result {
        Ok(ptc::Outcome::Entailed(_)) => Ok(ContractOutcome::Satisfied),
        Ok(ptc::Outcome::NotEntailed(_)) => Ok(ContractOutcome::Rejected),
        Ok(ptc::Outcome::Unknown {
            reason: ptc::UnknownReason::BudgetExhausted,
            ..
        })
        | Err(Failure::Budget) => Err(ErrorKind::StepsExhausted),
        Ok(ptc::Outcome::IllFormed(reason)) | Err(Failure::Invalid(reason)) => {
            Err(ErrorKind::InvalidTypeContract(reason))
        }
        Ok(ptc::Outcome::Unknown { .. }) | Err(Failure::Unresolved(_)) => {
            Ok(ContractOutcome::Unknown)
        }
    }
}

pub fn resolve_method_for_invocation(
    package: &n::Package,
    callee: &n::Expr,
    context: &Context,
    assumptions: &[n::TraitRequirement],
    remaining: &mut u64,
) -> Result<n::MethodInstance, comptime_interpretor::Error> {
    SourceInvocationQueries::new(package, context, assumptions).resolve(callee, remaining)
}

fn unlimited_steps() -> Budget {
    Budget {
        steps: usize::MAX,
        ..Budget::default()
    }
}

pub(super) fn invocation_failure(
    failure: nir::NominalAdmissionError,
    source: n::SourceRange,
) -> comptime_interpretor::Error {
    use comptime_interpretor::ErrorKind;
    comptime_interpretor::Error {
        kind: match failure {
            nir::NominalAdmissionError::Budget => ErrorKind::StepsExhausted,
            nir::NominalAdmissionError::Unresolved(_) => ErrorKind::TypeContractUnresolved,
            nir::NominalAdmissionError::Invalid(_) => ErrorKind::UnmaterializedDispatch,
        },
        source: Some(source),
    }
}
