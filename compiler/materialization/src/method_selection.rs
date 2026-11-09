//! Typed source-selection responses at the VM host boundary.
use super::*;
use std::cell::RefCell;

#[cfg(test)]
mod tests;

/// Declaration queries owned by one invocation in one immutable source scope.
/// Construct lazily: unused method services incur no index work. Completed
/// metadata and type views can be reused. Selection obligations are checked on
/// every demand; executable call contracts remain the VM's responsibility.
pub struct SourceMethodQueries<'a> {
    package: &'a n::Package,
    context: &'a Context,
    assumptions: &'a [n::TraitRequirement],
    world: RefCell<Option<DeclarationWorld<'a>>>,
}

impl<'a> SourceMethodQueries<'a> {
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

    pub fn identity(
        &self,
        callee: &n::Expr,
        fuel: &mut usize,
    ) -> Result<n::MethodInstance, nir::NominalAdmissionError> {
        query_budget::run(fuel, Budget::default(), |budget| {
            let mut world = self.world.borrow_mut();
            if world.is_none() {
                // Assignment happens only after complete index construction.
                // A failed first demand leaves construction retryable.
                *world = Some(DeclarationWorld::new_metered(
                    self.package,
                    budget,
                    Some((self.context, self.assumptions)),
                )?);
            }
            world.as_mut().unwrap().method_identity(callee, budget)
        })
    }

    pub fn resolve(
        &self,
        callee: &n::Expr,
        remaining: &mut u64,
    ) -> Result<n::MethodInstance, comptime_interpretor::Error> {
        let reserved = std::mem::take(remaining);
        let before = usize::try_from(reserved).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = self.identity(callee, &mut fuel);
        *remaining = reserved - (before - fuel) as u64;
        result.map_err(|failure| comptime_interpretor::Error {
            kind: match failure {
                nir::NominalAdmissionError::Budget => {
                    comptime_interpretor::ErrorKind::StepsExhausted
                }
                nir::NominalAdmissionError::Unresolved(_) => {
                    comptime_interpretor::ErrorKind::TypeContractUnresolved
                }
                nir::NominalAdmissionError::Invalid(_) => {
                    comptime_interpretor::ErrorKind::UnmaterializedDispatch
                }
            },
            source: Some(callee.source),
        })
    }
}

pub fn resolve_method_for_invocation(
    package: &n::Package,
    callee: &n::Expr,
    context: &Context,
    assumptions: &[n::TraitRequirement],
    remaining: &mut u64,
) -> Result<n::MethodInstance, comptime_interpretor::Error> {
    SourceMethodQueries::new(package, context, assumptions).resolve(callee, remaining)
}
