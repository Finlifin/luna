//! Positive type evidence retained by concrete materialization.
use super::*;
use nir::program::TypePredicate as P;

impl Interpreter<'_> {
    pub(super) fn retained_type_contract(
        &mut self,
        predicate: &P,
        depth: usize,
    ) -> ContractQueryResult {
        use ContractOutcome::*;
        // A source declaration catalog is not a proof of applicability. Only
        // concrete dispatch selected by materialization supplies this evidence.
        if self.graph.is_none() {
            return Ok(Unknown);
        }
        if depth >= 128 {
            return Err(ErrorKind::DepthExhausted);
        }
        self.tick(None).map_err(|error| error.kind)?;
        Ok(match predicate {
            P::Bool(value) => {
                if *value {
                    Satisfied
                } else {
                    Rejected
                }
            }
            P::Equal(left, right) => {
                self.charge_copy_work(None, |fuel| {
                    for ty in [left, right] {
                        nir::copy_budget::charge_type_substitution(ty, &[], &[], fuel)?;
                    }
                    Ok(())
                })
                .map_err(|error| error.kind)?;
                match predicate.trivial_truth() {
                    Some(true) => Satisfied,
                    Some(false) => Rejected,
                    None => Unknown,
                }
            }
            P::Trait(requirement) => {
                self.charge_copy_work(None, |fuel| {
                    for ty in std::iter::once(&requirement.subject)
                        .chain(&requirement.interface_arguments)
                    {
                        nir::copy_budget::charge_type_substitution(ty, &[], &[], fuel)?;
                    }
                    Ok(())
                })
                .map_err(|error| error.kind)?;
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
            P::Not(inner) => match self.retained_type_contract(inner, depth + 1)? {
                Satisfied => Rejected,
                Rejected => Satisfied,
                Unknown => Unknown,
            },
            P::And(parts) | P::Or(parts) => {
                let conjunction = matches!(predicate, P::And(_));
                let mut unknown = false;
                for part in parts {
                    match self.retained_type_contract(part, depth + 1)? {
                        Rejected if conjunction => return Ok(Rejected),
                        Satisfied if !conjunction => return Ok(Satisfied),
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
        })
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
                Ok(ContractOutcome::Unknown)
            );
            assert_eq!(
                vm.solve_type_predicate(&P::Not(Box::new(predicate))),
                Ok(ContractOutcome::Unknown)
            );
        }
        assert_eq!(
            vm.solve_type_predicate(&known),
            Ok(ContractOutcome::Satisfied)
        );
        assert_eq!(
            vm.solve_type_predicate(&P::Not(Box::new(known.clone()))),
            Ok(ContractOutcome::Rejected)
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
            assert_eq!(vm.solve_type_predicate(&predicate), Ok(expected));
        }
        vm.budget.steps = 0;
        assert_eq!(
            vm.solve_type_predicate(&known),
            Err(ErrorKind::StepsExhausted)
        );
        vm.budget.steps = 1000;
        assert_eq!(
            vm.solve_type_predicate(&known),
            Ok(ContractOutcome::Satisfied)
        );
        let missing_graph = iir::Package::default();
        let mut missing_vm = Interpreter::from_iir(&missing_graph, Budget::default()).unwrap();
        assert_eq!(
            missing_vm.solve_type_predicate(&known),
            Ok(ContractOutcome::Unknown)
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
            Ok(ContractOutcome::Rejected)
        );
    }
}
