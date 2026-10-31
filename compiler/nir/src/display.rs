//! Shared S-expression string escaping for owned values and program IR.
use std::fmt::{self, Write};

fn contents(writer: &mut (impl fmt::Write + ?Sized), value: &str) -> fmt::Result {
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        if !matches!(ch, '"' | '\\' | '\n' | '\r' | '\t') && !ch.is_control() {
            continue;
        }
        writer.write_str(&value[start..index])?;
        match ch {
            '"' => writer.write_str("\\\"")?,
            '\\' => writer.write_str("\\\\")?,
            '\n' => writer.write_str("\\n")?,
            '\r' => writer.write_str("\\r")?,
            '\t' => writer.write_str("\\t")?,
            c => write!(writer, "\\u{:04x}", c as u32)?,
        }
        start = index + ch.len_utf8();
    }
    writer.write_str(&value[start..])
}

pub(crate) fn string(f: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    f.write_str("\"")?;
    contents(f, value)?;
    f.write_str("\"")
}

/// Quote a Display value directly, without materializing its complete text.
/// Formatting chunks use exactly the same escaping as NIR string literals.
pub struct QuotedDisplay<'a, T: fmt::Display + ?Sized>(pub &'a T);

impl<T: fmt::Display + ?Sized> fmt::Display for QuotedDisplay<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Escaped<'a, 'b>(&'a mut fmt::Formatter<'b>);
        impl fmt::Write for Escaped<'_, '_> {
            fn write_str(&mut self, value: &str) -> fmt::Result {
                contents(self.0, value)
            }
        }
        f.write_str("\"")?;
        write!(Escaped(f), "{}", self.0)?;
        f.write_str("\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Chunks;
    impl fmt::Display for Chunks {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            for text in ["中文", "\"", "\\", "\n", "\r\t", "\0\u{85}", "end"] {
                f.write_str(text)?;
            }
            Ok(())
        }
    }

    #[test]
    fn chunked_display_matches_literal_escaping() {
        let expected = "\"中文\\\"\\\\\\n\\r\\t\\u0000\\u0085end\"";
        assert_eq!(QuotedDisplay(&Chunks).to_string(), expected);
        assert_eq!(
            crate::program::Literal::String(Chunks.to_string()).to_string(),
            format!("(string {expected})")
        );
        assert_eq!(QuotedDisplay("").to_string(), "\"\"");
    }
    #[test]
    fn failed_output_stops_the_display_producer_without_buffering_the_payload() {
        struct Producer<'a>(&'a std::cell::Cell<usize>);
        impl fmt::Display for Producer<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                for _ in 0..10_000 {
                    self.0.set(self.0.get() + 1);
                    f.write_str("part\n")?;
                }
                Ok(())
            }
        }
        struct Limited(usize);
        impl fmt::Write for Limited {
            fn write_str(&mut self, text: &str) -> fmt::Result {
                self.0 = self.0.checked_sub(text.len()).ok_or(fmt::Error)?;
                Ok(())
            }
        }
        let produced = std::cell::Cell::new(0);
        assert!(write!(Limited(32), "{}", QuotedDisplay(&Producer(&produced))).is_err());
        assert!(produced.get() > 0 && produced.get() < 10);
    }
}
