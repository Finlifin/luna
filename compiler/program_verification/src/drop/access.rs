//! Check the captured destructor's memory footprint against the current owner.
//! This does not acquire environments, prove program predicates, or run Drop.
use super::*;
use crate::PlaceId;
use crate::{
    memory,
    ownership::{Availability, CleanupTree, Place, State},
};
use std::collections::{BTreeMap, BTreeSet};

/// A payload layout is meaningful only under this cleanup branch's tag.
pub(super) struct VariantLayout {
    pub index: usize,
    pub fields: Vec<Term>,
}

/// Borrowed evidence for one cleanup instant, not a persistent certificate.
/// Every alternative in a stored pointer must satisfy the access obligation.
#[derive(Clone, Copy)]
pub struct PointerMemory<'a> {
    pub types: &'a BTreeMap<PlaceId, Term>,
    pub slots: &'a BTreeMap<Place, BTreeSet<Place>>,
    /// Current tag domains and per-target qualifiers, never entry-time facts.
    pub enum_domains: &'a BTreeMap<PlaceId, BTreeMap<Vec<usize>, BTreeSet<usize>>>,
    pub pointer_conditions: &'a BTreeMap<Place, BTreeMap<Place, Vec<(Vec<usize>, usize)>>>,
    /// Positions already destroyed earlier in this cleanup sequence.
    pub retired: &'a [Place],
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccessCheck {
    pub receiver: Place,
    pub constructed: CleanupTree,
    pub footprint: memory::Summary,
    /// Resolved places in the same order as footprint.accesses.
    pub targets: Vec<BTreeSet<Place>>,
}
impl fmt::Display for AccessCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(drop-access-check (receiver {}) (constructed {}) {} (targets",
            self.receiver, self.constructed, self.footprint
        )?;
        for targets in &self.targets {
            f.write_str(" (access")?;
            for target in targets {
                write!(f, " {target}")?;
            }
            f.write_str(")")?;
        }
        f.write_str("))")
    }
}

/// Check an already selected method; never reselect a destructor at cleanup.
/// The caller still owns the captured capability/environment and specification
/// obligations. This result only establishes the supported local access subset
/// at this state snapshot; mutations require a new check.
pub fn check_access(
    package: &Package,
    world: &World<'_>,
    selected: &SelectedDrop,
    state: &State,
    receiver: &Place,
    fuel: &mut usize,
) -> Result<AccessCheck, String> {
    check_access_with_memory(package, world, selected, state, receiver, None, fuel)
}

/// Resolve indirect accesses using a current pointer snapshot. This checks only
/// this invocation: scheduling must refresh availability after earlier cleanup.
pub fn check_access_with_memory(
    package: &Package,
    world: &World<'_>,
    selected: &SelectedDrop,
    state: &State,
    receiver: &Place,
    pointers: Option<PointerMemory<'_>>,
    fuel: &mut usize,
) -> Result<AccessCheck, String> {
    check_access_with_layout(
        package,
        world,
        selected,
        state,
        receiver,
        pointers,
        &BTreeMap::new(),
        fuel,
    )
}

