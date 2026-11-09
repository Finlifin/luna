//! Structural declaration indexes and metered proofs for driver-owned worlds.
use elaboration::{implementation::Index, ptc::Budget};
use nir::program::Package;

pub(crate) fn index(package: &Package, mut budget: Budget) -> Result<Index, String> {
    Index::from_package_metered(package, &mut budget).map_err(String::from)
}

/// Spend proof work from the invocation's counter before interpreting its result.
pub(crate) fn solve_metered(
    world: &elaboration::capabilities::World<'_>,
    predicate: &nir::program::TypePredicate,
    budget: elaboration::ptc::Budget,
    remaining: &mut u64,
) -> comptime_interpretor::ContractOutcome {
    use comptime_interpretor::ContractOutcome;
    use elaboration::ptc::Outcome;
    let mut proof = elaboration::ptc::Budget {
        steps: budget
            .steps
            .min(usize::try_from(*remaining).unwrap_or(usize::MAX)),
        ..budget
    };
    let before = proof.steps;
    let result = world.check_predicate_metered(predicate, &mut proof);
    *remaining = remaining.saturating_sub((before - proof.steps) as u64);
    match result {
        Ok(Outcome::Entailed(_)) => ContractOutcome::Satisfied,
        Ok(Outcome::NotEntailed(_)) => ContractOutcome::Rejected,
        _ => ContractOutcome::Unknown,
    }
}
