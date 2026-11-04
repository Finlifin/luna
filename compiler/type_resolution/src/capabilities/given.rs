//! Premise lookup is a partial query: unfinished work is never absence.
use super::*;
use nir::NominalAdmissionError as Error;
use nir::type_computations::charge_term;
use std::borrow::Cow;

pub(super) fn lookup(
    world: &World<'_>,
    requirement: &TraitRequirement,
    budget: Budget,
    scope: &[nir::Parameter],
    frames: &[Vec<nir::Sort>],
    fuel: &mut usize,
) -> Result<Option<TraitRequirement>, Error> {
    let reserved = budget.steps.min(*fuel);
    let mut remaining = reserved;
    let result = search(world, requirement, budget, scope, frames, &mut remaining);
    *fuel -= reserved - remaining;
    result
}

fn charge(requirement: &TraitRequirement, fuel: &mut usize) -> Result<(), Error> {
    for input in std::iter::once(&requirement.subject).chain(&requirement.interface_arguments) {
        charge_term(input, fuel)?;
    }
    Ok(())
}

fn valid(
    world: &World<'_>,
    requirement: &TraitRequirement,
    budget: Budget,
    frames: &[Vec<nir::Sort>],
    fuel: &mut usize,
) -> Result<(), Error> {
    charge(requirement, fuel)?;
    if requirement.subject.sort() != nir::Sort::Type {
        return Err("given subject is not a Type".into());
    }
    requirement
        .subject
        .check_scope(frames)
        .map_err(|e| e.to_string())?;
    if let Some(contract) = world
        .traits
        .iter()
        .find(|t| t.declaration == requirement.interface)
    {
        contract.validate_arguments_with_layouts(
            &requirement.subject,
            &requirement.interface_arguments,
            frames,
            world.traits,
            &world.index.parameter_domains,
            fuel,
            |left, right, fuel| {
                equivalent_domains(world.context, left, right, budget.max_depth, fuel)
            },
            Some(&world.index.nominal_layouts),
        )
    } else {
        for argument in &requirement.interface_arguments {
            if argument.sort() != nir::Sort::Type {
                return Err("given argument has no declared value domain".into());
            }
            argument.check_scope(frames).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn search(
    world: &World<'_>,
    requirement: &TraitRequirement,
    budget: Budget,
    scope: &[nir::Parameter],
    frames: &[Vec<nir::Sort>],
    fuel: &mut usize,
) -> Result<Option<TraitRequirement>, Error> {
    valid(world, requirement, budget, frames, fuel)?;
    let mut incomplete = None;
    for assumption in world.assumptions {
        // Borrow roots; expanded premises are owned. No context or full premise
        // snapshot is copied merely to ask whether a given witness exists.
        let mut pending = vec![(Cow::Borrowed(assumption), 0usize)];
        let mut seen = HashSet::new();
        while let Some((current, depth)) = pending.pop() {
            if *fuel == 0 {
                return Err(Error::Budget);
            }
            let result = (|| {
                valid(world, &current, budget, frames, fuel)?;
                charge(&current, fuel)?;
                if !seen.insert((current.interface_instance(), current.subject.clone())) {
                    return Ok(false);
                }
                if depth >= budget.max_depth.min(128) {
                    return Err(Error::Budget);
                }
                if current.interface == requirement.interface
                    && current.interface_arguments.len() == requirement.interface_arguments.len()
                {
                    let mut matches = crate::input_identity::equivalent(
                        world.context,
                        &current.subject,
                        &requirement.subject,
                        budget.max_depth,
                        scope,
                        frames,
                        fuel,
                    )?;
                    for (left, right) in current
                        .interface_arguments
                        .iter()
                        .zip(&requirement.interface_arguments)
                    {
                        if !matches {
                            break;
                        }
                        matches = crate::input_identity::equivalent(
                            world.context,
                            left,
                            right,
                            budget.max_depth,
                            scope,
                            frames,
                            fuel,
                        )?;
                    }
                    if matches {
                        return Ok(true);
                    }
                }
                if let Some(contract) = world
                    .traits
                    .iter()
                    .find(|t| t.declaration == current.interface)
                {
                    charge(&current, fuel)?;
                    let arguments: Vec<_> = std::iter::once(current.subject.clone())
                        .chain(current.interface_arguments.iter().cloned())
                        .collect();
                    let substitution = contract.input_substitution(&arguments, frames, fuel)?;
                    for capability in &contract.capabilities {
                        let capability =
                            substitution.instantiate_requirement_bounded(capability, fuel)?;
                        pending.push((Cow::Owned(capability), depth + 1));
                    }
                    for parent in &contract.parents {
                        let parent = substitution.instantiate_interface_bounded(parent, fuel)?;
                        charge_term(&current.subject, fuel)?;
                        pending.push((
                            Cow::Owned(TraitRequirement {
                                subject: current.subject.clone(),
                                interface: parent.declaration,
                                interface_arguments: parent.arguments,
                            }),
                            depth + 1,
                        ));
                    }
                }
                Ok(false)
            })();
            match result {
                Ok(true) => {
                    charge(assumption, fuel)?;
                    return Ok(Some(assumption.clone()));
                }
                Ok(false) => {}
                Err(error) => incomplete = Some(error),
            }
        }
    }
    match incomplete {
        Some(error) => Err(error),
        None => Ok(None),
    }
}
