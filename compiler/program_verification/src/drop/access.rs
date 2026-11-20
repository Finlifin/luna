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

#[derive(Clone, Copy)]
pub struct DropEnvContext<'a> {
    pub env_parameters: &'a [nir::program::RuntimeEnvParameter],
    pub runtime_deliveries: &'a BTreeMap<nir::RuntimeId, nir::program::Expr>,
    pub global_runtime_deliveries: &'a BTreeMap<nir::RuntimeId, nir::program::Expr>,
    pub fallback_source: nir::program::SourceRange,
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
    /// Enclosing function's runtime environment context for delivering Drop env arguments.
    pub env: Option<DropEnvContext<'a>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccessCheck {
    pub receiver: Place,
    pub constructed: CleanupTree,
    pub footprint: memory::Summary,
    /// Resolved places in the same order as footprint.accesses.
    pub targets: Vec<BTreeSet<Place>>,
    /// Delivered runtime environment expressions matching `selected.env_requirements`.
    pub env_arguments: Vec<nir::program::Expr>,
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
        f.write_str(")")?;
        if !self.env_arguments.is_empty() {
            f.write_str(" (env")?;
            for arg in &self.env_arguments {
                write!(f, " {arg}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
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

pub(super) struct AccessContext<'a> {
    pub declarations: &'a crate::declarations::Metadata<'a>,
    pub source: Option<::materialization::DeclarationSource<'a>>,
    pub methods: &'a mut memory::MethodCallees,
    pub summaries: &'a mut memory::SummaryCache,
    pub verified_drops: &'a mut std::collections::HashSet<(Term, DeclarationId, DeclarationId)>,
    pub text_provenance: memory::TextProvenance,
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
    check_access_with_context(
        package,
        world,
        selected,
        state,
        receiver,
        pointers,
        variant_layouts,
        None,
        fuel,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn check_access_with_context(
    package: &Package,
    world: &World<'_>,
    selected: &SelectedDrop,
    state: &State,
    receiver: &Place,
    pointers: Option<PointerMemory<'_>>,
    variant_layouts: &BTreeMap<Place, VariantLayout>,
    mut context: Option<&mut AccessContext<'_>>,
    fuel: &mut usize,
) -> Result<AccessCheck, String> {
    if *fuel == 0 {
        return Err("Drop access budget exhausted".into());
    }
    *fuel -= 1;
    let drop_key = (
        selected.subject.clone(),
        selected.implementation,
        selected.method.body,
    );
    let verified = context
        .as_deref()
        .is_some_and(|ctx| ctx.verified_drops.contains(&drop_key));
    if verified {
        memory::bound_type(&selected.subject, fuel)?;
    } else {
        if !world.index.type_resolution_is_current(selected.selection()) {
            return Err("Drop selection belongs to a different implementation world".into());
        }
        selected.evidence.check_instance(
            selected.protocol,
            &selected.subject,
            selected.implementation,
            &selected.implementation_arguments,
            fuel,
        )?;
        selected.evidence.check_world(world, fuel)?;
        let implementation = package
            .implementations
            .iter()
            .find(|item| item.declaration == selected.implementation)
            .ok_or("captured Drop implementation is missing")?;
        let substitution = nir::DeclarationSubstitution::bounded(
            &implementation.type_parameters,
            &implementation.parameter_types,
            &selected.implementation_arguments,
            &[],
            fuel,
        )
        .map_err(|e| e.to_string())?;
        let subject = substitution
            .instantiate_bounded(&implementation.subject, fuel)
            .map_err(|e| e.to_string())?;
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
            || instantiate_contract(member, contract.self_parameter, &selected.subject, fuel)?
                != selected.contract
        {
            return Err("captured Drop method disagrees with the current package".into());
        }
        let function = package
            .functions
            .iter()
            .find(|function| function.declaration == selected.method.body)
            .ok_or("captured Drop body is missing")?;
        let domains = if let Some(ctx) = context.as_deref() {
            ctx.declarations
                .parameter_domains(&function.type_parameters, fuel)
                .map_err(|e| e.to_string())?
        } else {
            memory::memory_domains(package, &function.type_parameters, fuel)?
        };
        let frames = [vec![Sort::Runtime]];
        let substitution = nir::DeclarationSubstitution::bounded(
            &function.type_parameters,
            &domains,
            &selected.method.arguments,
            &frames,
            fuel,
        )
        .map_err(|e| e.to_string())?;
        let program = nir::contracts::CallableProgram::function(function)
            .instantiate_contract(&substitution, fuel)
            .map_err(|e| e.to_string())?;
        if function.parameters != selected.parameters || program != selected.program {
            return Err("captured Drop body contract changed".into());
        }
        let expected_env = drop_environment(
            package,
            function,
            &substitution,
            &selected.method.arguments,
            fuel,
        )?;
        if expected_env != selected.env_requirements {
            return Err("captured Drop body environment contract changed".into());
        }
        if let Some(ctx) = context.as_deref_mut() {
            ctx.verified_drops.insert(drop_key);
        }
    }
    let mut env_arguments = Vec::with_capacity(selected.env_requirements.len());
    let mut resolved_envs: Vec<(BTreeSet<Place>, Term)> =
        Vec::with_capacity(selected.env_requirements.len());
    if !selected.env_requirements.is_empty() {
        let pointers_ctx =
            pointers.ok_or("Drop runtime environment access requires a memory snapshot")?;
        let env_ctx = pointers_ctx
            .env
            .ok_or("Drop runtime environment requires an active call-site environment context")?;
        for req in &selected.env_requirements {
            charge(fuel, 1)?;
            let (root, fields) = nir::runtime_fields::path(&req.identity, fuel)?;
            let delivered = env_ctx
                .env_parameters
                .iter()
                .find(|ep| match root {
                    Term::Runtime(id) => ep.runtime_id == Some(*id),
                    Term::Rigid {
                        declaration,
                        sort: Sort::Runtime,
                    } => ep.declaration == *declaration,
                    _ => false,
                })
                .map(|ep| nir::program::Expr {
                    source: env_ctx.fallback_source,
                    ty: ep.ty.clone(),
                    kind: nir::program::ExprKind::Local(ep.binding),
                })
                .or_else(|| {
                    let Term::Runtime(id) = root else { return None };
                    env_ctx
                        .runtime_deliveries
                        .get(id)
                        .or_else(|| env_ctx.global_runtime_deliveries.get(id))
                        .cloned()
                })
                .ok_or("Drop runtime environment argument is not deliverable in this scope")?;
            let delivered = nir::runtime_fields::address(
                delivered,
                &fields,
                env_ctx.fallback_source,
                fuel,
                |ty, index, fuel| {
                    nir::runtime_fields::struct_field(ty, index, Some(package), &[], fuel)
                },
            )?;
            if delivered.ty != req.ty {
                return Err(
                    "Drop runtime environment argument type disagrees with requirement".into(),
                );
            }
            if let Some(message) =
                nir::program::pointer_operation_error_in(&delivered, package.target)
            {
                return Err(message.into());
            }
            match &req.ty {
                Term::Pointer(pointee) => {
                    let targets = resolve_env_pointer_targets(
                        package,
                        &delivered,
                        pointee,
                        state,
                        pointers_ctx,
                        variant_layouts,
                        fuel,
                    )?;
                    resolved_envs.push((targets, (**pointee).clone()));
                }
                _ => {
                    return Err(
                        "Drop runtime environment value must currently be a lifted pointer".into(),
                    );
                }
            }
            env_arguments.push(delivered);
        }
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
    let footprint = if let Some(ctx) = context {
        memory::infer_in_package_with_full_context(
            package,
            selected.method.body,
            &selected.method.arguments,
            ctx.text_provenance,
            ctx.source,
            Some(&mut *ctx.methods),
            Some(ctx.declarations),
            &mut *ctx.summaries,
            fuel,
        )?
    } else {
        memory::infer_in_package(
            package,
            selected.method.body,
            &selected.method.arguments,
            fuel,
        )?
    };
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
        let (mut targets, mut ty) = if access.target.parameter == 0 {
            (BTreeSet::from([receiver.clone()]), selected.subject.clone())
        } else {
            let Some((env_targets, env_pointee_ty)) =
                resolved_envs.get(access.target.parameter - 1)
            else {
                return Err("Drop access targets an unavailable parameter".into());
            };
            (env_targets.clone(), env_pointee_ty.clone())
        };
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
            if crate::is_const_global_place(target.root) {
                if access.write {
                    return Err("Drop cannot write through a pointer to a global constant".into());
                }
                continue;
            }
            if access.write {
                if let Some(pointers) = pointers {
                    check_retired(pointers.retired, target, fuel)?;
                }
            }
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
        env_arguments,
    })
}

fn address_place(mut expr: &nir::program::Expr, fuel: &mut usize) -> Result<Place, String> {
    use nir::program::ExprKind as K;
    let mut fields = Vec::new();
    loop {
        charge(fuel, 1)?;
        match &expr.kind {
            K::Local(binding) => {
                fields.reverse();
                return Ok(Place {
                    root: PlaceId {
                        owner: u64::from(binding.owner),
                        local: binding.index,
                    },
                    fields,
                });
            }
            K::Field { value, index, .. } | K::AnonymousField { value, index, .. } => {
                fields.push(*index);
                expr = value;
            }
            _ => {
                return Err(
                    "Drop runtime environment address expression requires a local place".into(),
                );
            }
        }
    }
}

fn resolve_env_pointer_targets(
    package: &Package,
    delivered: &nir::program::Expr,
    pointee: &Term,
    state: &State,
    pointers: PointerMemory<'_>,
    variant_layouts: &BTreeMap<Place, VariantLayout>,
    fuel: &mut usize,
) -> Result<BTreeSet<Place>, String> {
    use nir::program::ExprKind as K;
    match &delivered.kind {
        K::PointerField { pointer, index, .. } | K::PointerAnonymousField { pointer, index, .. } => {
            let Term::Pointer(base) = &pointer.ty else {
                return Err("Drop field address requires a pointer".into());
            };
            let bases = resolve_env_pointer_targets(package, pointer, base, state, pointers, variant_layouts, fuel)?;
            let mut targets = BTreeSet::new();
            for mut place in bases {
                charge(fuel, 1)?;
                place.fields.push(*index);
                if !crate::is_const_global_place(place.root) {
                    check_retired(pointers.retired, &place, fuel)?;
                    state.require_present(&place).map_err(|e| format!("Drop environment field is unavailable: {e:?}"))?;
                }
                if (!crate::is_const_global_place(place.root) || pointers.types.contains_key(&place.root))
                    && place_type(package, pointers.types, &place, variant_layouts, fuel)? != *pointee {
                    return Err("Drop field address has the wrong target type".into());
                }
                targets.insert(place);
            }
            Ok(targets)
        }
        K::ConstAddress { declaration, value } => {
            if &value.ty != pointee {
                return Err(
                    "Drop runtime environment constant address disagrees with pointee type".into(),
                );
            }
            if !nir::free_bindings::collect(value, fuel)
                .map_err(|e| e.to_string())?
                .is_empty()
            {
                return Err(
                    "Drop runtime environment constant address retains free local bindings".into(),
                );
            }
            Ok(BTreeSet::from([Place {
                root: crate::const_global_place(*declaration),
                fields: vec![],
            }]))
        }
        K::AddressOf(place_expr) => {
            let place = address_place(place_expr, fuel)?;
            check_retired(pointers.retired, &place, fuel)?;
            state.require_present(&place).map_err(|e| {
                format!("Drop runtime environment target {place} is unavailable: {e:?}")
            })?;
            if place_type(package, pointers.types, &place, variant_layouts, fuel)? != *pointee {
                return Err(
                    "Drop runtime environment pointer target type disagrees with pointee".into(),
                );
            }
            Ok(BTreeSet::from([place]))
        }
        K::Local(binding) => {
            let slot_place = Place {
                root: PlaceId {
                    owner: u64::from(binding.owner),
                    local: binding.index,
                },
                fields: vec![],
            };
            check_retired(pointers.retired, &slot_place, fuel)?;
            state.require_present(&slot_place).map_err(|e| {
                format!(
                    "Drop runtime environment binding {slot_place} is unavailable: {e:?}"
                )
            })?;
            let values = pointers
                .slots
                .get(&slot_place)
                .filter(|values| !values.is_empty())
                .ok_or("Drop runtime environment pointer binding lacks target provenance")?;
            for target in values {
                charge(fuel, target.fields.len().saturating_add(1))?;
                if crate::is_const_global_place(target.root) {
                    if pointers.types.contains_key(&target.root)
                        && place_type(package, pointers.types, target, variant_layouts, fuel)?
                            != *pointee
                    {
                        return Err(
                            "Drop runtime environment constant target type disagrees with pointee"
                                .into(),
                        );
                    }
                } else {
                    check_retired(pointers.retired, target, fuel)?;
                    state.require_present(target).map_err(|e| {
                        format!(
                            "Drop runtime environment target {target} is unavailable: {e:?}"
                        )
                    })?;
                    if place_type(package, pointers.types, target, variant_layouts, fuel)?
                        != *pointee
                    {
                        return Err(
                            "Drop runtime environment pointer target type disagrees with pointee"
                                .into(),
                        );
                    }
                }
            }
            Ok(values.clone())
        }
        _ => Err(
            "Drop runtime environment delivery requires a local binding, local address, or constant address".into(),
        ),
    }
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
            let field = &definition
                .fields
                .get(index)
                .ok_or("Drop field index is out of bounds")?
                .1;
            nir::DeclarationSubstitution::bounded(
                &definition.type_parameters,
                &definition.parameter_types,
                arguments,
                &[],
                fuel,
            )
            .map_err(|e| e.to_string())?
            .instantiate_bounded(field, fuel)
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
