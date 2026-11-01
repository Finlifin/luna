//! Stable interface origin and instance inputs of a method value.
use crate::program::InterfaceInstance;
use crate::{DeclarationId, Term};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TraitMethodIdentity {
    pub interface: InterfaceInstance,
    pub subject: Term,
    pub member: DeclarationId,
    pub method_arguments: Vec<Term>,
}
impl fmt::Display for TraitMethodIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(runtime-trait-method {} (subject {}) (member {}) (method-arguments",
            self.interface, self.subject, self.member
        )?;
        for argument in &self.method_arguments {
            write!(f, " {argument}")?;
        }
        f.write_str("))")
    }
}
impl TraitMethodIdentity {
    /// Receiver first, then interface and method arguments in declaration order.
    pub fn terms(&self) -> impl Iterator<Item = &Term> {
        std::iter::once(&self.subject)
            .chain(&self.interface.arguments)
            .chain(&self.method_arguments)
    }
}
