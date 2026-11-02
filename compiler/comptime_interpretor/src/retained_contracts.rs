//! Positive type evidence retained by concrete materialization.
use super::*;
use nir::program::TypePredicate as P;

impl Interpreter<'_> {
    pub(super) fn retained_type_contract(
        &mut self,
        predicate: &P,
        depth: usize,
    ) -> ContractOutcome {
        use ContractOutcome::*;
        // A source declaration catalog is not a proof of applicability. Only
        // concrete dispatch selected by materialization supplies this evidence.
        if self.graph.is_none() || depth >= 128 || self.tick(None).is_err() {
            return Unknown;
        }
        match predicate {
            P::Bool(value) => {
                if *value {
                    Satisfied
                } else {
                    Rejected
                }
            }
            P::Equal(left, right) => {
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let charged = [left, right]
                    .into_iter()
                    .try_for_each(|ty| nir::type_computations::charge_term(ty, &mut fuel));
                self.budget.steps -= (before - fuel) as u64;
                if charged.is_err() {
                    return Unknown;
                }
                match predicate.trivial_truth() {
                    Some(true) => Satisfied,
                    Some(false) => Rejected,
                    None => Unknown,
                }
            }
            P::Trait(requirement) => {
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let charged = std::iter::once(&requirement.subject)
                    .chain(&requirement.interface_arguments)
                    .try_for_each(|ty| nir::type_computations::charge_term(ty, &mut fuel));
                self.budget.steps -= (before - fuel) as u64;
                if charged.is_err() {
                    return Unknown;
                }
                let key = (
                    requirement.interface_instance(),
                    requirement.subject.clone(),
                );
                if self.implemented_interfaces.contains_key(&key) {
                    Satisfied
                } else {
                    Unknown
                }
            }
            P::Not(inner) => match self.retained_type_contract(inner, depth + 1) {
                Satisfied => Rejected,
                Rejected => Satisfied,
                Unknown => Unknown,
            },
            P::And(parts) | P::Or(parts) => {
                let conjunction = matches!(predicate, P::And(_));
                let mut unknown = false;
                for part in parts {
                    match self.retained_type_contract(part, depth + 1) {
                        Rejected if conjunction => return Rejected,
                        Satisfied if !conjunction => return Satisfied,
                        Unknown => unknown = true,
                        _ => {}
                    }
                }
                if unknown {
                    Unknown
                } else if conjunction {
                    Satisfied
                } else {
                    Rejected
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requirement(tag: bool) -> nir::program::TraitRequirement {
        nir::program::TraitRequirement {
            interface: DeclarationId {
                package: 81,
                local: 1,
            },
            interface_arguments: vec![Term::Bool(tag)],
            subject: Term::Primitive(Primitive::Bool),
        }
    }

    fn graph() -> iir::Package {
        let selected = requirement(true);
        iir::Package {
            interfaces: vec![iir::InterfaceContract {
                interface: selected.interface_instance(),
                subject: selected.subject.clone(),
                methods: vec![],
            }],
            dispatch: vec![nir::program::DispatchInstance {
                interface: selected.interface,
                interface_arguments: selected.interface_arguments,
                subject: selected.subject,
                implementation: DeclarationId {
                    package: 81,
                    local: 2,
                },
                arguments: vec![],
                methods: vec![],
                associated: vec![],
                associated_initializers: vec![],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn retained_dispatch_proves_only_the_exact_positive_trait_instance() {
        let graph = graph();
        let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        let known = P::Trait(requirement(true));
        let missing = P::Trait(requirement(false));
        let mut wrong_subject = requirement(true);
        wrong_subject.subject = Term::Primitive(Primitive::Char);
        let mut wrong_interface = requirement(true);
        wrong_interface.interface.local += 1;
        for predicate in [
            missing.clone(),
            P::Trait(wrong_subject),
            P::Trait(wrong_interface),
        ] {
            assert_eq!(
                vm.solve_type_predicate(&predicate),
                ContractOutcome::Unknown
            );
            assert_eq!(
                vm.solve_type_predicate(&P::Not(Box::new(predicate))),
                ContractOutcome::Unknown
            );
        }
        assert_eq!(vm.solve_type_predicate(&known), ContractOutcome::Satisfied);
        assert_eq!(
            vm.solve_type_predicate(&P::Not(Box::new(known.clone()))),
            ContractOutcome::Rejected
        );
        for (predicate, expected) in [
            (
                P::And(vec![known.clone(), missing.clone()]),
                ContractOutcome::Unknown,
            ),
            (
                P::Or(vec![missing.clone(), known.clone()]),
                ContractOutcome::Satisfied,
            ),
            (
                P::And(vec![missing, P::Bool(false)]),
                ContractOutcome::Rejected,
            ),
        ] {
            assert_eq!(vm.solve_type_predicate(&predicate), expected);
        }
        vm.budget.steps = 0;
        assert_eq!(vm.solve_type_predicate(&known), ContractOutcome::Unknown);
        vm.budget.steps = 1000;
        assert_eq!(vm.solve_type_predicate(&known), ContractOutcome::Satisfied);
        let missing_graph = iir::Package::default();
        let mut missing_vm = Interpreter::from_iir(&missing_graph, Budget::default()).unwrap();
        assert_eq!(
            missing_vm.solve_type_predicate(&known),
            ContractOutcome::Unknown
        );
    }

    #[test]
    fn an_explicit_solver_keeps_authority_over_retained_evidence() {
        let graph = graph();
        let mut vm = Interpreter::from_iir(&graph, Budget::default())
            .unwrap()
            .with_type_solver(&|_| ContractOutcome::Rejected);
        assert_eq!(
            vm.solve_type_predicate(&P::Trait(requirement(true))),
            ContractOutcome::Rejected
        );
    }
}
