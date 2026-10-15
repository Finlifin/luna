//! Logical trait atoms. Selection and absence evidence belong to a particular
//! implementation world and must be supplied by that world's proof adapter.
//! Merely naming an atom here grants neither an implementation nor its absence.
use nir::{DeclarationId, Sort, Term};
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

    /// A trait atom is a proposition about a Type, even when its interface is
    /// indexed by a comptime value (for example a literal suffix symbol).
    /// This validates logical inputs only; the world adapter must still check
    /// the declared parameter domains and implementation evidence.
    pub(super) fn validate_inputs(&self, scope: &[Vec<Sort>]) -> Result<(), String> {
        if self.subject.sort() != Sort::Type {
            return Err("trait proposition requires a Type subject".into());
        }
        for argument in &self.arguments {
            if !matches!(argument.sort(), Sort::Type | Sort::ComptimeValue) {
                return Err("trait proposition requires Type or comptime-value arguments".into());
            }
        }
        for term in self.inputs() {
            term.check_scope(scope)
                .map_err(|e| format!("invalid trait input: {e:?}"))?;
        }
        Ok(())
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