pub(super) fn check_access_with_layout(
    package: &Package,
    world: &World<'_>,
    selected: &SelectedDrop,
    state: &State,
    receiver: &Place,
    pointers: Option<PointerMemory<'_>>,
    variant_layouts: &BTreeMap<Place, VariantLayout>,
    fuel: &mut usize,
) -> Result<AccessCheck, String> {
    if *fuel == 0 {
        return Err("Drop access budget exhausted".into());
    }
    *fuel -= 1;
    if !world.index.type_resolution_is_current(&selected.selection) {
        return Err("Drop selection belongs to a different implementation world".into());
    }
    let implementation = package
        .implementations
        .iter()
        .find(|item| item.declaration == selected.implementation)
        .ok_or("captured Drop implementation is missing")?;
    let subject = implementation
        .subject
        .instantiate_parameters(
            &implementation
                .parameter_binders()
                .map_err(|e| e.to_string())?,
            &selected.implementation_arguments,
        )
        .map_err(|error| error.to_string())?;
    let contract = package
        .traits
        .iter()
        .find(|item| item.declaration == selected.protocol)
        .ok_or("captured Drop protocol is missing")?;
    let member = contract
        .methods
        .iter()
        .find(|item| item.declaration == selected.method.member && item.name == "drop")
        .ok_or("captured Drop member is missing")?;
    let expected_arguments = if member.default == Some(selected.method.body) {
        vec![selected.subject.clone()]
    } else {
        selected.implementation_arguments.clone()
    };
    if subject != selected.subject
        || implementation.interface != Some(selected.protocol)
        || implementation
            .methods
            .iter()
            .filter(|(name, _)| name == "drop")
            .map(|(_, body)| *body)
            .collect::<Vec<_>>()
            != vec![selected.method.body]
        || selected.method.arguments != expected_arguments
        || instantiate_contract(member, contract.self_parameter, &selected.subject)?
            != selected.contract
    {
        return Err("captured Drop method disagrees with the current package".into());
    }
    let function = package
        .functions
        .iter()
        .find(|function| function.declaration == selected.method.body)
        .ok_or("captured Drop body is missing")?;
    let binders = memory::memory_binders(Some(package), &function.type_parameters, fuel)?;
    let program = ProgramContract {
        preconditions: function.preconditions.clone(),
        postconditions: function.postconditions.clone(),
        result_binding: function.result_binding,
        entry_snapshots: function.entry_snapshots.clone(),
    }
    .instantiate_parameters(&binders, &selected.method.arguments)
    .map_err(|error| error.to_string())?;
    if function.parameters != selected.parameters || program != selected.program {
        return Err("captured Drop body contract changed".into());
    }
    let constructed = state
        .cleanup_tree(receiver, fuel)
        .map_err(|error| format!("Drop receiver state: {error:?}"))?
        .ok_or("Drop receiver has no remaining construction responsibility")?;
    if constructed.availability != Availability::Present {
        return Err(
            "Drop receiver construction is absent or conditional; a construction guard is required"
                .into(),
        );
    }
    let footprint = memory::infer_in_package(
        package,
        selected.method.body,
        &selected.method.arguments,
        fuel,
    )?;
    if footprint.signature != signature(selected.subject.clone()) {
        return Err("captured Drop body signature changed".into());
    }
    if let Some(pointers) = pointers.filter(|_| {
        footprint
            .accesses
            .iter()
            .any(|a| !a.target.through.is_empty() || !a.target.conditions.is_empty())
    }) {
        if place_type(package, pointers.types, receiver, variant_layouts, fuel)? != selected.subject
        {
            return Err("Drop receiver type disagrees with memory snapshot".into());
        }
    }
    let mut resolved = Vec::new();
    for access in &footprint.accesses {
        if access
            .target
            .conditions
            .keys()
            .any(|stage| *stage > access.target.through.len())
        {
            return Err("Drop guard stage out of bounds".into());
        }
        if !access.target.input_fields.is_empty() {
            return Err(
                "Drop receiver footprint cannot originate in a by-value input field".into(),
            );
        }
        if access.target.parameter != 0 {
            return Err("Drop access targets a non-receiver parameter".into());
        }
        let mut targets = BTreeSet::from([receiver.clone()]);
        let mut ty = selected.subject.clone();
        for (stage, fields) in access.target.through.iter().enumerate() {
            let guards = access
                .target
                .conditions
                .get(&stage)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let projected = project_conditional(
                package,
                &mut ty,
                &mut targets,
                fields,
                guards,
                pointers,
                variant_layouts,
                state,
                fuel,
            )?;
            let pointers =
                pointers.ok_or("indirect Drop access requires stored pointer provenance")?;
            let Term::Pointer(subject) = ty else {
                return Err("Drop indirect path loads a non-pointer slot".into());
            };
            let mut loaded = BTreeSet::new();
            for slot in &targets {
                check_retired(pointers.retired, slot, fuel)?;
                if guards.is_empty() {
                    state
                        .require_present(slot)
                        .map_err(|e| format!("Drop pointer slot {slot} is unavailable: {e:?}"))?;
                }
                let values = pointers
                    .slots
                    .get(slot)
                    .filter(|values| !values.is_empty())
                    .ok_or("indirect Drop access requires stored pointer provenance")?;
                for target in values {
                    charge(fuel, target.fields.len().saturating_add(1))?;
                    if let Some(required) = projected.get(slot) {
                        if let Some(conditions) = pointers
                            .pointer_conditions
                            .get(slot)
                            .and_then(|entries| entries.get(target))
                        {
                            charge(
                                fuel,
                                conditions
                                    .iter()
                                    .map(|(path, _)| path.len() + 1)
                                    .sum::<usize>()
                                    .saturating_mul(required.len().saturating_add(1)),
                            )?;
                            if conditions.iter().any(|(path, tag)| {
                                required
                                    .iter()
                                    .any(|(selected, actual)| path == selected && tag != actual)
                            }) {
                                continue;
                            }
                        }
                    }
                    // Loading an address need not read its pointee, but its
                    // storage must still belong to the current state.
                    state.require_place(target).map_err(|e| {
                        format!("Drop pointer target {target} is unavailable: {e:?}")
                    })?;
                    if place_type(package, pointers.types, target, variant_layouts, fuel)?
                        != *subject
                    {
                        return Err("Drop pointer target type disagrees with slot pointee".into());
                    }
                    loaded.insert(target.clone());
                }
            }
            targets = loaded;
            ty = *subject;
        }
        let guards = access
            .target
            .conditions
            .get(&access.target.through.len())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        project_conditional(
            package,
            &mut ty,
            &mut targets,
            &access.target.fields,
            guards,
            pointers,
            variant_layouts,
            state,
            fuel,
        )?;
        if ty != access.ty {
            return Err("Drop access type disagrees with receiver layout".into());
        }
        for target in &targets {
            charge(fuel, target.fields.len().saturating_add(1))?;
            let checked = if !guards.is_empty() {
                // The typed projection checked its whole containing enum.
                Ok(())
            } else if access.read_value {
                if let Some(pointers) = pointers {
                    check_retired(pointers.retired, target, fuel)?;
                }
                state.require_present(target)
            } else {
                state.require_place(target)
            };
            checked.map_err(|error| {
                format!(
                    "Drop {} target {target} is unavailable: {error:?}",
                    if access.write {
                        "write"
                    } else if access.read_value {
                        "read"
                    } else {
                        "address"
                    }
                )
            })?;
        }
        resolved.push(targets);
    }
    Ok(AccessCheck {
        receiver: receiver.clone(),
        constructed,
        footprint,
        targets: resolved,
    })
}

