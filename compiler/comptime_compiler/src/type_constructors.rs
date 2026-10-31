//! Owned metadata for structural Type construction, separate from operand code.
use super::*;

#[derive(Debug)]
pub enum Kind {
    Function {
        comptime: bool,
        parameters: Vec<bool>,
    },
    Result,
    Optional,
    Pointer,
    Closure,
}
impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Function {
                comptime,
                parameters,
            } => {
                write!(f, "(function (comptime {comptime}) (parameters")?;
                for staged in parameters {
                    write!(f, " {staged}")?;
                }
                write!(f, "))")
            }
            Self::Result => write!(f, "(result)"),
            Self::Optional => write!(f, "(optional)"),
            Self::Pointer => write!(f, "(pointer)"),
            Self::Closure => write!(f, "(closure)"),
        }
    }
}
#[derive(Debug)]
pub struct Constructor {
    pub kind: Kind,
    pub ty: Term,
    pub inputs: Vec<Term>,
}
impl fmt::Display for Constructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(type-constructor {} (type {}) (inputs",
            self.kind, self.ty
        )?;
        for ty in &self.inputs {
            write!(f, " {ty}")?;
        }
        write!(f, "))")
    }
}
