//! Rebuild call-local inference after earlier inputs complete dependent domains.
//! Each rebuilt candidate replays the supplied relations; proposals are never
//! exported as an instance without the ordinary candidate completion checks.
use super::*;

impl Scheme {
    /// Declaration inputs occupy a prefix of the inference frame. Remaining
    /// slots may represent provisional ordinary Type arguments.
    /// Domain admission and declaration requirements remain the caller's job;
    /// this routine checks structural sorts/scopes and replays call relations.
    pub fn infer_declared_sparse_types_mode(
        &self,
        ids: &[DeclarationId],
        domains: &[Term],
        actual: &[Option<Term>],
        expected: Option<&Term>,
        partial: Option<&CallableContext>,
        context: &crate::ptc::Context,
        mut budget: Budget,
        discharge_effects: bool,
        scope: &[(Sort, Option<Term>)],
    ) -> Result<CallCandidate, Error> {
        if ids.len() != domains.len() {
            return Err(Error::InvalidSignature);
        }
        let exhausted = || Error::Match(MatchError::BudgetExhausted);
        let computation = |error| match error {
            nir::type_computations::Error::Scope(error) => Error::Shape(error),
            nir::type_computations::Error::Budget => exhausted(),
            error => Error::TypeView(error.to_string()),
        };
        budget.steps = budget.steps.checked_sub(ids.len()).ok_or_else(exhausted)?;
        for domain in domains {
            nir::type_computations::charge_term(domain, &mut budget.steps).map_err(computation)?;
        }
        let binders = nir::declaration_parameters(ids, domains)?;
        let original_sorts = self.input_sorts()?;
        if ids.len() > original_sorts.len()
            || binders
                .iter()
                .zip(&original_sorts)
                .any(|(a, b)| a.sort != *b)
        {
            return Err(Error::InvalidSignature);
        }
        if binders.iter().all(|binder| binder.sort == Sort::Type) {
            return self.infer_sparse_scoped_types_mode(
                actual,
                expected,
                partial,
                context,
                budget,
                discharge_effects,
                scope,
            );
        }
        nir::type_computations::charge_term(&self.signature, &mut budget.steps)
            .map_err(computation)?;
        let rigid_inputs = original_sorts
            .iter()
            .enumerate()
            .map(|(index, sort)| {
                if let Some(id) = ids.get(index) {
                    Term::Rigid {
                        declaration: *id,
                        sort: *sort,
                    }
                } else {
                    Term::Bound {
                        depth: 0,
                        index,
                        sort: *sort,
                    }
                }
            })
            .collect::<Vec<_>>();
        // Preserve the frame: ordinary provisional slots and caller references
        // keep their indices while declaration occurrences become domain-aware.
        let rigid_signature = self.signature.rebind_scope(&rigid_inputs)?;
        let mut current = Scheme {
            parameter_sorts: original_sorts.clone(),
            type_parameters: self.type_parameters,
            signature: self.signature.clone(),
        };
        let mut known = vec![None; ids.len()];
        loop {
            budget.max_depth = budget.max_depth.checked_sub(1).ok_or_else(exhausted)?;
            let calls = 1 + known.iter().filter(|value| value.is_some()).count();
            let round = Budget {
                steps: budget.steps / 2 / calls,
                max_depth: budget.max_depth,
            };
            budget.steps -= round.steps * calls;
            let mut candidate = current.infer_sparse_scoped_types_mode(
                actual,
                expected,
                partial,
                context,
                round,
                discharge_effects,
                scope,
            )?;
            for (index, value) in known.iter().enumerate() {
                if let Some(value) = value {
                    candidate.constrain_type_argument(index, value, round)?;
                }
            }
            for (index, value) in known.iter_mut().enumerate() {
                *value = match candidate
                    .engine
                    .inspect_type_metered(&candidate.types[index], &mut budget)
                {
                    Ok(value) => Some(value),
                    Err(crate::InferenceError::Unresolved(_)) => None,
                    Err(error) => return Err(Error::Inference(error)),
                };
            }
            let mut sorts = original_sorts.clone();
            let mut frames = if scope.is_empty() {
                vec![]
            } else {
                vec![scope.iter().map(|entry| entry.0).collect()]
            };
            frames.push(sorts.clone());
            let mut replacements = Vec::with_capacity(ids.len());
            for index in 0..ids.len() {
                let substitution = nir::DeclarationSubstitution::bounded(
                    &ids[..index],
                    &domains[..index],
                    &replacements,
                    &frames,
                    &mut budget.steps,
                )
                .map_err(computation)?;
                let domain = substitution
                    .instantiate_bounded(&domains[index], &mut budget.steps)
                    .map_err(computation)?;
                sorts[index] = if domain == Term::Primitive(nir::Primitive::Type) {
                    Sort::Type
                } else {
                    Sort::ComptimeValue
                };
                frames.last_mut().unwrap()[index] = sorts[index];
                replacements.push(match &known[index] {
                    Some(value) => value.abstract_parameters(&[])?,
                    None => Term::Bound {
                        depth: 0,
                        index,
                        sort: sorts[index],
                    },
                });
            }
            if sorts == current.parameter_sorts {
                return Ok(candidate);
            }
            let substitution = nir::DeclarationSubstitution::bounded(
                ids,
                domains,
                &replacements,
                &frames,
                &mut budget.steps,
            )
            .map_err(computation)?;
            current.signature = substitution
                .instantiate_bounded(&rigid_signature, &mut budget.steps)
                .map_err(computation)?;
            current.parameter_sorts = sorts;
        }
    }
}
