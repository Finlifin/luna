//! Ordinary input binding for type contracts, separate from program predicates.
use super::*;
use nir::program::TypePredicate as P;

impl Interpreter<'_> {
    /// Observe an already instantiated requirement using this VM's evidence.
    /// A passive lookup cannot run a source query or initialize a Type member.
    pub(super) fn retains_capability(
        &self,
        requirement: &nir::program::TraitRequirement,
        fuel: &mut usize,
    ) -> Result<bool, ErrorKind> {
        for term in std::iter::once(&requirement.subject).chain(&requirement.interface_arguments) {
            nir::type_computations::charge_term(term, fuel)
                .map_err(|_| ErrorKind::StepsExhausted)?;
        }
        if self.assumed_interfaces.contains(&(
            requirement.interface_instance(),
            requirement.subject.clone(),
        )) {
            return Ok(true);
        }
        let subject = selected_types::view(&requirement.subject, &self.associated_values, fuel)?;
        let arguments = requirement
            .interface_arguments
            .iter()
            .map(|value| selected_types::view(value, &self.associated_values, fuel))
            .collect::<Result<Vec<_>, _>>()?;
        let key = (
            InterfaceInstance {
                declaration: requirement.interface,
                arguments,
            },
            subject,
        );
        Ok(
            self.implemented_interfaces.contains_key(&key)
                || self.assumed_interfaces.contains(&key),
        )
    }

    /// A dispatch table is retained positive evidence, not a complete account
    /// of the source world's capabilities. Demand missing evidence through the
    /// existing bounded type-query service without installing method bodies.
    pub(super) fn check_capability_contract(
        &mut self,
        requirement: &nir::program::TraitRequirement,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.tick(Some(source))?;
        self.charge_copy_work(Some(source), |fuel| {
            for term in
                std::iter::once(&requirement.subject).chain(&requirement.interface_arguments)
            {
                nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let retained = self.retains_capability(requirement, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        match retained {
            Ok(true) => return Ok(()),
            // A missing passive selection still permits the ordinary bounded
            // proof query. Resource failure must not start a second attempt.
            Ok(false) | Err(ErrorKind::UnmaterializedDispatch) => {}
            Err(kind) => return Err(self.failure(kind, Some(source))),
        }
        self.check_type_contract(&P::Trait(requirement.clone()), source)
    }

    /// Discharge closed obligations once in the receiving world. Only the
    /// specialized predicates that read ordinary binders travel to binding.
    #[inline(never)]
    pub(super) fn pending_callable_requirements(
        &mut self,
        formation: nir::callable_source::Formation<'_>,
        inputs: &nir::DeclarationSubstitution<'_>,
        source: SourceRange,
    ) -> Result<Vec<P>, Error> {
        let mut pending = Vec::new();
        for requirement in formation.requirements {
            let predicate = self.query_admission(Some(source), |_, fuel| {
                Ok(inputs.instantiate_predicate_bounded(requirement, fuel)?)
            })?;
            if formation_predicate_is_closed(&predicate) {
                self.check_type_contract(&predicate, source)?;
            } else {
                pending.push(predicate);
            }
        }
        for capability in formation.capabilities {
            let requirement = self.query_admission(Some(source), |_, fuel| {
                Ok(inputs.instantiate_requirement_bounded(capability, fuel)?)
            })?;
            if !formation_capability_is_closed(&requirement) {
                pending.push(P::Trait(requirement));
            } else if self.graph.is_none() {
                // Concrete IIR already carries the producer's closed evidence.
                self.check_capability_contract(&requirement, source)?;
            }
        }
        Ok(pending)
    }

    /// Fill only obligations retained by this activation, using its original
    /// ordinary binding. Keep the predicate tree and its argument buffers.
    pub(super) fn bind_invocation_contract(
        &mut self,
        predicate: &mut P,
        scope: &nir::FunctionInputBinding,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        self.tick(Some(source))?;
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        let mut rebind =
            |term: &mut Term| {
                *term = self.query_admission(Some(source), |_, fuel| {
                    Ok(scope.rebind_bounded(term, fuel)?)
                })?;
                term.check_scope(&[])
                    .map_err(|_| self.failure(ErrorKind::TypeContractUnresolved, Some(source)))
            };
        match predicate {
            P::Bool(_) | P::IntLe(..) => Ok(()),
            P::Equal(a, b) => {
                rebind(a)?;
                rebind(b)
            }
            P::Kind(ty, _) | P::NotSized(ty) => rebind(ty),
            P::Trait(requirement) => {
                rebind(&mut requirement.subject)?;
                requirement
                    .interface_arguments
                    .iter_mut()
                    .try_for_each(rebind)
            }
            P::Not(inner) => self.bind_invocation_contract(inner, scope, source, depth + 1),
            P::And(items) | P::Or(items) => items
                .iter_mut()
                .try_for_each(|item| self.bind_invocation_contract(item, scope, source, depth + 1)),
        }
    }
}

pub(super) fn formation_capability_is_closed(requirement: &nir::program::TraitRequirement) -> bool {
    std::iter::once(&requirement.subject)
        .chain(&requirement.interface_arguments)
        .all(|term| term.check_scope(&[]).is_ok())
}

pub(super) fn formation_predicate_is_closed(predicate: &nir::program::TypePredicate) -> bool {
    use nir::program::TypePredicate as P;
    match predicate {
        P::Bool(_) | P::IntLe(..) => true,
        P::Trait(requirement) => formation_capability_is_closed(requirement),
        P::Equal(left, right) => left.check_scope(&[]).is_ok() && right.check_scope(&[]).is_ok(),
        P::Kind(ty, _) | P::NotSized(ty) => ty.check_scope(&[]).is_ok(),
        P::Not(inner) => formation_predicate_is_closed(inner),
        P::And(parts) | P::Or(parts) => parts.iter().all(formation_predicate_is_closed),
    }
}
