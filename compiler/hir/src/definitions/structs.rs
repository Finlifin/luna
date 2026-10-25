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
    for field in fields {
        let field_id = nodes.reserve(nodes.root(), fuel)?;
        let ty_id = nodes.reserve(field_id, fuel)?;
        let ty = arena.alloc_expr(Expr {
            hir_id: ty_id,
            kind: ExprKind::DeliveredType(field.ty.clone()),
            span: field.span,
        });
        nodes.register(Node::Expr(ty), fuel)?;
        output.push(FieldDef {
            hir_id: field_id,
            ident: field.name,
            ty,
            default: None,
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
