//! Alias and nominal representation declarations retain distinct HIR kinds.
use super::{DefinitionNodes, Error, charge};
use crate::*;
use rustc_span::Span;

/// A type expression and its declaration-side input telescope. The callback
/// registers these nodes in the supplied draft; assembling does not evaluate
/// the expression or replace an opaque normal-form head with its body.
pub type TypeDeclaration<'hir> = (
    &'hir Expr<'hir>,
    &'hir [ClauseParam<'hir>],
    &'hir [ClauseConstraint<'hir>],
);

/// Assemble an alias with ordinary alias requirements and substitution rules.
/// The callback registers every reserved expression/clause node. Publication
/// checks Type formation and constraints; no nominal identity is fabricated.
/// Callback effects and arena allocations are not rolled back on failure.
pub fn assemble_type_alias<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    make: impl FnOnce(&mut DefinitionNodes<'hir>, &mut usize) -> Result<TypeDeclaration<'hir>, Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    assemble(arena, owner, name, span, fuel, false, make)
}

/// Assemble a nominal newtype with an explicit representation expression.
/// This does not grant conversions or expose the representation as type equality.
/// Inputs and constraints follow the same node-registration contract as aliases.
pub fn assemble_newtype<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    make: impl FnOnce(&mut DefinitionNodes<'hir>, &mut usize) -> Result<TypeDeclaration<'hir>, Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    assemble(arena, owner, name, span, fuel, true, make)
}

fn assemble<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    nominal: bool,
    make: impl FnOnce(&mut DefinitionNodes<'hir>, &mut usize) -> Result<TypeDeclaration<'hir>, Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    charge(fuel)?;
    let (representation, parameters, constraints) = make(&mut nodes, fuel)?;
    let kind = if nominal {
        ItemKind::Newtype(representation, parameters, constraints)
    } else {
        ItemKind::TypeAlias(representation, parameters, constraints)
    };
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span,
        kind,
    });
    nodes.finish(item, fuel)
}
