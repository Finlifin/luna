//! Publish concrete cleanup operations from checked acquired responsibilities.
//! Includes source control-flow, default, closure and concrete callback contexts.
mod method;

pub use method::{MethodCallCleanup, lower_method_call_cleanup};

use elaboration::{
    capabilities::World,
    materialization::DeclarationSource,
    ptc::Budget,
    verification::{
        drop::{self, AcquiredCleanup, CleanupStep},
        moves::{self, VariantSelection},
        ownership::{Place, State},
    },
};
use nir::program::Package;
use std::fmt;

pub struct Prepared {
    pub program: iir::cleanup::Program,
    pub graph: iir::Package,
}

impl fmt::Display for Prepared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(prepared-cleanup {} {})", self.program, self.graph)
    }
}

pub fn prepare(
    package: &Package,
    world: &World<'_>,
    acquired: &AcquiredCleanup,
    state: &State,
    receiver: &Place,
    variants: &[VariantSelection],
    fuel: &mut usize,
    budget: Budget,
) -> Result<Prepared, String> {
    let plan = drop::plan_cleanup_with_variants(
        package, world, acquired, state, receiver, variants, fuel,
    )?;
    let mut roots = vec![];
    let steps = lower(&plan.steps, receiver, &mut roots, &mut vec![], fuel, 0)?;
    let graph = elaboration::materialization::instantiate_with_types(
        package,
        &roots,
        std::slice::from_ref(acquired.ty()),
        Default::default(),
        budget,
    )?;
    graph.validate_cleanup_control(fuel)?;
    Ok(Prepared {
        program: iir::cleanup::Program {
            ty: acquired.ty().clone(),
            guards: variants
                .iter()
                .map(|v| (v.fields.clone(), v.index))
                .collect(),
            steps,
        },
        graph,
    })
}

fn lower(
    steps: &[CleanupStep],
    receiver: &Place,
    roots: &mut Vec<iir::InstanceKey>,
    receivers: &mut Vec<(iir::InstanceKey, nir::Term)>,
    fuel: &mut usize,
    depth: usize,
) -> Result<Vec<iir::cleanup::Step>, String> {
    if depth > 128 {
        return Err("cleanup lowering depth exhausted".into());
    }
    let mut output = vec![];
    for step in steps {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("cleanup lowering budget exhausted")?;
        let relative = |place: &Place| -> Result<Vec<usize>, String> {
            if place.root != receiver.root {
                return Err("cleanup step belongs to another owner".into());
            }
            place
                .fields
                .strip_prefix(receiver.fields.as_slice())
                .map(|p| p.to_vec())
                .ok_or("cleanup step is outside its receiver".into())
        };
        output.push(match step {
            CleanupStep::SelectVariantPlan { plans } => {
                let mut cases = vec![];
                for plan in plans {
                    if &plan.receiver != receiver {
                        return Err("residual selection changes cleanup receiver".into());
                    }
                    // The enclosing program already checks the subject type.
                    // Each case retains exact tag premises and verified steps.
                    cases.push(iir::cleanup::VariantCase {
                        guards: plan
                            .variants
                            .iter()
                            .map(|v| (v.fields.clone(), v.index))
                            .collect(),
                        steps: lower(&plan.steps, receiver, roots, receivers, fuel, depth + 1)?,
                    });
                }
                iir::cleanup::Step::Select { cases }
            }
            CleanupStep::IfConstructed { place, steps } => iir::cleanup::Step::IfConstructed {
                path: relative(place)?,
                steps: lower(steps, receiver, roots, receivers, fuel, depth + 1)?,
            },
            CleanupStep::Invoke {
                selected,
                access,
                env_arguments,
            } => {
                let method = iir::InstanceKey {
                    declaration: selected.method.body,
                    arguments: selected.method.arguments.clone().into(),
                };
                if !roots.contains(&method) {
                    roots.push(method.clone());
                }
                if let Some((_, subject)) = receivers.iter().find(|(key, _)| *key == method) {
                    if subject != &selected.subject {
                        return Err("cleanup method has inconsistent receiver types".into());
                    }
                } else {
                    receivers.push((method.clone(), selected.subject.clone()));
                }
                iir::cleanup::Step::Invoke {
                    path: relative(&access.receiver)?,
                    subject: selected.subject.clone(),
                    method,
                    env_arguments: env_arguments.clone(),
                }
            }
            CleanupStep::Release { place, ty } => iir::cleanup::Step::Release {
                path: relative(place)?,
                ty: ty.clone(),
            },
            CleanupStep::SwitchVariant { place, branches } => {
                if branches
                    .windows(2)
                    .any(|pair| pair[0].index >= pair[1].index)
                {
                    return Err("noncanonical cleanup variant order".into());
                }
                if branches
                    .iter()
                    .enumerate()
                    .any(|(index, branch)| branch.index != index)
                {
                    let mut cases = vec![];
                    for branch in branches {
                        cases.push(iir::cleanup::VariantCase {
                            guards: vec![(relative(place)?, branch.index)],
                            steps: lower(
                                &branch.steps,
                                receiver,
                                roots,
                                receivers,
                                fuel,
                                depth + 1,
                            )?,
                        });
                    }
                    output.push(iir::cleanup::Step::Select { cases });
                    continue;
                }
                let mut output = vec![];
                for branch in branches {
                    output.push(lower(
                        &branch.steps,
                        receiver,
                        roots,
                        receivers,
                        fuel,
                        depth + 1,
                    )?);
                }
                iir::cleanup::Step::Switch {
                    path: relative(place)?,
                    branches: output,
                }
            }
        });
    }
    Ok(output)
}

#[derive(Debug)]
pub struct Executable {
    pub entry: iir::InstanceKey,
    pub graph: iir::Package,
    pub schedule: iir::cleanup::Schedule,
}

impl fmt::Display for Executable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(resource-executable (entry {}) {} {})",
            self.entry, self.schedule, self.graph
        )
    }
}

fn term_contains_str(ty: &nir::Term, fuel: &mut usize) -> Result<bool, String> {
    let mut stack = vec![(ty, 0usize)];
    while let Some((term, depth)) = stack.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("text detection type budget exhausted")?;
        if depth >= 128 {
            return Ok(true);
        }
        if matches!(term, nir::Term::Primitive(nir::Primitive::Str)) {
            return Ok(true);
        }
        term.visit_children::<String>(|child| {
            stack.push((child, depth + 1));
            Ok(())
        })?;
    }
    Ok(false)
}

fn type_already_in_graph(graph: &iir::Package, ty: &nir::Term, depth: usize) -> bool {
    if depth >= 32 {
        return false;
    }
    match ty {
        nir::Term::Primitive(_) => true,
        nir::Term::Nominal { .. } => graph.nominals.iter().any(|n| n.ty == *ty),
        nir::Term::Optional(inner) | nir::Term::Pointer(inner) => {
            type_already_in_graph(graph, inner, depth + 1)
        }
        nir::Term::ErrorResult { errors, result } => {
            type_already_in_graph(graph, result, depth + 1)
                && errors
                    .iter()
                    .all(|e| type_already_in_graph(graph, e, depth + 1))
        }
        nir::Term::Tuple(items) => items
            .iter()
            .all(|item| type_already_in_graph(graph, item, depth + 1)),
        nir::Term::Record(fields) => fields
            .iter()
            .all(|(_, field)| type_already_in_graph(graph, field, depth + 1)),
        _ => false,
    }
}

#[derive(Default)]
struct InstanceDeps {
    callees: std::collections::HashSet<iir::InstanceKey>,
    dynamic_or_unknown: bool,
}

fn affected_cleanup_instances(
    changed: &std::collections::HashSet<iir::InstanceKey>,
    instance_deps: &std::collections::HashMap<iir::InstanceKey, InstanceDeps>,
    schedule_deps: &std::collections::HashMap<
        iir::InstanceKey,
        std::collections::HashSet<iir::InstanceKey>,
    >,
    borrowed_callables: &[iir::InstanceKey],
) -> std::collections::HashSet<iir::InstanceKey> {
    let mut affected = std::collections::HashSet::new();
    if changed.is_empty() {
        return affected;
    }
    let mut worklist = std::collections::VecDeque::new();
    let mut enqueue = |key: iir::InstanceKey| {
        if affected.insert(key.clone()) {
            worklist.push_back(key);
        }
    };
    for key in changed {
        enqueue(key.clone());
    }
    for key in borrowed_callables {
        enqueue(key.clone());
    }
    for (caller, deps) in instance_deps {
        if deps.dynamic_or_unknown {
            enqueue(caller.clone());
        }
    }

    let mut callers_of: std::collections::HashMap<&iir::InstanceKey, Vec<&iir::InstanceKey>> =
        std::collections::HashMap::new();
    for (caller, deps) in instance_deps {
        for callee in &deps.callees {
            callers_of.entry(callee).or_default().push(caller);
        }
    }
    for (caller, callees) in schedule_deps {
        for callee in callees {
            callers_of.entry(callee).or_default().push(caller);
        }
    }

    while let Some(key) = worklist.pop_front() {
        if let Some(callers) = callers_of.get(&key) {
            for &caller in callers {
                if affected.insert(caller.clone()) {
                    worklist.push_back(caller.clone());
                }
            }
        }
    }
    affected
}

