//! Driver-side language-item consumers. Bindings come from trusted source
//! identities, not spelling. Drop selection fixes method/contract identity but
//! does not authorize cleanup. Copy classification is not permission to duplicate
//! resource responsibilities; the resource pass must establish that separately.
use elaboration::{capabilities::World, ptc::Budget, verification::copy};
use intrinsic::{LangItem, LangItemDef, LangItems};

mod source;
pub mod splice;
pub mod type_constructors;

pub fn classify_copy(
    world: &World<'_>,
    items: &LangItems,
    ty: &nir::Term,
    budget: Budget,
) -> Result<copy::Classification, String> {
    let Some(LangItemDef::UserDef(protocol)) = items.get(LangItem::Copy) else {
        return Err("Copy language item is not bound to a source trait".into());
    };
    // The query revalidates the declaration against this world: a registry from
    // a different compilation cannot turn a missing trait into an assumption.
    copy::classify(world, protocol, ty, budget)
}

/// Select representation delivery only after checking explicit capability and
/// every represented cleanup responsibility. Pointer target access is separate.
pub(crate) fn copy_delivery(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    ty: &nir::Term,
    budget: Budget,
) -> Result<elaboration::verification::moves::Delivery, String> {
    use elaboration::verification::moves::Delivery;
    match classify_copy(world, items, ty, budget)? {
        copy::Classification::Intrinsic => Ok(Delivery::Copy),
        copy::Classification::Affine => Ok(Delivery::Move),
        copy::Classification::Pending(reason) => Err(reason),
        copy::Classification::Declared { .. } | copy::Classification::Assumed(_) => {
            let Some(LangItemDef::UserDef(copy_protocol)) = items.get(LangItem::Copy) else {
                return Err("Copy language item is not bound to a source trait".into());
            };
            let Some(LangItemDef::UserDef(drop_protocol)) = items.get(LangItem::Drop) else {
                return Err(
                    "Copy representation and resource legality require a bound Drop protocol"
                        .into(),
                );
            };
            let mut fuel = budget.steps;
            copy::acquire(
                package,
                world,
                copy_protocol,
                drop_protocol,
                ty,
                budget,
                &mut fuel,
            )
            .map_err(|reason| format!("Copy representation and resource legality: {reason}"))?;
            Ok(Delivery::Copy)
        }
    }
}

pub(crate) fn copy_delivery_cached(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    ty: &nir::Term,
    budget: Budget,
    cache: &mut std::collections::BTreeMap<
        elaboration::verification::memory::OrderedTerm,
        elaboration::verification::moves::Delivery,
    >,
) -> Result<elaboration::verification::moves::Delivery, String> {
    let key = elaboration::verification::memory::OrderedTerm(ty.clone());
    if let Some(delivery) = cache.get(&key) {
        return Ok(delivery.clone());
    }
    let delivery = copy_delivery(package, world, items, ty, budget)?;
    cache.insert(key, delivery.clone());
    Ok(delivery)
}

fn convert_text_callbacks(
    callables: &[elaboration::verification::moves::CallableParameter],
) -> Option<(
    Vec<elaboration::verification::memory::TextCallbackInput>,
    Vec<iir::InstanceKey>,
)> {
    use elaboration::verification::memory::TextCallbackInput;
    use nir::program::ExprKind as K;

    let expr_key = |expr: &nir::program::Expr| -> Option<iir::InstanceKey> {
        match &expr.kind {
            K::Function(declaration) => Some(iir::InstanceKey {
                declaration: *declaration,
                arguments: vec![].into(),
            }),
            K::FunctionInstance {
                declaration,
                arguments,
            } => Some(iir::InstanceKey {
                declaration: *declaration,
                arguments: arguments.clone().into(),
            }),
            _ => None,
        }
    };

    let mut converted = Vec::with_capacity(callables.len());
    let mut instances = Vec::new();
    for callable in callables {
        if !callable.captures.is_empty() || !callable.enum_captures.is_empty() {
            return None;
        }
        let mut targets = Vec::new();
        if callable.choices.is_empty() {
            targets.push(expr_key(&callable.definition)?);
            for alt in &callable.alternatives {
                targets.push(expr_key(alt)?);
            }
        } else {
            for choice in &callable.choices {
                if !choice.captures.is_empty()
                    || !choice.enum_captures.is_empty()
                    || !choice.choices.is_empty()
                {
                    return None;
                }
                targets.push(expr_key(&choice.definition)?);
                for alt in &choice.alternatives {
                    targets.push(expr_key(alt)?);
                }
            }
        }
        if targets.is_empty() {
            return None;
        }
        instances.extend(targets.iter().cloned());
        converted.push(TextCallbackInput::Owned {
            parameter: callable.parameter,
            fields: callable.fields.clone(),
            targets,
        });
    }
    Some((converted, instances))
}

