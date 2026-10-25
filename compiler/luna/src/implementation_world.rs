//! Complete immutable implementation evidence for driver-owned execution worlds.
use elaboration::{
    implementation::{ImplementationHead, Index},
    ptc::Budget,
};
use nir::program::Package;

pub(crate) fn index(package: &Package, budget: Budget) -> Result<Index, String> {
    let mut index = Index::for_target(package.target);
    index.set_normal_form_metadata(package.normal_forms.clone());
    index.set_nominal_layouts(
        package.structs.clone(),
        package.enums.clone(),
        package.newtypes.clone(),
    );
    let mut domain_fuel = budget.steps;
    for (id, domain) in package.declaration_parameter_domains(&mut domain_fuel)? {
        index.register_parameter_domain(id, domain)?;
    }
    // Finalized inherent impls belong to member lookup, not trait candidates.
    for implementation in package
        .implementations
        .iter()
        .filter(|i| i.interface.is_some())
    {
        index
            .insert(
                ImplementationHead {
                    parameter_types: implementation.parameter_types.clone(),
                    declaration: implementation.declaration,
                    interface: implementation.interface,
                    interface_arguments: implementation.interface_arguments.clone(),
                    subject: implementation.subject.clone(),
                    type_parameters: implementation.type_parameters.clone(),
                    requirements: implementation.requirements.clone(),
                },
                budget,
            )
            .map_err(|error| format!("{error:?}"))?;
    }
    Ok(index)
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
