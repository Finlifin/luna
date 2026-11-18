//! Immutable declaration-instance metadata; clause computation stays in code.
use nir::{DeclarationId, Term};
use std::fmt;

#[derive(Debug)]
pub struct Constructor {
    pub declaration: DeclarationId,
    pub operands: Vec<(crate::Register, Term)>,
    pub output: Term,
}
impl fmt::Display for Constructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(instance-constructor {} (operands", self.declaration)?;
        for (register, domain) in &self.operands {
            write!(f, " (%{register} {domain})")?;
        }
        write!(f, ") (output {}))", self.output)
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
