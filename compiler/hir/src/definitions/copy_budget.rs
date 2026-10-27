//! Meter the owned data cloned for an isolated publication candidate.
use super::{Definition, DefinitionBatch, Error};
use crate::{DeferredDefinitionAttribute, Package};

fn charge(fuel: &mut usize, count: usize) -> Result<(), Error> {
    *fuel = fuel.checked_sub(count).ok_or(Error {
        kind: "budget exhausted",
        owner: None,
    })?;
    Ok(())
}

fn attributes(values: &[DeferredDefinitionAttribute<'_>], fuel: &mut usize) -> Result<(), Error> {
    charge(fuel, values.len())?;
    for value in values {
        let DeferredDefinitionAttribute {
            target: _,
            lexical_owner: _,
            expression,
            value: _,
            configuration_key,
            span: _,
        } = value;
        charge(fuel, expression.len())?;
        if let Some(key) = configuration_key {
            charge(fuel, key.len())?;
        }
    }
    Ok(())
}

impl DefinitionBatch<'_, '_> {
    pub(super) fn charge_candidate_copy(&self, fuel: &mut usize) -> Result<(), Error> {
        charge_package(self.package, fuel)?;
        // Bodies, arena nodes and Arc candidate slices only copy references.
        charge(fuel, self.definitions.len())?;
        for definition in self.definitions.iter().flatten() {
            let Definition {
                info,
                identity: _,
                lexical_parent: _,
            } = definition;
            charge(fuel, info.nodes.len())?;
        }
        charge(fuel, self.bodies.len())?;
        charge(fuel, self.body_slots.capacity())?;
        charge(fuel, self.resolution.len())?;
        charge(fuel, self.staged_parameters.len())?;
        attributes(&self.attributes, fuel)?;
        Ok(())
    }
}

pub(super) fn charge_package(package: &Package<'_>, fuel: &mut usize) -> Result<(), Error> {
    // Exhaustive destructuring makes new package tables require an explicit
    // copying decision here. HashMap clone retains allocated bucket storage;
    // Vec clone copies its logical elements, including sparse index slots.
    let Package {
        owners,
        bodies,
        root_mod: _,
        pending_semantics,
        definition_attributes,
        member_projections,
        deferred_member_projections,
        member_access,
        definitions,
        owner_definitions,
        clause_definitions,
        next_definition_index: _,
        lexical_parents,
        staged_parameter_definitions,
    } = package;
    charge(fuel, 1)?;
    charge(fuel, owners.len())?;
    for (_, owner) in owners.iter_enumerated() {
        if let Some(owner) = owner {
            charge(fuel, owner.nodes.len())?;
        }
    }
    charge(fuel, bodies.capacity())?;
    charge(fuel, pending_semantics.len())?;
    for (_, text) in pending_semantics {
        charge(fuel, text.len())?;
    }
    attributes(definition_attributes, fuel)?;
    for count in [
        member_projections.capacity(),
        deferred_member_projections.capacity(),
        member_access.capacity(),
        definitions.capacity(),
        owner_definitions.capacity(),
        clause_definitions.capacity(),
        lexical_parents.capacity(),
        staged_parameter_definitions.capacity(),
    ] {
        charge(fuel, count)?;
    }
    Ok(())
}