fn enclosing_variant_conditions(
    place: &Place,
    layouts: &BTreeMap<Place, VariantLayout>,
    fuel: &mut usize,
) -> Result<Vec<(Vec<usize>, usize)>, String> {
    let mut conditions = Vec::new();
    for (container, layout) in layouts {
        charge(fuel, container.fields.len() + place.fields.len() + 1)?;
        if container.root == place.root && place.fields.starts_with(&container.fields) {
            conditions.push((container.fields.clone(), layout.index));
        }
    }
    Ok(conditions)
}

/// Project one load stage. Conditions refer to this root, and an opaque enum's
/// complete owner supplies availability for its conditionally existing payload.
fn project_conditional(
    package: &Package,
    ty: &mut Term,
    targets: &mut BTreeSet<Place>,
    fields: &[(nir::program::FieldOwner, usize)],
    guards: &[(Vec<usize>, usize)],
    pointers: Option<PointerMemory<'_>>,
    variant_layouts: &BTreeMap<Place, VariantLayout>,
    state: &State,
    fuel: &mut usize,
) -> Result<BTreeMap<Place, Vec<(Vec<usize>, usize)>>, String> {
    if guards.is_empty() {
        if variant_layouts.is_empty() {
            project(package, ty, targets, fields, fuel)?;
            return Ok(BTreeMap::new());
        }
        let mut selected = BTreeMap::new();
        for base in targets.iter() {
            let conditions = enclosing_variant_conditions(base, variant_layouts, fuel)?;
            if !conditions.is_empty() {
                charge(fuel, base.fields.len() + fields.len() + 1)?;
                let mut slot = base.clone();
                slot.fields.extend(fields.iter().map(|(_, index)| *index));
                selected.insert(slot, conditions);
            }
        }
        project(package, ty, targets, fields, fuel)?;
        return Ok(selected);
    }
    let pointers = pointers.ok_or("conditional Drop access requires a memory snapshot")?;
    let mut traversal = memory::StorageTraversal::new(Some(package), ty, &[], fuel)?;
    traversal.then_guarded(Some(package), fields, guards, fuel)?;
    let mut selected = BTreeMap::new();
    for base in targets.iter() {
        charge(fuel, base.fields.len() + fields.len() + 1)?;
        let domains = pointers.enum_domains.get(&base.root);
        let inherited = enclosing_variant_conditions(base, variant_layouts, fuel)?;
        let mut qualified = Vec::new();
        for (depth, field) in base.fields.iter().enumerate() {
            charge(fuel, inherited.len() + 1)?;
            for (_, tag) in inherited.iter().filter(|(path, _)| path.len() == depth) {
                qualified.extend([usize::MAX, *tag]);
            }
            qualified.push(*field);
        }
        charge(fuel, qualified.len().saturating_mul(guards.len()))?;
        let activity = memory::storage_guard_activity(guards, fuel, |path, tag| {
            let mut absolute = qualified.clone();
            absolute.extend(path);
            match domains.and_then(|domains| domains.get(&absolute)) {
                Some(tags) if !tags.contains(&tag) => memory::GuardActivity::Inactive,
                Some(tags) if tags.len() == 1 => memory::GuardActivity::Active,
                _ => memory::GuardActivity::Conditional,
            }
        })?;
        if activity == memory::GuardActivity::Inactive {
            continue;
        }
        let mut owner = base.clone();
        owner.fields.extend(&guards[0].0);
        check_retired(pointers.retired, &owner, fuel)?;
        state
            .require_present(&owner)
            .map_err(|e| format!("Drop conditional owner {owner} is unavailable: {e:?}"))?;
        let mut absolute = inherited;
        for (path, tag) in guards {
            charge(fuel, base.fields.len() + path.len() + 1)?;
            let mut container = base.fields.clone();
            container.extend(path);
            absolute.push((container, *tag));
        }
        let mut slot = base.clone();
        slot.fields.extend(fields.iter().map(|(_, index)| *index));
        selected.insert(slot, absolute);
    }
    *ty = traversal.target.ty;
    *targets = selected.keys().cloned().collect();
    Ok(selected)
}

