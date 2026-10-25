//! Borrowed metadata selection. Selected expressions remain in their origin's
//! materialization scope; merging must not clone or detach closure environments.
use super::*;
use comptime_runtime::object::{MergeError, merge_named_entries};

#[derive(Clone, Copy, Debug)]
pub struct SelectedValue<'a> {
    pub key: &'a str,
    pub origin: &'a EvaluatedAttribute,
    /// None denotes the configuration value; Some identifies an Object entry.
    pub entry: Option<usize>,
    pub value: &'a n::Expr,
}

#[derive(Debug)]
pub struct MergedTarget<'a> {
    pub target: hir::AttributeTarget,
    values: BTreeMap<&'a str, SelectedValue<'a>>,
}
impl<'a> MergedTarget<'a> {
    pub fn get(&self, key: &str) -> Option<SelectedValue<'a>> {
        self.values.get(key).copied()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, SelectedValue<'a>)> + '_ {
        self.values.iter().map(|(key, value)| (*key, *value))
    }
}
impl std::fmt::Display for MergedTarget<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(merged-attributes (target {}) (properties", self.target)?;
        for (key, selected) in self.iter() {
            write!(f, " (property (key {key:?}) (entry")?;
            if let Some(entry) = selected.entry {
                write!(f, " {entry}")?;
            }
            // Describe the selection, not a detached executable expression.
            // The raw inspection retains its scope; avoid dumping that entire
            // Object again for every selected key (quadratic output).
            write!(
                f,
                ") (source {} {}) (lexical-owner {}))",
                selected.origin.source.start,
                selected.origin.source.end,
                selected.origin.lexical_owner
            )?;
        }
        f.write_str("))")
    }
}

#[derive(Debug)]
pub struct MergeFailure {
    pub source: n::SourceRange,
    pub message: String,
}
impl std::fmt::Display for MergeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(attribute-merge-error (source {} {}) (message {:?}))",
            self.source.start, self.source.end, self.message
        )
    }
}
impl std::error::Error for MergeFailure {}
fn failure(attribute: &EvaluatedAttribute, message: impl ToString) -> MergeFailure {
    MergeFailure {
        source: attribute.source,
        message: message.to_string(),
    }
}

/// Select named properties independently for each target. All configurations
/// apply before prefixes; prefix order is already inner-to-outer in HIR.
/// Positional entries have no named-property meaning and are rejected here,
/// while the original inspection data remains intact. Failure exposes no partial
/// result. Work and map comparisons consume the supplied shared budget.
pub fn merge<'a>(
    attributes: &'a [EvaluatedAttribute],
    max_keys_per_target: usize,
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<MergedTarget<'a>>, MergeFailure> {
    type Key = (OwnerId, Option<hir::ItemLocalId>);
    let mut groups: BTreeMap<
        Key,
        (
            hir::AttributeTarget,
            &EvaluatedAttribute,
            Vec<(&str, SelectedValue<'a>)>,
        ),
    > = BTreeMap::new();
    for configuration in [true, false] {
        for attribute in attributes {
            // Account for both scans, including entries skipped in this pass.
            budget.consume(1).map_err(|e| failure(attribute, e))?;
            if attribute.configuration_key.is_some() != configuration {
                continue;
            }
            let key = match attribute.target {
                hir::AttributeTarget::Definition(owner) => (owner, None),
                hir::AttributeTarget::Member(id) => (id.owner, Some(id.local_id)),
            };
            let levels = (usize::BITS - groups.len().leading_zeros()) as usize + 1;
            budget
                .consume(levels.saturating_mul(16))
                .map_err(|e| failure(attribute, e))?;
            let (_, _, entries) = groups
                .entry(key)
                .or_insert_with(|| (attribute.target, attribute, Vec::new()));
            if let Some(name) = &attribute.configuration_key {
                entries.push((
                    name,
                    SelectedValue {
                        key: name,
                        origin: attribute,
                        entry: None,
                        value: &attribute.value,
                    },
                ));
                continue;
            }
            let mut root = &attribute.value;
            while let K::Block {
                result: Some(result),
                ..
            } = &root.kind
            {
                budget.consume(1).map_err(|e| failure(attribute, e))?;
                root = result;
            }
            let K::Object { names, fields } = &root.kind else {
                return Err(failure(attribute, "prefix is not a materialized Object"));
            };
            if root.ty != p(P::Object) || names.len() != fields.len() {
                return Err(failure(attribute, "invalid materialized Object shape"));
            }
            budget
                .consume(fields.len())
                .map_err(|e| failure(attribute, e))?;
            for (index, (name, value)) in names.iter().zip(fields).enumerate() {
                let Some(name) = name else {
                    return Err(failure(
                        attribute,
                        format!("positional Object entry {index} is not a named attribute"),
                    ));
                };
                entries.push((
                    name,
                    SelectedValue {
                        key: name,
                        origin: attribute,
                        entry: Some(index),
                        value,
                    },
                ));
            }
        }
    }
    let mut result = Vec::new();
    for (_, (target, first, entries)) in groups {
        // Capture the failing insertion's provenance without cloning payloads.
        let mut current = None;
        let merged = merge_named_entries(
            entries.iter().map(|(key, selected)| {
                current = Some(selected.origin);
                Ok::<_, MergeError>((*key, selected))
            }),
            max_keys_per_target,
            budget,
        )
        .map_err(|error| {
            if let Some(origin) = current {
                failure(origin, error)
            } else {
                failure(first, error)
            }
        })?;
        result.push(MergedTarget {
            target,
            values: merged
                .iter()
                .map(|(_, value)| (value.key, *value))
                .collect(),
        });
    }
    Ok(result)
}
