//! Owned entry metadata for ordered heterogeneous construction.
use super::*;

#[derive(Debug)]
pub struct ObjectConstructor {
    pub ty: Term,
    pub names: Vec<Option<String>>,
    pub fields: Vec<Term>,
}
impl fmt::Display for ObjectConstructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(object-constructor (type {}) (names", self.ty)?;
        for name in &self.names {
            match name {
                Some(name) => write!(f, " (name {})", nir::QuotedDisplay(name))?,
                None => write!(f, " (positional)")?,
            }
        }
        write!(f, ") (fields")?;
        for ty in &self.fields {
            write!(f, " {ty}")?;
        }
        write!(f, "))")
    }
}