fn charge(fuel: &mut usize, amount: usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(amount)
        .ok_or("Drop access budget exhausted")?;
    Ok(())
}

fn field_type(
    package: &Package,
    ty: &Term,
    index: usize,
    fuel: &mut usize,
) -> Result<Term, String> {
    charge(fuel, 1)?;
    memory::bound_type(ty, fuel)?;
    if let Some(fields) = nir::normal_forms::tuple_fields(Some(package), ty) {
        return fields
            .get(index)
            .cloned()
            .ok_or("Drop tuple field index is out of bounds".into());
    }
    match ty {
        Term::Closure { environment, .. } => environment
            .get(index)
            .cloned()
            .ok_or("Drop capture field index is out of bounds".into()),
        Term::Record(fields) => fields
            .get(index)
            .map(|(_, ty)| ty.clone())
            .ok_or("Drop field index is out of bounds".into()),
        Term::Nominal {
            declaration,
            arguments,
        } => {
            let definition = package
                .structs
                .iter()
                .find(|item| item.declaration == *declaration)
                .ok_or("Drop field layout is missing")?;
            definition
                .fields
                .get(index)
                .ok_or("Drop field index is out of bounds")?
                .1
                .instantiate_parameters(
                    &definition.parameter_binders().map_err(|e| e.to_string())?,
                    arguments,
                )
                .map_err(|e| e.to_string())
        }
        _ => Err("Drop field access has no aggregate layout".into()),
    }
}

