//! Lexical source evidence for a member use, before implementation selection.
use crate::Symbol;
use std::fmt;
use symbol::DefId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitSourceName {
    pub name: Symbol,
    /// None denotes an ordinary binding that may shadow an outer trait.
    pub fallback: Result<Option<DefId>, String>,
    /// Exact trait identities in an ambiguous spelling; None means no such evidence.
    pub ambiguous_traits: Option<Vec<DefId>>,
    /// Original file/node identities, innermost first; not generated HIR ids.
    pub pending_exports: Vec<(u32, u32)>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraitSourcePlan {
    pub names: Vec<TraitSourceName>,
    /// Unknown generated names make even an empty snapshot incomplete.
    pub pending_exports: Vec<(u32, u32)>,
}
impl TraitSourcePlan {
    /// Bound inspection/cloning before walking an externally assembled plan.
    pub fn charge(&self, fuel: &mut usize) -> Result<(), &'static str> {
        let mut spend = |count| -> Result<(), &'static str> {
            *fuel = fuel
                .checked_sub(count)
                .ok_or("trait source budget exhausted")?;
            Ok(())
        };
        spend(self.names.len().saturating_add(self.pending_exports.len()))?;
        for name in &self.names {
            spend(name.pending_exports.len())?;
            spend(name.ambiguous_traits.as_ref().map_or(0, Vec::len))?;
            if let Err(error) = &name.fallback {
                spend(error.len())?;
            }
        }
        Ok(())
    }
}
fn sites(f: &mut fmt::Formatter<'_>, values: &[(u32, u32)]) -> fmt::Result {
    for (file, node) in values {
        write!(f, " (site (file {file}) (node {node}))")?;
    }
    Ok(())
}
impl fmt::Display for TraitSourcePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(trait-source-plan (names")?;
        for name in &self.names {
            write!(f, " (name {:?} (fallback ", name.name.as_str())?;
            match &name.fallback {
                Ok(Some(id)) => write!(f, "(def {} {})", id.pkg, id.index)?,
                Ok(None) => f.write_str("ordinary")?,
                Err(error) => write!(f, "(error {error:?})")?,
            }
            f.write_str(")")?;
            if let Some(traits) = &name.ambiguous_traits {
                f.write_str(" (ambiguous-traits")?;
                for id in traits {
                    write!(f, " (def {} {})", id.pkg, id.index)?;
                }
                f.write_str(")")?;
            }
            f.write_str(" (pending-exports")?;
            sites(f, &name.pending_exports)?;
            f.write_str("))")?;
        }
        f.write_str(") (pending-exports")?;
        sites(f, &self.pending_exports)?;
        f.write_str("))")
    }
}