/// Publish schedules for every reachable concrete function, including captured
/// destructors. Unmapped responsibilities are rejected before execution.
fn field_borrows(
    session: &mut elaboration::verification::PackageSession<'_>,
    function: &nir::program::Function,
    text_provenance: elaboration::verification::memory::TextProvenance,
    fuel: &mut usize,
) -> Result<Vec<moves::BorrowedField>, String> {
    let nir::Term::Function { parameters, .. } = function.signature.callable_signature() else {
        return Err("field borrow analysis lacks a callable signature".into());
    };
    let mut fields = vec![];
    for (parameter, input) in parameters.iter().enumerate() {
        if input.sort != nir::Sort::Runtime
            && matches!(
                input.ty,
                nir::Term::Primitive(nir::Primitive::Type) | nir::Term::ClosureContextType
            )
        {
            continue;
        }
        // Closure code identity is supplied separately, but its owned
        // environment still needs parameter-relative pointee premises.
        for leaf in session.pointer_input_layout(&input.ty, text_provenance, fuel)? {
            let path = leaf.input_fields;
            let pointee = leaf.pointee;
            if !path.is_empty() {
                fields.push(moves::BorrowedField {
                    parameter,
                    fields: path,
                    pointee,
                });
            }
        }
    }
    for (offset, env) in function.env_parameters.iter().enumerate() {
        let parameter = parameters.len() + offset;
        for leaf in session.pointer_input_layout(&env.ty, text_provenance, fuel)? {
            let path = leaf.input_fields;
            let pointee = leaf.pointee;
            if !path.is_empty() {
                fields.push(moves::BorrowedField {
                    parameter,
                    fields: path,
                    pointee,
                });
            }
        }
    }
    Ok(fields)
}

fn register_missing_roots(
    missing: Vec<drop::RequiredDropInstance>,
    initial_roots_len: usize,
    roots: &mut Vec<iir::InstanceKey>,
    receivers: &mut Vec<(iir::InstanceKey, nir::Term)>,
) -> Result<bool, String> {
    if missing.is_empty() {
        return Ok(false);
    }
    for item in missing {
        let method = iir::InstanceKey {
            declaration: item.declaration,
            arguments: item.arguments.into(),
        };
        if roots[..initial_roots_len].contains(&method) {
            return Err("destructor runtime instance was not materialized".into());
        }
        if !roots.contains(&method) {
            roots.push(method.clone());
        }
        if let Some((_, subject)) = receivers.iter().find(|(key, _)| *key == method) {
            if subject != &item.subject {
                return Err("cleanup method has inconsistent receiver types".into());
            }
        } else {
            receivers.push((method, item.subject));
        }
    }
    Ok(true)
}

pub fn prepare_source(
    package: &Package,
    world: &World<'_>,
    items: &intrinsic::LangItems,
    entry: iir::InstanceKey,
    fuel: &mut usize,
    budget: Budget,
) -> Result<Executable, String> {
    prepare_source_impl(
        DeclarationSource::Borrowed(package),
        world,
        items,
        entry,
        fuel,
        budget,
        false,
    )
}

/// Resource-checked source execution also checks contracts exposed by staging.
/// The public low-level planner remains usable to test rejection cleanup on
/// deliberately malformed NIR, without claiming source verification.
pub(crate) fn prepare_checked_source(
    protocols: &crate::resources::ProtocolContext,
    entry: iir::InstanceKey,
    fuel: &mut usize,
    budget: Budget,
) -> Result<Executable, String> {
    prepare_source_impl(
        protocols.source(),
        &protocols.world(),
        &protocols.items,
        entry,
        fuel,
        budget,
        true,
    )
}

