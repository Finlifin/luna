//! Validate merged struct layout attributes.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedStructLayout {
    pub owner: OwnerId,
    pub enabled: bool,
    pub source: n::SourceRange,
}
impl std::fmt::Display for PackedStructLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(packed-struct-layout (owner {}) (enabled {}) (source {} {}))",
            self.owner, self.enabled, self.source.start, self.source.end
        )
    }
}

/// Validate only the selected `packed` property on struct definitions.
pub fn packed_structs(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<PackedStructLayout>, MergeFailure> {
    let mut output = Vec::new();
    for target in merged {
        let location = target
            .iter()
            .next()
            .map(|(_, value)| value.origin.source)
            .unwrap_or(n::SourceRange { start: 0, end: 0 });
        budget.consume(1).map_err(|e| MergeFailure {
            source: location,
            message: e.to_string(),
        })?;
        let Some(selected) = target.get("packed") else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure("packed requires a struct definition"));
        };
        if !package
            .item(owner)
            .is_some_and(|item| matches!(item.kind, ItemKind::Struct(_)))
        {
            return Err(failure("packed requires a struct definition"));
        }
        if selected.value.ty != p(P::Bool) {
            return Err(failure("packed requires the Bool domain"));
        }
        let K::Literal(n::Literal::Bool(enabled)) = selected.value.kind else {
            return Err(failure("packed requires a completed Bool value"));
        };
        output.push(PackedStructLayout {
            owner,
            enabled,
            source: location,
        });
    }
    Ok(output)
}
