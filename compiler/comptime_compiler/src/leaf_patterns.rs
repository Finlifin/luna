//! Operand-free leaf tests. Binding, rollback and structure traversal are instructions.
use crate::Error;
use nir::program::{Literal, Pattern, SourceRange};
use std::fmt;

#[derive(Debug)]
pub enum Leaf {
    Wild,
    Unit,
    OptionalAbsent,
    Literal(Literal),
}
impl Leaf {
    pub(crate) fn copy(
        pattern: &Pattern,
        fuel: &mut usize,
        source: SourceRange,
    ) -> Result<Self, Error> {
        if !matches!(
            pattern,
            Pattern::Wild | Pattern::Unit | Pattern::OptionalAbsent | Pattern::Literal(_)
        ) {
            return Err(Error::Unsupported(source));
        }
        let mut budget = *fuel as u64;
        let charged = nir::copy_budget::charge_pattern_rewrite(pattern, &mut budget);
        *fuel = budget as usize;
        charged.map_err(|_| Error::Budget(source))?;
        Ok(match pattern {
            Pattern::Wild => Self::Wild,
            Pattern::Unit => Self::Unit,
            Pattern::OptionalAbsent => Self::OptionalAbsent,
            Pattern::Literal(value) => Self::Literal(value.clone()),
            _ => unreachable!(),
        })
    }
}
impl fmt::Display for Leaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wild => f.write_str("(wild)"),
            Self::Unit => f.write_str("(unit)"),
            Self::OptionalAbsent => f.write_str("(optional-absent)"),
            Self::Literal(value) => write!(f, "(literal {value})"),
        }
    }
}
