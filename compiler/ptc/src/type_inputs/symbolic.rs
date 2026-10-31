//! Relative solutions preserve ambient Type inputs instead of exporting one
//! model's concrete interpretation of them. All proposed equalities are proved.
use super::*;

pub(super) fn candidates<'a>(
    problem: &'a Context,
    inputs: &[DeclarationId],
    budget: &mut Budget,
) -> Result<Vec<&'a Term>, UnknownReason> {
    let requested = requested(inputs, budget)?;
    let mut candidates = Vec::new();
    for term in problem.mentioned_types_metered(budget)? {
        if !within_input_limits(
            Input::Term(term),
            budget.max_depth.min(128),
            &mut budget.steps,
        ) {
            return Err(UnknownReason::BudgetExhausted);
        }
        let references = term
            .rigid_parameters()
            .map_err(|_| UnknownReason::UnvalidatedModel)?;
        // Exclude both direct cycles and dependencies on another requested
        // variable. Publication must never leak a temporary search identity.
        if !references.is_empty() && references.keys().all(|id| !requested.contains(id)) {
            candidates.push(term);
        }
    }
    Ok(candidates)
}

pub(super) fn binding(
    problem: &Context,
    variable: DeclarationId,
    candidates: &[&Term],
    budget: &mut Budget,
) -> Result<Option<(Term, Proof)>, UnknownReason> {
    for &candidate in candidates {
        if !within_input_limits(
            Input::Term(candidate),
            budget.max_depth.min(128),
            &mut budget.steps,
        ) {
            return Err(UnknownReason::BudgetExhausted);
        }
        match problem.prove_metered(
            Formula::equal(type_variable(variable), candidate.clone()),
            budget,
        ) {
            Outcome::Entailed(proof) => return Ok(Some((candidate.clone(), proof))),
            Outcome::Unknown {
                reason: UnknownReason::BudgetExhausted,
                ..
            } => {
                return Err(UnknownReason::BudgetExhausted);
            }
            _ => {}
        }
    }
    Ok(None)
}

pub(super) fn same_ambient(
    first: &Witness,
    second: &Witness,
    inputs: &[DeclarationId],
    budget: &mut Budget,
) -> Result<bool, UnknownReason> {
    let requested = requested(inputs, budget)?;
    for model in [first, second] {
        for value in model.types.values() {
            if !within_input_limits(
                Input::Term(value),
                budget.max_depth.min(128),
                &mut budget.steps,
            ) {
                return Err(UnknownReason::BudgetExhausted);
            }
        }
        budget.steps = budget
            .steps
            .checked_sub(model.integers.len())
            .ok_or(UnknownReason::BudgetExhausted)?;
    }
    Ok(first.integers == second.integers
        && first
            .types
            .iter()
            .filter(|(id, _)| !requested.contains(id))
            .eq(second
                .types
                .iter()
                .filter(|(id, _)| !requested.contains(id))))
}

fn requested(
    inputs: &[DeclarationId],
    budget: &mut Budget,
) -> Result<BTreeSet<DeclarationId>, UnknownReason> {
    budget.steps = budget
        .steps
        .checked_sub(inputs.len())
        .ok_or(UnknownReason::BudgetExhausted)?;
    Ok(inputs.iter().copied().collect())
}
