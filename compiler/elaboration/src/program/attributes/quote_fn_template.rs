//! Validate merged `quote_fn_template` attributes on bodyless `template quote fn` declarations.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuoteFnTemplateFlags {
    pub owner: OwnerId,
    pub for_match_arms: bool,
    pub allow_non_exhaustive_instance: bool,
    pub source: n::SourceRange,
}

impl std::fmt::Display for QuoteFnTemplateFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(quote-fn-template-flags (owner {}) (for-match-arms {}) (allow-non-exhaustive-instance {}) (source {} {}))",
            self.owner,
            self.for_match_arms,
            self.allow_non_exhaustive_instance,
            self.source.start,
            self.source.end
        )
    }
}

fn read_bool_flag(
    target: &MergedTarget<'_>,
    key: &str,
    package: &hir::Package<'_>,
) -> Result<Option<(OwnerId, bool, n::SourceRange)>, MergeFailure> {
    let Some(selected) = target.get(key) else {
        return Ok(None);
    };
    let location = selected.origin.source;
    let failure = |message: &str| MergeFailure {
        source: location,
        message: message.into(),
    };
    if selected.entry.is_none() || selected.origin.configuration_key.is_some() {
        return Err(failure(&format!(
            "{key} cannot be attached via definition configuration; use ^attrs.quote_fn_template"
        )));
    }
    let hir::AttributeTarget::Definition(owner) = target.target else {
        return Err(failure(&format!(
            "{key} requires a bodyless template quote fn declaration"
        )));
    };
    let valid_target = matches!(
        package.item(owner).map(|item| &item.kind),
        Some(ItemKind::FnDeclaration(sig))
            if sig.modifiers.is_quote && sig.modifiers.is_function_template
    );
    if !valid_target {
        return Err(failure(&format!(
            "{key} requires a bodyless template quote fn declaration"
        )));
    }
    if selected.value.ty != p(P::Bool) {
        return Err(failure(&format!("{key} requires the Bool domain")));
    }
    let K::Literal(n::Literal::Bool(enabled)) = selected.value.kind else {
        return Err(failure(&format!("{key} requires a completed Bool value")));
    };
    Ok(Some((owner, enabled, location)))
}

/// Validate `__flurry_quote_fn_template_for_match_arms` and
/// `__flurry_quote_fn_template_allow_non_exhaustive_instance` on `template quote fn`.
pub fn quote_fn_template_flags(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<QuoteFnTemplateFlags>, MergeFailure> {
    let mut output = Vec::new();
    for target in merged {
        let location = target.source();
        budget.consume(1).map_err(|e| MergeFailure {
            source: location,
            message: e.to_string(),
        })?;
        let for_arms =
            read_bool_flag(target, "__flurry_quote_fn_template_for_match_arms", package)?;
        let allow_non_exhaustive = read_bool_flag(
            target,
            "__flurry_quote_fn_template_allow_non_exhaustive_instance",
            package,
        )?;
        if for_arms.is_none() && allow_non_exhaustive.is_none() {
            continue;
        }
        let owner = for_arms
            .as_ref()
            .or(allow_non_exhaustive.as_ref())
            .expect("checked template flag")
            .0;
        let for_match_arms = for_arms.as_ref().is_some_and(|entry| entry.1);
        let allow_non_exhaustive_instance =
            allow_non_exhaustive.as_ref().is_some_and(|entry| entry.1);
        let source = allow_non_exhaustive
            .as_ref()
            .map(|entry| entry.2)
            .or_else(|| for_arms.as_ref().map(|entry| entry.2))
            .unwrap_or(location);
        if allow_non_exhaustive_instance && !for_match_arms {
            return Err(MergeFailure {
                source,
                message: "__flurry_quote_fn_template_allow_non_exhaustive_instance requires __flurry_quote_fn_template_for_match_arms".into(),
            });
        }
        output.push(QuoteFnTemplateFlags {
            owner,
            for_match_arms,
            allow_non_exhaustive_instance,
            source,
        });
    }
    Ok(output)
}
