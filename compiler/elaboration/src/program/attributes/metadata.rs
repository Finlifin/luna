//! Validate merged user metadata wrapper attributes.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataEntry {
    pub owner: OwnerId,
    pub source: n::SourceRange,
}
impl std::fmt::Display for MetadataEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(metadata-attribute (owner {}) (source {} {}))",
            self.owner, self.source.start, self.source.end
        )
    }
}

/// Validate `metadata` attribute values on definitions.
pub fn metadata_entries(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<MetadataEntry>, MergeFailure> {
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
        let Some(selected) = target.get("__flurry_metadata") else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure("metadata requires a definition target"));
        };
        if package.item(owner).is_none() {
            return Err(failure("metadata requires a definition target"));
        }
        if selected.value.ty != p(P::Object) {
            return Err(failure("metadata requires the Object domain"));
        }
        output.push(MetadataEntry {
            owner,
            source: location,
        });
    }
    Ok(output)
}
