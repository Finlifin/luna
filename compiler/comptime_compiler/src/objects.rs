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

/// Immutable operation metadata; operand code remains in the instruction stream.
#[derive(Debug)]
pub enum ObjectOperation {
    Query(nir::program::ObjectQueryKind),
    Edit(nir::program::ObjectEditKind),
}

#[derive(Debug)]
pub struct ObjectAccess {
    pub operation: ObjectOperation,
    pub ty: Term,
    pub input: Term,
}
impl fmt::Display for ObjectAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.operation {
            ObjectOperation::Query(query) => write!(f, "(object-query {query}")?,
            ObjectOperation::Edit(edit) => write!(f, "(object-edit {edit}")?,
        }
        write!(f, " (type {}) (input {}))", self.ty, self.input)
    }
}
