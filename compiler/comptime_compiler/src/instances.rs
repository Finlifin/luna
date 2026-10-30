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
