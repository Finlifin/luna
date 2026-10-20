//! Declaration domains for callable clause inputs, independent of executable
//! parameters and bodies. Runtime instantiation must retain value input sorts.
use crate::{DeclarationId, DeclarationParameter, Term, TypeError};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub declaration: DeclarationId,
    pub parameters: Vec<DeclarationId>,
    pub domains: Vec<Term>,
}

impl Declaration {
    pub fn binders(&self) -> Result<Vec<DeclarationParameter>, TypeError> {
        crate::declaration_parameters(&self.parameters, &self.domains)
    }
}

impl fmt::Display for Declaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(function-parameters (declaration {}) (inputs",
            self.declaration
        )?;
        for parameter in &self.parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (domains")?;
        for domain in &self.domains {
            write!(f, " {domain}")?;
        }
        f.write_str("))")
    }
}
