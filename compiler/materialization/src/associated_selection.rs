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
            return Err(nir::NominalAdmissionError::from(
                "expected associated value access",
            ));
        };
        let mut budget = Budget {
            steps: fuel,
            ..Default::default()
        };
        let selected =
            world.resolve_associated_value_plan_metered(interface, *member, subject, &mut budget);
        fuel = budget.steps;
        let selected = selected?;
        let initializer =
            nir::associated_initializers::AssociatedInitializer::new(selected.value, &mut fuel)?;
        Ok(comptime_interpretor::AssociatedSelection {
            initializer,
            requires_invocation: selected.requires_invocation,
        })
    })();
    *remaining -= (before - fuel) as u64;
    result.map_err(
        |failure: nir::NominalAdmissionError| comptime_interpretor::Error {
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
            source: Some(access.source),
        },
    )
}
