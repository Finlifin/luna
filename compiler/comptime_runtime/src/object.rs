//! Ordered comptime Object data and the named-property attribute merge.
//!
//! Values remain generic: this container does not authorize staging, freeze a
//! value, infer its type, or execute an attribute consumer. Evaluators must do
//! that work before handing completed values to the merge operation.
use crate::Budget;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry<V> {
    Value(V),
    Property { key: String, value: V },
}
impl<V> Entry<V> {
    pub fn value(&self) -> &V {
        match self {
            Self::Value(value) | Self::Property { value, .. } => value,
        }
    }
    pub fn into_value(self) -> V {
        match self {
            Self::Value(value) | Self::Property { value, .. } => value,
        }
    }
    pub fn key(&self) -> Option<&str> {
        match self {
            Self::Value(_) => None,
            Self::Property { key, .. } => Some(key),
        }
    }
}

/// Preserve expression order and duplicate keys until a consumer interprets
/// them. This is distinct from a record type and from the merged attribute map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object<V> {
    entries: Vec<Entry<V>>,
}
impl<V> Object<V> {
    /// Adopt already allocated/evaluated entries without copying their values.
    /// The producer remains responsible for its allocation and evaluation budget.
    pub fn from_entries(entries: Vec<Entry<V>>) -> Self {
        Self { entries }
    }
    pub fn entries(&self) -> &[Entry<V>] {
        &self.entries
    }
    pub fn into_entries(self) -> Vec<Entry<V>> {
        self.entries
    }
}
impl<V: fmt::Display> fmt::Display for Object<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(object")?;
        for entry in &self.entries {
            match entry {
                Entry::Value(value) => write!(f, " (value {value})")?,
                Entry::Property { key, value } => {
                    write!(f, " (property (key {key:?}) (value {value}))")?
                }
            }
        }
        f.write_str(")")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeError {
    BudgetExhausted,
    EntryLimit,
    /// The named-property projection does not assign a meaning to positional
    /// Object entries. Keep the original outer-to-inner object and entry index.
    PositionalEntry {
        object: usize,
        entry: usize,
    },
}
impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BudgetExhausted => f.write_str("(attribute-merge-error budget-exhausted)"),
            Self::EntryLimit => f.write_str("(attribute-merge-error entry-limit)"),
            Self::PositionalEntry { object, entry } => write!(
                f,
                "(attribute-merge-error (positional-entry (object {object}) (entry {entry})))"
            ),
        }
    }
}
impl std::error::Error for MergeError {}

/// Immutable borrowed results: merging never clones values or mutates an input.
/// A value may be large or non-Clone, and may require a separate freezing check.
#[derive(Debug)]
pub struct Attributes<'a, V> {
    values: BTreeMap<&'a str, &'a V>,
}
impl<'a, V> Attributes<'a, V> {
    pub fn get(&self, key: &str) -> Option<&'a V> {
        self.values.get(key).copied()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, &'a V)> + '_ {
        self.values.iter().map(|(key, value)| (*key, *value))
    }
}
impl<V: fmt::Display> fmt::Display for Attributes<'_, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(attributes")?;
        for (key, value) in self.iter() {
            write!(f, " (property (key {key:?}) (value {value}))")?;
        }
        f.write_str(")")
    }
}

/// Insert one application-order entry after charging comparison work.
/// On error the map is unchanged, while spent budget is retained. Callers
/// publishing a complete merge must discard their private map on failure.
/// Values are moved into place; neither payload cloning nor a staging list
/// is required. The limit counts unique keys, so overrides remain permitted.
pub fn insert_named_entry<'a, V>(
    values: &mut BTreeMap<&'a str, V>,
    key: &'a str,
    value: V,
    limit: usize,
    budget: &mut Budget,
) -> Result<(), MergeError> {
    // Ordered-map lookup uses O(log n) comparisons, each bounded by the
    // searched key's byte length plus one. Cover lookup and insertion.
    let levels = (usize::BITS - values.len().leading_zeros()) as usize + 1;
    let work = key
        .len()
        .saturating_add(1)
        .saturating_mul(levels)
        .saturating_mul(16);
    budget
        .consume(work)
        .map_err(|_| MergeError::BudgetExhausted)?;
    if values.len() >= limit && !values.contains_key(key) {
        return Err(MergeError::EntryLimit);
    }
    values.insert(key, value);
    Ok(())
}

/// Apply configuration first, then prefix Objects from inner to outer. Within
/// one Object, process properties in their retained expression order. This is
/// the named-property part of the attribute protocol, not general Object lookup
/// semantics or an evaluation order for attribute expressions.
///
/// Charge entries and a conservative key-comparison cost before each insertion.
/// Limit unique keys independently of fuel. Rejection returns no partial map and
/// leaves all input values untouched; spent fuel is not restored.
pub fn merge_named_attributes<'a, V>(
    configuration: impl IntoIterator<Item = (&'a str, &'a V)>,
    prefixes_outer_first: &[&'a Object<V>],
    max_entries: usize,
    budget: &mut Budget,
) -> Result<Attributes<'a, V>, MergeError> {
    budget.consume(1).map_err(|_| MergeError::BudgetExhausted)?;
    let mut values = BTreeMap::new();
    for (key, value) in configuration {
        insert_named_entry(&mut values, key, value, max_entries, budget)?;
    }
    for (object_index, object) in prefixes_outer_first.iter().enumerate().rev() {
        budget.consume(1).map_err(|_| MergeError::BudgetExhausted)?;
        for (entry_index, entry) in object.entries().iter().enumerate() {
            match entry {
                Entry::Property { key, value } => {
                    insert_named_entry(&mut values, key, value, max_entries, budget)?
                }
                Entry::Value(_) => {
                    return Err(MergeError::PositionalEntry {
                        object: object_index,
                        entry: entry_index,
                    });
                }
            }
        }
    }
    Ok(Attributes { values })
}

/// Merge a caller-supplied application-order stream without cloning payloads.
/// The caller retains provenance in V and is responsible for selecting the
/// configuration-first, inner-to-outer sequence. Iterator errors abort the merge.
pub fn merge_named_entries<'a, V: 'a>(
    entries: impl IntoIterator<Item = Result<(&'a str, &'a V), MergeError>>,
    max_entries: usize,
    budget: &mut Budget,
) -> Result<Attributes<'a, V>, MergeError> {
    budget.consume(1).map_err(|_| MergeError::BudgetExhausted)?;
    let mut values = BTreeMap::new();
    for entry in entries {
        let (key, value) = entry?;
        insert_named_entry(&mut values, key, value, max_entries, budget)?;
    }
    Ok(Attributes { values })
}
