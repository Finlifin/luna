//! Owned closure metadata. Capture expressions stay in the instruction stream;
//! the body is compiled through ordinary invocation when the closure is called.
use super::*;

#[derive(Debug)]
pub struct Constructor {
    pub ty: Term,
    pub parameters: Vec<LocalId>,
    pub captures: Vec<(LocalId, Term)>,
    pub body: Arc<Expr>,
}

impl fmt::Display for Constructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(closure-constructor (type {}) (parameters", self.ty)?;
        for parameter in &self.parameters {
            write!(f, " {parameter}")?;
        }
        write!(f, ") (captures")?;
        for (binding, ty) in &self.captures {
            write!(f, " ({binding} {ty})")?;
        }
        write!(f, ") (body {}))", self.body)
    }
}
