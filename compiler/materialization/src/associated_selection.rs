//! Invocation-demanded selection in an immutable declaration world.
use super::*;

pub fn resolve_associated_value_for_invocation(
    world: &capabilities::World<'_>,
    access: &n::Expr,
    remaining: &mut u64,
) -> Result<comptime_interpretor::AssociatedSelection, comptime_interpretor::Error> {
    query_budget::invocation(
        remaining,
        Budget {
            steps: usize::MAX,
            ..Budget::default()
        },
        |budget| select(world, access, budget),
    )
    .map_err(|failure| source_queries::invocation_failure(failure, access.source))
}

pub(super) fn select(
    world: &capabilities::World<'_>,
    access: &n::Expr,
    budget: &mut Budget,
) -> Result<comptime_interpretor::AssociatedSelection, nir::NominalAdmissionError> {
    let K::AssociatedValue {
        interface,
        subject,
        member,
    } = &access.kind
    else {
        return Err("expected associated value access".into());
    };
    let selected =
        world.resolve_associated_value_plan_metered(interface, *member, subject, budget)?;
    let initializer = nir::associated_initializers::AssociatedInitializer::new(
        selected.value,
        &mut budget.steps,
    )?;
    Ok(comptime_interpretor::AssociatedSelection {
        initializer,
        requires_invocation: selected.requires_invocation,
    })
}