fn prepare_source_impl(
    source: DeclarationSource<'_>,
    world: &World<'_>,
    items: &intrinsic::LangItems,
    entry: iir::InstanceKey,
    fuel: &mut usize,
    budget: Budget,
    validate_contracts: bool,
) -> Result<Executable, String> {
    let package = source.package();
    use elaboration::verification::moves::{CleanupAction, CleanupReason};
    use iir::cleanup::{
        Action, Boundary, ContextKey, DiscardCleanup, FunctionSchedule, Schedule, Site, Target,
    };
    use nir::program::ExprKind as K;
    let mut roots = vec![entry.clone()];
    let mut types = vec![];
    let mut receivers: Vec<(iir::InstanceKey, nir::Term)> = vec![];
    let mut schedule = Schedule::default();
    let mut generated_layouts = std::collections::HashSet::new();
    let mut cleanup_accesses: Vec<(iir::InstanceKey, Vec<nir::program::RuntimeCleanupAccess>)> =
        vec![];
    let mut instantiation = elaboration::materialization::InstantiationSession::new(source);
    // Graph construction and resource planning are distinct bounded stages.
    // Rebuilding for new destructor roots consumes the same graph allowance;
    // it must not receive a fresh budget on each pass through the loop.
    let mut construction_budget = budget;
    let mut verified_text_roots = std::collections::HashSet::new();
    let mut validated_instances = std::collections::HashSet::new();
    let mut instance_deps: std::collections::HashMap<iir::InstanceKey, InstanceDeps> =
        std::collections::HashMap::new();
    let mut schedule_deps: std::collections::HashMap<
        iir::InstanceKey,
        std::collections::HashSet<iir::InstanceKey>,
    > = std::collections::HashMap::new();
    let mut scheduled_roots: std::collections::HashSet<iir::InstanceKey> =
        std::collections::HashSet::new();
    let mut graph_has_text = false;
    let mut retained_caches = elaboration::verification::SessionCaches::default();
    let mut owned_layouts: Option<Package> = None;
    let mut copy_cache = std::collections::HashMap::new();
    let mut cached_graph: Option<(Vec<iir::InstanceKey>, Vec<nir::Term>, iir::Package)> = None;

    loop {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("source cleanup publication budget exhausted")?;
        // Generated return storage is recreated by its owning function, not
        // an independently materializable source type root.
        let source_types = types
            .iter()
            .filter(|ty| !generated_layouts.contains(*ty))
            .cloned()
            .collect::<Vec<_>>();
        let graph_changed = !matches!(
            &cached_graph,
            Some((cached_roots, cached_types, _))
                if cached_roots == &roots && cached_types == &source_types
        );
        if graph_changed {
            let graph = elaboration::materialization::extend_with_types_in_session(
                &mut instantiation,
                &roots,
                &source_types,
                Default::default(),
                &mut construction_budget,
            )?;
            cached_graph = Some((roots.clone(), source_types, graph));
        }
        let graph = &cached_graph.as_ref().unwrap().2;
        if graph_changed {
            let active_enums = owned_layouts
                .as_ref()
                .map_or(&package.enums, |view| &view.enums);
            let generated = graph
                .nominals
                .iter()
                .filter_map(|nominal| match &nominal.definition {
                    iir::NominalDefinition::Enum(definition)
                        if !active_enums
                            .iter()
                            .any(|source| source.declaration == definition.declaration) =>
                    {
                        Some((nominal.ty.clone(), definition.clone()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            if !generated.is_empty() {
                // New roots can change the traversal order of late expansions.
                // Recompute schedules against this graph's identities and sources.
                schedule = Schedule::default();
                copy_cache.clear();
                retained_caches = Default::default();
                verified_text_roots.clear();
                validated_instances.clear();
                instance_deps.clear();
                schedule_deps.clear();
                scheduled_roots.clear();
                graph_has_text = false;
                let view = owned_layouts.get_or_insert_with(|| package.clone());
                for (ty, definition) in generated {
                    nir::type_computations::charge_term(&ty, fuel).map_err(|e| e.to_string())?;
                    *fuel = fuel
                        .checked_sub(definition.variants.len().saturating_add(1))
                        .ok_or("generated enum layout budget exhausted")?;
                    for variant in &definition.variants {
                        for field in &variant.fields {
                            nir::type_computations::charge_term(field, fuel)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    generated_layouts.insert(ty);
                    view.enums.push(definition);
                }
            } else {
                retained_caches.on_graph_extended();
            }
            for instance in &graph.functions {
                if !validated_instances.insert(instance.key.clone()) {
                    continue;
                }
                nir::type_computations::charge_term(&instance.function.signature, fuel)
                    .map_err(|e| e.to_string())?;
                if !graph_has_text && term_contains_str(&instance.function.signature, fuel)? {
                    graph_has_text = true;
                }
                for argument in instance.key.arguments.iter() {
                    nir::type_computations::charge_term(argument, fuel)
                        .map_err(|e| e.to_string())?;
                    if !graph_has_text && term_contains_str(argument, fuel)? {
                        graph_has_text = true;
                    }
                }
                let mut deps = InstanceDeps::default();
                nir::visit::expressions(&instance.function, fuel, |expr, fuel| {
                    graph.validate_cleanup_control_expr(expr, fuel)?;
                    nir::type_computations::charge_term(&expr.ty, fuel)
                        .map_err(|e| e.to_string())?;
                    if !graph_has_text && term_contains_str(&expr.ty, fuel)? {
                        graph_has_text = true;
                    }
                    match &expr.kind {
                        K::Function(declaration) => {
                            deps.callees.insert(iir::InstanceKey {
                                declaration: *declaration,
                                arguments: vec![].into(),
                            });
                        }
                        K::FunctionInstance {
                            declaration,
                            arguments,
                        } => {
                            deps.callees.insert(iir::InstanceKey {
                                declaration: *declaration,
                                arguments: arguments.clone().into(),
                            });
                        }
                        K::TraitMethod {
                            interface,
                            interface_arguments,
                            subject,
                            member,
                            method_arguments,
                        } => {
                            let mut matched = false;
                            for dispatch in &graph.dispatch {
                                if dispatch.interface == *interface
                                    && dispatch.interface_arguments == *interface_arguments
                                    && dispatch.subject == *subject
                                {
                                    for method in &dispatch.methods {
                                        if method.member == *member
                                            && method.method_arguments == *method_arguments
                                        {
                                            deps.callees.insert(iir::InstanceKey {
                                                declaration: method.body,
                                                arguments: method.arguments.clone().into(),
                                            });
                                            matched = true;
                                        }
                                    }
                                }
                            }
                            if !matched {
                                deps.dynamic_or_unknown = true;
                            }
                        }
                        K::Call { callee, .. }
                            if !matches!(
                                callee.kind,
                                K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. }
                            ) =>
                        {
                            deps.dynamic_or_unknown = true;
                        }
                        K::EffectHandle { .. } | K::EffectRequest { .. } | K::Closure { .. } => {
                            deps.dynamic_or_unknown = true;
                        }
                        _ => {}
                    }
                    Ok(())
                })?;
                instance_deps.insert(instance.key.clone(), deps);
            }
        }
        let mut cleanup_changed = false;
        let mut changed_cleanup_keys = std::collections::HashSet::new();
        let mut round_affected = std::collections::HashSet::new();
        let view = owned_layouts.get_or_insert_with(|| package.clone());
        if graph_changed || view.runtime_instances.len() != graph.functions.len() {
            view.runtime_instances = graph
                .functions
                .iter()
                .map(|instance| nir::program::RuntimeInstance {
                    declaration: instance.key.declaration,
                    arguments: instance.key.arguments.to_vec(),
                    function: instance.function.clone(),
                    cleanup_accesses: cleanup_accesses
                        .iter()
                        .find(|(key, _)| key == &instance.key)
                        .map(|(_, accesses)| accesses.clone())
                        .unwrap_or_default(),
                })
                .collect();
        } else {
            let access_count: usize = cleanup_accesses.iter().map(|(_, a)| a.len()).sum();
            *fuel = fuel
                .checked_sub(access_count.saturating_add(1))
                .ok_or("cleanup footprint overlay budget exhausted")?;
            for runtime_instance in &mut view.runtime_instances {
                if let Some((_, accesses)) = cleanup_accesses.iter().find(|(key, _)| {
                    key.declaration == runtime_instance.declaration
                        && key.arguments.as_ref() == runtime_instance.arguments.as_slice()
                }) && runtime_instance.cleanup_accesses != *accesses
                {
                    runtime_instance.cleanup_accesses = accesses.clone();
                }
            }
        }
        let package: &Package = &*view;
        let dispatch_catalog = elaboration::materialization::BorrowedCatalog::new(package);
        let dispatch_source =
            elaboration::materialization::DeclarationSource::Prepared(&dispatch_catalog);
        let mut verification_session = elaboration::verification::PackageSession::new(package)
            .with_source(dispatch_source)
            .with_dispatch(&graph.dispatch)
            .with_global_runtime_deliveries(&graph.global_runtime_deliveries)
            .with_caches(std::mem::take(&mut retained_caches));
        let text_provenance = if graph_has_text {
            for root in &roots {
                if !verified_text_roots.contains(root) {
                    verification_session.infer_text_regions(
                        root.declaration,
                        &root.arguments,
                        fuel,
                    )?;
                    verified_text_roots.insert(root.clone());
                }
            }
            elaboration::verification::memory::TextProvenance::VerifiedByRegionAnalysis
        } else {
            elaboration::verification::memory::TextProvenance::Unchecked
        };
        let mut cleanup_session = match items.get(intrinsic::LangItem::Drop) {
            Some(intrinsic::LangItemDef::UserDef(protocol)) => Some(
                verification_session
                    .cleanup_session(world, protocol, budget)
                    .require_runtime_instances()
                    .with_text_provenance(text_provenance),
            ),
            _ => None,
        };
        let previous = (roots.len(), types.len());
        let initial_roots_len = roots.len();
        let mut deferred = false;
        let mut deferred_nested = false;
        'instances: for instance in graph.functions.iter().rev() {
            if scheduled_roots.contains(&instance.key) {
                continue;
            }
            if cleanup_changed && round_affected.contains(&instance.key) {
                continue;
            }
            // Higher-order bodies are verified against each call's concrete
            // inputs below, never published as an unconditional root schedule.
            if !verification_session
                .callable_parameter_paths(
                    &instance.function.signature,
                    fuel,
                    instance.function.body.source,
                )
                .map_err(|error| error.to_string())?
                .is_empty()
                || !moves::effect_result_slots_with_envs(
                    package,
                    &instance.function.signature,
                    &instance
                        .function
                        .env_parameters
                        .iter()
                        .map(|env| env.ty.clone())
                        .collect::<Vec<_>>(),
                    fuel,
                    instance.function.body.source,
                )
                .map_err(|error| error.to_string())?
                .is_empty()
            {
                if instance.key == entry {
                    return Err("higher-order entry requires callable input context".into());
                }
                continue;
            }
            *fuel = fuel
                .checked_sub(1)
                .ok_or("source cleanup function budget exhausted")?;
            let function = &instance.function;
            let mut borrows = vec![];
            let mut fields = vec![];
            let mut independent_report = None;
            if let Some((_, subject)) = receivers.iter().find(|(key, _)| key == &instance.key) {
                borrows.push((0, subject.clone()));
                for (offset, env) in function.env_parameters.iter().enumerate() {
                    if let Some((_, env_subject)) =
                        verification_session.pointer_representation(&env.ty, fuel)?
                    {
                        borrows.push((1 + offset, env_subject));
                    }
                }
            } else if instance.key != entry {
                let nir::Term::Function { parameters, .. } =
                    function.signature.callable_signature()
                else {
                    return Err("cleanup function lacks a callable signature".into());
                };
                for (index, parameter) in parameters.iter().enumerate() {
                    if let Some((_, subject)) =
                        verification_session.pointer_representation(&parameter.ty, fuel)?
                    {
                        borrows.push((index, subject));
                    }
                }
                for (offset, env) in function.env_parameters.iter().enumerate() {
                    if let Some((_, subject)) =
                        verification_session.pointer_representation(&env.ty, fuel)?
                    {
                        borrows.push((parameters.len() + offset, subject));
                    }
                }
                fields = field_borrows(&mut verification_session, function, text_provenance, fuel)?;
                if !fields.is_empty() {
                    // Moving an aggregate need not borrow its pointees. Try an
                    // unconditional plan before demanding field validity.
                    let allowance = (*fuel / 2).min(4096);
                    let (result, missing) = crate::protocols::analyze_cleanup_in_session(
                        &mut verification_session,
                        cleanup_session.as_mut(),
                        &mut copy_cache,
                        world,
                        items,
                        function,
                        allowance,
                        budget,
                        &borrows,
                        &[],
                        &[],
                        vec![],
                        &[],
                        &[],
                        text_provenance,
                    );
                    if register_missing_roots(
                        missing,
                        initial_roots_len,
                        &mut roots,
                        &mut receivers,
                    )? {
                        deferred = true;
                        continue 'instances;
                    }
                    match result {
                        Ok(report) => {
                            fields.clear();
                            independent_report = Some(report);
                        }
                        Err(_) => *fuel -= allowance,
                    }
                }
                if !borrows.is_empty() || !fields.is_empty() {
                    // Non-entry pointer calls are checked by the caller's memory
                    // analysis. Only supported, initialization-preserving body
                    // footprints justify these conditional parameter roots.
                    let footprint = match verification_session.infer_memory(
                        instance.key.declaration,
                        &instance.key.arguments,
                        text_provenance,
                        fuel,
                    ) {
                        Ok(footprint) => footprint,
                        // Like owned callbacks, borrowed callbacks need each
                        // invocation's code premises, never a root schedule.
                        Err(reason)
                            if reason
                                .contains("dynamic call requires a richer memory contract") =>
                        {
                            *fuel = fuel
                                .checked_sub(schedule.borrowed_callable_functions.len() + 1)
                                .ok_or("borrowed callable schedule budget exhausted")?;
                            if !schedule.borrowed_callable_functions.contains(&instance.key) {
                                schedule
                                    .borrowed_callable_functions
                                    .push(instance.key.clone());
                            }
                            continue;
                        }
                        Err(reason) => {
                            return Err(format!("borrowed helper {}: {reason}", function.name));
                        }
                    };
                    if &footprint.signature != function.signature.callable_signature() {
                        return Err(
                            "borrowed helper signature disagrees with memory footprint".into()
                        );
                    }
                }
            }
            let report = if let Some(report) = independent_report {
                report
            } else {
                let (result, missing) = crate::protocols::analyze_cleanup_in_session(
                    &mut verification_session,
                    cleanup_session.as_mut(),
                    &mut copy_cache,
                    world,
                    items,
                    function,
                    *fuel,
                    budget,
                    &borrows,
                    &[],
                    &[],
                    vec![],
                    &fields,
                    &[],
                    text_provenance,
                );
                if register_missing_roots(missing, initial_roots_len, &mut roots, &mut receivers)? {
                    deferred = true;
                    continue 'instances;
                }
                result.map_err(|e| e.to_string())?
            };
            if report.borrowed_parameters != borrows || report.borrowed_fields != fields {
                return Err("cleanup report disagrees with borrowed entry premises".into());
            }
            let mut scopes =
                std::collections::VecDeque::from([(vec![], function.clone(), report, vec![])]);
            let mut named_contexts: Vec<(
                Vec<ContextKey>,
                iir::InstanceKey,
                Vec<moves::CallableParameter>,
                Vec<moves::EnumParameter>,
                Vec<moves::EffectResultInput>,
                Vec<moves::BorrowedCode>,
            )> = vec![];
            while let Some((context, scope, report, observations)) = scopes.pop_front() {
                let footprint_key = if context.is_empty() {
                    Some(&instance.key)
                } else if let Some(ContextKey::Function { callee, .. }) = context.last() {
                    Some(callee)
                } else {
                    None
                };
                if let Some(footprint_key) = footprint_key.filter(|_| {
                    !report.borrowed_parameters.is_empty() || !report.borrowed_fields.is_empty()
                }) {
                    let accesses =
                        report
                            .cleanup_memory_accesses(package, fuel)
                            .map_err(|reason| {
                                format!("cleanup memory export for {}: {reason}", scope.name)
                            })?;
                    let position = if let Some(position) = cleanup_accesses
                        .iter()
                        .position(|(key, _)| key == footprint_key)
                    {
                        position
                    } else {
                        cleanup_accesses.push((footprint_key.clone(), vec![]));
                        cleanup_accesses.len() - 1
                    };
                    for access in accesses {
                        let access = nir::program::RuntimeCleanupAccess {
                            parameter: access.target.parameter,
                            input_fields: access.target.input_fields,
                            through: access.target.through,
                            conditions: access.target.conditions,
                            fields: access.target.fields,
                            ty: access.ty,
                            write: access.write,
                            read_value: access.read_value,
                        };
                        let effects = &mut cleanup_accesses[position].1;
                        *fuel = fuel
                            .checked_sub(effects.len() + 1)
                            .ok_or("cleanup footprint publication budget exhausted")?;
                        if !effects.contains(&access) {
                            let cost = access.through.iter().fold(
                                access
                                    .input_fields
                                    .len()
                                    .saturating_add(access.fields.len())
                                    .saturating_add(1),
                                |cost, path| cost.saturating_add(path.len()).saturating_add(1),
                            );
                            *fuel = fuel
                                .checked_sub(cost)
                                .ok_or("cleanup footprint overlay budget exhausted")?;
                            nir::type_computations::charge_term(&access.ty, fuel)
                                .map_err(|e| e.to_string())?;
                            effects.push(access);
                            if changed_cleanup_keys.insert(cleanup_accesses[position].0.clone()) {
                                round_affected = affected_cleanup_instances(
                                    &changed_cleanup_keys,
                                    &instance_deps,
                                    &schedule_deps,
                                    &schedule.borrowed_callable_functions,
                                );
                            }
                            cleanup_changed = true;
                        }
                    }
                }

                if let Some(ContextKey::Function { callee, .. }) = context.last() {
                    named_contexts.push((
                        context.clone(),
                        callee.clone(),
                        report.callable_parameters.clone(),
                        report.enum_parameters.clone(),
                        moves::copy_effect_inputs(
                            &report.effect_parameters,
                            fuel,
                            scope.body.source,
                        )
                        .map_err(|e| e.to_string())?,
                        moves::copy_borrowed_code(&report.borrowed_code, fuel, scope.body.source)
                            .map_err(|e| e.to_string())?,
                    ));
                }
                if !report.deferred_calls.is_empty() {
                    return Err(
                        "executable cleanup contains unresolved callable obligations".into(),
                    );
                }
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or("cleanup scope budget exhausted")?;
                if schedule
                    .functions
                    .iter()
                    .any(|f| f.instance == instance.key && f.context == context)
                {
                    return Err("ambiguous repeated cleanup context".into());
                }
                let function = &scope;
                let mut default_roots = vec![];
                let mut hooks = if context.is_empty()
                    || matches!(
                        context.last(),
                        Some(ContextKey::Closure { .. } | ContextKey::Function { .. })
                    ) {
                    vec![(function.body.source, Boundary::FunctionEnd)]
                } else if matches!(context.last(), Some(ContextKey::EffectHandler { .. })) {
                    vec![(function.body.source, Boundary::HandlerEnd)]
                } else {
                    vec![]
                };
                let mut discard_hooks = vec![];
                let mut projection_hooks =
                    if matches!(context.last(), Some(ContextKey::EffectHandler { .. })) {
                        vec![function.body.source]
                    } else {
                        vec![]
                    };
                let mut callee_hooks = vec![];
                let mut prepared_hooks = vec![];
                let mut result_hooks = vec![function.body.source];
                hooks.extend(
                    function
                        .preconditions
                        .iter()
                        .map(|p| (p.source, Boundary::PredicateFailure)),
                );
                iir::visit::expressions_in_scope_with_observations(
                    function,
                    &observations,
                    fuel,
                    |expr, _| {
                        if !matches!(expr.kind, K::CallPreparation { .. })
                            && !expr.shares_discard_temporary_boundary()
                        {
                            hooks.push((expr.source, Boundary::TemporaryEnd));
                        }
                        if let K::Match { arms, .. } = &expr.kind {
                            for arm in arms {
                                projection_hooks.push(arm.body.source);
                                if !matches!(
                                    arm.body.kind,
                                    K::Block { .. } | K::CallPreparation { .. }
                                ) {
                                    hooks.push((arm.body.source, Boundary::ScopeEnd));
                                }
                            }
                        }
                        if let K::EnumName(value) = &expr.kind {
                            projection_hooks.push(value.source);
                        }
                        if let K::LetPattern { value, .. } = &expr.kind {
                            projection_hooks.push(expr.source);
                            projection_hooks.push(value.source);
                        }
                        if let K::AnonymousField { value, .. } | K::Field { value, .. } = &expr.kind
                        {
                            if !matches!(
                                value.kind,
                                K::AnonymousField { .. } | K::Field { .. } | K::Local(_)
                            ) {
                                projection_hooks.push(value.source);
                            }
                        }
                        if matches!(expr.kind, K::Return(_)) {
                            result_hooks.push(expr.source);
                        }
                        if let K::Discard(value) = &expr.kind {
                            discard_hooks.push((expr.source, value.source));
                        }
                        match &expr.kind {
                            K::Tuple(fields)
                            | K::Object { fields, .. }
                            | K::AnonymousRecord { fields }
                            | K::Record { fields, .. }
                            | K::Variant { fields, .. } => {
                                for (slot, value) in fields.iter().enumerate() {
                                    prepared_hooks.push((expr.source, slot, value.source));
                                }
                            }
                            K::ErrorConvert { value, .. } => {
                                prepared_hooks.push((expr.source, 0, value.source));
                            }
                            K::Closure { captures, .. } => {
                                for (slot, capture) in captures.iter().enumerate() {
                                    prepared_hooks.push((
                                        expr.source,
                                        slot,
                                        capture
                                            .value
                                            .as_ref()
                                            .map_or(expr.source, |value| value.source),
                                    ));
                                }
                            }
                            K::Call {
                                callee, arguments, ..
                            } => {
                                callee_hooks.push(callee.source);
                                if !matches!(
                                    callee.kind,
                                    K::Local(_)
                                        | K::Function(_)
                                        | K::FunctionInstance { .. }
                                        | K::TraitMethod { .. }
                                ) {
                                    projection_hooks.push(callee.source);
                                }
                                hooks.push((expr.source, Boundary::InterfacePreconditionFailure));
                                if matches!(callee.kind, K::TraitMethod { .. }) {
                                    hooks.push((
                                        expr.source,
                                        Boundary::InterfacePostconditionFailure,
                                    ));
                                    let nir::Term::Function { parameters, .. } =
                                        callee.ty.callable_signature()
                                    else {
                                        return Err(
                                            "interface result cleanup lacks a callable signature"
                                                .into(),
                                        );
                                    };
                                    prepared_hooks.push((
                                        expr.source,
                                        parameters.len(),
                                        expr.source,
                                    ));
                                }
                                for argument in arguments {
                                    if argument.is_default {
                                        hooks.push((
                                            expr.source,
                                            Boundary::DefaultFailure(argument.parameter),
                                        ));
                                        default_roots.push((
                                            ContextKey::Default {
                                                construction: expr.source,
                                                parameter: argument.parameter,
                                            },
                                            argument.value.clone(),
                                        ));
                                    }
                                    prepared_hooks.push((
                                        expr.source,
                                        argument.parameter,
                                        argument.value.source,
                                    ));
                                }
                            }
                            _ => (),
                        }
                        if iir::effects::suspension_boundary(expr) {
                            hooks.push((expr.source, Boundary::EffectEscape));
                            hooks.push((expr.source, Boundary::EffectReturn));
                            result_hooks.push(expr.source);
                        }
                        if let K::EffectRequest { arguments, .. } = &expr.kind {
                            for (slot, argument) in arguments.iter().enumerate() {
                                prepared_hooks.push((expr.source, slot, argument.source));
                            }
                        }
                        if matches!(&expr.kind, K::EffectHandle { handlers, .. }
                        if handlers.iter().any(|h|h.abi == nir::effects::Abi::DirectPassing))
                        {
                            hooks.push((expr.source, Boundary::PredicateFailure));
                        }
                        let hook = match &expr.kind {
                            K::Block { .. } | K::CallPreparation { .. } => {
                                Some((expr.source, Boundary::ScopeEnd))
                            }
                            K::Return(_) => Some((expr.source, Boundary::Return)),
                            K::EffectResume(_) => Some((expr.source, Boundary::EffectResume)),
                            K::Break(_) => Some((expr.source, Boundary::Break)),
                            K::Continue(_) => Some((expr.source, Boundary::Continue)),
                            K::Assign { .. } => Some((expr.source, Boundary::Replacement)),
                            K::RequirePredicate(condition)
                            | K::EnsurePredicate(condition)
                            | K::CheckPredicate(condition) => {
                                Some((condition.source, Boundary::PredicateFailure))
                            }
                            K::Call { .. }
                            | K::EffectRequest {
                                abi: nir::effects::Abi::Direct,
                                ..
                            } => Some((expr.source, Boundary::PredicateFailure)),
                            _ => None,
                        };
                        if let Some(hook) = hook {
                            hooks.push(hook);
                        }
                        Ok(())
                    },
                )?;
                for site in &report.cleanup_sites {
                    let boundary = match site.reason {
                        CleanupReason::EffectBreak(id) => Some(Boundary::EffectBreak(id)),
                        CleanupReason::EffectContinue(id) => Some(Boundary::EffectContinue(id)),
                        _ => None,
                    };
                    if let Some(boundary) = boundary {
                        if !hooks.contains(&(site.source, Boundary::EffectEscape)) {
                            return Err("effect loop edge has no suspension boundary".into());
                        }
                        if !hooks.contains(&(site.source, boundary)) {
                            hooks.push((site.source, boundary));
                        }
                    }
                }
                if !function.postconditions.is_empty() {
                    for origin in result_hooks {
                        if function.result_binding.is_none() {
                            prepared_hooks.push((origin, 0, origin));
                        }
                        for condition in &function.postconditions {
                            hooks.push((condition.source, Boundary::PostconditionFailure(origin)));
                        }
                    }
                }
                for call in report.named_calls {
                    let key = match &call.callee.kind {
                        K::Function(declaration) => iir::InstanceKey {
                            declaration: *declaration,
                            arguments: vec![].into(),
                        },
                        K::FunctionInstance {
                            declaration,
                            arguments,
                        } => iir::InstanceKey {
                            declaration: *declaration,
                            arguments: arguments.clone().into(),
                        },
                        _ => return Err("named callable context lacks a static instance".into()),
                    };
                    let child = graph
                        .function(&key)
                        .ok_or("named callable instance is missing")?
                        .function
                        .clone();
                    if child.signature.callable_signature() != call.callee.ty.callable_signature() {
                        return Err("named callable context signature mismatch".into());
                    }
                    let mut recursive_target = None;
                    for (ancestor, callee, callables, enums, effects, borrowed_code) in
                        &named_contexts
                    {
                        *fuel = fuel
                            .checked_sub(ancestor.len() + 1)
                            .ok_or("recursive cleanup context budget exhausted")?;
                        if callee == &key
                            && context.starts_with(ancestor)
                            && callables == &call.callables
                            && enums == &call.enums
                            && effects == &call.effects
                            && borrowed_code == &call.borrowed_code
                        {
                            recursive_target = Some(ancestor.clone());
                            break;
                        }
                    }
                    if let Some(target) = recursive_target {
                        schedule_deps
                            .entry(instance.key.clone())
                            .or_default()
                            .insert(key.clone());
                        let mut recursive_context = context.clone();
                        recursive_context.push(ContextKey::Function {
                            call: call.source,
                            callee: key.clone(),
                        });
                        let alias = iir::cleanup::ContextAlias {
                            instance: instance.key.clone(),
                            context: recursive_context,
                            target,
                        };
                        if let Some(previous) = schedule.aliases.iter().find(|previous| {
                            previous.instance == alias.instance && previous.context == alias.context
                        }) {
                            if previous != &alias {
                                return Err("ambiguous recursive cleanup context".into());
                            }
                        } else {
                            schedule.aliases.push(alias);
                        }
                        continue;
                    }
                    if context.len() >= 128 {
                        return Err("named callable context depth exhausted".into());
                    }
                    let mut borrows = vec![];
                    if let nir::Term::Function { parameters, .. } =
                        child.signature.callable_signature()
                    {
                        for (slot, parameter) in parameters.iter().enumerate() {
                            if let Some((_, subject)) =
                                verification_session.pointer_representation(&parameter.ty, fuel)?
                            {
                                borrows.push((slot, subject));
                            }
                        }
                        for (offset, env) in child.env_parameters.iter().enumerate() {
                            if let Some((_, subject)) =
                                verification_session.pointer_representation(&env.ty, fuel)?
                            {
                                borrows.push((parameters.len() + offset, subject));
                            }
                        }
                    }
                    let mut fields =
                        field_borrows(&mut verification_session, &child, text_provenance, fuel)?;
                    let mut independent_invocation = None;
                    if !fields.is_empty() && call.borrowed_code.is_empty() {
                        let effects = moves::copy_effect_inputs(&call.effects, fuel, call.source)
                            .map_err(|e| e.to_string())?;
                        let allowance = (*fuel / 2).min(4096);
                        let (result, missing) = crate::protocols::analyze_cleanup_in_session(
                            &mut verification_session,
                            cleanup_session.as_mut(),
                            &mut copy_cache,
                            world,
                            items,
                            &child,
                            allowance,
                            budget,
                            &borrows,
                            &call.callables,
                            &call.enums,
                            effects,
                            &[],
                            &[],
                            text_provenance,
                        );
                        if register_missing_roots(
                            missing,
                            initial_roots_len,
                            &mut roots,
                            &mut receivers,
                        )? {
                            deferred_nested = true;
                            continue 'instances;
                        }
                        match result {
                            Ok(report) => {
                                fields.clear();
                                independent_invocation = Some(report);
                            }
                            Err(_) => *fuel -= allowance,
                        }
                    }
                    if !borrows.is_empty() || !fields.is_empty() {
                        let (result, missing) =
                            crate::protocols::infer_borrowed_callable_memory_in_session(
                                &mut verification_session,
                                cleanup_session.as_mut(),
                                key.declaration,
                                &key.arguments,
                                &call.callables,
                                &call.enums,
                                &call.effects,
                                &call.borrowed_code,
                                text_provenance,
                                fuel,
                                call.source,
                            );
                        if register_missing_roots(
                            missing,
                            initial_roots_len,
                            &mut roots,
                            &mut receivers,
                        )? {
                            deferred_nested = true;
                            continue 'instances;
                        }
                        let footprint = result?;
                        if footprint.signature != child.signature {
                            return Err(
                                "contextual helper signature disagrees with memory footprint"
                                    .into(),
                            );
                        }
                    }
                    let invocation = if let Some(report) = independent_invocation {
                        report
                    } else {
                        let effects = moves::copy_effect_inputs(&call.effects, fuel, call.source)
                            .map_err(|e| e.to_string())?;
                        let (result, missing) = crate::protocols::analyze_cleanup_in_session(
                            &mut verification_session,
                            cleanup_session.as_mut(),
                            &mut copy_cache,
                            world,
                            items,
                            &child,
                            *fuel,
                            budget,
                            &borrows,
                            &call.callables,
                            &call.enums,
                            effects,
                            &fields,
                            &call.borrowed_code,
                            text_provenance,
                        );
                        if register_missing_roots(
                            missing,
                            initial_roots_len,
                            &mut roots,
                            &mut receivers,
                        )? {
                            deferred_nested = true;
                            continue 'instances;
                        }
                        result.map_err(|e| e.to_string())?
                    };
                    if invocation.borrowed_parameters != borrows
                        || invocation.borrowed_fields != fields
                    {
                        return Err(
                            "contextual cleanup report disagrees with borrowed premises".into()
                        );
                    }
                    if invocation.callable_parameters != call.callables
                        || invocation.enum_parameters != call.enums
                        || invocation.effect_parameters != call.effects
                        || invocation.borrowed_code != call.borrowed_code
                    {
                        return Err("named callable context lost its input premises".into());
                    }
                    schedule_deps
                        .entry(instance.key.clone())
                        .or_default()
                        .insert(key.clone());
                    let mut child_context = context.clone();
                    child_context.push(ContextKey::Function {
                        call: call.source,
                        callee: key,
                    });
                    scopes.push_back((child_context, child, invocation, vec![]));
                }
                for handler in report.handlers {
                    let mut child = scope.clone();
                    child.body = handler.definition.body;
                    child.signature = nir::Term::Function {
                        parameters: vec![],
                        result: Box::new(handler.result),
                    };
                    child.parameters.clear();
                    child.defaults.clear();
                    let mut observations =
                        nir::visit::into_pattern_observations(handler.definition.pattern, fuel)?;
                    observations.extend(handler.definition.guard);
                    child.preconditions.clear();
                    child.postconditions.clear();
                    child.entry_snapshots.clear();
                    child.result_binding = None;
                    let mut child_context = context.clone();
                    child_context.push(ContextKey::EffectHandler {
                        installation: handler.installation,
                        handler: handler.handler,
                        arm: handler.arm,
                    });
                    scopes.push_back((child_context, child, handler.invocation, observations));
                }
                for closure in report
                    .closures
                    .into_iter()
                    .filter(|closure| closure.invoked)
                {
                    let K::Closure {
                        parameters, body, ..
                    } = &closure.definition.kind
                    else {
                        return Err("closure report has no definition".into());
                    };
                    let mut child = scope.clone();
                    child.body = body.as_ref().clone();
                    child.signature = closure.definition.ty.callable_signature().clone();
                    child.parameters = parameters.clone();
                    child.defaults.clear();
                    child.preconditions.clear();
                    child.postconditions.clear();
                    child.entry_snapshots.clear();
                    child.result_binding = None;
                    let mut child_context = context.clone();
                    child_context.push(ContextKey::Closure {
                        call: closure.source,
                        definition: closure.definition.source,
                    });
                    schedule
                        .closure_definitions
                        .push(iir::cleanup::ClosureDefinition {
                            instance: instance.key.clone(),
                            context: child_context.clone(),
                            definition: closure.definition,
                        });
                    scopes.push_back((child_context, child, closure.invocation, vec![]));
                }
                for default in report.defaults {
                    let key = ContextKey::Default {
                        construction: default.construction,
                        parameter: default.parameter,
                    };
                    let mut matches = default_roots
                        .iter()
                        .filter(|(candidate, _)| candidate == &key);
                    let body = matches
                        .next()
                        .ok_or("default report has no runtime expression")?
                        .1
                        .clone();
                    if matches.next().is_some() {
                        return Err("default report has ambiguous runtime expressions".into());
                    }
                    let mut child = scope.clone();
                    child.body = body;
                    child.defaults.clear();
                    child.preconditions.clear();
                    child.postconditions.clear();
                    let mut child_context = context.clone();
                    child_context.push(key);
                    scopes.push_back((child_context, child, default.evaluation, vec![]));
                }
                let mut sites: Vec<Site> = vec![];
                let mut discards: Vec<DiscardCleanup> = vec![];
                for mut site in report.cleanup_sites {
                    let mut boundary = match site.reason {
                        CleanupReason::EffectEscape => Boundary::EffectEscape,
                        CleanupReason::EffectReturn => Boundary::EffectReturn,
                        CleanupReason::EffectBreak(id) => Boundary::EffectBreak(id),
                        CleanupReason::EffectContinue(id) => Boundary::EffectContinue(id),
                        CleanupReason::HandlerEnd => Boundary::HandlerEnd,
                        CleanupReason::EffectResume => Boundary::EffectResume,
                        CleanupReason::ScopeEnd => Boundary::ScopeEnd,
                        CleanupReason::FunctionEnd => Boundary::FunctionEnd,
                        CleanupReason::Return => Boundary::Return,
                        CleanupReason::Break => Boundary::Break,
                        CleanupReason::Continue => Boundary::Continue,
                        CleanupReason::Replacement => Boundary::Replacement,
                        CleanupReason::PredicateFailure => Boundary::PredicateFailure,
                        CleanupReason::TemporaryEnd => Boundary::TemporaryEnd,
                    };
                    if site.interface_precondition {
                        boundary = Boundary::InterfacePreconditionFailure;
                    }
                    if site.interface_postcondition {
                        boundary = Boundary::InterfacePostconditionFailure;
                    }
                    if let Some(origin) = site.result_origin {
                        boundary = Boundary::PostconditionFailure(origin);
                    }
                    if let Some((construction, parameter)) = site.default_origin {
                        site.source = construction;
                        boundary = Boundary::DefaultFailure(parameter);
                    }
                    let mut actions = vec![];
                    for action in site.actions {
                        let CleanupAction::Planned { candidate, plan } = action else {
                            return Err("unplanned resource candidate".into());
                        };
                        let candidate = match *candidate {
                            CleanupAction::Alternatives(mut candidates) => {
                                let Some(CleanupAction::Temporary {
                                    identity,
                                    temporary,
                                    presence,
                                    ..
                                }) = candidates.first()
                                else {
                                    return Err(
                                        "residual alternatives need one retained temporary target"
                                            .into(),
                                    );
                                };
                                for candidate in &candidates {
                                    *fuel = fuel
                                        .checked_sub(1)
                                        .ok_or("residual target validation budget exhausted")?;
                                    if !matches!(candidate, CleanupAction::Temporary { identity: other_id, temporary: other, presence: other_presence, .. }
                                        if identity == other_id && temporary == other && presence == other_presence)
                                    {
                                        return Err(
                                            "residual alternatives change cleanup storage".into()
                                        );
                                    }
                                }
                                candidates.remove(0)
                            }
                            candidate => candidate,
                        };
                        let (ty,target) = match candidate {
                        CleanupAction::Owned {ty,remaining} if report.callee_temporaries.iter().any(|(root,_)| *root==remaining.place.root) => {
                            let mut origins=report.callee_temporaries.iter().filter(|(root,_)|*root==remaining.place.root);
                            let (_,origin)=origins.next().unwrap();
                            if origins.next().is_some() || !remaining.place.fields.is_empty() || callee_hooks.iter().filter(|source|*source==origin).count()!=1 {
                                return Err("temporary callee has no unique runtime storage".into());
                            }
                            (ty,Target::Temporary {source:*origin,optional:false})
                        },
                        CleanupAction::Owned {ty,remaining} => (ty,Target::Local {
                            binding:nir::program::LocalId {
                                owner:u32::try_from(remaining.place.root.owner).map_err(|_|"cleanup local identity exceeds runtime representation")?,
                                index:remaining.place.root.local,
                            },fields:remaining.place.fields,
                        }),
                        CleanupAction::Prepared {construction,value}=>{
                            if prepared_hooks.iter().filter(|(source,slot,expr)|*source==construction && *slot==value.slot && *expr==value.source).count()!=1 {
                                return Err("prepared cleanup has no unique runtime slot".into());
                            }
                            (value.ty,Target::Prepared {construction,slot:value.slot,value_source:value.source})
                        },
                        CleanupAction::Temporary {temporary,presence,..}
                            if site.reason==CleanupReason::TemporaryEnd
                                && presence==elaboration::verification::ownership::Availability::Present
                                && temporary.fields.is_empty()
                                && temporary.scope.boundary==elaboration::verification::moves::TemporaryBoundary::FullExpression
                                && temporary.scope.source==site.source
                                && discard_hooks.iter().filter(|(source,value)|*source==site.source && *value==temporary.source).count()==1 => {
                            if discards.iter().any(|d|d.source==site.source) {
                                return Err("multiple temporary owners cannot map to one discarded result".into());
                            }
                            let mut methods=vec![];
                            let steps=lower(&plan.steps,&plan.receiver,&mut methods,&mut receivers,fuel,0)?;
                            for method in methods {
                                schedule_deps
                                    .entry(instance.key.clone())
                                    .or_default()
                                    .insert(method.clone());
                                if !roots.contains(&method) {
                                    roots.push(method);
                                }
                            }
                            if !type_already_in_graph(graph, &temporary.ty, 0)
                                && !types.contains(&temporary.ty)
                            {
                                types.push(temporary.ty.clone());
                            }
                            discards.push(DiscardCleanup {source:site.source,value_source:temporary.source,
                                program:iir::cleanup::Program {ty:temporary.ty,guards:plan.variants.iter().map(|v|(v.fields.clone(),v.index)).collect(),steps}});
                            continue;
                        },
                        CleanupAction::Temporary {temporary,presence,..}
                            if temporary.fields.is_empty()
                                && projection_hooks.iter().filter(|source| **source==temporary.source).count()==1 => {
                            (temporary.ty,Target::Temporary {source:temporary.source,optional:presence!=elaboration::verification::ownership::Availability::Present})
                        },
                        other => {
                            let ty = match &other {
                                CleanupAction::Temporary { temporary, .. } => &temporary.ty,
                                CleanupAction::Prepared { value, .. } => &value.ty,
                                _ => return Err("nested cleanup wrapper".into()),
                            };
                            // Plain scalar/pointer representations have no runtime
                            // teardown when the captured plan contains only releases.
                            if (nir::program::intrinsic_pointer_value(ty)
                                || (text_provenance
                                    == elaboration::verification::memory::TextProvenance::VerifiedByRegionAnalysis
                                    && *ty == nir::Term::Primitive(nir::Primitive::Str)))
                                && plan
                                    .steps
                                    .iter()
                                    .all(|s| matches!(s, CleanupStep::Release { .. }))
                            {
                                continue;
                            }
                            return Err(format!("temporary/prepared ownership needs runtime cleanup storage mapping: {other:?}"));
                        }
                    };
                        if hooks
                            .iter()
                            .filter(|(source, kind)| *source == site.source && *kind == boundary)
                            .count()
                            != 1
                        {
                            return Err(format!(
                                "cleanup site has no unique runtime boundary in {} at {}..{} ({boundary:?}, {} matches)",
                                function.name,
                                site.source.start,
                                site.source.end,
                                hooks
                                    .iter()
                                    .filter(|(source, kind)| *source == site.source
                                        && *kind == boundary)
                                    .count()
                            ));
                        }
                        let mut methods = vec![];
                        let steps = lower(
                            &plan.steps,
                            &plan.receiver,
                            &mut methods,
                            &mut receivers,
                            fuel,
                            0,
                        )?;
                        for method in methods {
                            schedule_deps
                                .entry(instance.key.clone())
                                .or_default()
                                .insert(method.clone());
                            if !roots.contains(&method) {
                                roots.push(method);
                            }
                        }
                        if !type_already_in_graph(graph, &ty, 0) && !types.contains(&ty) {
                            types.push(ty.clone());
                        }
                        actions.push(Action {
                            target,
                            program: iir::cleanup::Program {
                                ty,
                                guards: plan
                                    .variants
                                    .iter()
                                    .map(|v| (v.fields.clone(), v.index))
                                    .collect(),
                                steps,
                            },
                        });
                    }
                    if boundary == Boundary::InterfacePostconditionFailure
                        && hooks
                            .iter()
                            .filter(|(source, kind)| *source == site.source && *kind == boundary)
                            .count()
                            != 1
                    {
                        return Err(
                            "interface result cleanup lacks a unique runtime boundary".into()
                        );
                    }
                    if actions.is_empty() && boundary != Boundary::InterfacePostconditionFailure {
                        continue;
                    }
                    if let Some(existing) = sites
                        .iter_mut()
                        .find(|s| s.source == site.source && s.boundary == boundary)
                    {
                        // Distinct failing exits of one invocation can leave
                        // the exact same ordered cleanup at its caller boundary.
                        // Publish it once; differing programs or order still
                        // require explicit path discrimination.
                        let failure_alternatives = matches!(
                            boundary,
                            Boundary::DefaultFailure(_)
                                | Boundary::InterfacePreconditionFailure
                                | Boundary::InterfacePostconditionFailure
                        );
                        if (boundary == Boundary::PredicateFailure || failure_alternatives)
                            && existing.actions.len() == actions.len()
                            && existing
                                .actions
                                .iter()
                                .zip(&actions)
                                .all(|(a, b)| a.target == b.target && a.program == b.program)
                        {
                            continue;
                        }
                        if failure_alternatives {
                            return Err(format!(
                                "failure cleanup differs across alternatives in {} at {} ({boundary})",
                                scope.name, site.source
                            ));
                        }
                        if actions.iter().any(|a| {
                            existing
                                .actions
                                .iter()
                                .any(|b| match (&a.target, &b.target) {
                                    (
                                        Target::Temporary { source: a, .. },
                                        Target::Temporary { source: b, .. },
                                    ) => a == b,
                                    _ => a.target == b.target,
                                })
                        }) {
                            return Err(format!(
                                "ambiguous repeated cleanup owner at one runtime boundary: function {} at {:?} {:?}, context {:?}",
                                scope.name, site.source, boundary, context
                            ));
                        }
                        existing.actions.extend(actions);
                    } else {
                        sites.push(Site {
                            source: site.source,
                            boundary,
                            actions,
                        });
                    }
                }
                let mut transfers = vec![];
                let mut seen_transfers = std::collections::BTreeSet::new();
                for (source, place) in report.transfers {
                    *fuel = fuel
                        .checked_sub(place.fields.len().saturating_add(1))
                        .ok_or("cleanup transfer publication budget exhausted")?;
                    let transfer = iir::cleanup::Transfer {
                        source,
                        binding: nir::program::LocalId {
                            owner: u32::try_from(place.root.owner).map_err(
                                |_| "transfer local identity exceeds runtime representation",
                            )?,
                            index: place.root.local,
                        },
                        fields: place.fields,
                    };
                    if seen_transfers.insert((
                        source.start,
                        source.end,
                        transfer.binding,
                        transfer.fields.clone(),
                    )) {
                        transfers.push(transfer);
                    }
                }
                let mut copies = vec![];
                let mut seen_copies = std::collections::BTreeSet::new();
                for (source, place) in report.copies {
                    *fuel = fuel
                        .checked_sub(place.fields.len().saturating_add(1))
                        .ok_or("copy delivery publication budget exhausted")?;
                    let copy = iir::cleanup::Transfer {
                        source,
                        binding: nir::program::LocalId {
                            owner: u32::try_from(place.root.owner).map_err(
                                |_| "copy local identity exceeds runtime representation",
                            )?,
                            index: place.root.local,
                        },
                        fields: place.fields,
                    };
                    if seen_copies.insert((
                        source.start,
                        source.end,
                        copy.binding,
                        copy.fields.clone(),
                    )) {
                        copies.push(copy);
                    }
                }
                let mut pattern_transfers = vec![];
                let mut seen_patterns = std::collections::BTreeSet::new();
                for (source, binding, fields) in report.pattern_transfers {
                    *fuel = fuel
                        .checked_sub(fields.len().saturating_add(1))
                        .ok_or("pattern transfer publication budget exhausted")?;
                    if seen_patterns.insert((source.start, source.end, binding, fields.clone())) {
                        pattern_transfers.push(iir::cleanup::Transfer {
                            source,
                            binding,
                            fields,
                        });
                    }
                }
                let mut pattern_orders = vec![];
                for (source, alternatives) in report.pattern_orders {
                    for order in &alternatives {
                        *fuel = fuel
                            .checked_sub(order.len().saturating_add(1))
                            .ok_or("pattern order publication budget exhausted")?;
                    }
                    let order = iir::cleanup::BindingOrder {
                        source,
                        alternatives,
                    };
                    if !pattern_orders.contains(&order) {
                        pattern_orders.push(order);
                    }
                }
                schedule.functions.push(FunctionSchedule {
                    instance: instance.key.clone(),
                    context,
                    sites,
                    discards,
                    transfers,
                    copies,
                    pattern_transfers,
                    pattern_orders,
                });
            }
            scheduled_roots.insert(instance.key.clone());
        }
        if deferred_nested {
            // Every caller must be rechecked against the grown transitive footprint.
            // No schedule from a provisional body-only round is published.
            drop(cleanup_session);
            retained_caches = verification_session.into_caches();
            retained_caches.retain_unaffected(&changed_cleanup_keys);
            schedule = Schedule::default();
            scheduled_roots.clear();
            schedule_deps.clear();
            continue;
        }
        if cleanup_changed {
            // Every caller must be rechecked against the grown transitive footprint.
            // Retain unaffected schedules across fixpoint rounds.
            drop(cleanup_session);
            retained_caches = verification_session.into_caches();
            retained_caches.retain_unaffected(&changed_cleanup_keys);
            let affected = affected_cleanup_instances(
                &changed_cleanup_keys,
                &instance_deps,
                &schedule_deps,
                &schedule.borrowed_callable_functions,
            );
            schedule
                .functions
                .retain(|item| !affected.contains(&item.instance));
            schedule
                .closure_definitions
                .retain(|item| !affected.contains(&item.instance));
            schedule
                .aliases
                .retain(|item| !affected.contains(&item.instance));
            schedule
                .borrowed_callable_functions
                .retain(|key| !affected.contains(key));
            scheduled_roots.retain(|key| !affected.contains(key));
            schedule_deps.retain(|key, _| !affected.contains(key));
            continue;
        }
        if deferred {
            drop(cleanup_session);
            retained_caches = verification_session.into_caches();
            retained_caches.retain_unaffected(&changed_cleanup_keys);
            continue;
        }
        if previous == (roots.len(), types.len()) {
            // Contract proofs use the final runtime-instance and cleanup-access
            // overlay. Provisional rounds are never published, and repeating
            // these proofs while dependencies grow provides no reusable evidence.
            // Include higher-order bodies even if they have no root schedule.
            if validate_contracts {
                for instance in &graph.functions {
                    let mut contract_limits = elaboration::verification::Limits::default();
                    contract_limits.arithmetic.steps = *fuel;
                    let checked = verification_session
                        .validate_residual_contracts(&instance.function, &mut contract_limits);
                    *fuel = contract_limits.arithmetic.steps;
                    checked.map_err(|mut error| {
                        error.source = error
                            .source
                            .original(&graph.expansion_sources)
                            .unwrap_or(error.source);
                        format!(
                            "residual program contract: {}: {error}",
                            instance.function.name
                        )
                    })?;
                }
            }
            let order: std::collections::HashMap<iir::InstanceKey, usize> = graph
                .functions
                .iter()
                .enumerate()
                .map(|(index, instance)| (instance.key.clone(), index))
                .collect();
            schedule
                .functions
                .sort_by_key(|item| order.get(&item.instance).copied().unwrap_or(usize::MAX));
            drop(cleanup_session);
            drop(verification_session);
            let (_, _, graph) = cached_graph.take().unwrap();
            return Ok(Executable {
                entry,
                graph,
                schedule,
            });
        }
        drop(cleanup_session);
        retained_caches = verification_session.into_caches();
        retained_caches.retain_unaffected(&changed_cleanup_keys);
    }
}

#[cfg(test)]
mod tests {
    use super::{InstanceDeps, affected_cleanup_instances};
    use nir::DeclarationId;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn affected_cleanup_instances_evicts_transitive_dynamic_drop_and_borrowed_keeps_disjoint() {
        let key = |local| iir::InstanceKey {
            declaration: DeclarationId { package: 1, local },
            arguments: vec![].into(),
        };
        let leaf_changed = key(1);
        let direct_caller = key(2);
        let transitive_caller = key(3);
        let drop_caller = key(4);
        let dynamic_fn = key(5);
        let borrowed_fn = key(6);
        let disjoint_fn = key(7);

        let mut instance_deps = HashMap::new();
        instance_deps.insert(leaf_changed.clone(), InstanceDeps::default());
        instance_deps.insert(
            direct_caller.clone(),
            InstanceDeps {
                callees: HashSet::from([leaf_changed.clone()]),
                dynamic_or_unknown: false,
            },
        );
        instance_deps.insert(
            transitive_caller.clone(),
            InstanceDeps {
                callees: HashSet::from([direct_caller.clone()]),
                dynamic_or_unknown: false,
            },
        );
        instance_deps.insert(drop_caller.clone(), InstanceDeps::default());
        instance_deps.insert(
            dynamic_fn.clone(),
            InstanceDeps {
                callees: HashSet::new(),
                dynamic_or_unknown: true,
            },
        );
        instance_deps.insert(disjoint_fn.clone(), InstanceDeps::default());

        let schedule_deps =
            HashMap::from([(drop_caller.clone(), HashSet::from([leaf_changed.clone()]))]);
        let borrowed_callables = vec![borrowed_fn.clone()];
        let changed = HashSet::from([leaf_changed.clone()]);

        let affected = affected_cleanup_instances(
            &changed,
            &instance_deps,
            &schedule_deps,
            &borrowed_callables,
        );

        assert!(affected.contains(&leaf_changed));
        assert!(affected.contains(&direct_caller));
        assert!(affected.contains(&transitive_caller));
        assert!(affected.contains(&drop_caller));
        assert!(affected.contains(&dynamic_fn));
        assert!(affected.contains(&borrowed_fn));
        assert!(!affected.contains(&disjoint_fn));
    }

    #[test]
    fn type_already_in_graph_recognizes_primitives_and_materialized_nominals_conservatively() {
        let mut graph = iir::Package::default();
        let present_nominal = nir::Term::Nominal {
            declaration: DeclarationId {
                package: 1,
                local: 10,
            },
            arguments: vec![],
        };
        let missing_nominal = nir::Term::Nominal {
            declaration: DeclarationId {
                package: 1,
                local: 11,
            },
            arguments: vec![],
        };
        graph.nominals.push(iir::NominalInstance {
            ty: present_nominal.clone(),
            definition: iir::NominalDefinition::Struct(nir::program::Struct {
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                declaration: DeclarationId {
                    package: 1,
                    local: 10,
                },
                name: "Present".into(),
                fields: vec![],
                packed: false,
            }),
        });

        assert!(super::type_already_in_graph(
            &graph,
            &nir::Term::Primitive(nir::Primitive::U32),
            0
        ));
        assert!(super::type_already_in_graph(&graph, &present_nominal, 0));
        assert!(super::type_already_in_graph(
            &graph,
            &nir::Term::Optional(Box::new(present_nominal.clone())),
            0
        ));
        assert!(super::type_already_in_graph(
            &graph,
            &nir::Term::ErrorResult {
                errors: vec![present_nominal.clone()],
                result: Box::new(nir::Term::Primitive(nir::Primitive::U64)),
            },
            0
        ));
        assert!(super::type_already_in_graph(
            &graph,
            &nir::Term::Tuple(vec![
                present_nominal.clone(),
                nir::Term::Pointer(Box::new(nir::Term::Primitive(nir::Primitive::U8))),
            ]),
            0
        ));
        assert!(!super::type_already_in_graph(&graph, &missing_nominal, 0));
        assert!(!super::type_already_in_graph(
            &graph,
            &nir::Term::Optional(Box::new(missing_nominal)),
            0
        ));
        assert!(!super::type_already_in_graph(
            &graph,
            &nir::Term::NormalForm {
                head: DeclarationId {
                    package: 1,
                    local: 12,
                },
                sort: nir::Sort::Type,
                arguments: vec![],
            },
            0
        ));
    }
}
