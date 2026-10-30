//! The immediate shape of a value pattern, independent of its child programs.
use crate::{DeclarationId, Term, program::Pattern};
use std::fmt;

#[derive(Clone, Copy, Debug)]
pub enum Structure<T = Term, N = Vec<String>> {
    Tuple {
        fields: usize,
    },
    Record {
        declaration: DeclarationId,
        fields: usize,
    },
    Variant {
        declaration: DeclarationId,
        index: usize,
        fields: usize,
    },
    AnonymousRecord {
        names: N,
        fields: usize,
    },
    OptionalPresent,
    ErrorResult {
        label: Option<T>,
    },
}

pub type View<'a> = Structure<&'a Term, &'a [String]>;

impl<'a> View<'a> {
    pub fn from_pattern(pattern: &'a Pattern) -> Option<(Self, &'a [Pattern])> {
        Some(match pattern {
            Pattern::Tuple(fields) => (
                Self::Tuple {
                    fields: fields.len(),
                },
                fields,
            ),
            Pattern::Record {
                declaration,
                fields,
            } => (
                Self::Record {
                    declaration: *declaration,
                    fields: fields.len(),
                },
                fields,
            ),
            Pattern::Variant {
                declaration,
                index,
                fields,
            } => (
                Self::Variant {
                    declaration: *declaration,
                    index: *index,
                    fields: fields.len(),
                },
                fields,
            ),
            Pattern::AnonymousRecord { names, fields } => (
                Self::AnonymousRecord {
                    names,
                    fields: fields.len(),
                },
                fields,
            ),
            Pattern::OptionalPresent(inner) => (Self::OptionalPresent, std::slice::from_ref(inner)),
            Pattern::ErrorResult { label, inner } => (
                Self::ErrorResult {
                    label: label.as_ref(),
                },
                std::slice::from_ref(inner),
            ),
            _ => return None,
        })
    }

    /// Callers meter the owned metadata copy; no child pattern is copied.
    pub fn to_owned(self) -> Structure {
        match self {
            Self::Tuple { fields } => Structure::Tuple { fields },
            Self::Record {
                declaration,
                fields,
            } => Structure::Record {
                declaration,
                fields,
            },
            Self::Variant {
                declaration,
                index,
                fields,
            } => Structure::Variant {
                declaration,
                index,
                fields,
            },
            Self::AnonymousRecord { names, fields } => Structure::AnonymousRecord {
                names: names.to_vec(),
                fields,
            },
            Self::OptionalPresent => Structure::OptionalPresent,
            Self::ErrorResult { label } => Structure::ErrorResult {
                label: label.cloned(),
            },
        }
    }
}
impl Structure {
    pub fn view(&self) -> View<'_> {
        match self {
            Self::Tuple { fields } => View::Tuple { fields: *fields },
            Self::Record {
                declaration,
                fields,
            } => View::Record {
                declaration: *declaration,
                fields: *fields,
            },
            Self::Variant {
                declaration,
                index,
                fields,
            } => View::Variant {
                declaration: *declaration,
                index: *index,
                fields: *fields,
            },
            Self::AnonymousRecord { names, fields } => View::AnonymousRecord {
                names,
                fields: *fields,
            },
            Self::OptionalPresent => View::OptionalPresent,
            Self::ErrorResult { label } => View::ErrorResult {
                label: label.as_ref(),
            },
        }
    }
}
impl<T: fmt::Display, N: AsRef<[String]>> fmt::Display for Structure<T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tuple { fields } => write!(f, "(tuple-pattern-shape (fields {fields}))"),
            Self::Record {
                declaration,
                fields,
            } => write!(f, "(record-pattern-shape {declaration} (fields {fields}))"),
            Self::Variant {
                declaration,
                index,
                fields,
            } => write!(
                f,
                "(variant-pattern-shape {declaration} (index {index}) (fields {fields}))"
            ),
            Self::AnonymousRecord { names, fields } => {
                write!(f, "(anonymous-pattern-shape (fields {fields}) (names")?;
                for name in names.as_ref() {
                    write!(f, " {}", crate::QuotedDisplay(name))?;
                }
                write!(f, "))")
            }
            Self::OptionalPresent => write!(f, "(optional-pattern-shape)"),
            Self::ErrorResult { label } => {
                write!(f, "(error-pattern-shape (label")?;
                if let Some(label) = label {
                    write!(f, " {label}")?;
                }
                write!(f, "))")
            }
        }
    }
}
