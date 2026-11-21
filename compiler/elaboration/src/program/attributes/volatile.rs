//! Validate merged `__flurry_volatile` attributes on structs, newtypes, and struct fields.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VolatileTarget {
    Definition {
        owner: OwnerId,
        enabled: bool,
        source: n::SourceRange,
    },
    Field {
        owner: OwnerId,
        field_index: usize,
        enabled: bool,
        source: n::SourceRange,
    },
}

impl std::fmt::Display for VolatileTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Definition {
                owner,
                enabled,
                source,
            } => write!(
                f,
                "(volatile-target definition (owner {owner}) (enabled {enabled}) (source {} {}))",
                source.start, source.end
            ),
            Self::Field {
                owner,
                field_index,
                enabled,
                source,
            } => write!(
                f,
                "(volatile-target field (owner {owner}) (index {field_index}) (enabled {enabled}) (source {} {}))",
                source.start, source.end
            ),
        }
    }
}

/// Validate `__flurry_volatile` entries from merged attribute targets.
/// Rejects definition configurations (`.__flurry_volatile`), non-Bool payloads,
/// targets other than structs/newtypes/struct-fields, and `packed` combinations.
pub fn volatile_targets(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    packed_owners: &BTreeSet<OwnerId>,
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<VolatileTarget>, MergeFailure> {
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
        let Some(selected) = target.get("__flurry_volatile") else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        if selected.entry.is_none() || selected.origin.configuration_key.is_some() {
            return Err(failure(
                "__flurry_volatile cannot be attached via definition configuration; use ^attrs.mem.volatile or ^attrs.volatile",
            ));
        }
        if selected.value.ty != p(P::Bool) {
            return Err(failure("__flurry_volatile requires the Bool domain"));
        }
        let K::Literal(n::Literal::Bool(enabled)) = selected.value.kind else {
            return Err(failure("__flurry_volatile requires a completed Bool value"));
        };
        match target.target {
            hir::AttributeTarget::Definition(owner) => {
                if !package.item(owner).is_some_and(|item| {
                    matches!(item.kind, ItemKind::Struct(_) | ItemKind::Newtype(..))
                }) {
                    return Err(failure(
                        "__flurry_volatile requires a struct definition, newtype definition, or struct field",
                    ));
                }
                if enabled && packed_owners.contains(&owner) {
                    return Err(failure("packed structs cannot be marked volatile"));
                }
                output.push(VolatileTarget::Definition {
                    owner,
                    enabled,
                    source: location,
                });
            }
            hir::AttributeTarget::Member(hir_id) => {
                let owner = hir_id.owner;
                let Some(def) = package.item(owner).and_then(|item| match &item.kind {
                    ItemKind::Struct(def) => Some(def),
                    _ => None,
                }) else {
                    return Err(failure(
                        "__flurry_volatile requires a struct definition, newtype definition, or struct field",
                    ));
                };
                let Some(field_index) = def.fields.iter().position(|f| f.hir_id == hir_id) else {
                    return Err(failure(
                        "__flurry_volatile requires a struct definition, newtype definition, or struct field",
                    ));
                };
                if enabled && packed_owners.contains(&owner) {
                    return Err(failure("packed structs cannot be marked volatile"));
                }
                output.push(VolatileTarget::Field {
                    owner,
                    field_index,
                    enabled,
                    source: location,
                });
            }
        }
    }
    Ok(output)
}
