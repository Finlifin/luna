//! Replay against the original declaration before publishing a staged head.
//! Domain-query failures remain distinct from invalid or out-of-range proofs.
use super::*;
use nir::NominalAdmissionError as QueryError;

impl ImplementationHead {
    pub(in crate::implementation) fn verify_subject_evidence_with_domains(
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
    ) -> Result<(), EvidenceError> {
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
            return Err(EvidenceError::Proof(ProofError::Invalid));
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
            return Err(EvidenceError::Proof(ProofError::BudgetExhausted));
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
                _ => return Err(EvidenceError::Proof(ProofError::Invalid)),
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
            return Err(EvidenceError::Proof(ProofError::Invalid));
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
                return Err(EvidenceError::Proof(ProofError::Invalid));
            }
            expected.push(Formula::equal(template, actual.clone()));
        }
        if expected.len() != evidence.head_proofs.len()
            || self.requirements.len() != evidence.requirement_proofs.len()
        {
            return Err(EvidenceError::Proof(ProofError::Invalid));
        }
        for (goal, proof) in expected.iter().zip(&evidence.head_proofs) {
            if goal != &proof.goal {
                return Err(EvidenceError::Proof(ProofError::Invalid));
            }
            context.verify_proof(proof, budget)?;
        }
        for (requirement, proof) in self.requirements.iter().zip(&evidence.requirement_proofs) {
            let instantiated =
                substitution.instantiate_predicate_bounded(requirement, &mut domain_fuel)?;
            if Formula::from_predicate(&instantiated) != proof.goal {
                return Err(EvidenceError::Proof(ProofError::Invalid));
            }
            verify_requirement(proof, budget)?;
        }
        Ok(())
    }
}
