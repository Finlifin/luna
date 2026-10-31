//! Consume merged default-registration metadata without selecting a strategy.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultStrategyMarker {
    pub owner: OwnerId,
    pub enabled: bool,
    pub source: n::SourceRange,
}
impl std::fmt::Display for DefaultStrategyMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(default-strategy-marker (owner {}) (enabled {}) (source {} {}))",
            self.owner, self.enabled, self.source.start, self.source.end
        )
    }
}

/// Consume only the selected derivation_default property. Disabled markers
/// remain explicit so an outer override cannot accidentally register a default.
/// Ownership, visibility and applicability are separate obligations.
pub fn default_strategy_markers(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<DefaultStrategyMarker>, MergeFailure> {
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
        let Some(selected) = target.get("derivation_default") else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure("derivation_default requires a deriver definition"));
        };
        if !package
            .item(owner)
            .is_some_and(|item| matches!(item.kind, ItemKind::Deriver(_)))
        {
            return Err(failure("derivation_default requires a deriver definition"));
        }
        if selected.value.ty != p(P::Bool) {
            return Err(failure("derivation_default requires the Bool domain"));
        }
        let K::Literal(n::Literal::Bool(enabled)) = selected.value.kind else {
            return Err(failure(
                "derivation_default requires a completed Bool value",
            ));
        };
        output.push(DefaultStrategyMarker {
            owner,
            enabled,
            source: location,
        });
    }
    Ok(output)
}