fn verify_text_provenance_for_session<'a>(
    session: &mut elaboration::verification::PackageSession<'a>,
    declaration: nir::DeclarationId,
    arguments: &[nir::Term],
    extra_roots: &[(&nir::program::Function, &[nir::Term])],
    callables: &[elaboration::verification::moves::CallableParameter],
    has_unmodeled_context: bool,
    fuel: &mut usize,
) -> Result<elaboration::verification::memory::TextProvenance, String> {
    use elaboration::verification::memory::{
        TextProvenance, infer_text_regions_with_callbacks, reachable_contains_str,
    };
    let package = session.package();
    if has_unmodeled_context {
        return Ok(TextProvenance::Unchecked);
    }
    let Some((function, body_arguments)) = package.runtime_body(declaration, arguments) else {
        return Ok(TextProvenance::Unchecked);
    };
    let Some((text_callbacks, extra_instances)) = convert_text_callbacks(callables) else {
        return Ok(TextProvenance::Unchecked);
    };
    let mut roots = Vec::with_capacity(1 + extra_roots.len());
    roots.push((function, body_arguments));
    roots.extend_from_slice(extra_roots);
    if !reachable_contains_str(package, &roots, arguments, &extra_instances, fuel)? {
        return Ok(TextProvenance::Unchecked);
    }
    if text_callbacks.is_empty() {
        session.infer_text_regions(declaration, arguments, fuel)?;
    } else {
        infer_text_regions_with_callbacks(package, declaration, arguments, &text_callbacks, fuel)?;
    }
    Ok(TextProvenance::VerifiedByRegionAnalysis)
}

/// Analyze moves using bound protocol identity. Explicit or assumed Copy
/// capabilities still require representation/resource evidence before this
/// driver can authorize copying them. No optimistic fallback to Move is made.
pub fn analyze_moves(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    function: &nir::program::Function,
    fuel: usize,
    query_budget: Budget,
) -> Result<elaboration::verification::moves::Report, elaboration::verification::moves::Error> {
    use elaboration::verification::moves;
    moves::analyze_in_package(
        package,
        function,
        |ty| copy_delivery(package, world, items, ty, query_budget),
        fuel,
    )
}

/// Select a concrete destructor and preserve its contracts. This does not
/// authorize cleanup or acquire the runtime environment needed by that cleanup.
pub fn select_drop(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    ty: &nir::Term,
    budget: Budget,
) -> Result<elaboration::verification::drop::Resolution, String> {
    let Some(LangItemDef::UserDef(protocol)) = items.get(LangItem::Drop) else {
        return Err("Drop language item is not bound to a source trait".into());
    };
    elaboration::verification::drop::select(package, world, protocol, ty, budget)
}

/// Check the previously captured destructor's supported memory footprint at a
/// cleanup candidate. No implementation reselection or destructor invocation
/// takes place; environment and program obligations remain separate.
pub fn check_drop_access(
    package: &nir::program::Package,
    world: &World<'_>,
    selected: &elaboration::verification::drop::SelectedDrop,
    state: &elaboration::verification::ownership::State,
    receiver: &elaboration::verification::ownership::Place,
    fuel: &mut usize,
) -> Result<elaboration::verification::drop::AccessCheck, String> {
    elaboration::verification::drop::check_access(package, world, selected, state, receiver, fuel)
}

