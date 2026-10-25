//! Indexed enum assembly using ordinary HIR payloads and shared construction fuel.
use super::{DefinitionNodes, Error, charge, error};
use crate::*;
use rustc_span::Span;

/// Assemble an ordinary enum in declaration order. The callback constructs each
/// payload under its reserved variant identity and registers all payload nodes.
/// This accepts every HIR VariantKind, including contextual patterns, explicit
/// discriminants and sub-enums; their semantics are checked at publication.
///
/// Only the outer variants are registered here. Direct payload roots must have
/// been reserved under their variant. Callback work shares fuel with assembly;
/// unfinished reservations, callback errors or exhaustion publish nothing. Arena
/// allocations and arbitrary callback side effects are not rolled back.
/// Configurations, generic clauses and intrinsic enums use general HIR assembly.
pub fn assemble_enum<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    variants: &[Ident],
    span: Span,
    fuel: &mut usize,
    mut make_payload: impl FnMut(
        usize,
        HirId,
        &mut DefinitionNodes<'hir>,
        &mut usize,
    ) -> Result<VariantKind<'hir>, Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    variants
        .len()
        .checked_add(1)
        .filter(|n| *n <= u32::MAX as usize)
        .ok_or_else(|| error("local node identity exhausted", owner))?;
    let work = variants
        .len()
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| error("enum assembly budget overflow", owner))?;
    if *fuel < work {
        return Err(Error {
            kind: "budget exhausted",
            owner: None,
        });
    }
    let mut output = Vec::with_capacity(variants.len());
    for (index, ident) in variants.iter().enumerate() {
        let hir_id = nodes.reserve(nodes.root(), fuel)?;
        let kind = make_payload(index, hir_id, &mut nodes, fuel)?;
        let mut check = |child| {
            charge(fuel)?;
            if nodes.reserved_parent(child) != Some(hir_id) {
                return Err(error(
                    "variant payload root must be reserved under its variant",
                    owner,
                ));
            }
            Ok(())
        };
        match &kind {
            VariantKind::Unit => {}
            VariantKind::Pattern(pattern) => check(pattern.hir_id)?,
            VariantKind::Const(value) => check(value.hir_id)?,
            VariantKind::Tuple(types) => {
                for ty in *types {
                    check(ty.hir_id)?;
                }
            }
            VariantKind::Struct(fields) => {
                for field in *fields {
                    check(field.hir_id)?;
                }
            }
            VariantKind::SubEnum(children) => {
                for child in *children {
                    check(child.hir_id)?;
                }
            }
        }
        output.push(Variant {
            hir_id,
            ident: *ident,
            kind,
            span: ident.span,
        });
    }
    let variants = arena.alloc_variant_slice(output);
    for variant in variants {
        nodes.register(Node::Variant(variant), fuel)?;
    }
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span,
        kind: ItemKind::Enum(EnumDef {
            is_intrinsic: false,
            variants,
            configurations: vec![],
            clause_params: &[],
            clause_constraints: &[],
            nested_items: vec![],
        }),
    });
    nodes.finish(item, fuel)
}
