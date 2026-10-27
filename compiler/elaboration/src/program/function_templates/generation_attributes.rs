//! Move strategy-scope attributes into the generated impl's equivalent scope.
use super::*;
use hir::common::DefId;

pub(in crate::program) fn relocate<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    strategy: OwnerId,
    shell: &mut ClonedMember<'h>,
    members: &mut [(ClonedMember<'h>, DefId)],
    definitions: &HashMap<DefId, DefId>,
    captures: &HashMap<HirId, InputCapture>,
    fuel: &mut usize,
) -> Result<(), Error> {
    *fuel = fuel
        .checked_sub(definitions.len().saturating_add(captures.len()))
        .ok_or_else(|| {
            error(
                shell.item.span,
                "generated attribute identity budget exhausted",
            )
        })?;
    let mut rebind = Rebind {
        source,
        arena,
        namespace: shell.item.owner_id,
        bindings: std::mem::take(&mut shell.bindings),
        budget: *fuel,
        bodies: Vec::new(),
        definitions: definitions.clone(),
        // Only bindings introduced inside the copied expression may be read.
        // Free generation inputs require value substitution, not fresh locals.
        member_owner: Some(OwnerId::INVALID),
        captures: captures.clone(),
        capture_uses: BTreeSet::new(),
    };
    let result = (|| {
        for (member, _) in members {
            for attribute in &mut member.attributes {
                rebind.charge(attribute.span, 0)?;
                if attribute.lexical_owner == strategy {
                    attribute.value = rebind.expr(attribute.value, 0)?;
                    attribute.lexical_owner = shell.item.owner_id;
                }
            }
        }
        rebind.resolution()
    })();
    *fuel = rebind.budget;
    shell.bindings = rebind.bindings;
    let resolution = result?;
    shell.resolution = resolution;
    shell.bodies.extend(rebind.bodies);
    shell.capture_uses.extend(rebind.capture_uses);
    Ok(())
}
