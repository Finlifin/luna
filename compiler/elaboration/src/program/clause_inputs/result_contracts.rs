//! Specialize a direct producer's declared result facts, conditional on
//! reaching its return. The source computation still runs with all operands.
use super::*;
use std::borrow::Borrow;

pub(in crate::program) struct ResultClauses<'arg, 'domain> {
    operands: CallableClauseView<'arg>,
    slots: BTreeMap<DeclarationId, (usize, &'domain Term)>,
}

impl<'arg, 'domain> ResultClauses<'arg, 'domain> {
    pub(in crate::program) fn new<D: Borrow<Term>>(
        parameters: &[DeclarationId],
        domains: &'domain [D],
        operands: CallableClauseView<'arg>,
        fuel: &mut usize,
    ) -> Result<Self, String> {
        if parameters.len() != operands.len() || parameters.len() != domains.len() {
            return Err("result producer clause arity mismatch".into());
        }
        *fuel = fuel
            .checked_sub(parameters.len())
            .ok_or("result clause budget exhausted")?;
        let slots = parameters
            .iter()
            .copied()
            .zip(
                domains
                    .iter()
                    .enumerate()
                    .map(|(slot, domain)| (slot, domain.borrow())),
            )
            .collect();
        Ok(Self { operands, slots })
    }

    // A proposition needs only the formals it mentions and the dependencies
    // of their domains. Never follow a replacement as a callee template.
    pub(in crate::program) fn specialize(
        &self,
        predicate: &n::TypePredicate,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Option<n::TypePredicate>, String> {
        let mut steps = *fuel as u64;
        let observed = nir::copy_budget::predicate_inputs(predicate, &mut steps);
        *fuel = steps as usize;
        let used = observed.map_err(|e| e.to_string())?;
        if !used.parameters.is_empty() {
            return Ok(None);
        }
        let mut pending = used.declarations.into_keys().collect::<BTreeSet<_>>();
        let mut needed = BTreeMap::new();
        while let Some(id) = pending.pop_first() {
            let Some(&(slot, domain)) = self.slots.get(&id) else {
                continue;
            };
            if needed.contains_key(&slot) {
                continue;
            }
            let Some(value) = self.operands.identity(slot) else {
                return Ok(None);
            };
            nir::type_computations::rigid_declarations_bounded(domain, fuel, &mut pending)
                .map_err(|e| e.to_string())?;
            needed.insert(slot, (id, domain, value));
        }
        let mut ids = Vec::with_capacity(needed.len());
        let mut declared = Vec::with_capacity(needed.len());
        let mut actual = Vec::with_capacity(needed.len());
        for (id, domain, value) in needed.into_values() {
            ids.push(id);
            declared.push(domain);
            actual.push(nir::copy_budget::clone_term(value, fuel).map_err(|e| format!("{e:?}"))?);
        }
        match nir::DeclarationSubstitution::bounded_domains(
            &ids,
            &declared,
            &actual,
            nir::DeclarationInputScope::Caller(frames),
            fuel,
        )
        .and_then(|sub| sub.instantiate_predicate_bounded(predicate, fuel))
        {
            Ok(fact) => Ok(Some(fact)),
            Err(nir::type_computations::Error::Scope(nir::TypeError::InvalidBound { .. })) => {
                Ok(None)
            }
            Err(failure) => Err(failure.to_string()),
        }
    }
}

impl Checker<'_, '_> {
    /// The checked call may still lack operand identities in an open
    /// description. Only facts closed over the published result survive then.
    pub(in crate::program) fn source_result_facts(
        &self,
        owner: OwnerId,
        operands: Option<CallableClauseView<'_>>,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Option<(DeclarationId, Vec<n::TypePredicate>)>, String> {
        let Some((binder, guarantees)) = self.result_guarantees.get(&owner) else {
            return Ok(None);
        };
        let mut facts = Vec::new();
        if let Some(operands) = operands {
            let parameters = self
                .type_parameters
                .get(&owner)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mut domains = Vec::new();
            for id in parameters {
                let domain = self
                    .observe_input_domain(*id, || {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or(nir::type_computations::Error::Budget)?;
                        Ok(())
                    })
                    .map_err(|e| e.to_string())?;
                domains.push(domain.unwrap_or(&Term::Primitive(P::Type)));
            }
            let clauses = ResultClauses::new(parameters, &domains, operands, fuel)?;
            for guarantee in guarantees {
                if let Some(fact) = clauses.specialize(guarantee, frames, fuel)? {
                    facts.push(fact);
                }
            }
        } else {
            let subject = nir::DeclarationParameter {
                declaration: *binder,
                sort: Sort::Type,
            };
            for guarantee in guarantees {
                let mut steps = *fuel as u64;
                let closed = nir::copy_budget::closed_refinement(guarantee, subject, &mut steps);
                *fuel = steps as usize;
                if closed.map_err(|e| format!("{e:?}"))? {
                    facts.push(guarantee.clone());
                }
            }
        }
        Ok((!facts.is_empty()).then_some((*binder, facts)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_domain_observation_withholds_only_the_dependent_fact() {
        let domain = DeclarationId {
            package: 2,
            local: 1,
        };
        let value = DeclarationId {
            package: 2,
            local: 2,
        };
        let result = DeclarationId {
            package: 2,
            local: 3,
        };
        let formal = |declaration, sort| Term::Rigid { declaration, sort };
        let domain_type = formal(domain, Sort::Type);
        let subject = formal(result, Sort::Type);
        let parameters = [domain, value];
        let domains = [p(P::Type), domain_type.clone()];
        let local_operand = |index, ty| n::Expr {
            ty,
            source: n::SourceRange { start: 10, end: 15 },
            kind: K::Local(n::LocalId { owner: 1, index }),
        };
        let mut packet = n::ClauseOperands {
            arguments: vec![local_operand(0, p(P::Type)), local_operand(1, domain_type)],
            ..Default::default()
        };
        packet.observations.insert(1, Term::Bool(true));
        let independent = n::TypePredicate::Kind(subject.clone(), nir::TypeKind::Tuple);
        let tag = |argument| Term::Nominal {
            declaration: DeclarationId {
                package: 2,
                local: 4,
            },
            arguments: vec![argument],
        };
        let dependent =
            n::TypePredicate::Equal(subject.clone(), tag(formal(value, Sort::ComptimeValue)));
        let project = |packet: &n::ClauseOperands, fact, fuel: &mut usize| {
            ResultClauses::new(
                &parameters,
                &domains,
                CallableClauseView::Prepared(packet),
                fuel,
            )?
            .specialize(fact, &[], fuel)
        };
        assert_eq!(
            project(&packet, &independent, &mut 2048).unwrap(),
            Some(independent.clone())
        );
        assert_eq!(project(&packet, &dependent, &mut 2048).unwrap(), None);
        packet.observations.insert(0, p(P::Bool));
        assert!(project(&packet, &dependent, &mut 1).is_err());
        assert_eq!(
            project(&packet, &dependent, &mut 2048).unwrap(),
            Some(n::TypePredicate::Equal(subject, tag(Term::Bool(true))))
        );
    }
}
