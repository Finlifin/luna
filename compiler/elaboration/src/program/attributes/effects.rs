//! Validate merged direct-passing effect attributes.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectDirectPassing {
    pub owner: OwnerId,
    pub enabled: bool,
    pub source: n::SourceRange,
}

impl std::fmt::Display for EffectDirectPassing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(effect-direct-passing (owner {}) (enabled {}) (source {} {}))",
            self.owner, self.enabled, self.source.start, self.source.end
        )
    }
}

/// Validate only the selected `direct_passing` property on effect declarations.
pub fn direct_passing_effects(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<EffectDirectPassing>, MergeFailure> {
    let mut output = Vec::new();
    for target in merged {
        let location = target.source();
        budget.consume(1).map_err(|e| MergeFailure {
            source: location,
            message: e.to_string(),
        })?;
        let Some(selected) = target.get("__flurry_direct_passing") else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure("direct_passing requires an effect declaration"));
        };
        let Some(ItemKind::Effect(sig, mods)) = package.item(owner).map(|item| &item.kind) else {
            return Err(failure("direct_passing requires an effect declaration"));
        };
        if selected.value.ty != p(P::Bool) {
            return Err(failure("direct_passing requires the Bool domain"));
        }
        let K::Literal(n::Literal::Bool(enabled)) = selected.value.kind else {
            return Err(failure("direct_passing requires a completed Bool value"));
        };
        if enabled && (mods.is_async || !sig.params.is_empty()) {
            return Err(failure(
                "direct passing effects must be synchronous and have no parameters",
            ));
        }
        output.push(EffectDirectPassing {
            owner,
            enabled,
            source: location,
        });
    }
    Ok(output)
}
