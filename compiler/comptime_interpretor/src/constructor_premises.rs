//! Expand caller-established trait premises under the constructor's allowance.
//! These premises discharge obligations; they do not provide executable methods.
use crate::{Error, ErrorKind};
use nir::program::{InterfaceInstance, Trait, TraitRequirement};
use nir::{DeclarationId, Term};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};

pub(super) fn failure(reason: nir::type_computations::Error) -> Error {
    Error {
        kind: match reason {
            nir::type_computations::Error::Budget => ErrorKind::StepsExhausted,
            _ => ErrorKind::InvalidCallPlan,
        },
        source: None,
    }
}

fn reserve(count: usize, fuel: &mut usize) -> Result<(), Error> {
    *fuel = fuel
        .checked_sub(count)
        .ok_or_else(|| failure(nir::type_computations::Error::Budget))?;
    Ok(())
}

pub(super) fn expand(
    assumptions: &[TraitRequirement],
    traits: &BTreeMap<DeclarationId, &Trait>,
    fuel: &mut usize,
) -> Result<HashSet<(InterfaceInstance, Term)>, Error> {
    use nir::type_computations::charge_term;
    // Borrow initial premises. Reserve each worklist batch before allocating;
    // nested payloads are bounded before cloning or hashing their keys.
    reserve(assumptions.len(), fuel)?;
    let mut pending: Vec<_> = assumptions
        .iter()
        .map(|premise| (Cow::Borrowed(premise), 0usize))
        .collect();
    let mut established = HashSet::new();
    while let Some((premise, depth)) = pending.pop() {
        if depth >= 64 {
            return Err(Error {
                kind: ErrorKind::DepthExhausted,
                source: None,
            });
        }
        let contract = traits.get(&premise.interface).ok_or(Error {
            kind: ErrorKind::InvalidCallPlan,
            source: None,
        })?;
        if contract.type_parameters.len() != premise.interface_arguments.len() {
            return Err(Error {
                kind: ErrorKind::InvalidCallPlan,
                source: None,
            });
        }
        charge_term(&premise.subject, fuel).map_err(failure)?;
        for argument in &premise.interface_arguments {
            charge_term(argument, fuel).map_err(failure)?;
        }
        if !established.insert((premise.interface_instance(), premise.subject.clone())) {
            continue;
        }
        reserve(contract.capabilities.len(), fuel)?;
        reserve(contract.parents.len(), fuel)?;
        // This second copy becomes the substitution input and remains distinct
        // from the owned key retained in the established-premise set.
        charge_term(&premise.subject, fuel).map_err(failure)?;
        for argument in &premise.interface_arguments {
            charge_term(argument, fuel).map_err(failure)?;
        }
        let arguments: Vec<_> = std::iter::once(premise.subject.clone())
            .chain(premise.interface_arguments.iter().cloned())
            .collect();
        let substitution = contract
            .input_substitution(&arguments, &[], fuel)
            .map_err(failure)?;
        for capability in &contract.capabilities {
            pending.push((
                Cow::Owned(
                    substitution
                        .instantiate_requirement_bounded(capability, fuel)
                        .map_err(failure)?,
                ),
                depth + 1,
            ));
        }
        for parent in &contract.parents {
            let parent = substitution
                .instantiate_interface_bounded(parent, fuel)
                .map_err(failure)?;
            charge_term(&premise.subject, fuel).map_err(failure)?;
            pending.push((
                Cow::Owned(TraitRequirement {
                    interface: parent.declaration,
                    subject: premise.subject.clone(),
                    interface_arguments: parent.arguments,
                }),
                depth + 1,
            ));
        }
    }
    Ok(established)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Budget, Interpreter};
    use nir::program::Package;
    use nir::{Primitive, Sort};

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 1, local }
    }

    fn rigid(local: u32) -> Term {
        Term::Rigid {
            declaration: id(local),
            sort: Sort::Type,
        }
    }

    fn contract(local: u32) -> Trait {
        Trait {
            comptime_only: false,
            parameter_types: vec![],
            associated: vec![],
            requirements: vec![],
            capabilities: vec![],
            type_parameters: vec![id(local + 2)],
            declaration: id(local),
            parents: vec![],
            self_parameter: id(local + 1),
            name: format!("Protocol{local}"),
            methods: vec![],
        }
    }

    fn world() -> (Package, TraitRequirement) {
        let base = contract(10);
        let mut derived = contract(20);
        derived.parents.push(InterfaceInstance {
            declaration: base.declaration,
            arguments: vec![rigid(22)],
        });
        derived.capabilities.push(TraitRequirement {
            interface: base.declaration,
            interface_arguments: vec![rigid(21)],
            subject: rigid(22),
        });
        (
            Package {
                traits: vec![base, derived],
                ..Default::default()
            },
            TraitRequirement {
                interface: id(20),
                interface_arguments: vec![Term::Primitive(Primitive::U8)],
                subject: Term::Primitive(Primitive::Bool),
            },
        )
    }

    #[test]
    fn inherited_premises_share_constructor_fuel_and_retry_without_partial_state() {
        let (package, premise) = world();
        let before = package.to_string();
        let inputs = [premise.clone(), premise.clone()];
        let vm = Interpreter::for_comptime_with_assumptions(
            &package,
            &inputs,
            Budget {
                steps: 10_000,
                depth: 128,
            },
        )
        .unwrap();
        let cost = 10_000 - vm.remaining_steps();
        assert_eq!(vm.assumed_interfaces.len(), 3);
        assert!(vm.assumed_interfaces.contains(&(
            InterfaceInstance {
                declaration: id(10),
                arguments: vec![Term::Primitive(Primitive::U8)],
            },
            Term::Primitive(Primitive::Bool)
        )));
        assert!(vm.assumed_interfaces.contains(&(
            InterfaceInstance {
                declaration: id(10),
                arguments: vec![Term::Primitive(Primitive::Bool)],
            },
            Term::Primitive(Primitive::U8)
        )));
        assert!(
            vm.method_bodies.is_empty(),
            "premises cannot supply method bodies"
        );
        for steps in 0..cost {
            let error = Interpreter::for_comptime_with_assumptions(
                &package,
                &inputs,
                Budget { steps, depth: 128 },
            )
            .err()
            .expect("insufficient constructor fuel");
            assert_eq!(error.kind, ErrorKind::StepsExhausted, "steps {steps}");
        }
        let retried = Interpreter::for_comptime_with_assumptions(
            &package,
            &inputs,
            Budget {
                steps: cost,
                depth: 128,
            },
        )
        .unwrap();
        assert_eq!(retried.remaining_steps(), 0);
        assert_eq!(retried.assumed_interfaces, vm.assumed_interfaces);
        assert!(retried.events().is_empty());
        assert_eq!(package.to_string(), before);
    }

    #[test]
    fn premise_payloads_and_expanded_replacements_are_bounded() {
        let (mut package, mut premise) = world();
        // One use of the input in a parent becomes many uses. The expanded
        // payload, not just the small source skeleton, must consume fuel.
        package.traits[1].parents[0].arguments = vec![Term::Tuple(vec![rigid(22); 32])];
        premise.interface_arguments = vec![Term::Tuple(vec![Term::Primitive(Primitive::Bool); 32])];
        let traits = package.traits.iter().map(|t| (t.declaration, t)).collect();
        let mut short = 256;
        assert_eq!(
            expand(std::slice::from_ref(&premise), &traits, &mut short)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert!(short > 0, "failed width reservation can retain fuel");
        let mut generous = 100_000;
        let established = expand(std::slice::from_ref(&premise), &traits, &mut generous).unwrap();
        assert!(established.contains(&(
            InterfaceInstance {
                declaration: id(10),
                arguments: vec![Term::Tuple(vec![
                    premise.interface_arguments[0].clone();
                    32
                ])],
            },
            premise.subject.clone()
        )));
        let mut too_short_for_input = 8;
        assert_eq!(
            expand(&[premise], &traits, &mut too_short_for_input)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
    }

    #[test]
    fn malformed_premises_still_fail_and_duplicate_cycles_terminate() {
        let (mut package, premise) = world();
        package.traits[1].parents.push(InterfaceInstance {
            declaration: id(20),
            arguments: vec![rigid(22)],
        });
        let traits = package.traits.iter().map(|t| (t.declaration, t)).collect();
        assert_eq!(
            expand(std::slice::from_ref(&premise), &traits, &mut 10_000)
                .unwrap()
                .len(),
            3
        );
        for invalid in [
            TraitRequirement {
                interface: id(999),
                ..premise.clone()
            },
            TraitRequirement {
                interface_arguments: vec![],
                ..premise.clone()
            },
            TraitRequirement {
                interface_arguments: vec![Term::Bool(true)],
                ..premise.clone()
            },
        ] {
            assert_eq!(
                expand(&[invalid], &traits, &mut 10_000).unwrap_err().kind,
                ErrorKind::InvalidCallPlan
            );
        }
        assert_eq!(expand(&[premise], &traits, &mut 10_000).unwrap().len(), 3);
    }

    #[test]
    fn leaf_premises_validate_inputs_even_without_outgoing_edges() {
        let mut leaf = contract(10);
        let premise = TraitRequirement {
            interface: id(10),
            interface_arguments: vec![Term::Bool(true)],
            subject: Term::Primitive(Primitive::Bool),
        };
        let traits = BTreeMap::from([(leaf.declaration, &leaf)]);
        assert_eq!(
            expand(std::slice::from_ref(&premise), &traits, &mut 10_000)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidCallPlan
        );

        // A bool domain receives an ordinary comptime value. This boundary
        // checks substitution scopes/sorts, not scalar admission or obligations.
        leaf.parameter_types = vec![Term::Primitive(Primitive::Bool)];
        let traits = BTreeMap::from([(leaf.declaration, &leaf)]);
        assert!(
            expand(std::slice::from_ref(&premise), &traits, &mut 10_000)
                .unwrap()
                .contains(&(premise.interface_instance(), premise.subject.clone()))
        );
        let invalid_subject = TraitRequirement {
            subject: Term::Bool(false),
            ..premise
        };
        assert_eq!(
            expand(&[invalid_subject], &traits, &mut 10_000)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidCallPlan
        );
    }
}
