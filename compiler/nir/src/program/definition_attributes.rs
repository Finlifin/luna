use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// A required invocation retains the attribute state of its own source world.
/// Before publication only annotated declarations have an outstanding dependency.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum DefinitionAttributes {
    #[default]
    Empty,
    Pending(Arc<BTreeSet<DeclarationId>>),
    /// Objects frozen for an early reader can be published by the same handle.
    Frozen(Arc<BTreeMap<DeclarationId, Arc<Term>>>),
}

impl DefinitionAttributes {
    pub fn lookup(&self, id: DeclarationId) -> Result<Option<&Term>, DeclarationId> {
        match self {
            Self::Pending(ids) if ids.contains(&id) => Err(id),
            Self::Empty | Self::Pending(_) => Ok(None),
            Self::Frozen(values) => Ok(values.get(&id).map(Arc::as_ref)),
        }
    }

    /// Inspect the completed calling-convention property in its owning world.
    pub fn has_c_abi(&self, id: DeclarationId, fuel: &mut usize) -> Result<bool, &'static str> {
        let object = self
            .lookup(id)
            .map_err(|_| "C ABI metadata is not available")?;
        let Some(Term::NominalValue { fields, .. }) = object else {
            return Ok(false);
        };
        *fuel = fuel
            .checked_sub(fields.len())
            .ok_or("C ABI metadata budget exhausted")?;
        Ok(fields.iter().any(|entry| {
            matches!(crate::objects::entry_parts(entry),
            Some((Some("__flurry_abi"), _, Term::Symbol(convention))) if convention.as_ref() == "c")
        }))
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Pending(ids) => ids.len(),
            Self::Frozen(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Display for DefinitionAttributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("(definition-attributes")?,
            Self::Pending(ids) => {
                f.write_str("(pending-definition-attributes")?;
                for id in ids.iter() {
                    write!(f, " {id}")?;
                }
            }
            Self::Frozen(values) => {
                f.write_str("(definition-attributes")?;
                for (id, object) in values.iter() {
                    write!(f, " (entry {id} {object})")?;
                }
            }
        }
        f.write_str(")")
    }
}
