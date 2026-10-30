//! Resume a candidate after inferred inputs complete another input's domain.
//! Provisional assignments remain local until the original head is replayed.
use super::*;

impl ImplementationHead {
    pub(super) fn resume_completed_domains(
        &self,
        candidate: &Engine,
        inputs: &[I],
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        mut budget: Budget,
        domains: &BTreeMap<DeclarationId, Term>,
        layouts: Option<&nir::program::Package>,
    ) -> Result<Option<HeadMatch>, TypeAssessment> {
        let pending = |error: String| TypeAssessment::Pending(error);
        let computation = |error: nir::type_computations::Error| match error {
            nir::type_computations::Error::Budget => pending(error.to_string()),
            error => TypeAssessment::Invalid(error.to_string()),
        };
        if budget.max_depth == 0 {
            return Err(pending("dependent head staging depth exhausted".into()));
        }
        budget.steps = validate(self, subject, interface_arguments, budget).map_err(pending)?;
        let parameters = self
            .parameter_binders()
            .map_err(|error| TypeAssessment::Invalid(error.to_string()))?;
        let mut known = Vec::new();
        let mut known_parameters = Vec::new();
        let mut known_by_position = vec![None; parameters.len()];
        for (position, (parameter, input)) in parameters.iter().zip(inputs).enumerate() {
            match candidate.inspect_type_metered(input, &mut budget) {
                Ok(value) => {
                    known_by_position[position] = Some(known.len());
                    known_parameters.push(*parameter);
                    known.push(value);
                }
                Err(crate::InferenceError::Unresolved(_)) => {}
                Err(error) => {
                    return Err(pending(format!("dependent input observation: {error:?}")));
                }
            }
        }
        if known.is_empty() || known.len() == parameters.len() {
            return Ok(None);
        }
        let mut replacements = Vec::new();
        let mut completed_domains = Vec::new();
        let mut changed = false;
        for (index, parameter) in parameters.iter().enumerate() {
            let declared = self
                .parameter_types
                .get(index)
                .unwrap_or(&Term::Primitive(nir::Primitive::Type));
            let domain = nir::type_computations::instantiate_parameters_bounded(
                declared,
                &known_parameters,
                &known,
                &[],
                &mut budget.steps,
            )
            .map_err(computation)?;
            let sort = if domain == Term::Primitive(nir::Primitive::Type) {
                Sort::Type
            } else {
                Sort::ComptimeValue
            };
            if let Some(position) = known_by_position[index] {
                nir::type_computations::charge_term(&known[position], &mut budget.steps)
                    .map_err(computation)?;
                replacements.push(known[position].clone());
            } else {
                changed |= sort != parameter.sort;
                replacements.push(Term::Rigid {
                    declaration: parameter.declaration,
                    sort,
                });
            }
            completed_domains.push(domain);
        }
        if !changed {
            return Ok(None);
        }
        let substitution = nir::DeclarationSubstitution::bounded(
            &self.type_parameters,
            &self.parameter_types,
            &replacements,
            &[],
            &mut budget.steps,
        )
        .map_err(computation)?;
        for (index, domain) in completed_domains.iter_mut().enumerate() {
            let declared = self
                .parameter_types
                .get(index)
                .unwrap_or(&Term::Primitive(nir::Primitive::Type));
            *domain = substitution
                .instantiate_bounded(declared, &mut budget.steps)
                .map_err(computation)?;
        }
        let subject_template = substitution
            .instantiate_bounded(&self.subject, &mut budget.steps)
            .map_err(computation)?;
        let interfaces = self
            .interface_arguments
            .iter()
            .map(|term| substitution.instantiate_bounded(term, &mut budget.steps))
            .collect::<Result<Vec<_>, _>>()
            .map_err(computation)?;
        let requirements = self
            .requirements
            .iter()
            .map(|term| substitution.instantiate_predicate_bounded(term, &mut budget.steps))
            .collect::<Result<Vec<_>, _>>()
            .map_err(computation)?;
        let unresolved: Vec<_> = (0..parameters.len())
            .filter(|index| known_by_position[*index].is_none())
            .collect();
        let residual = ImplementationHead {
            declaration: self.declaration,
            interface: self.interface,
            type_parameters: unresolved
                .iter()
                .map(|index| self.type_parameters[*index])
                .collect(),
            parameter_types: completed_domains
                .into_iter()
                .enumerate()
                .filter_map(|(index, domain)| known_by_position[index].is_none().then_some(domain))
                .collect(),
            subject: subject_template,
            interface_arguments: interfaces,
            requirements,
        };
        // A successful round strictly removes inferred parameters. Reserve a
        // separate share for replay against the unspecialized declaration.
        let next_budget = Budget {
            steps: budget.steps / 2,
            max_depth: budget.max_depth - 1,
        };
        budget.steps -= next_budget.steps;
        let mut matched = residual.match_subject_types_with_domains(
            subject,
            interface_arguments,
            context,
            next_budget,
            domains,
            layouts,
        )?;
        for (index, value) in unresolved.iter().zip(&matched.evidence.type_arguments) {
            nir::type_computations::charge_term(value, &mut budget.steps).map_err(computation)?;
            replacements[*index] = value.clone();
        }
        matched.evidence.type_arguments = replacements;
        let substitution = nir::DeclarationSubstitution::bounded(
            &self.type_parameters,
            &self.parameter_types,
            &matched.evidence.type_arguments,
            &[],
            &mut budget.steps,
        )
        .map_err(computation)?;
        let expected_requirements = self
            .requirements
            .iter()
            .map(|term| substitution.instantiate_predicate_bounded(term, &mut budget.steps))
            .collect::<Result<Vec<_>, _>>()
            .map_err(computation)?;
        if expected_requirements != matched.requirements {
            return Err(TypeAssessment::Invalid(
                "staged head requirements changed".into(),
            ));
        }
        budget.steps = validate(self, subject, interface_arguments, budget).map_err(pending)?;
        let mut original = self.clone();
        original.requirements.clear();
        original
            .verify_subject_evidence_with_domains(
                &matched.evidence,
                subject,
                interface_arguments,
                context,
                budget,
                domains,
                layouts,
                |proof, budget| context.verify_proof(proof, budget),
            )
            .map_err(|error| pending(format!("dependent head replay: {error:?}")))?;
        Ok(Some(matched))
    }
}
