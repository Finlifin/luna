//! A deferred result can inherit a producer's declared postconditions on
//! normal completion. Observed identities specialize proofs, never VM inputs.
use super::*;

impl PreparedRequired {
    pub(in crate::program) fn direct_call_result_guarantees(
        &self,
        steps: &mut u64,
        span: Span,
    ) -> Result<Option<(DeclarationId, Vec<n::TypePredicate>)>, Error> {
        let before = usize::try_from(*steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let checked = self.result_contract_in(&mut fuel);
        *steps -= (before - fuel) as u64;
        checked.map_err(|message| error(span, message))
    }

    fn result_contract_in(
        &self,
        fuel: &mut usize,
    ) -> Result<Option<(DeclarationId, Vec<n::TypePredicate>)>, String> {
        let package = self.world.package();
        let lookup = |id, fuel: &mut usize| {
            for function in &package.functions {
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or("result contract lookup budget exhausted")?;
                if function.declaration == id {
                    return Ok(Some(function));
                }
            }
            Ok::<_, String>(None)
        };
        let Some(entry) = lookup(self.entry, fuel)? else {
            return Ok(None);
        };
        let Some((id, operands, call)) = CallableClauseView::direct_call(&entry.body, fuel)? else {
            return Ok(None);
        };
        let Some(target) = lookup(id, fuel)? else {
            return Ok(None);
        };
        let Some(binder) = target.result_type_binder else {
            return Ok(None);
        };
        if target.result_guarantees.is_empty() {
            return Ok(None);
        }
        if target.type_parameters.len() != operands.len() {
            return Err("result producer clause arity mismatch".into());
        }
        let owned;
        let domains = match &self.world.phase {
            Some(phase) => phase.parameter_domains(),
            None => {
                owned = package
                    .declaration_parameter_domains_checked(fuel)
                    .map_err(|e| e.to_string())?;
                &owned
            }
        };
        let domains =
            nir::function_parameters::ordered_domains(domains, &target.type_parameters, fuel)
                .map_err(|e| e.to_string())?;
        let clauses =
            clause_inputs::ResultClauses::new(&target.type_parameters, &domains, operands, fuel)?;
        // Entry slots are compacted. A capture can refer to a later slot of
        // the original declaration even when it is the first entry parameter.
        let Term::Function { parameters, .. } = entry.signature.callable_signature() else {
            return Err("deferred result entry requires a function signature".into());
        };
        if self.inputs.len() != parameters.len()
            || self.scope.parameter_slots.len() != parameters.len()
        {
            return Err("deferred result entry capture arity mismatch".into());
        }
        *fuel = fuel
            .checked_sub(parameters.len())
            .ok_or("result capture budget exhausted")?;
        let entry_frames = [parameters.iter().map(|parameter| parameter.sort).collect()];
        let source_frame = self.scope.input_frames.last();
        let mut captured = BTreeMap::new();
        let mut actuals = Vec::with_capacity(parameters.len());
        for (index, ((id, _, frozen), source_slot)) in self
            .inputs
            .iter()
            .zip(&self.scope.parameter_slots)
            .enumerate()
        {
            if entry.parameters.get(index) != Some(id) {
                return Err("deferred result entry capture binding mismatch".into());
            }
            let sort = parameters[index].sort;
            captured.insert(
                *id,
                Term::Bound {
                    depth: 0,
                    index,
                    sort,
                },
            );
            let value = frozen.as_ref().map(std::borrow::Cow::Borrowed).or_else(|| {
                let slot = (*source_slot)?;
                (sort != Sort::Runtime && source_frame?.get(slot) == Some(&sort)).then_some(
                    std::borrow::Cow::Owned(Term::Bound {
                        depth: 0,
                        index: slot,
                        sort,
                    }),
                )
            });
            actuals.push(value);
        }
        let supplied = actuals
            .iter()
            .enumerate()
            .filter_map(|(slot, value)| Some((slot, value.as_deref()?)))
            .collect();
        let observe = |demanded: &BTreeMap<usize, Sort>, fuel: &mut usize| {
            clause_inputs::ordinary_identities(call, demanded, |id| captured.get(id), fuel)
        };
        let restore =
            nir::DeclarationSubstitution::bounded(&[], &[], &[], &self.scope.input_frames, fuel)
                .map_err(|e| e.to_string())?;
        let mut facts = Vec::new();
        for predicate in &target.result_guarantees {
            let Some(compact) = clauses.specialize(predicate, observe, &entry_frames, fuel)? else {
                continue;
            };
            let mut fact =
                match restore.instantiate_call_predicate_bounded(&compact, &supplied, fuel) {
                    Ok(fact) => fact,
                    Err(nir::type_computations::Error::Scope(nir::TypeError::MissingArgument(
                        _,
                    ))) => continue,
                    Err(failure) => return Err(failure.to_string()),
                };
            if !self.scope.scoped_parameters.is_empty() {
                nir::copy_budget::charge_predicate_substitution(
                    &fact,
                    &self.scope.scoped_parameters,
                    &self.scope.scoped_arguments,
                    fuel,
                )
                .map_err(|e| format!("{e:?}"))?;
                fact = fact
                    .instantiate_parameters_in_scope(
                        &self.scope.scoped_parameters,
                        &self.scope.scoped_arguments,
                        &self.scope.input_frames,
                    )
                    .map_err(|e| e.to_string())?;
            }
            facts.push(fact);
        }
        Ok((!facts.is_empty()).then_some((binder, facts)))
    }
}
