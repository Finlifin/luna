//! Source draft failures retain their definition/member identity at the VM boundary.
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefinitionBuildError {
    FieldCount {
        names: usize,
        types: usize,
    },
    DuplicateField {
        owner: String,
        name: String,
    },
    DuplicateVariant {
        name: String,
    },
    VariantShape {
        name: String,
        shape: String,
        names: usize,
        types: usize,
    },
    SymbolCodeCount {
        variants: usize,
        codes: usize,
    },
    SymbolPayload {
        name: String,
    },
    SymbolEncoding {
        reason: String,
    },
    FixedEncoding {
        reason: String,
    },
    InvalidPath,
}

impl fmt::Display for DefinitionBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use nir::QuotedDisplay as Q;
        f.write_str("(definition-build-error ")?;
        match self {
            Self::FieldCount { names, types } => {
                write!(f, "(field-count (names {names}) (types {types}))")?
            }
            Self::DuplicateField { owner, name } => write!(
                f,
                "(duplicate-field (owner {}) (name {}))",
                Q(owner),
                Q(name)
            )?,
            Self::DuplicateVariant { name } => write!(f, "(duplicate-variant (name {}))", Q(name))?,
            Self::VariantShape {
                name,
                shape,
                names,
                types,
            } => write!(
                f,
                "(variant-shape (name {}) (shape {}) (names {names}) (types {types}))",
                Q(name),
                Q(shape)
            )?,
            Self::SymbolCodeCount { variants, codes } => write!(
                f,
                "(symbol-code-count (variants {variants}) (codes {codes}))"
            )?,
            Self::SymbolPayload { name } => write!(f, "(symbol-payload (name {}))", Q(name))?,
            Self::SymbolEncoding { reason } => {
                write!(f, "(symbol-encoding (reason {}))", Q(reason))?
            }
            Self::FixedEncoding { reason } => write!(f, "(fixed-encoding (reason {}))", Q(reason))?,
            Self::InvalidPath => {
                f.write_str("(invalid-path (expected nonempty-symbol-segments-without-dots))")?
            }
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn definition_errors_escape_names_and_retain_source_ranges() {
        let error = crate::Error {
            kind: crate::ErrorKind::InvalidDefinition(DefinitionBuildError::DuplicateField {
                owner: "owner\n\"".into(),
                name: "field\\name".into(),
            }),
            source: Some(nir::program::SourceRange { start: 3, end: 9 }),
        };
        assert_eq!(
            error.to_string(),
            "interpretation (definition-build-error (duplicate-field (owner \"owner\\n\\\"\") (name \"field\\\\name\"))) at (source 3 9)"
        );
    }
}
