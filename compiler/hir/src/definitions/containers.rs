//! Module membership and ordinary typed constant declarations.
use super::{DefinitionNodes, Error, charge, error};
use crate::*;
use rustc_span::Span;

/// Assemble a module containing separately reserved owners in declaration order.
/// Publication validates membership, including missing owners and cycles. This
/// does not reopen source name resolution or make members implicitly public.
pub fn assemble_module<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    items: &[OwnerId],
    span: Span,
    fuel: &mut usize,
) -> Result<OwnerInfo<'hir>, Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    charge(fuel)?;
    for _ in items {
        charge(fuel)?;
    }
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span,
        kind: ItemKind::Mod(ModDef {
            items: items.to_vec(),
        }),
    });
    nodes.finish(item, fuel)
}

/// Assemble a typed constant. The callback registers its annotation and
/// initializer directly under the owner, with all child nodes registered in the
/// same draft. Evaluation, type admission and dependency cycles are checked by
/// ordinary elaboration; assembly does not execute the initializer. Callback
/// side effects and arena allocations are not rolled back on failure.
pub fn assemble_constant<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    make: impl FnOnce(
        &mut DefinitionNodes<'hir>,
        &mut usize,
    ) -> Result<(&'hir Expr<'hir>, &'hir Expr<'hir>), Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    charge(fuel)?;
    let (annotation, initializer) = make(&mut nodes, fuel)?;
    for value in [annotation, initializer] {
        if nodes.reserved_parent(value.hir_id) != Some(nodes.root()) {
            return Err(error(
                "constant roots must be reserved under their owner",
                owner,
            ));
        }
    }
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span,
        kind: ItemKind::Const(annotation, initializer),
    });
    nodes.finish(item, fuel)
}
