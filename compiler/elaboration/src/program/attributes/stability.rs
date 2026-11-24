//! Validate merged `__flurry_stable_since` ABI stability version anchors.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StableSinceEntry {
    pub owner: OwnerId,
    pub version: String,
    pub source: n::SourceRange,
}

impl std::fmt::Display for StableSinceEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(stable-since-attribute (owner {}) (version {:?}) (source {} {}))",
            self.owner, self.version, self.source.start, self.source.end
        )
    }
}

/// Validate strict SemVer 2.0.0 syntax (`MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]`).
pub fn is_valid_semver(version: &str) -> bool {
    let (core_and_pre, build) = match version.split_once('+') {
        Some((head, tail)) => {
            if tail.is_empty()
                || !tail.split('.').all(|id| {
                    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
            {
                return false;
            }
            (head, Some(tail))
        }
        None => (version, None),
    };
    let _ = build;
    let (core, pre) = match core_and_pre.split_once('-') {
        Some((head, tail)) => {
            if tail.is_empty()
                || !tail.split('.').all(|id| {
                    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    {
                        return false;
                    }
                    if id.bytes().all(|b| b.is_ascii_digit()) {
                        is_canonical_numeric_id(id)
                    } else {
                        true
                    }
                })
            {
                return false;
            }
            (head, Some(tail))
        }
        None => (core_and_pre, None),
    };
    let _ = pre;
    let mut parts = core.split('.');
    let (Some(major), Some(minor), Some(patch), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    is_canonical_numeric_id(major)
        && is_canonical_numeric_id(minor)
        && is_canonical_numeric_id(patch)
}

fn is_canonical_numeric_id(segment: &str) -> bool {
    if segment.is_empty() || !segment.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    segment == "0" || !segment.starts_with('0')
}

/// Validate `__flurry_stable_since` on anchorable nominal or normal-form definitions.
pub fn stable_since_versions(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<StableSinceEntry>, MergeFailure> {
    let mut output = Vec::new();
    for target in merged {
        let location = target.source();
        budget.consume(1).map_err(|e| MergeFailure {
            source: location,
            message: e.to_string(),
        })?;
        let Some(selected) = target.get("__flurry_stable_since") else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        if selected.entry.is_none() || selected.origin.configuration_key.is_some() {
            return Err(failure(
                "__flurry_stable_since cannot be attached via definition configuration; use ^attrs.stable_since",
            ));
        }
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure(
                "__flurry_stable_since requires a struct, enum, newtype, or normal-form definition",
            ));
        };
        let anchorable = package.item(owner).is_some_and(|item| match &item.kind {
            ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..) => true,
            ItemKind::Fn(sig, _) => sig.modifiers.is_normal_form_head,
            _ => false,
        });
        if !anchorable {
            return Err(failure(
                "__flurry_stable_since requires a struct, enum, newtype, or normal-form definition",
            ));
        }
        if selected.value.ty != p(P::Str) {
            return Err(failure("__flurry_stable_since requires the Str domain"));
        }
        let version = match &selected.value.kind {
            K::Literal(n::Literal::String(text)) => text.as_str(),
            K::ComptimeValue(Term::String(text)) => text.as_ref(),
            _ => {
                return Err(failure(
                    "__flurry_stable_since requires a completed str value",
                ));
            }
        };
        budget
            .consume(version.len().saturating_add(1))
            .map_err(|e| MergeFailure {
                source: location,
                message: e.to_string(),
            })?;
        if !is_valid_semver(version) {
            return Err(failure(
                "__flurry_stable_since requires a valid SemVer 2.0.0 version string (MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD])",
            ));
        }
        output.push(StableSinceEntry {
            owner,
            version: version.to_owned(),
            source: location,
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::is_valid_semver;

    #[test]
    fn validates_strict_semver_2_syntax() {
        for valid in [
            "0.0.0",
            "1.0.0",
            "1.2.3",
            "10.20.30",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-0.3.7",
            "1.0.0-x.7.z.92",
            "1.0.0-x-y-z.--",
            "1.0.0+20130313144700",
            "1.0.0-beta+exp.sha.5114f85",
            "1.0.0+001",
        ] {
            assert!(is_valid_semver(valid), "expected valid SemVer: {valid}");
        }
        for invalid in [
            "",
            "1",
            "1.0",
            "1.0.0.0",
            "v1.0.0",
            "01.0.0",
            "1.02.0",
            "1.0.03",
            "1.0.0-",
            "1.0.0+",
            "1.0.0-alpha.",
            "1.0.0-alpha..1",
            "1.0.0-01",
            "1.0.0-alpha_1",
            "1.0.0+build_1",
            " 1.0.0",
            "1.0.0 ",
        ] {
            assert!(
                !is_valid_semver(invalid),
                "expected invalid SemVer: {invalid}"
            );
        }
    }
}
