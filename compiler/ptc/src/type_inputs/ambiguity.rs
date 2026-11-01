//! Ambiguity needs two solutions in the same ambient world. Pin one model's
//! environment in a private search, then replay the result in the real problem.
//! A unique solution in this one world never establishes a general binding.
use super::*;

pub(super) fn alternative(
    problem: &Context,
    model: &Witness,
    variable: DeclarationId,
    inputs: &[DeclarationId],
    budget: &mut Budget,
) -> Result<Option<Witness>, UnknownReason> {
    budget.steps = budget
        .steps
        .checked_sub(inputs.len())
        .ok_or(UnknownReason::BudgetExhausted)?;
    let requested: BTreeSet<_> = inputs.iter().copied().collect();
    let mut pinned = problem.fork();
    for (&id, value) in &model.types {
        if !within_input_limits(
            Input::Term(value),
            budget.max_depth.min(128),
            &mut budget.steps,
        ) {
            return Err(UnknownReason::BudgetExhausted);
        }
        if !requested.contains(&id) {
            pinned.assume(Formula::equal(type_variable(id), value.clone()));
        }
    }
    for (&id, &value) in &model.integers {
        budget.steps = budget
            .steps
            .checked_sub(1)
            .ok_or(UnknownReason::BudgetExhausted)?;
        let input = IntegerTerm::variable(id, 0);
        let value = IntegerTerm::constant(value);
        pinned.assume(Formula::int_le(input.clone(), value.clone()));
        pinned.assume(Formula::int_le(value, input));
    }
    for application in &model.applications {
        for term in application
            .arguments
            .iter()
            .chain(std::iter::once(&application.result))
        {
            if !within_input_limits(
                Input::Term(term),
                budget.max_depth.min(128),
                &mut budget.steps,
            ) {
                return Err(UnknownReason::BudgetExhausted);
            }
        }
        pinned.assume(Formula::equal(
            Term::NormalForm {
                head: application.head,
                sort: Sort::Type,
                arguments: application.arguments.clone(),
            },
            application.result.clone(),
        ));
    }
    let goal = Formula::equal(type_variable(variable), model.types[&variable].clone());
    let mut alternative = match pinned.prove_metered(goal, budget) {
        Outcome::NotEntailed(witness) => witness,
        Outcome::Unknown { reason, .. } => return Err(reason),
        // In particular, a proof under pinned premises cannot solve an input.
        Outcome::Entailed(_) | Outcome::IllFormed(_) => return Ok(None),
    };
    if !symbolic::same_ambient(model, &alternative, inputs, budget)? {
        return Ok(None);
    }
    for fact in problem.assumptions() {
        if !within_limits(fact, budget.max_depth.min(128), &mut budget.steps) {
            return Err(UnknownReason::BudgetExhausted);
        }
    }
    alternative.assumptions = problem.assumptions().to_vec();
    problem
        .verify_witness_metered(&alternative, budget)
        .map_err(|error| match error {
            ProofError::BudgetExhausted => UnknownReason::BudgetExhausted,
            ProofError::ArithmeticRange => UnknownReason::ArithmeticRange,
            ProofError::Invalid => UnknownReason::UnvalidatedModel,
        })?;
    Ok(Some(alternative))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniqueness_in_one_pinned_world_does_not_determine_an_input() {
        let input = DeclarationId {
            package: 81,
            local: 0,
        };
        let ambient = DeclarationId {
            package: 81,
            local: 1,
        };
        let mut problem = Context::default();
        problem.assume(Formula::equal(type_variable(input), type_variable(ambient)));
        let Outcome::NotEntailed(model) = problem.prove(Formula::False, Budget::default()) else {
            panic!("the equation is satisfiable");
        };
        let original = problem.clone();
        assert!(
            alternative(&problem, &model, input, &[input], &mut Budget::default())
                .unwrap()
                .is_none()
        );
        assert_eq!(problem, original);
    }
}
