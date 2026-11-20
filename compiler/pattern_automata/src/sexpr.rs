//! Shared Lisp-style S-expression escaping and formatting helpers.

use std::fmt;

/// Write string contents with deterministic escaping so S-expression output
/// never contains unescaped quotes, backslashes, or raw newlines.
pub(crate) fn write_escaped_contents(f: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        if !matches!(ch, '"' | '\\' | '\n' | '\r' | '\t') && !ch.is_control() {
            continue;
        }
        f.write_str(&value[start..index])?;
        match ch {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            other => write!(f, "\\u{:04x}", other as u32)?,
        }
        start = index + ch.len_utf8();
    }
    f.write_str(&value[start..])
}

/// Format a string as a double-quoted, escaped S-expression string literal.
pub(crate) fn write_quoted_str(f: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    f.write_str("\"")?;
    write_escaped_contents(f, value)?;
    f.write_str("\"")
}

/// Format a single Unicode scalar value as a double-quoted escaped string.
pub(crate) fn write_quoted_char(f: &mut fmt::Formatter<'_>, ch: char) -> fmt::Result {
    let mut buf = [0u8; 4];
    let text = ch.encode_utf8(&mut buf);
    write_quoted_str(f, text)
}
