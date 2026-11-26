//! Consume merged intrinsic-enum compilation attributes.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntrinsicEnumMarker {
    pub owner: OwnerId,
    pub enabled: bool,
    pub source: n::SourceRange,
}

impl std::fmt::Display for IntrinsicEnumMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(intrinsic-enum-marker (owner {}) (enabled {}) (source {} {}))",
            self.owner, self.enabled, self.source.start, self.source.end
        )
    }
}

/// Validate the selected `__flurry_intrinsic_enum` property on enum definitions.
pub fn intrinsic_enum_markers(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<IntrinsicEnumMarker>, MergeFailure> {
    let mut output = Vec::new();
    for target in merged {
        let location = target.source();
        budget.consume(1).map_err(|e| MergeFailure {
            source: location,
            message: e.to_string(),
        })?;
        let Some(selected) = target.get("__flurry_intrinsic_enum") else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        if selected.entry.is_none() || selected.origin.configuration_key.is_some() {
            return Err(failure(
                "__flurry_intrinsic_enum cannot be attached via definition configuration; use ^attrs.compilation.intrinsic_enum",
            ));
        }
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure(
                "__flurry_intrinsic_enum requires an enum declaration",
            ));
        };
        if !package
            .item(owner)
            .is_some_and(|item| matches!(item.kind, ItemKind::Enum(_)))
        {
            return Err(failure(
                "__flurry_intrinsic_enum requires an enum declaration",
            ));
        }
        if selected.value.ty != p(P::Bool) {
            return Err(failure("__flurry_intrinsic_enum requires the Bool domain"));
        }
        let K::Literal(n::Literal::Bool(enabled)) = selected.value.kind else {
            return Err(failure(
                "__flurry_intrinsic_enum requires a completed Bool value",
            ));
        };
        output.push(IntrinsicEnumMarker {
            owner,
            enabled,
            source: location,
        });
    }
    Ok(output)
}
