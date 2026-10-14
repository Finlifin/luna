//! Logical trait atoms. Selection and absence evidence belong to a particular
//! implementation world and must be supplied by that world's proof adapter.
//! Merely naming an atom here grants neither an implementation nor its absence.
use nir::{DeclarationId, Term};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TraitQuery {
    pub interface: DeclarationId,
    pub arguments: Vec<Term>,
    pub subject: Term,
}

impl TraitQuery {
    pub fn inputs(&self) -> impl Iterator<Item = &Term> {
        std::iter::once(&self.subject).chain(&self.arguments)
    }
}

impl From<&nir::program::TraitRequirement> for TraitQuery {
    fn from(requirement: &nir::program::TraitRequirement) -> Self {
        Self {
            interface: requirement.interface,
            arguments: requirement.interface_arguments.clone(),
            subject: requirement.subject.clone(),
        }
    }
}

impl fmt::Display for TraitQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(trait-bound (interface {}) (subject {}) (arguments",
            self.interface, self.subject
        )?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        f.write_str("))")
    }
}
