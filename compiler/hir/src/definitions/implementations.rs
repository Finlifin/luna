//! Inherent implementation assembly for completed receiver Type descriptions.
use super::{DefinitionNodes, Error, charge, error};
use crate::*;
use rustc_span::Span;
use std::sync::Arc;

/// Assemble an inherent impl and its receiver type node. Members may be forward
/// reservations in the same batch; publication checks their existence and normal
/// elaboration checks bodies and receiver contracts. Member containment belongs
/// in `items`, not in the block-local `lexical_parent` metadata.
///
/// Generic clauses and trait implementation references use the general HIR path.
pub fn assemble_inherent_impl<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    self_ty: Arc<nir::Term>,
    members: &[OwnerId],
    span: Span,
    fuel: &mut usize,
) -> Result<OwnerInfo<'hir>, Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    let work = members
        .len()
        .checked_add(3)
        .ok_or_else(|| error("implementation assembly budget overflow", owner))?;
    if *fuel < work {
        return Err(Error {
            kind: "budget exhausted",
            owner: None,
        });
    }
    let mut items = Vec::with_capacity(members.len());
    for member in members {
        charge(fuel)?;
        items.push(*member);
    }
    let ty_id = nodes.reserve(nodes.root(), fuel)?;
    let self_ty = arena.alloc_expr(Expr {
        hir_id: ty_id,
        kind: ExprKind::DeliveredType(self_ty),
        span,
    });
    nodes.register(Node::Expr(self_ty), fuel)?;
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span,
        kind: ItemKind::Impl(ImplDef {
            self_ty,
            trait_ref: None,
            clause_params: &[],
            clause_constraints: &[],
            items,
        }),
    });
    nodes.finish(item, fuel)
}
