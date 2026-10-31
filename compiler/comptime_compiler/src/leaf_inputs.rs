//! Read-only local or literal selectors for borrowed aggregate operations.
use super::*;
#[derive(Debug)]
pub enum Selector {
    Local(Register, SourceRange),
    Literal(Arc<Expr>),
}
impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(slot, source) => write!(
                f,
                "(local %{slot} (source {} {}))",
                source.start, source.end
            ),
            Self::Literal(value) => write!(f, "(literal {value})"),
        }
    }
}
