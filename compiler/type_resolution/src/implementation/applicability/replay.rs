//! Replay against the original declaration before publishing a staged head.
//! Domain-query failures remain distinct from invalid or out-of-range proofs.
use super::*;
use crate::ptc::ProofError;
use nir::NominalAdmissionError as QueryError;

#[derive(Debug)]
pub(super) enum Failure {
    Query(QueryError),
    Proof(ProofError),
}
impl From<QueryError> for Failure {
    fn from(error: QueryError) -> Self {
        Self::Query(error)
    }
}
impl From<nir::type_computations::Error> for Failure {
    fn from(error: nir::type_computations::Error) -> Self {
        Self::Query(error.into())
    }
}
impl From<ProofError> for Failure {
    fn from(error: ProofError) -> Self {
        Self::Proof(error)
    }
}
impl Failure {
    pub(super) fn into_assessment(self) -> TypeAssessment {
        match self {
            Self::Query(QueryError::Budget) | Self::Proof(ProofError::BudgetExhausted) => {
                TypeAssessment::BudgetExhausted
            }
            Self::Query(QueryError::Unresolved(reason)) => TypeAssessment::Pending(reason),
            Self::Query(QueryError::Invalid(reason)) => TypeAssessment::Invalid(reason),
            Self::Proof(ProofError::ArithmeticRange) => TypeAssessment::Pending(
                "dependent head replay exceeds the arithmetic proof range".into(),
            ),
            Self::Proof(ProofError::Invalid) => TypeAssessment::Invalid(
                "dependent head evidence failed replay against the original declaration".into(),
            ),
        }
    }
    // The old public proof API has no unresolved-query variant. Keep this lossy
    // compatibility conversion at its boundary, outside staged candidate replay.
    pub(super) fn into_proof_error(self) -> ProofError {
        match self {
            Self::Proof(error) => error,
            Self::Query(QueryError::Budget) => ProofError::BudgetExhausted,
            Self::Query(_) => ProofError::Invalid,
        }
    }
}

impl ImplementationHead {
    pub(super) fn replay_subject_evidence(
        &self,
        evidence: &TypeEvidence,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
        domains: &BTreeMap<DeclarationId, Term>,
        layouts: Option<&nir::program::Package>,
        mut verify_requirement: impl FnMut(
            &crate::ptc::Proof,
            Budget,
        ) -> Result<(), crate::ptc::ProofError>,
    ) -> Result<(), Failure> {
        use crate::ptc::{Equality, ProofError};
        let remaining = validate(self, subject, interface_arguments, budget)?;
        let remaining = validate(
            self,
            subject,
            &evidence.type_arguments,
            Budget {
                steps: remaining,
                max_depth: budget.max_depth,
            },
        )?;
        if evidence.implementation != self.declaration
            || evidence.type_arguments.len() != self.type_parameters.len()
            || interface_arguments.len() != self.interface_arguments.len()
        {
            return Err(Failure::Proof(ProofError::Invalid));
        }
        let parameters = self
            .parameter_binders()
            .map_err(|error| QueryError::Invalid(error.to_string()))?;
        let mut domain_fuel = remaining;
        validate_head_arguments_with_domains(
            &parameters,
            &self.parameter_types,
            &evidence.type_arguments,
            domains,
            Some((context, budget.max_depth)),
            layouts,
            &mut domain_fuel,
        )?;
        let substitution = nir::DeclarationSubstitution::bounded(
            &self.type_parameters,
            &self.parameter_types,
            &evidence.type_arguments,
            context.scope(),
            &mut domain_fuel,
        )?;
        let allowance =
            domain_fuel / (4 + evidence.head_proofs.len() + evidence.requirement_proofs.len());
        if allowance == 0 {
            return Err(Failure::Proof(ProofError::BudgetExhausted));
        }
        // Proof replay has no returned meter. Reserve each proof slot once;
        // value traversal and substitution may spend only the remainder.
        let proof_slots = evidence.head_proofs.len()
            + evidence.requirement_proofs.len()
            + usize::from(evidence.eligibility_proof.is_some());
        domain_fuel -= allowance * proof_slots;
        let budget = Budget {
            steps: allowance,
            max_depth: budget.max_depth,
        };
        let shape = if let Some(proof) = &evidence.eligibility_proof {
            context.verify_proof(proof, budget)?;
            match &proof.goal {
                Formula::Equal(Equality(original, shape)) if original == subject => shape,
                goal if eligible_alternatives(context, subject, goal) => subject,
                _ => return Err(Failure::Proof(ProofError::Invalid)),
            }
        } else {
            subject
        };
        if !eligible_shape(context, shape)
            && !evidence
                .eligibility_proof
                .as_ref()
                .is_some_and(|proof| eligible_alternatives(context, subject, &proof.goal))
        {
            return Err(Failure::Proof(ProofError::Invalid));
        }
        let instantiated = substitution.instantiate_bounded(&self.subject, &mut domain_fuel)?;
        let mut expected = vec![Formula::equal(instantiated.clone(), subject.clone())];
        if shape != subject {
            expected.push(Formula::equal(instantiated, shape.clone()));
        }
        for (template, actual) in self.interface_arguments.iter().zip(interface_arguments) {
            let template = substitution.instantiate_bounded(template, &mut domain_fuel)?;
            if template == *actual
                && matches!(actual, Term::Rigid { declaration, sort: Sort::ComptimeValue } if domains.contains_key(declaration))
            {
                continue;
            }
            if template.is_symbolic_index_value() && actual.is_symbolic_index_value() {
                let type_relations = template
                    .index_type_relations_budgeted(actual, &mut domain_fuel)?
                    .ok_or(ProofError::Invalid)?;
                for (left, right) in type_relations {
                    expected.push(Formula::equal(left.clone(), right.clone()));
                }
                continue;
            }
            if template.sort() != Sort::Type || actual.sort() != Sort::Type {
                return Err(Failure::Proof(ProofError::Invalid));
            }
            expected.push(Formula::equal(template, actual.clone()));
        }
        if expected.len() != evidence.head_proofs.len()
            || self.requirements.len() != evidence.requirement_proofs.len()
        {
            return Err(Failure::Proof(ProofError::Invalid));
        }
        for (goal, proof) in expected.iter().zip(&evidence.head_proofs) {
            if goal != &proof.goal {
                return Err(Failure::Proof(ProofError::Invalid));
            }
            context.verify_proof(proof, budget)?;
        }
        for (requirement, proof) in self.requirements.iter().zip(&evidence.requirement_proofs) {
            let instantiated =
                substitution.instantiate_predicate_bounded(requirement, &mut domain_fuel)?;
            if Formula::from_predicate(&instantiated) != proof.goal {
                return Err(Failure::Proof(ProofError::Invalid));
            }
            verify_requirement(proof, budget)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 1, local }
    }
    fn ty(local: u32) -> Term {
        Term::Rigid {
            declaration: id(local),
            sort: Sort::Type,
        }
    }
    fn value(local: u32) -> Term {
        Term::Rigid {
            declaration: id(local),
            sort: Sort::ComptimeValue,
        }
    }
    fn domain(argument: Term) -> Term {
        Term::NormalForm {
            head: id(80),
            sort: Sort::Type,
            arguments: vec![argument],
        }
    }

