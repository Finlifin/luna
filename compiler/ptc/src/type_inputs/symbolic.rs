//! Relative solutions preserve ambient Type inputs instead of exporting one
//! model's concrete interpretation of them. All proposed equalities are proved.
use super::*;

pub(super) fn candidates(
    problem: &Context,
    inputs: &[DeclarationId],
    bindings: &BTreeMap<DeclarationId, (Term, Proof)>,
    budget: &mut Budget,
) -> Result<Vec<Term>, UnknownReason> {
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
        // Only proved bindings may eliminate requested variables from a
        // candidate. Unresolved cycles stay out of the exported Type values.
        if references.is_empty()
            || references
                .keys()
                .any(|id| requested.contains(id) && !bindings.contains_key(id))
        {
            continue;
        }
        let mut parameters = Vec::new();
        let mut arguments = Vec::new();
        for id in references.keys().filter(|id| requested.contains(id)) {
            let value = &bindings[id].0;
            if !within_input_limits(
                Input::Term(value),
                budget.max_depth.min(128),
                &mut budget.steps,
            ) {
                return Err(UnknownReason::BudgetExhausted);
            }
            parameters.push(nir::DeclarationParameter {
                declaration: *id,
                sort: Sort::Type,
            });
            arguments.push(value.clone());
        }
        nir::copy_budget::charge_type_substitution(
            term,
            &parameters,
            &arguments,
            &mut budget.steps,
        )
        .map_err(|_| UnknownReason::BudgetExhausted)?;
        let candidate = term
            .instantiate_parameters_in_scope(&parameters, &arguments, problem.scope())
            .map_err(|_| UnknownReason::UnvalidatedModel)?;
        // Substitution may increase depth even though each input fit separately.
        if !within_input_limits(
            Input::Term(&candidate),
            budget.max_depth.min(128),
            &mut budget.steps,
        ) {
            return Err(UnknownReason::BudgetExhausted);
        }
        candidates.push(candidate);
    }
    Ok(candidates)
}

pub(super) fn binding(
    problem: &Context,
    variable: DeclarationId,
    candidates: &[Term],
    budget: &mut Budget,
) -> Result<Option<(Term, Proof)>, UnknownReason> {
    for candidate in candidates {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composed_candidates_keep_opaque_heads_and_require_original_proofs() {
        let id = |local| DeclarationId { package: 7, local };
        let variable = |local| type_variable(id(local));
        let head = |arg| Term::NormalForm {
            head: id(100),
            sort: Sort::Type,
            arguments: vec![arg],
        };
        let mut problem = Context::default();
        problem.admit_congruence(id(100));
        problem.assume(Formula::equal(variable(0), head(variable(1))));
        let equality = Formula::equal(variable(1), variable(9));
        problem.assume(equality.clone());
        let Outcome::Entailed(proof) = problem.prove_metered(equality, &mut Budget::default())
        else {
            panic!("the explicit input equality must be provable")
        };
        let bindings = BTreeMap::from([(id(1), (variable(9), proof))]);
        let proposed =
            candidates(&problem, &[id(0), id(1)], &bindings, &mut Budget::default()).unwrap();
        assert!(proposed.contains(&head(variable(9))));
        let (value, proof) = binding(&problem, id(0), &proposed, &mut Budget::default())
            .unwrap()
            .unwrap();
        assert_eq!(value, head(variable(9)));
        problem.verify_proof(&proof, Budget::default()).unwrap();
    }
}
