//! Opaque normal-form construction metadata; no definition body is executed.
use super::*;

#[derive(Debug)]
pub struct Constructor {
    pub head: nir::DeclarationId,
    pub ty: Term,
    pub count: usize,
    pub diverges: bool,
}
impl fmt::Display for Constructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(normal-type (head {}) (type {}) (inputs {}) (diverges {}))",
            self.head, self.ty, self.count, self.diverges
        )
    }
}

/// Contextual scalar inputs keep their spelling until the preceding inputs
/// determine the domain. Expression code remains outside this metadata.
#[derive(Debug)]
pub enum Input {
    Value(Term),
    Unit,
    Integer { spelling: String, negative: bool },
}
impl Input {
    pub fn diverges(&self) -> bool {
        matches!(self, Self::Value(Term::Primitive(Primitive::Never)))
    }
}
impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(ty) => write!(f, "(value {ty})"),
            Self::Unit => write!(f, "(unit)"),
            Self::Integer { spelling, negative } => {
                write!(f, "(integer {spelling:?} (negative {negative}))")
            }
        }
    }
}