/// Capture fixed-layout destruction choices using the bound language-item
/// identity. This subset rejects selections needing runtime environments.
pub fn acquire_cleanup(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    ty: &nir::Term,
    budget: Budget,
    fuel: &mut usize,
) -> Result<elaboration::verification::drop::AcquiredCleanup, String> {
    let Some(LangItemDef::UserDef(protocol)) = items.get(LangItem::Drop) else {
        return Err("Drop language item is not bound to a source trait".into());
    };
    elaboration::verification::drop::acquire_cleanup(package, world, protocol, ty, budget, fuel)
}

/// Capture Drop choices while ownership responsibilities are acquired, then
/// validate each cleanup candidate against its exact current state. Uses one
/// immutable world; runtime environment capture and cleanup execution are not
/// supplied by this entrypoint.
pub fn analyze_cleanup(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    function: &nir::program::Function,
    fuel: usize,
    query_budget: Budget,
) -> Result<elaboration::verification::moves::Report, elaboration::verification::moves::Error> {
    analyze_cleanup_with_borrows(package, world, items, function, fuel, query_budget, &[])
}

pub(crate) fn analyze_cleanup_with_borrows(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    function: &nir::program::Function,
    fuel: usize,
    query_budget: Budget,
    borrows: &[(usize, nir::Term)],
) -> Result<elaboration::verification::moves::Report, elaboration::verification::moves::Error> {
    analyze_cleanup_with_inputs(
        package,
        world,
        items,
        function,
        fuel,
        query_budget,
        borrows,
        &[],
    )
}

/// Acquire cleanup plans conditional on the recorded concrete callable inputs.
/// Invocation publication must discharge these premises against prepared values.
pub fn analyze_cleanup_with_inputs(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    function: &nir::program::Function,
    fuel: usize,
    query_budget: Budget,
    borrows: &[(usize, nir::Term)],
    callables: &[elaboration::verification::moves::CallableParameter],
) -> Result<elaboration::verification::moves::Report, elaboration::verification::moves::Error> {
    analyze_cleanup_with_context(
        package,
        world,
        items,
        function,
        fuel,
        query_budget,
        borrows,
        callables,
        &[],
    )
}

/// Acquire cleanup under code identity and enum tag premises of one invocation.
pub fn analyze_cleanup_with_context(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    function: &nir::program::Function,
    fuel: usize,
    query_budget: Budget,
    borrows: &[(usize, nir::Term)],
    callables: &[elaboration::verification::moves::CallableParameter],
    enums: &[elaboration::verification::moves::EnumParameter],
) -> Result<elaboration::verification::moves::Report, elaboration::verification::moves::Error> {
    analyze_cleanup_with_effect_context(
        package,
        world,
        items,
        function,
        fuel,
        query_budget,
        borrows,
        callables,
        enums,
        vec![],
    )
}

pub fn analyze_cleanup_with_effect_context(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    function: &nir::program::Function,
    fuel: usize,
    query_budget: Budget,
    borrows: &[(usize, nir::Term)],
    callables: &[elaboration::verification::moves::CallableParameter],
    enums: &[elaboration::verification::moves::EnumParameter],
    effects: Vec<elaboration::verification::moves::EffectResultInput>,
) -> Result<elaboration::verification::moves::Report, elaboration::verification::moves::Error> {
    analyze_cleanup_with_field_context(
        package,
        world,
        items,
        function,
        fuel,
        query_budget,
        borrows,
        callables,
        enums,
        effects,
        &[],
    )
}

