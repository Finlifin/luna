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
        let Some((id, operands)) = CallableClauseView::direct_call(&entry.body, fuel)? else {
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
        let mut facts = Vec::new();
        for predicate in &target.result_guarantees {
            let Some(mut fact) = clauses.specialize(predicate, &[], fuel)? else {
                continue;
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
            // The receiving refinement owns a separate result binder. A free
            // ordinary slot cannot be imported there by numerical coincidence.
            match nir::copy_budget::predicate_capture_slots(&fact, &[], fuel) {
                Ok(_) => facts.push(fact),
                Err(nir::type_computations::Error::Scope(nir::TypeError::InvalidBound {
                    ..
                })) => {}
                Err(failure) => return Err(failure.to_string()),
            }
        }
        Ok((!facts.is_empty()).then_some((binder, facts)))
    }
}
