//! Merge checked complementary method contributions without rebinding their bodies.
use super::*;
use std::collections::HashSet;

type CapabilityKey = (DeclarationId, Term, Vec<Term>);

fn spend(fuel: &mut u64, work: usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(work as u64)
        .ok_or("implementation phase merge budget exhausted")?;
    Ok(())
}

fn capabilities(
    implementation: &n::Implementation,
    fuel: &mut u64,
) -> Result<HashSet<CapabilityKey>, String> {
    spend(fuel, implementation.type_parameters.len())?;
    let parameters = implementation
        .parameter_binders()
        .map_err(|e| e.to_string())?;
    let mut bind = |term: &Term| {
        spend(fuel, parameters.len())?;
        let mut remaining = usize::try_from(*fuel).unwrap_or(usize::MAX);
        let before = remaining;
        let checked = nir::type_computations::charge_term(term, &mut remaining);
        *fuel = fuel.saturating_sub((before - remaining) as u64);
        checked.map_err(|e| e.to_string())?;
        term.check_scope(&[]).map_err(|e| e.to_string())?;
        term.abstract_parameters(&parameters)
            .map_err(|e| e.to_string())
    };
    implementation
        .capabilities
        .iter()
        .map(|requirement| {
            Ok((
                requirement.interface,
                bind(&requirement.subject)?,
                requirement
                    .interface_arguments
                    .iter()
                    .map(&mut bind)
                    .collect::<Result<Vec<_>, String>>()?,
            ))
        })
        .collect()
}

pub(super) fn merge(
    contributions: Vec<n::Implementation>,
    relationships: &BTreeMap<DeclarationId, DeclarationId>,
    traits: &BTreeMap<DeclarationId, n::Trait>,
    fuel: &mut u64,
) -> Result<Vec<n::Implementation>, (DeclarationId, String)> {
    let mut output: Vec<n::Implementation> = Vec::new();
    let mut slots = BTreeMap::new();
    for contribution in contributions {
        let own = contribution.declaration;
        let result = (|| {
            spend(fuel, 1)?;
            let relationship = relationships.get(&own).copied().unwrap_or(own);
            let Some(slot) = slots.get(&relationship).copied() else {
                if relationship != own {
                    return Err("missing canonical implementation contribution".into());
                }
                slots.insert(relationship, output.len());
                output.push(contribution);
                return Ok(());
            };
            let target: &mut n::Implementation = &mut output[slot];
            if target.interface != contribution.interface
                || (target.phases.runtime && contribution.phases.runtime)
                || (target.phases.comptime && contribution.phases.comptime)
            {
                return Err("implementation phase contributions overlap".into());
            }
            if capabilities(target, fuel)? != capabilities(&contribution, fuel)? {
                return Err("complementary implementation capability conditions differ".into());
            }
            let interface = target
                .interface
                .and_then(|id| traits.get(&id))
                .ok_or("phase contribution has no trait contract")?;
            spend(
                fuel,
                interface
                    .methods
                    .len()
                    .saturating_add(contribution.methods.len()),
            )?;
            let order: BTreeMap<_, _> = interface
                .methods
                .iter()
                .enumerate()
                .map(|(slot, member)| (member.name.as_str(), slot))
                .collect();
            let mut names: BTreeSet<_> = target
                .methods
                .iter()
                .map(|(name, _)| name.clone())
                .collect();
            for (name, _) in &contribution.methods {
                spend(fuel, name.len().saturating_add(1))?;
                if !names.insert(name.clone()) || !order.contains_key(name.as_str()) {
                    return Err("duplicate or unknown method across implementation phases".into());
                }
            }
            target.phases.runtime |= contribution.phases.runtime;
            target.phases.comptime |= contribution.phases.comptime;
            target.methods.extend(contribution.methods);
            target
                .methods
                .sort_by_key(|(name, _)| order.get(name.as_str()).copied());
            Ok(())
        })();
        result.map_err(|message| (own, message))?;
    }
    Ok(output)
}
