//! Remove compile-time generation sources from an isolated expansion candidate.
use super::*;
use crate::{AttributeTarget, HirArena, OwnerNode};

/// Fork a package and omit source definitions after their generation work has
/// been captured. This is an unchecked transformation, not semantic publication.
/// References from ordinary expressions still fail ordinary name/type checking.
pub fn omit_generation_sources<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    omitted: &BTreeSet<OwnerId>,
    fuel: &mut usize,
) -> Result<Package<'h>, Error> {
    if omitted.contains(&source.root_mod) {
        return Err(error("cannot omit package root", source.root_mod));
    }
    let mut package = fork_package(source, fuel)?;
    for owner in omitted {
        charge(fuel)?;
        if let Some(info) = package.owners.get_mut(owner.def_id).and_then(Option::take) {
            for parameter in members::clauses(info.node.expect_item()) {
                charge(fuel)?;
                if let Some(id) = parameter.definition {
                    let count = package.clause_definitions.get_mut(&id).unwrap();
                    *count -= 1;
                    if *count == 0 {
                        package.clause_definitions.remove(&id);
                    }
                }
            }
        }
    }
    for (_, info) in package.owners.iter_enumerated_mut() {
        charge(fuel)?;
        let Some(info) = info else { continue };
        let original = info.node.expect_item();
        let count = match &original.kind {
            ItemKind::Mod(def) => def.items.len(),
            ItemKind::Struct(def) => def.nested_items.len(),
            ItemKind::Enum(def) => def.nested_items.len(),
            ItemKind::Trait(def) => def.items.len(),
            ItemKind::Impl(def) => def.items.len(),
            _ => continue,
        };
        for _ in 0..count {
            charge(fuel)?;
        }
        let mut item = original.clone();
        let children = match &mut item.kind {
            ItemKind::Mod(def) => &mut def.items,
            ItemKind::Struct(def) => &mut def.nested_items,
            ItemKind::Enum(def) => &mut def.nested_items,
            ItemKind::Trait(def) => &mut def.items,
            ItemKind::Impl(def) => &mut def.items,
            _ => continue,
        };
        children.retain(|owner| !omitted.contains(owner));
        let item = arena.alloc_item(item);
        info.node = OwnerNode::Item(item);
        // The root index must point at the replacement descriptor.
        if info.nodes.get(crate::ItemLocalId::new(0)).is_some() {
            info.nodes.insert(
                crate::ItemLocalId::new(0),
                crate::ItemLocalId::new(0),
                crate::Node::Item(item),
            );
        }
    }
    package
        .bodies
        .retain(|id, _| !omitted.contains(&id.hir_id.owner));
    package
        .definitions
        .retain(|_, owner| !omitted.contains(owner));
    package
        .owner_definitions
        .retain(|owner, _| !omitted.contains(owner));
    package
        .lexical_parents
        .retain(|owner, parent| !omitted.contains(owner) && !omitted.contains(parent));
    package
        .member_projections
        .retain(|id, _| !omitted.contains(&id.owner));
    package
        .deferred_member_projections
        .retain(|id, _| !omitted.contains(&id.owner));
    package
        .trait_sources
        .retain(|id, _| !omitted.contains(&id.owner));
    package
        .member_access
        .retain(|id, _| !omitted.contains(&id.owner));
    package
        .staged_parameter_definitions
        .retain(|id, _| !omitted.contains(&id.owner));
    package.definition_attributes.retain(|attribute| {
        let owner = match attribute.target {
            AttributeTarget::Definition(owner) => owner,
            AttributeTarget::Member(id) => id.owner,
        };
        !omitted.contains(&owner)
    });
    Ok(package)
}
