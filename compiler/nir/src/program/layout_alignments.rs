use crate::{DeclarationId, display::string};
use std::collections::BTreeMap;
use std::fmt;

/// Explicit alignment overrides (in bytes, positive powers of two) derived from
/// checked `__flurry_align` attributes (`^attrs.layout.align(bytes)`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutAlignments {
    pub declarations: BTreeMap<DeclarationId, u64>,
    pub fields: BTreeMap<(DeclarationId, usize), u64>,
}

impl LayoutAlignments {
    pub fn is_empty(&self) -> bool {
        self.declarations.is_empty() && self.fields.is_empty()
    }

    pub fn entry_count(&self) -> usize {
        self.declarations
            .len()
            .saturating_add(self.fields.len())
            .saturating_add(1)
    }

    pub fn declaration_align(&self, declaration: DeclarationId) -> Option<u64> {
        self.declarations.get(&declaration).copied()
    }

    pub fn field_align(&self, declaration: DeclarationId, index: usize) -> Option<u64> {
        self.fields.get(&(declaration, index)).copied()
    }
}

impl fmt::Display for LayoutAlignments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(layout-alignments (declarations")?;
        for (decl, align) in &self.declarations {
            write!(f, " (align {decl} {align})")?;
        }
        f.write_str(") (fields")?;
        for ((decl, index), align) in &self.fields {
            write!(f, " (field-align {decl} {index} {align})")?;
        }
        f.write_str("))")
    }
}

#[allow(dead_code)]
fn _keep_string_import() -> fn(&mut fmt::Formatter<'_>, &str) -> fmt::Result {
    string
}
