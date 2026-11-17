//! Information available to ordinary source-definition checking. Unknown means
//! that this observer has no domain evidence; it is not an explicit Any value,
//! a pending computation owner, an inference variable or a runtime type.
use nir::Term;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TypeObservation {
    Unknown,
    Known(Term),
}

impl fmt::Display for TypeObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("(unknown-type-observation)"),
            Self::Known(ty) => write!(f, "(known-type-observation {ty})"),
        }
    }
}
