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
    package: &'a n::Package,
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
        Self {
            package,
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
            *world = Some(DeclarationWorld::new_metered(
                self.package,
                budget,
                Some((self.context, self.assumptions)),
            )?);
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
            self.query(budget, |world, budget| {
                associated_selection::select(&world.capabilities(), access, budget)
            })
        })
        .map_err(|failure| invocation_failure(failure, access.source))
    }

    pub fn check(
        &self,
        predicate: &n::TypePredicate,
        limits: Budget,
        remaining: &mut u64,
    ) -> comptime_interpretor::ContractOutcome {
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
    ) -> comptime_interpretor::ContractOutcome {
        use comptime_interpretor::ContractOutcome;
        let result = query_budget::invocation(remaining, limits, |budget| {
            self.query(budget, |world, budget| {
                let mut proof = world.capabilities();
                proof.context = context;
                proof.check_predicate_metered(predicate, budget)
            })
        });
        match result {
            Ok(ptc::Outcome::Entailed(_)) => ContractOutcome::Satisfied,
            Ok(ptc::Outcome::NotEntailed(_)) => ContractOutcome::Rejected,
            _ => ContractOutcome::Unknown,
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
