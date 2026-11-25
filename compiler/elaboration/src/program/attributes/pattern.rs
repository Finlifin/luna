//! Validate merged `^attrs.pattern.*` attributes on `case` declarations.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternCaseStability {
    pub owner: OwnerId,
    pub stable: bool,
    pub source: n::SourceRange,
}

impl std::fmt::Display for PatternCaseStability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(pattern-case-stability (owner {}) (stable {}) (source {} {}))",
            self.owner, self.stable, self.source.start, self.source.end
        )
    }
}

/// Validate only the selected `__flurry_pattern_stable` property on `case` declarations.
pub fn stable_case_patterns(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<PatternCaseStability>, MergeFailure> {
    let mut output = Vec::new();
    for target in merged {
        let location = target.source();
        budget.consume(1).map_err(|e| MergeFailure {
            source: location,
            message: e.to_string(),
        })?;
        let Some(selected) = target.get("__flurry_pattern_stable") else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure("attrs.pattern.stable requires a case declaration"));
        };
        let Some(ItemKind::Case(..)) = package.item(owner).map(|item| &item.kind) else {
            return Err(failure("attrs.pattern.stable requires a case declaration"));
        };
        if selected.value.ty != p(P::Bool) {
            return Err(failure("attrs.pattern.stable requires the Bool domain"));
        }
        let K::Literal(n::Literal::Bool(stable)) = selected.value.kind else {
            return Err(failure(
                "attrs.pattern.stable requires a completed Bool value",
            ));
        };
        output.push(PatternCaseStability {
            owner,
            stable,
            source: location,
        });
    }
    Ok(output)
}
