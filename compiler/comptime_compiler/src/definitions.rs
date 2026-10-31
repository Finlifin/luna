//! Checked definition publication descriptors; operands remain ordinary code.
use super::*;

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Struct,
    Enum,
}
impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Struct => "struct",
            Self::Enum => "enum",
        })
    }
}
#[derive(Debug)]
pub struct Build<T = Term> {
    pub kind: Kind,
    pub output: T,
}
impl<T: fmt::Display> fmt::Display for Build<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(definition-build {} (output {}))",
            self.kind, self.output
        )
    }
}
