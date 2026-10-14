//! Shared S-expression string escaping for owned values and program IR.
use std::fmt;

pub(crate) fn string(f: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    f.write_str("\"")?;
    for ch in value.chars() {
        match ch {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c if c.is_control() => write!(f, "\\u{:04x}", c as u32)?,
            c => write!(f, "{c}")?,
        }
    }
    f.write_str("\"")
}
