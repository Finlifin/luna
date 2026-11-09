//! Bounded payload specialization for key method selection. Program predicates
//! remain opaque code; only TypePredicate values enter the type prover.
use super::*;
use nir::{
    DeclarationParameter,
    program::{Expr, TypePredicate},
};

pub(super) fn spend(budget: &mut Budget, amount: usize) -> Result<(), String> {
    budget.steps = budget
        .steps
        .checked_sub(amount)
        .ok_or("comptime key selection budget exhausted")?;
    Ok(())
}

pub(super) fn charge_term(value: &Term, budget: &mut Budget) -> Result<(), String> {
    nir::copy_budget::charge_type_substitution(value, &[], &[], &mut budget.steps)
        .map_err(|error| format!("comptime key payload budget exhausted: {error:?}"))
}

pub(super) fn term(
    value: &Term,
    parameters: &[DeclarationParameter],
    arguments: &[Term],
    budget: &mut Budget,
) -> Result<Term, String> {
    nir::copy_budget::charge_type_substitution(value, parameters, arguments, &mut budget.steps)
        .map_err(|error| format!("comptime key type specialization budget exhausted: {error:?}"))?;
    value
        .instantiate_parameters(parameters, arguments)
        .map_err(|error| error.to_string())
}

pub(super) fn predicate(
    value: &TypePredicate,
    parameters: &[DeclarationParameter],
    arguments: &[Term],
    budget: &mut Budget,
) -> Result<TypePredicate, String> {
    nir::copy_budget::charge_predicate_substitution(
        value,
        parameters,
        arguments,
        &mut budget.steps,
    )
    .map_err(|error| {
        format!("comptime key predicate specialization budget exhausted: {error:?}")
    })?;
    value
        .instantiate_parameters(parameters, arguments)
        .map_err(|error| error.to_string())
}

pub(super) fn requirement(
    value: &TraitRequirement,
    parameters: &[DeclarationParameter],
    arguments: &[Term],
    budget: &mut Budget,
) -> Result<TraitRequirement, String> {
    spend(budget, value.interface_arguments.len())?;
    Ok(TraitRequirement {
        interface: value.interface,
        interface_arguments: value
            .interface_arguments
            .iter()
            .map(|ty| term(ty, parameters, arguments, budget))
            .collect::<Result<_, _>>()?,
        subject: term(&value.subject, parameters, arguments, budget)?,
    })
}

pub(super) fn program(
    preconditions: &[Expr],
    postconditions: &[Expr],
    result_binding: Option<LocalId>,
    snapshots: &[nir::program::EntrySnapshot],
    parameters: &[DeclarationParameter],
    arguments: &[Term],
    budget: &mut Budget,
) -> Result<ProgramContract, String> {
    spend(
        budget,
        preconditions
            .len()
            .saturating_add(postconditions.len())
            .saturating_add(snapshots.len()),
    )?;
    let mut specialize = |expression: &Expr| {
        nir::copy_budget::charge_substitution(expression, parameters, arguments, &mut budget.steps)
            .map_err(|error| {
                format!("comptime key program specialization budget exhausted: {error:?}")
            })?;
        expression
            .instantiate_parameters(parameters, arguments)
            .map_err(|error| error.to_string())
    };
    Ok(ProgramContract {
        preconditions: preconditions
            .iter()
            .map(&mut specialize)
            .collect::<Result<_, _>>()?,
        postconditions: postconditions
            .iter()
            .map(&mut specialize)
            .collect::<Result<_, _>>()?,
        result_binding,
        entry_snapshots: snapshots.to_vec(),
    })
}

pub(super) fn prove(
    world: &World<'_>,
    predicate: &nir::program::TypePredicate,
    budget: &mut Budget,
) -> Result<bool, String> {
    // Formula conversion copies the predicate before the prover consumes it.
    nir::copy_budget::charge_predicate_substitution(predicate, &[], &[], &mut budget.steps)
        .map_err(|error| format!("comptime key formula budget exhausted: {error:?}"))?;
    match world
        .context
        .prove_metered(Formula::from_predicate(predicate), budget)
    {
        Outcome::Entailed(_) => Ok(true),
        Outcome::Unknown {
            reason: crate::ptc::UnknownReason::BudgetExhausted,
            ..
        } => Err("comptime key proof budget exhausted".into()),
        _ => Ok(false),
    }
}
