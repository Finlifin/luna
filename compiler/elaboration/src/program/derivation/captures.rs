//! Retain externally needed generation values and their annotation dependencies.
use super::*;
use function_templates::ClonedMember;

pub(super) fn retain_used<'h>(
    drafts: &mut [(ClonedMember<'h>, DefId)],
    slots: &HashMap<DefId, usize>,
    arena: &'h HirArena,
    fuel: &mut usize,
) -> Result<BTreeSet<usize>, Error> {
    let span = rustc_span::DUMMY_SP;
    *fuel = fuel
        .checked_sub(drafts.len().saturating_add(slots.len()))
        .ok_or_else(|| error(span, "generation capture analysis budget exhausted"))?;
    let indices: HashMap<_, _> = drafts
        .iter()
        .enumerate()
        .map(|(index, (_, identity))| (*identity, index))
        .collect();
    let mut pending = BTreeSet::new();
    for (draft, identity) in drafts.iter() {
        if !slots.contains_key(identity) {
            for dependency in &draft.capture_uses {
                charge(fuel, draft.item.span)?;
                pending.insert(*dependency);
            }
        }
    }
    let mut needed = BTreeSet::new();
    while let Some(identity) = pending.pop_first() {
        charge(fuel, span)?;
        if !needed.insert(identity) {
            continue;
        }
        let index = indices
            .get(&identity)
            .ok_or_else(|| error(span, "generated capture has no value draft"))?;
        for dependency in &drafts[*index].0.capture_uses {
            charge(fuel, span)?;
            if !needed.contains(dependency) {
                pending.insert(*dependency);
            }
        }
    }
    let mut exported = BTreeSet::new();
    for (draft, identity) in drafts {
        let Some(index) = slots.get(identity) else {
            continue;
        };
        charge(fuel, draft.item.span)?;
        if needed.contains(identity) {
            exported.insert(*index);
            continue;
        }
        // Reservations must still be defined. These private, unreachable value
        // slots become unit constants; their original computation remains in
        // the shared generation block and is checked/executed there.
        let ty = arena.alloc_expr(hir::Expr {
            hir_id: hir::HirId::new(draft.item.owner_id, hir::ItemLocalId::new(1)),
            span: draft.item.span,
            kind: E::TyVoid,
        });
        let value = arena.alloc_expr(hir::Expr {
            hir_id: hir::HirId::new(draft.item.owner_id, hir::ItemLocalId::new(2)),
            span: draft.item.span,
            kind: E::Unit,
        });
        draft.item.kind = ItemKind::Const(ty, value);
        draft.bodies.clear();
        draft.bindings.clear();
        draft.resolution.clear();
        draft.capture_uses.clear();
    }
    Ok(exported)
}
