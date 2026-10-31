//! Typed metadata/text observations with admission before operand evaluation.
use super::*;

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    EnumName,
    StructFields { names: bool },
    EnumStructure(nir::program::EnumStructureQuery),
    SymbolName,
    Text(nir::program::TextQueryKind),
}
impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StructFields { names } => {
                write!(f, "struct-field-{}", if *names { "names" } else { "types" })
            }
            Self::EnumStructure(query) => write!(f, "(enum-structure {query})"),
            Self::EnumName => write!(f, "enum-name"),
            Self::SymbolName => write!(f, "symbol-name"),
            Self::Text(query) => write!(f, "(text {query})"),
        }
    }
}
#[derive(Debug)]
pub struct Observation<T = Term> {
    pub kind: Kind,
    pub input: T,
    pub output: T,
    pub operand_source: SourceRange,
}
impl<T: fmt::Display> fmt::Display for Observation<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(observation {} (input {}) (output {}) (operand-source {} {}))",
            self.kind, self.input, self.output, self.operand_source.start, self.operand_source.end
        )
    }
}