fn project(
    package: &Package,
    ty: &mut Term,
    targets: &mut BTreeSet<Place>,
    fields: &[(nir::program::FieldOwner, usize)],
    fuel: &mut usize,
) -> Result<(), String> {
    // These are internal storage paths exported by memory analysis, including
    // explicit newtype conversion representation steps. Source field lookup
    // remains a separate frontend check.
    let mut traversal = memory::StorageTraversal::new(Some(package), ty, &[], fuel)?;
    traversal.then_guarded(Some(package), fields, &[], fuel)?;
    let mut next = BTreeSet::new();
    for target in targets.iter() {
        charge(
            fuel,
            target
                .fields
                .len()
                .saturating_add(fields.len())
                .saturating_add(1),
        )?;
        let mut target = target.clone();
        target.fields.extend(fields.iter().map(|(_, index)| *index));
        next.insert(target);
    }
    *targets = next;
    *ty = traversal.target.ty;
    Ok(())
}

fn place_type(
    package: &Package,
    types: &BTreeMap<PlaceId, Term>,
    place: &Place,
    variant_layouts: &BTreeMap<Place, VariantLayout>,
    fuel: &mut usize,
) -> Result<Term, String> {
    let ty = types
        .get(&place.root)
        .ok_or("Drop pointer target type is missing")?;
    memory::bound_type(ty, fuel)?;
    let mut ty = ty.clone();
    for (depth, index) in place.fields.iter().enumerate() {
        charge(fuel, depth + 1)?;
        let prefix = Place {
            root: place.root,
            fields: place.fields[..depth].to_vec(),
        };
        if let Some(layout) = variant_layouts.get(&prefix) {
            let selected = layout
                .fields
                .get(*index)
                .ok_or("Drop variant field index is out of bounds")?;
            memory::bound_type(selected, fuel)?;
            ty = selected.clone();
            continue;
        }
        // Cleanup places include private representation children; source
        // projections above still go through the public field layout only.
        if let Term::Nominal {
            declaration,
            arguments,
        } = &ty
        {
            if let Some(definition) = package
                .newtypes
                .iter()
                .find(|definition| definition.declaration == *declaration)
            {
                charge(fuel, 1)?;
                if *index != 0 {
                    return Err("Drop newtype representation index is out of bounds".into());
                }
                ty = definition
                    .representation_budgeted(arguments, &[], fuel)
                    .map_err(|error| error.to_string())?;
                continue;
            }
        }
        ty = field_type(package, &ty, *index, fuel)?;
    }
    Ok(ty)
}

pub(super) fn check_retired(
    retired: &[Place],
    target: &Place,
    fuel: &mut usize,
) -> Result<(), String> {
    for prior in retired {
        charge(
            fuel,
            prior
                .fields
                .len()
                .saturating_add(target.fields.len())
                .saturating_add(1),
        )?;
        if prior.root == target.root
            && (target.fields.starts_with(&prior.fields)
                || prior.fields.starts_with(&target.fields))
        {
            return Err(format!(
                "Drop target {target} is unavailable after earlier cleanup of {prior}"
            ));
        }
    }
    Ok(())
}
