//! Immutable register plan for source-ordered writable bit selections.
use super::*;
use nir::program::BitSelectionLayout as L;

#[derive(Debug)]
pub struct Input {
    pub register: Register,
    pub ty: Term,
    pub source: SourceRange,
}
impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(input %{} {} (source {} {}))",
            self.register, self.ty, self.source.start, self.source.end
        )
    }
}
#[derive(Debug)]
pub struct Plan {
    pub local: Register,
    pub fields: Vec<(FieldOwner, usize)>,
    pub storage: Term,
    pub value: Term,
    pub output: Term,
    pub selections: Vec<L<Input>>,
}
impl Plan {
    pub fn layouts(&self) -> impl Iterator<Item = L<&Term>> {
        self.selections.iter().map(|s| match s {
            L::Static { start, end } => L::Static {
                start: *start,
                end: *end,
            },
            L::Index(i) => L::Index(&i.ty),
            L::Window { start, end, width } => L::Window {
                start: &start.ty,
                end: &end.ty,
                width: *width,
            },
        })
    }
}
impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(bit-update (local %{}) (storage {}) (value {}) (output {}) (fields",
            self.local, self.storage, self.value, self.output
        )?;
        for (owner, index) in &self.fields {
            write!(f, " (field {owner} {index})")?;
        }
        write!(f, ") (selections")?;
        for selection in &self.selections {
            write!(f, " {selection}")?;
        }
        write!(f, "))")
    }
}
