//! One bounded construction path for a completed source declaration index.
use super::{Budget, ImplementationHead, Index};
use crate::declaration_budget as construction;
use nir::{NominalAdmissionError as Failure, program::Package};

impl Index {
    /// Index declaration-side type metadata with a single shared allowance.
    /// No code, program contract, or invocation result is visited. This creates
    /// candidates and their domains, not evidence that any trait is satisfied.
    /// Failure drops the private partial index and preserves actual unused fuel.
    pub fn from_package_metered(package: &Package, budget: &mut Budget) -> Result<Self, Failure> {
        construction::spend(
            &mut budget.steps,
            package.builtin_implementation_packages.len(),
        )?;
        construction::layouts(package, &mut budget.steps)?;
        let mut index = Self::for_builtin_packages(
            package.target,
            package.builtin_implementation_packages.iter().copied(),
        );
        index.set_normal_form_metadata(package.normal_forms.clone());
        index.sync_nominal_layouts(&package.structs, &package.enums, &package.newtypes);
        for implementation in &package.implementations {
            construction::spend(&mut budget.steps, 1)?;
            // Completed inherent impls belong to member lookup, not trait
            // candidates. Their declared parameter domains are still retained.
            if implementation.interface.is_none() {
                continue;
            }
            construction::implementation(implementation, &mut budget.steps)?;
            index
                .insert_metered(
                    ImplementationHead {
                        parameter_types: implementation.parameter_types.clone(),
                        declaration: implementation.declaration,
                        interface: implementation.interface,
                        interface_arguments: implementation.interface_arguments.clone(),
                        subject: implementation.subject.clone(),
                        type_parameters: implementation.type_parameters.clone(),
                        requirements: implementation.requirements.clone(),
                    },
                    budget,
                )
                .map_err(|error| match error {
                    super::Error::BudgetExhausted => Failure::Budget,
                    error => Failure::Invalid(format!("source implementation index: {error:?}")),
                })?;
        }
        let domains = package.declaration_parameter_domains_checked(&mut budget.steps)?;
        for (id, domain) in domains {
            index.register_parameter_domain_metered(id, domain, &mut budget.steps)?;
        }
        for (runtime, domain) in &package.runtime_domains {
            index.register_runtime_domain_metered(*runtime, domain.clone(), &mut budget.steps)?;
        }
        Ok(index)
    }
}
