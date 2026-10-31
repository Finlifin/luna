//! Effect operations and groups retain their own declaration kinds and ABI inputs.
use super::{DefinitionNodes, Error, charge, parameters::register_parameters};
use crate::{item::EffectModifiers, *};
use rustc_span::Span;

/// Assemble a bodyless effect operation. Reserve parameters under the owner in
/// the callback; this helper registers their parameter nodes. Register all other
/// signature nodes in the callback, as for a function declaration. No function
/// body, handler or error-result wrapper is synthesized. Ordinary elaboration
/// validates signature formation and interprets the retained effect modifiers.
pub fn assemble_effect<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    make: impl FnOnce(
        &mut DefinitionNodes<'hir>,
        &mut usize,
    ) -> Result<(FnSig<'hir>, EffectModifiers), Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    charge(fuel)?;
    let (signature, modifiers) = make(&mut nodes, fuel)?;
    register_parameters(
        arena,
        &mut nodes,
        signature.params,
        fuel,
        "effect parameter identity or index is inconsistent",
    )?;
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span,
        kind: ItemKind::Effect(signature, modifiers),
    });
    nodes.finish(item, fuel)
}

/// Assemble an effect group with its complete input telescope and operation
/// order. Operations are separately reserved owners in the same publication
/// batch. Callback allocations and external side effects are not rolled back.
pub fn assemble_effect_group<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    make: impl FnOnce(
        &mut DefinitionNodes<'hir>,
        &mut usize,
    ) -> Result<item::EffectGroupDef<'hir>, Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    charge(fuel)?;
    let definition = make(&mut nodes, fuel)?;
    for _ in &definition.operations {
        charge(fuel)?;
    }
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span,
        kind: ItemKind::EffectGroup(definition),
    });
    nodes.finish(item, fuel)
}
