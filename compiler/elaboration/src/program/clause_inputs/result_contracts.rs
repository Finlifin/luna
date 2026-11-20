//! Specialize a direct producer's declared result facts, conditional on
//! reaching its return. The source computation still runs with all operands.
use super::*;
use std::borrow::Borrow;

/// Read only already represented identities. A default's local names its
/// declaration-side prefix, whereas an explicit operand reads the caller.
pub(in crate::program) fn ordinary_identities<'a>(
    call: &'a n::Expr,
    demanded: &BTreeMap<usize, Sort>,
    local: impl Fn(&n::LocalId) -> Option<&'a Term>,
    fuel: &mut usize,
) -> Result<BTreeMap<usize, &'a Term>, String> {
    let K::Call {
        callee, arguments, ..
    } = &call.kind
    else {
        return Err("result producer requires a checked call".into());
    };
    let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
        return Err("result producer requires a callable signature".into());
    };
    *fuel = fuel
        .checked_sub(arguments.len())
        .ok_or("result argument budget exhausted")?;
    let mut locals: BTreeMap<_, _> = arguments
        .iter()
        .filter_map(|arg| {
            if let K::Local(id) = arg.value.kind
                && demanded.contains_key(&arg.parameter)
                && !arg.is_default
                && arg.passing == n::ArgumentPassing::Value
                && local(&id).is_some()
            {
                Some((id, &arg.value))
            } else {
                None
            }
        })
        .collect();
    Checker::invalidate_mutated_type_bindings(callee, &mut locals, fuel)?;
    let mut known = BTreeMap::new();
    for arg in arguments {
        let param = parameters
            .get(arg.parameter)
            .ok_or("invalid result producer argument slot")?;
        if demanded.contains_key(&arg.parameter)
            && param.sort != Sort::Runtime
            && arg.passing == n::ArgumentPassing::Value
        {
            let identity = match &arg.value.kind {
                K::TypeValue(ty) | K::ComptimeValue(ty) => Some(ty),
                K::Local(id) if !arg.is_default && locals.contains_key(id) => local(id),
                _ => None,
            };
            if let Some(identity) = identity {
                known.insert(arg.parameter, identity);
            }
        }
        Checker::invalidate_mutated_type_bindings(&arg.value, &mut locals, fuel)?;
    }
    Ok(known)
}

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
    pub(in crate::program) fn specialize<'v>(
        &self,
        predicate: &n::TypePredicate,
        observe: impl FnOnce(
            &BTreeMap<usize, Sort>,
            &mut usize,
        ) -> Result<BTreeMap<usize, &'v Term>, String>,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Option<n::TypePredicate>, String> {
        let mut steps = *fuel as u64;
        let observed = nir::copy_budget::predicate_inputs(predicate, &mut steps);
        *fuel = steps as usize;
        let used = observed.map_err(|e| e.to_string())?;
        let ordinary = if used.parameters.is_empty() {
            BTreeMap::new()
        } else {
            observe(&used.parameters, fuel)?
        };
        if used
            .parameters
            .iter()
            .any(|(slot, sort)| ordinary.get(slot).is_none_or(|value| value.sort() != *sort))
        {
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
        .and_then(|sub| sub.instantiate_call_predicate_bounded(predicate, &ordinary, fuel))
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
        operands: Option<(CallableClauseView<'_>, &n::Expr)>,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Option<(DeclarationId, Vec<n::TypePredicate>)>, String> {
        let Some((binder, guarantees)) = self.result_guarantees.get(&owner) else {
            return Ok(None);
        };
        let mut facts = Vec::new();
        if let Some((operands, call)) = operands {
            let observe = |demanded: &BTreeMap<usize, Sort>, fuel: &mut usize| {
                ordinary_identities(
                    call,
                    demanded,
                    |id| {
                        self.input_facts
                            .type_identity(id)
                            .or_else(|| self.input_facts.value_identity(id).map(|(value, _)| value))
                    },
                    fuel,
                )
            };
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
                if let Some(fact) = clauses.specialize(guarantee, observe, frames, fuel)? {
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
            .specialize(
                fact,
                |_, _| panic!("clause-only fact read ordinary arguments"),
                &[],
                fuel,
            )
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
