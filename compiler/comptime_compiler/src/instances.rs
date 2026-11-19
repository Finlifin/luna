//! Immutable declaration-instance metadata; clause computation stays in code.
use nir::{DeclarationId, Term};
use std::fmt;

#[derive(Debug)]
pub struct Constructor {
    /// Only concrete graph code carries this link. Source calls can compute
    /// previously unseen instances in their declaration world.
    pub link: Option<std::sync::Arc<[Term]>>,
    pub declaration: DeclarationId,
    pub arity: usize,
    pub output: Term,
}
impl fmt::Display for Constructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(instance-constructor {} (arity {})",
            self.declaration, self.arity
        )?;
        if let Some(inputs) = &self.link {
            write!(f, " (graph-link")?;
            for term in inputs.iter() {
                write!(f, " {term}")?;
            }
            write!(f, ")")?;
        }
        write!(f, " (output {}))", self.output)
    }
}

/// Completed clause arguments, still subject to formation contracts at each use.
#[derive(Debug)]
pub struct Function {
    pub declaration: DeclarationId,
    pub arguments: Vec<Term>,
    pub output: Term,
}
impl fmt::Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(function-instance {} (arguments", self.declaration)?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") (output {}))", self.output)
    }
}
