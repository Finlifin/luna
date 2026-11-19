//! A source member selection closes type obligations, never executable code.
use super::*;
use crate::ptc::{Outcome, UnknownReason};
use std::collections::{HashSet, VecDeque};

impl DeclarationWorld<'_> {
    /// Select a method in this immutable world. Interface formation and its
    /// transitive premises are checked here. The VM admits actual method inputs
    /// and checks the specialized member's type/program contracts at invocation.
    pub fn method_identity(
        &mut self,
        callee: &n::Expr,
        budget: &mut Budget,
    ) -> Result<n::MethodInstance, AdmissionError> {
        let n::ExprKind::TraitMethod {
            interface,
            interface_arguments,
            subject,
            member,
            method_arguments,
        } = &callee.kind
        else {
            return Err("method selection requires a trait method expression".into());
        };
        for term in std::iter::once(subject)
            .chain(interface_arguments)
            .chain(method_arguments)
        {
            nir::type_computations::charge_term(term, &mut budget.steps)?;
        }
        let mut pending = VecDeque::from([(
            n::TraitRequirement {
                interface: *interface,
                interface_arguments: interface_arguments.clone(),
                subject: subject.clone(),
            },
            true,
        )]);
        let mut seen = HashSet::new();
        let mut selected = None;
        while let Some((requirement, root)) = pending.pop_front() {
            budget.steps = budget.steps.checked_sub(1).ok_or(AdmissionError::Budget)?;
            let subject = self.normalized_type(&requirement.subject, *budget, &mut budget.steps)?;
            let arguments = self.normalized_interface_arguments(
                requirement.interface,
                &subject,
                &requirement.interface_arguments,
                budget,
                &mut |_, _, _| Ok(false),
            )?;
            let requirement = n::TraitRequirement {
                subject,
                interface: requirement.interface,
                interface_arguments: arguments,
            };
            for input in
                std::iter::once(&requirement.subject).chain(&requirement.interface_arguments)
            {
                nir::type_computations::charge_term(input, &mut budget.steps)?;
            }
            if !seen.insert((
                requirement.interface_instance(),
                requirement.subject.clone(),
            )) {
                continue;
            }
            let world = self.capabilities();
            if !root && world.given_metered(&requirement, budget)?.is_some() {
                continue;
            }
            let interface = requirement.interface_instance();
            let (implementation, arguments) =
                self.select_interface(&interface, &requirement.subject, budget)?;
            let contract = self
                .scope
                .package
                .traits
                .iter()
                .find(|t| t.declaration == requirement.interface)
                .ok_or("missing method interface")?;
            let implementation_contract = self
                .scope
                .package
                .implementations
                .iter()
                .find(|i| i.declaration == implementation)
                .ok_or("missing selected implementation")?;
            let implementation_substitution = nir::DeclarationSubstitution::bounded(
                &implementation_contract.type_parameters,
                &implementation_contract.parameter_types,
                &arguments,
                &[],
                &mut budget.steps,
            )?;
            for capability in &implementation_contract.capabilities {
                pending.push_back((
                    implementation_substitution
                        .instantiate_requirement_bounded(capability, &mut budget.steps)?,
                    false,
                ));
            }
            let inputs: Vec<_> = std::iter::once(requirement.subject.clone())
                .chain(requirement.interface_arguments.iter().cloned())
                .collect();
            let substitution = contract.input_substitution(&inputs, &[], &mut budget.steps)?;
            for predicate in &contract.requirements {
                let predicate =
                    substitution.instantiate_predicate_bounded(predicate, &mut budget.steps)?;
                match world.check_predicate_metered(&predicate, budget)? {
                    Outcome::Entailed(_) => {}
                    Outcome::Unknown {
                        reason: UnknownReason::BudgetExhausted,
                        ..
                    } => return Err(AdmissionError::Budget),
                    Outcome::IllFormed(reason) => return Err(AdmissionError::Invalid(reason)),
                    _ => {
                        return Err(AdmissionError::Unresolved(
                            "method interface formation is not established".into(),
                        ));
                    }
                }
            }
            for capability in &contract.capabilities {
                pending.push_back((
                    substitution.instantiate_requirement_bounded(capability, &mut budget.steps)?,
                    false,
                ));
            }
            for parent in &contract.parents {
                let parent =
                    substitution.instantiate_interface_bounded(parent, &mut budget.steps)?;
                nir::type_computations::charge_term(&requirement.subject, &mut budget.steps)?;
                pending.push_back((
                    n::TraitRequirement {
                        subject: requirement.subject.clone(),
                        interface: parent.declaration,
                        interface_arguments: parent.arguments,
                    },
                    false,
                ));
            }
            if root {
                let instance = n::DispatchInstance {
                    interface: requirement.interface,
                    interface_arguments: requirement.interface_arguments,
                    subject: requirement.subject,
                    implementation,
                    arguments,
                    methods: vec![],
                    associated: vec![],
                    associated_initializers: vec![],
                };
                selected = Some(self.prepare_method_identity(
                    &instance,
                    *member,
                    method_arguments,
                    &mut budget.steps,
                )?);
            }
        }
        selected.ok_or_else(|| "method selection produced no identity".into())
    }
}