    #[test]
    fn replay_retains_unresolved_domains_then_recovers_with_congruence_evidence() {
        let subject = Term::Primitive(nir::Primitive::Bool);
        let head = ImplementationHead {
            declaration: id(90),
            interface: Some(id(100)),
            subject: subject.clone(),
            type_parameters: vec![id(3)],
            parameter_types: vec![domain(ty(1))],
            interface_arguments: vec![value(3)],
            requirements: vec![],
        };
        let arguments = [value(4)];
        let domains = BTreeMap::from([(id(4), domain(ty(2)))]);
        let mut incomplete = Context::default();
        incomplete.assume(Formula::equal(ty(1), ty(2)));
        let mut complete = incomplete.clone();
        complete.admit_congruence(id(80));
        let matched = head
            .match_types_with_domains(&subject, &arguments, &complete, Budget::default(), &domains)
            .unwrap();
        let replay = |context: &Context, budget| {
            head.replay_subject_evidence(
                &matched.evidence,
                &subject,
                &arguments,
                context,
                budget,
                &domains,
                None,
                |proof, allowance| context.verify_proof(proof, allowance),
            )
        };
        let failure = replay(&incomplete, Budget::default()).unwrap_err();
        assert!(
            matches!(&failure, Failure::Query(QueryError::Unresolved(reason)) if reason.contains("UnvalidatedModel"))
        );
        assert!(matches!(
            failure.into_assessment(),
            TypeAssessment::Pending(_)
        ));
        for steps in [0, 1, 10] {
            let failure = replay(
                &complete,
                Budget {
                    steps,
                    ..Budget::default()
                },
            )
            .unwrap_err();
            assert!(matches!(
                failure.into_assessment(),
                TypeAssessment::BudgetExhausted
            ));
        }
        replay(&complete, Budget::default()).unwrap();
        let mut forged = matched.evidence.clone();
        forged.head_proofs.clear();
        let failure = head
            .replay_subject_evidence(
                &forged,
                &subject,
                &arguments,
                &complete,
                Budget::default(),
                &domains,
                None,
                |proof, allowance| complete.verify_proof(proof, allowance),
            )
            .unwrap_err();
        assert!(matches!(
            failure.into_assessment(),
            TypeAssessment::Invalid(_)
        ));
        replay(&complete, Budget::default()).unwrap();
    }
}
