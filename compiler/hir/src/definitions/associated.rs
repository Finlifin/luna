//! Associated value/type contracts and definitions with dependent telescopes.
use super::{DefinitionNodes, Error, charge, parameters::register_parameters};
use crate::item::AssociatedDecl;
use crate::*;
use rustc_span::Span;

/// Assemble an associated declaration or implementation value. The callback
/// reserves parameter nodes under the owner and registers all other expression
/// and clause nodes. Parameter registration is derived from the descriptor, as
/// for generated functions; no synthetic function body or BodyId is introduced.
/// Defaults, parameter dependencies and constraints are retained unchanged.
/// Ordinary publication checks the containing trait/impl and the value contract.
pub fn assemble_associated<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    make: impl FnOnce(&mut DefinitionNodes<'hir>, &mut usize) -> Result<AssociatedDecl<'hir>, Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    charge(fuel)?;
    let definition = make(&mut nodes, fuel)?;
    register_parameters(
        arena,
        &mut nodes,
        definition.parameters,
        fuel,
        "associated parameter identity or index is inconsistent",
    )?;
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span,
        kind: ItemKind::Associated(definition),
    });
    nodes.finish(item, fuel)
}
