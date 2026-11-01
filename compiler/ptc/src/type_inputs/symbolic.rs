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

/// Rejections belong to one immutable problem, not a reusable global cache.
/// New proved bindings only expose candidates; they never change this context.
pub(super) struct Resolver<'a> {
    problem: &'a Context,
    rejected: HashMap<DeclarationId, std::collections::HashSet<Term>>,
}
impl<'a> Resolver<'a> {
    pub(super) fn new(problem: &'a Context) -> Self {
        Self {
            problem,
            rejected: HashMap::new(),
        }
    }

    pub(super) fn binding(
        &mut self,
        variable: DeclarationId,
        candidates: &[Term],
        budget: &mut Budget,
    ) -> Result<Option<(Term, Proof)>, UnknownReason> {
        let rejected = self.rejected.entry(variable).or_default();
        for candidate in candidates {
            // Charge traversal before hashing, comparing or copying the term.
            if !within_input_limits(
                Input::Term(candidate),
                budget.max_depth.min(128),
                &mut budget.steps,
            ) {
                return Err(UnknownReason::BudgetExhausted);
            }
            if rejected.contains(candidate) {
                continue;
            }
            match self.problem.prove_metered(
                Formula::equal(type_variable(variable), candidate.clone()),
                budget,
            ) {
                Outcome::Entailed(proof) => return Ok(Some((candidate.clone(), proof))),
                Outcome::NotEntailed(_) => {
                    rejected.insert(candidate.clone());
                }
                Outcome::Unknown {
                    reason: UnknownReason::BudgetExhausted,
                    ..
                } => return Err(UnknownReason::BudgetExhausted),
                // Unknown is not a countermodel and cannot exclude a candidate.
                _ => {}
            }
        }
        Ok(None)
    }
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
        for application in &model.applications {
            for value in application
                .arguments
                .iter()
                .chain(std::iter::once(&application.result))
            {
                if !within_input_limits(
                    Input::Term(value),
                    budget.max_depth.min(128),
                    &mut budget.steps,
                ) {
                    return Err(UnknownReason::BudgetExhausted);
                }
            }
        }
        budget.steps = budget
            .steps
            .checked_sub(model.integers.len())
            .ok_or(UnknownReason::BudgetExhausted)?;
    }
    Ok(first.applications == second.applications
        && first.integers == second.integers
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
        let (value, proof) = Resolver::new(&problem)
            .binding(id(0), &proposed, &mut Budget::default())
            .unwrap()
            .unwrap();
        assert_eq!(value, head(variable(9)));
        problem.verify_proof(&proof, Budget::default()).unwrap();
    }

    #[test]
    fn rejected_candidates_are_local_to_the_problem_and_input() {
        let id = |local| DeclarationId { package: 71, local };
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let pointer = Term::Pointer(Box::new(boolean.clone()));
        let mut problem = Context::default();
        problem.assume(Formula::equal(type_variable(id(0)), pointer.clone()));
        problem.assume(Formula::equal(type_variable(id(1)), boolean.clone()));
        let mut resolver = Resolver::new(&problem);
        let wrong = [boolean.clone(), Term::Primitive(nir::Primitive::U8)];
        let mut cold = Budget::default();
        let initial = cold.steps;
        assert!(
            resolver
                .binding(id(0), &wrong, &mut cold)
                .unwrap()
                .is_none()
        );
        let cold_work = initial - cold.steps;
        let mut warm = Budget::default();
        let initial = warm.steps;
        assert!(
            resolver
                .binding(id(0), &wrong, &mut warm)
                .unwrap()
                .is_none()
        );
        let warm_work = initial - warm.steps;
        assert!(warm_work < cold_work, "{warm_work} versus {cold_work}");
        assert_eq!(resolver.rejected[&id(0)].len(), 2);

        // A newly exposed candidate still receives an independent proof.
        let candidates = [wrong[0].clone(), pointer.clone()];
        let (value, proof) = resolver
            .binding(id(0), &candidates, &mut Budget::default())
            .unwrap()
            .unwrap();
        assert_eq!(value, pointer);
        problem.verify_proof(&proof, Budget::default()).unwrap();
        let (value, proof) = resolver
            .binding(id(1), &wrong, &mut Budget::default())
            .unwrap()
            .unwrap();
        assert_eq!(value, boolean);
        problem.verify_proof(&proof, Budget::default()).unwrap();

        let mut changed = Context::default();
        changed.assume(Formula::equal(type_variable(id(0)), boolean.clone()));
        let (_, proof) = Resolver::new(&changed)
            .binding(id(0), &wrong, &mut Budget::default())
            .unwrap()
            .unwrap();
        changed.verify_proof(&proof, Budget::default()).unwrap();
    }

    #[test]
    fn interrupted_candidate_checks_are_retryable() {
        let variable = DeclarationId {
            package: 72,
            local: 0,
        };
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let mut problem = Context::default();
        problem.assume(Formula::equal(type_variable(variable), boolean.clone()));
        let candidates = [boolean];
        for steps in 0..32 {
            let mut resolver = Resolver::new(&problem);
            let result = resolver.binding(
                variable,
                &candidates,
                &mut Budget {
                    steps,
                    ..Budget::default()
                },
            );
            assert!(resolver.rejected[&variable].is_empty());
            if let Ok(Some((_, proof))) = result {
                problem.verify_proof(&proof, Budget::default()).unwrap();
            }
            let (_, proof) = resolver
                .binding(variable, &candidates, &mut Budget::default())
                .unwrap()
                .unwrap();
            problem.verify_proof(&proof, Budget::default()).unwrap();
        }
    }

    #[test]
    fn unavailable_trait_models_do_not_exclude_candidates() {
        let variable = DeclarationId {
            package: 73,
            local: 0,
        };
        let mut problem = Context::default();
        problem.assume(Formula::Trait(crate::TraitQuery {
            interface: DeclarationId {
                package: 73,
                local: 1,
            },
            subject: type_variable(variable),
            arguments: Vec::new(),
        }));
        let candidate = Term::Primitive(nir::Primitive::Bool);
        assert!(matches!(
            problem.prove_metered(
                Formula::equal(type_variable(variable), candidate.clone()),
                &mut Budget::default()
            ),
            Outcome::Unknown { .. }
        ));
        let mut resolver = Resolver::new(&problem);
        assert!(
            resolver
                .binding(variable, &[candidate], &mut Budget::default())
                .unwrap()
                .is_none()
        );
        assert!(resolver.rejected[&variable].is_empty());
    }
}
