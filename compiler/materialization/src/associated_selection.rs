//! Invocation-demanded selection in an immutable declaration world.
use super::*;

pub fn resolve_associated_value_for_invocation(
    world: &capabilities::World<'_>,
    access: &n::Expr,
    remaining: &mut u64,
) -> Result<comptime_interpretor::AssociatedSelection, comptime_interpretor::Error> {
    let before = usize::try_from(*remaining).unwrap_or(usize::MAX);
    let mut fuel = before;
    let result = (|| {
        let K::AssociatedValue {
            interface,
            subject,
            member,
        } = &access.kind
        else {
            return Err("expected associated value access".to_owned());
        };
        let (selected, rest) = world.resolve_associated_value_plan_budgeted(
            interface,
            *member,
            subject,
            Budget {
                steps: fuel,
                ..Default::default()
            },
        )?;
        fuel = rest;
        let initializer =
            nir::associated_initializers::AssociatedInitializer::new(selected.value, &mut fuel)?;
        Ok(comptime_interpretor::AssociatedSelection {
            initializer,
            requires_invocation: selected.requires_invocation,
        })
    })();
    if result.is_err() {
        fuel = 0;
    }
    *remaining -= (before - fuel) as u64;
    result.map_err(|_: String| comptime_interpretor::Error {
        kind: comptime_interpretor::ErrorKind::UnmaterializedDispatch,
        source: Some(access.source),
    })
}
