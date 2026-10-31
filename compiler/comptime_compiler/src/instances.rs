//! Immutable declaration-instance metadata; clause computation stays in code.
use nir::{DeclarationId, Term};
use std::fmt;

#[derive(Debug)]
pub struct Constructor {
    pub declaration: DeclarationId,
    pub input: Term,
    pub output: Term,
}
impl fmt::Display for Constructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(instance-constructor {} (input {}) (output {}))",
            self.declaration, self.input, self.output
        )
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
