//! Immutable, origin-bound build inputs carried by compiled code.
use super::*;
pub type Entry = (String, u64, Option<Term>);
#[derive(Debug)]
pub struct Snapshot {
    pub owner: u64,
    pub output: Term,
    pub entries: Vec<Entry>,
}
impl fmt::Display for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(build-metadata (owner {}) (output {}) (entries",
            self.owner, self.output
        )?;
        for (alias, provider, value) in &self.entries {
            write!(
                f,
                " (entry {} (provider {}) ",
                nir::QuotedDisplay(alias),
                provider
            )?;
            if let Some(value) = value {
                write!(f, "(value {value})")?;
            } else {
                write!(f, "(unavailable)")?;
            }
            write!(f, ")")?;
        }
        write!(f, "))")
    }
}
