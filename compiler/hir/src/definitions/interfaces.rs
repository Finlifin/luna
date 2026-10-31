//! Trait and implementation assembly through owner-local node drafts.
use super::{DefinitionNodes, Error, charge};
use crate::*;
use rustc_span::Span;

/// Assemble a trait without restricting its super-trait expression to one head.
/// The callback reserves and registers all clause/expression nodes in this draft.
/// Members retain their separate owner identities and may be forward reservations
/// in the same publication batch. Member contracts, visibility and trait bounds
/// are checked by ordinary publication/elaboration, not by this assembly helper.
/// Callback effects and arena allocations are not rolled back on failure.
pub fn assemble_trait<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    make_trait: impl FnOnce(&mut DefinitionNodes<'hir>, &mut usize) -> Result<TraitDef<'hir>, Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    charge(fuel)?;
    let definition = make_trait(&mut nodes, fuel)?;
    for _ in &definition.items {
        charge(fuel)?;
    }
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span,
        kind: ItemKind::Trait(definition),
    });
    nodes.finish(item, fuel)
}

/// Assemble either a trait implementation or an inherent implementation with
/// its full ordinary HIR receiver, trait expression and generic clauses. The
/// callback registers their nodes in this owner-local draft. Members are separate
/// definitions; their bodies and this descriptor must be published together.
/// No Type expression is invoked or normalized while assembling the descriptor.
pub fn assemble_impl<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    make_impl: impl FnOnce(&mut DefinitionNodes<'hir>, &mut usize) -> Result<ImplDef<'hir>, Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    charge(fuel)?;
    let definition = make_impl(&mut nodes, fuel)?;
    for _ in &definition.items {
        charge(fuel)?;
    }
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span,
        kind: ItemKind::Impl(definition),
    });
    nodes.finish(item, fuel)
}
