//! List descriptors retain ordinary declared domains; no library spelling is used.
use super::*;
#[derive(Debug)]
pub struct Construction {
    pub ty: Term,
    pub inputs: Vec<Term>,
}
impl fmt::Display for Construction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(list-construction (type {}) (inputs", self.ty)?;
        for input in &self.inputs {
            write!(f, " {input}")?;
        }
        write!(f, "))")
    }
}
#[derive(Debug)]
pub struct Operation {
    pub ty: Term,
    pub left: Term,
    pub right: Option<Term>,
}
impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(list-operation (type {}) (left {}) (right",
            self.ty, self.left
        )?;
        if let Some(right) = &self.right {
            write!(f, " {right}")?;
        }
        write!(f, "))")
    }
}

#[derive(Debug)]
pub struct Index {
    pub ty: Term,
    pub list: Term,
    pub index: Term,
}
impl fmt::Display for Index {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(list-index-contract (type {}) (list {}) (index {}))",
            self.ty, self.list, self.index
        )
    }
}
