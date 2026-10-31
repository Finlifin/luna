//! Validate merged phase metadata without discarding its declaration provenance.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberPhase {
    pub owner: OwnerId,
    pub phase: hir::ImplementationPhase,
    pub source: n::SourceRange,
}
impl std::fmt::Display for MemberPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(member-phase (owner {}) (phase {}) (source {} {}))",
            self.owner, self.phase, self.source.start, self.source.end
        )
    }
}

/// Validate only the selected `comptime_phase` properties. Other properties stay
/// available to their own consumers; this function does not authorize them or
/// mutate the HIR. A failure returns no partially applied phase assignment.
pub fn member_phases(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<MemberPhase>, MergeFailure> {
    let failure = |source, message: String| MergeFailure { source, message };
    let mut containers = BTreeMap::new();
    let mut output = Vec::new();
    let mut slots = BTreeMap::new();
    for (container, info) in package.owners() {
        let item = info.node.expect_item();
        let location = source(item.span);
        budget
            .consume(1)
            .map_err(|e| failure(location, e.to_string()))?;
        let members = match &item.kind {
            ItemKind::Trait(definition) => &definition.items,
            ItemKind::Impl(definition) if definition.trait_ref.is_some() => &definition.items,
            _ => continue,
        };
        for owner in members {
            if matches!(&item.kind, ItemKind::Trait(def) if def.comptime_only) {
                slots.insert(*owner, output.len());
                output.push(MemberPhase {
                    owner: *owner,
                    phase: hir::ImplementationPhase::Comptime,
                    source: location,
                });
            }
            let levels = (usize::BITS - containers.len().leading_zeros()) as usize + 1;
            budget
                .consume(levels.saturating_mul(16))
                .map_err(|e| failure(location, e.to_string()))?;
            if containers.insert(*owner, container).is_some() {
                return Err(failure(
                    location,
                    "trait phase member has multiple containers".into(),
                ));
            }
        }
    }
    for target in merged {
        let location = target
            .iter()
            .next()
            .map(|(_, value)| value.origin.source)
            .unwrap_or(n::SourceRange { start: 0, end: 0 });
        budget
            .consume(1)
            .map_err(|e| failure(location, e.to_string()))?;
        let Some(selected) = target.get("comptime_phase") else {
            continue;
        };
        let location = selected.origin.source;
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure(
                location,
                "comptime_phase requires a trait method or associated declaration".into(),
            ));
        };
        let levels = (usize::BITS - containers.len().leading_zeros()) as usize + 1;
        budget
            .consume(levels.saturating_mul(16))
            .map_err(|e| failure(location, e.to_string()))?;
        if !containers.contains_key(&owner)
            || !package.item(owner).is_some_and(|item| {
                matches!(
                    item.kind,
                    ItemKind::Fn(..) | ItemKind::FnDeclaration(_) | ItemKind::Associated(_)
                )
            })
        {
            return Err(failure(
                location,
                "comptime_phase requires a trait method or associated declaration".into(),
            ));
        }
        let K::Literal(n::Literal::Bool(enabled)) = selected.value.kind else {
            return Err(failure(
                location,
                "comptime_phase requires a completed Bool value".into(),
            ));
        };
        if selected.value.ty != p(P::Bool) {
            return Err(failure(
                location,
                "comptime_phase requires the Bool domain".into(),
            ));
        }
        budget
            .consume(1)
            .map_err(|e| failure(location, e.to_string()))?;
        if !enabled
            && containers.get(&owner).is_some_and(|container| {
                package.item(*container).is_some_and(
                    |item| matches!(&item.kind, ItemKind::Trait(def) if def.comptime_only),
                )
            })
        {
            return Err(failure(
                location,
                "comptime-only trait cannot contain runtime phase members".into(),
            ));
        }
        let entry = MemberPhase {
            owner,
            source: location,
            phase: if enabled {
                hir::ImplementationPhase::Comptime
            } else {
                hir::ImplementationPhase::Runtime
            },
        };
        if let Some(index) = slots.get(&owner) {
            output[*index] = entry;
        } else {
            slots.insert(owner, output.len());
            output.push(entry);
        }
    }
    Ok(output)
}
