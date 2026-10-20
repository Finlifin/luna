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
    use elaboration::verification::{drop::CleanupSession, moves};
    let Some(LangItemDef::UserDef(protocol)) = items.get(LangItem::Drop) else {
        return Err(moves::Error {
            source: function.body.source,
            reason: "Drop language item is not bound to a source trait".into(),
        });
    };
    let mut session = CleanupSession::new(package, world, protocol, query_budget);
    moves::analyze_in_package_with_field_context(
        package,
        function,
        |ty| copy_delivery(package, world, items, ty, query_budget),
        fuel,
        Some(&mut session),
        borrows,
        callables,
        enums,
        effects,
        fields,
    )
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
    let item = match operation {
        elaboration::key_protocols::Operation::Eq => LangItem::Eq,
        elaboration::key_protocols::Operation::Hash => LangItem::Hash,
    };
    let Some(LangItemDef::UserDef(protocol)) = items.get(item) else {
        return Err(format!(
            "{operation} language item is not bound to a source trait"
        ));
    };
    elaboration::key_protocols::select(package, world, protocol, ty, operation, budget)
}
