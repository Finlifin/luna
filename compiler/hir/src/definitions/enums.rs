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
/// Configurations, generic clauses and intrinsic enums use assemble_enum_definition.
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
        check_payload(&nodes, owner, hir_id, &kind, fuel)?;
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

fn check_payload(
    nodes: &DefinitionNodes<'_>,
    owner: OwnerId,
    hir_id: HirId,
    kind: &VariantKind<'_>,
    fuel: &mut usize,
) -> Result<(), Error> {
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
    match kind {
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
    Ok(())
}

/// Assemble an enum with its clauses, configurations and intrinsic marker.
/// Reserve top-level variants under the root; register their payload nodes and
/// all other nodes in the callback. This helper registers the returned variants.
/// Nested declarations remain separate batch entries. Semantic publication
/// checks payload types, configurations and requirements without weakening them.
/// Callback effects and arena allocations are not rolled back on failure.
pub fn assemble_enum_definition<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    make_enum: impl FnOnce(&mut DefinitionNodes<'hir>, &mut usize) -> Result<EnumDef<'hir>, Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    charge(fuel)?;
    let definition = make_enum(&mut nodes, fuel)?;
    for variant in definition.variants {
        if nodes.reserved_parent(variant.hir_id) != Some(nodes.root()) {
            return Err(error(
                "variant must be reserved under its definition",
                owner,
            ));
        }
        check_payload(&nodes, owner, variant.hir_id, &variant.kind, fuel)?;
        nodes.register(Node::Variant(variant), fuel)?;
    }
    for _ in &definition.nested_items {
        charge(fuel)?;
    }
    for _ in &definition.configurations {
        charge(fuel)?;
    }
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span,
        kind: ItemKind::Enum(definition),
    });
    nodes.finish(item, fuel)
}