pub fn analyze_cleanup_with_field_context(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    function: &nir::program::Function,
    fuel: usize,
    query_budget: Budget,
    borrows: &[(usize, nir::Term)],
    callables: &[elaboration::verification::moves::CallableParameter],
    enums: &[elaboration::verification::moves::EnumParameter],
    effects: Vec<elaboration::verification::moves::EffectResultInput>,
    fields: &[elaboration::verification::moves::BorrowedField],
) -> Result<elaboration::verification::moves::Report, elaboration::verification::moves::Error> {
    analyze_cleanup_with_borrowed_code_context(
        package,
        world,
        items,
        function,
        fuel,
        query_budget,
        borrows,
        callables,
        enums,
        effects,
        fields,
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
fn analyze_cleanup_with_session_provenance(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    function: &nir::program::Function,
    text_region_target: Option<(nir::DeclarationId, &[nir::Term])>,
    mut fuel: usize,
    query_budget: Budget,
    borrows: &[(usize, nir::Term)],
    callables: &[elaboration::verification::moves::CallableParameter],
    enums: &[elaboration::verification::moves::EnumParameter],
    effects: Vec<elaboration::verification::moves::EffectResultInput>,
    fields: &[elaboration::verification::moves::BorrowedField],
    borrowed: &[elaboration::verification::moves::BorrowedCode],
) -> Result<elaboration::verification::moves::Report, elaboration::verification::moves::Error> {
    use elaboration::verification::{PackageSession, moves};
    let Some(LangItemDef::UserDef(protocol)) = items.get(LangItem::Drop) else {
        return Err(moves::Error {
            source: function.body.source,
            reason: "Drop language item is not bound to a source trait".into(),
        });
    };
    let mut verification_session = PackageSession::new(package);
    let has_unmodeled_context = !borrows.is_empty()
        || !enums.is_empty()
        || !effects.is_empty()
        || !fields.is_empty()
        || !borrowed.is_empty();
    let text_provenance = if let Some((declaration, arguments)) = text_region_target {
        verify_text_provenance_for_session(
            &mut verification_session,
            declaration,
            arguments,
            &[(function, &[])],
            callables,
            has_unmodeled_context,
            &mut fuel,
        )
        .map_err(|reason| moves::Error {
            source: function.body.source,
            reason,
        })?
    } else {
        elaboration::verification::memory::TextProvenance::Unchecked
    };
    let mut cleanup_session = verification_session
        .cleanup_session(world, protocol, query_budget)
        .with_text_provenance(text_provenance);
    moves::analyze_in_package_with_session(
        &mut verification_session,
        function,
        |ty| copy_delivery(package, world, items, ty, query_budget),
        fuel,
        Some(&mut cleanup_session),
        borrows,
        callables,
        enums,
        effects,
        fields,
        borrowed,
        text_provenance,
    )
}

pub fn analyze_cleanup_with_borrowed_code_context(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    function: &nir::program::Function,
    fuel: usize,
    query_budget: Budget,
    borrows: &[(usize, nir::Term)],
    callables: &[elaboration::verification::moves::CallableParameter],
    enums: &[elaboration::verification::moves::EnumParameter],
    effects: Vec<elaboration::verification::moves::EffectResultInput>,
    fields: &[elaboration::verification::moves::BorrowedField],
    borrowed: &[elaboration::verification::moves::BorrowedCode],
) -> Result<elaboration::verification::moves::Report, elaboration::verification::moves::Error> {
    let text_region_target = function
        .type_parameters
        .is_empty()
        .then_some((function.declaration, &[][..]));
    analyze_cleanup_with_session_provenance(
        package,
        world,
        items,
        function,
        text_region_target,
        fuel,
        query_budget,
        borrows,
        callables,
        enums,
        effects,
        fields,
        borrowed,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn analyze_cleanup_in_session<'a>(
    session: &mut elaboration::verification::PackageSession<'a>,
    cleanup: Option<&mut elaboration::verification::drop::CleanupSession<'a, '_>>,
    copy_cache: &mut std::collections::BTreeMap<
        elaboration::verification::memory::OrderedTerm,
        elaboration::verification::moves::Delivery,
    >,
    world: &World<'_>,
    items: &LangItems,
    function: &nir::program::Function,
    fuel: usize,
    query_budget: Budget,
    borrows: &[(usize, nir::Term)],
    callables: &[elaboration::verification::moves::CallableParameter],
    enums: &[elaboration::verification::moves::EnumParameter],
    effects: Vec<elaboration::verification::moves::EffectResultInput>,
    fields: &[elaboration::verification::moves::BorrowedField],
    borrowed: &[elaboration::verification::moves::BorrowedCode],
    text_provenance: elaboration::verification::memory::TextProvenance,
) -> (
    Result<elaboration::verification::moves::Report, elaboration::verification::moves::Error>,
    Vec<elaboration::verification::drop::RequiredDropInstance>,
) {
    use elaboration::verification::moves;
    let Some(cleanup) = cleanup else {
        return (
            Err(moves::Error {
                source: function.body.source,
                reason: "Drop language item is not bound to a source trait".into(),
            }),
            vec![],
        );
    };
    let package = session.package();
    let result = moves::analyze_in_package_with_session(
        session,
        function,
        |ty| copy_delivery_cached(package, world, items, ty, query_budget, copy_cache),
        fuel,
        Some(&mut *cleanup),
        borrows,
        callables,
        enums,
        effects,
        fields,
        borrowed,
        text_provenance,
    );
    (result, cleanup.take_missing_runtime_instances())
}

/// Resolve the comptime phase using trusted language-item identity. Returned
/// program/capability obligations still need discharge before invocation.
pub fn select_comptime_key(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    ty: &nir::Term,
    operation: elaboration::key_protocols::Operation,
    budget: Budget,
) -> Result<elaboration::key_protocols::Selected, String> {
    select_comptime_key_metered(package, world, items, ty, operation, &mut { budget })
}

pub fn select_comptime_key_metered(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    ty: &nir::Term,
    operation: elaboration::key_protocols::Operation,
    budget: &mut Budget,
) -> Result<elaboration::key_protocols::Selected, String> {
    let item = match operation {
        elaboration::key_protocols::Operation::Eq => LangItem::Eq,
        elaboration::key_protocols::Operation::Hash => LangItem::Hash,
    };
    let Some(LangItemDef::UserDef(protocol)) = items.get(item) else {
        return Err(format!(
            "{operation} language item is not bound to a source trait"
        ));
    };
    elaboration::key_protocols::select_metered(package, world, protocol, ty, operation, budget)
}

/// Recheck a conditional memory footprint in the same Drop world as cleanup.
pub fn infer_borrowed_callable_memory(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    declaration: nir::DeclarationId,
    arguments: &[nir::Term],
    callables: &[elaboration::verification::moves::CallableParameter],
    enums: &[elaboration::verification::moves::EnumParameter],
    effects: &[elaboration::verification::moves::EffectResultInput],
    borrowed: &[elaboration::verification::moves::BorrowedCode],
    fuel: &mut usize,
    budget: Budget,
    source: nir::program::SourceRange,
) -> Result<elaboration::verification::memory::Summary, String> {
    use elaboration::verification::{PackageSession, moves};
    let Some(LangItemDef::UserDef(protocol)) = items.get(LangItem::Drop) else {
        return Err("Drop language item is not bound to a source trait".into());
    };
    let mut verification_session = PackageSession::new(package);
    let has_unmodeled_context = !enums.is_empty() || !effects.is_empty() || !borrowed.is_empty();
    let text_provenance = verify_text_provenance_for_session(
        &mut verification_session,
        declaration,
        arguments,
        &[],
        callables,
        has_unmodeled_context,
        fuel,
    )?;
    let mut session = verification_session
        .cleanup_session(world, protocol, budget)
        .with_text_provenance(text_provenance);
    moves::infer_callable_memory_with_session(
        &mut verification_session,
        declaration,
        arguments,
        callables,
        enums,
        effects,
        borrowed,
        Some(&mut session),
        text_provenance,
        fuel,
        source,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_borrowed_callable_memory_in_session<'a>(
    session: &mut elaboration::verification::PackageSession<'a>,
    cleanup: Option<&mut elaboration::verification::drop::CleanupSession<'a, '_>>,
    declaration: nir::DeclarationId,
    arguments: &[nir::Term],
    callables: &[elaboration::verification::moves::CallableParameter],
    enums: &[elaboration::verification::moves::EnumParameter],
    effects: &[elaboration::verification::moves::EffectResultInput],
    borrowed: &[elaboration::verification::moves::BorrowedCode],
    text_provenance: elaboration::verification::memory::TextProvenance,
    fuel: &mut usize,
    source: nir::program::SourceRange,
) -> (
    Result<elaboration::verification::memory::Summary, String>,
    Vec<elaboration::verification::drop::RequiredDropInstance>,
) {
    use elaboration::verification::moves;
    let Some(cleanup) = cleanup else {
        return (
            Err("Drop language item is not bound to a source trait".into()),
            vec![],
        );
    };
    let result = moves::infer_callable_memory_with_session(
        session,
        declaration,
        arguments,
        callables,
        enums,
        effects,
        borrowed,
        Some(&mut *cleanup),
        text_provenance,
        fuel,
        source,
    );
    (result, cleanup.take_missing_runtime_instances())
}

/// Invocation-scoped resource evidence, including the interface frame used to
/// derive it. It is not a body-only summary or an executable caller schedule.
#[derive(Debug)]
pub struct MethodCleanup {
    pub invocation: elaboration::verification::memory::MethodInvocation,
    pub report: elaboration::verification::moves::Report,
}

impl std::fmt::Display for MethodCleanup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(method-cleanup {} {})", self.invocation, self.report)
    }
}

/// Acquire the interface invocation's entry/result cleanup under the selected
/// Copy/Drop protocols. Publishing this at a caller still requires prepared
/// input provenance and rebasing the frame's boundaries to that call site.
pub fn analyze_method_cleanup(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    callee: &nir::program::Expr,
    fuel: usize,
    query_budget: Budget,
) -> Result<MethodCleanup, elaboration::verification::moves::Error> {
    analyze_method_cleanup_with_context(
        package,
        world,
        items,
        callee,
        fuel,
        query_budget,
        &[],
        &[],
        &[],
        vec![],
        &[],
        &[],
    )
}

/// Analyze the interface frame under the actual invocation's resource premises.
/// Parameter indices refer to the interface signature, including field paths;
/// the returned report retains these premises and cannot certify other calls.
/// As for ordinary functions, callers must discharge them against prepared
/// arguments before publishing an executable schedule.
/// `Ok` is a frame report, not transitive completion: deferred calls must be
/// resolved and contextual named/closure reports must be checked recursively.
pub fn analyze_method_cleanup_with_context(
    package: &nir::program::Package,
    world: &World<'_>,
    items: &LangItems,
    callee: &nir::program::Expr,
    mut fuel: usize,
    query_budget: Budget,
    borrows: &[(usize, nir::Term)],
    callables: &[elaboration::verification::moves::CallableParameter],
    enums: &[elaboration::verification::moves::EnumParameter],
    effects: Vec<elaboration::verification::moves::EffectResultInput>,
    fields: &[elaboration::verification::moves::BorrowedField],
    borrowed: &[elaboration::verification::moves::BorrowedCode],
) -> Result<MethodCleanup, elaboration::verification::moves::Error> {
    let invocation =
        elaboration::verification::memory::prepare_method_invocation(package, callee, &mut fuel)
            .map_err(|reason| elaboration::verification::moves::Error {
                source: callee.source,
                reason,
            })?;
    let report = analyze_cleanup_with_session_provenance(
        package,
        world,
        items,
        &invocation.function,
        Some((
            invocation.implementation.declaration,
            &invocation.implementation.arguments,
        )),
        fuel,
        query_budget,
        borrows,
        callables,
        enums,
        effects,
        fields,
        borrowed,
    )?;
    Ok(MethodCleanup { invocation, report })
}
