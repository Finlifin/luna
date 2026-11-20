//! Deterministic Lisp-style S-expression formatting helpers.
//!
//! All text atoms are quoted and escaped so serialized IR, automata, plans,
//! proofs, and reports never emit raw newlines or unescaped delimiters.

use std::fmt::{self, Display, Write};

/// Formats a string slice as a double-quoted, escaped S-expression atom.
pub struct Quoted<'a>(pub &'a str);

impl Display for Quoted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_char('"')?;
        for ch in self.0.chars() {
            match ch {
                '"' => f.write_str("\\\"")?,
                '\\' => f.write_str("\\\\")?,
                '\n' => f.write_str("\\n")?,
                '\r' => f.write_str("\\r")?,
                '\t' => f.write_str("\\t")?,
                c if c.is_control() => write!(f, "\\u{:04x}", c as u32)?,
                c => f.write_char(c)?,
            }
        }
        f.write_char('"')
    }
}

/// Writes a space-separated list of `Display` items wrapped in parentheses.
pub fn write_list<T: Display>(f: &mut fmt::Formatter<'_>, items: &[T]) -> fmt::Result {
    f.write_char('(')?;
    for (idx, item) in items.iter().enumerate() {
        if idx != 0 {
            f.write_char(' ')?;
        }
        item.fmt(f)?;
    }
    f.write_char(')')
}

/// Formats an optional value as `(some ...)` or `(none)`.
pub struct Maybe<'a, T>(pub Option<&'a T>);

impl<T: Display> Display for Maybe<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(val) => write!(f, "(some {val})"),
            None => f.write_str("(none)"),
        }
    }
}
