//! Interface inputs are admitted against their declared domains, not names.
use super::*;
use nir::{Primitive, Sort};

impl DeclarationWorld<'_> {
    pub fn normalized_interface_arguments(
        &mut self,
        declaration: Id,
        subject: &Term,
        arguments: &[Term],
        budget: &mut Budget,
        declared: &mut dyn FnMut(&Term, &Term, &mut usize) -> Result<bool, AdmissionError>,
    ) -> Result<Vec<Term>, AdmissionError> {
        budget.steps = budget.steps.checked_sub(1).ok_or(AdmissionError::Budget)?;
        let contract = self
            .scope
            .source
            .package()
            .traits
            .iter()
            .find(|contract| contract.declaration == declaration)
            .ok_or("missing dispatch interface input contract")?;
        budget.steps = budget
            .steps
            .checked_sub(arguments.len().saturating_add(1))
            .ok_or(AdmissionError::Budget)?;
        let mut inputs = Vec::with_capacity(arguments.len().saturating_add(1));
        inputs.push(self.normalized_type(subject, *budget, &mut budget.steps)?);
        for argument in arguments {
            inputs.push(if argument.sort() == Sort::Type {
                self.normalized_type(argument, *budget, &mut budget.steps)?
            } else {
                nir::type_computations::charge_term(argument, &mut budget.steps)?;
                argument.clone()
            });
        }
        let substitution = contract.input_substitution(&inputs, &[], &mut budget.steps)?;
        for (index, argument) in inputs.iter().skip(1).enumerate() {
            let domain = contract
                .parameter_types
                .get(index)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            let domain = substitution.instantiate_bounded(domain, &mut budget.steps)?;
            let domain = self.normalized_type(&domain, *budget, &mut budget.steps)?;
            if domain == Term::Primitive(Primitive::Type) {
                continue;
            }
            self.with_admission_queries(|queries| {
                nir::value_admission::validate(
                    argument,
                    &domain,
                    &[],
                    &mut budget.steps,
                    &mut nir::value_admission::DomainQueries {
                        layout: queries.layout,
                        declared,
                        collection: queries.collection,
                        callable: queries.callable,
                        target: queries.target,
                    },
                )
            })?;
        }
        Ok(inputs.into_iter().skip(1).collect())
    }
}
