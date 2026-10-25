//! Assemble completed field Type inputs with canonical owner-local identities.
use super::{DefinitionNodes, Error, error};
use crate::*;
use rustc_span::Span;
use std::{fmt, sync::Arc};

/// A completed field description, not a published member or access permission.
/// Defaults and contextual code continue to use the general HIR assembly path.
#[derive(Clone, Debug)]
pub struct TypedField {
    pub name: Ident,
    pub ty: Arc<nir::Term>,
    pub span: Span,
}
impl fmt::Display for TypedField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(typed-field (name {:?}) (type {}))",
            self.name.name.as_str(),
            self.ty
        )
    }
}

/// Construct an indexed struct from completed field descriptions. This neither
/// reserves a package owner nor publishes a Type: pass the result through the
/// ordinary DefinitionBatch semantic admission boundary. Duplicate names and
/// invalid Type descriptions are diagnosed there, alongside source definitions.
///
/// All identity/budget checks precede arena allocation. A failed preparation
/// leaves the caller's existing batch descriptions and package unchanged.
pub fn assemble_struct<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    fields: &[TypedField],
    span: Span,
    fuel: &mut usize,
) -> Result<OwnerInfo<'hir>, Error> {
    assemble_struct_with_defaults(arena, owner, name, fields, span, fuel, |_, _, _, _| {
        Ok(None)
    })
}

/// Build optional defaults under each field's identity. The callback reserves
/// the default root directly under `field_id`, registers its children, and
/// returns the unregistered root. This function registers that root and checks
/// its parent. All expressions retain normal HIR checking and CMTT semantics.
///
/// Base field/index work is preflighted; callback work shares the remaining
/// budget. Failure publishes nothing, but arena allocations and arbitrary
/// callback side effects are not rolled back. Existing batch entries are untouched
/// by this assembler. General node construction must register all reserved nodes.
pub fn assemble_struct_with_defaults<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    fields: &[TypedField],
    span: Span,
    fuel: &mut usize,
    mut make_default: impl FnMut(
        usize,
        HirId,
        &mut DefinitionNodes<'hir>,
        &mut usize,
    ) -> Result<Option<&'hir Expr<'hir>>, Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    let count = fields.len();
    count
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .filter(|n| *n <= u32::MAX as usize)
        .ok_or_else(|| error("local node identity exhausted", owner))?;
    let work = count
        .checked_mul(4)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| error("struct assembly budget overflow", owner))?;
    if *fuel < work {
        return Err(Error {
            kind: "budget exhausted",
            owner: None,
        });
    }
    let mut output = Vec::with_capacity(count);
    for (index, field) in fields.iter().enumerate() {
        let field_id = nodes.reserve(nodes.root(), fuel)?;
        let ty_id = nodes.reserve(field_id, fuel)?;
        let ty = arena.alloc_expr(Expr {
            hir_id: ty_id,
            kind: ExprKind::DeliveredType(field.ty.clone()),
            span: field.span,
        });
        nodes.register(Node::Expr(ty), fuel)?;
        let default = make_default(index, field_id, &mut nodes, fuel)?;
        if let Some(default) = default {
            if nodes.reserved_parent(default.hir_id) != Some(field_id) {
                return Err(error(
                    "field default root must be reserved under its field",
                    owner,
                ));
            }
            nodes.register(Node::Expr(default), fuel)?;
        }
        output.push(FieldDef {
            hir_id: field_id,
            ident: field.name,
            ty,
            default,
            span: field.span,
        });
    }
    let fields = arena.alloc_field_def_slice(output);
    for field in fields {
        nodes.register(Node::FieldDef(field), fuel)?;
    }
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span,
        kind: ItemKind::Struct(StructDef {
            fields,
            clause_params: &[],
            clause_constraints: &[],
            nested_items: vec![],
        }),
    });
    nodes.finish(item, fuel)
}
