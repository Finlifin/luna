//! UTF-8 <-> UTF-16 LSP position conversion, incremental buffer edits, and file URIs.

use std::path::{Path, PathBuf};

use crate::json::JsonValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

impl LspPosition {
    pub fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }

    pub fn from_json(v: &JsonValue) -> Option<Self> {
        Some(Self {
            line: v.get_u32("line")?,
            character: v.get_u32("character")?,
        })
    }

    pub fn to_json(self) -> JsonValue {
        JsonValue::object([
            ("line", JsonValue::from(self.line)),
            ("character", JsonValue::from(self.character)),
        ])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

impl LspRange {
    pub fn new(start: LspPosition, end: LspPosition) -> Self {
        Self { start, end }
    }

    pub fn from_json(v: &JsonValue) -> Option<Self> {
        Some(Self {
            start: LspPosition::from_json(v.get("start")?)?,
            end: LspPosition::from_json(v.get("end")?)?,
        })
    }

    pub fn to_json(self) -> JsonValue {
        JsonValue::object([("start", self.start.to_json()), ("end", self.end.to_json())])
    }
}

#[derive(Debug, Clone)]
pub struct LineIndex {
    /// Byte offset of the start of each line in `text`. Always begins with `0`.
    line_starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (idx, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(idx + 1);
            }
        }
        Self {
            line_starts,
            len: text.len(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Convert a UTF-8 byte offset inside `text` to an LSP UTF-16 `LspPosition`.
    pub fn offset_to_position(&self, text: &str, offset: usize) -> LspPosition {
        let clamped = offset.min(text.len());
        let line_idx = match self.line_starts.binary_search(&clamped) {
            Ok(exact) => exact,
            Err(insert) => insert.saturating_sub(1),
        };
        let line_start = self.line_starts[line_idx];
        let mut clamped_boundary = clamped;
        while clamped_boundary > line_start && !text.is_char_boundary(clamped_boundary) {
            clamped_boundary -= 1;
        }
        let slice = &text[line_start..clamped_boundary];
        let utf16_col: usize = slice.chars().map(char::len_utf16).sum();
        LspPosition {
            line: line_idx as u32,
            character: utf16_col as u32,
        }
    }

    /// Convert an LSP UTF-16 `LspPosition` to a UTF-8 byte offset inside `text`.
    pub fn position_to_offset(&self, text: &str, pos: LspPosition) -> usize {
        let line_idx = pos.line as usize;
        if line_idx >= self.line_starts.len() {
            return self.len.min(text.len());
        }
        let line_start = self.line_starts[line_idx];
        let line_end = self
            .line_starts
            .get(line_idx + 1)
            .copied()
            .unwrap_or(text.len());
        let line_slice = &text[line_start..line_end];
        let line_body = line_slice.strip_suffix('\n').unwrap_or(line_slice);
        let line_body = line_body.strip_suffix('\r').unwrap_or(line_body);

        let target_utf16 = pos.character as usize;
        let mut seen_utf16 = 0usize;
        for (byte_rel, ch) in line_body.char_indices() {
            if seen_utf16 >= target_utf16 {
                return line_start + byte_rel;
            }
            seen_utf16 += ch.len_utf16();
        }
        line_start + line_body.len()
    }

    pub fn span_to_range(&self, text: &str, lo: usize, hi: usize) -> LspRange {
        let start = self.offset_to_position(text, lo);
        let end = self.offset_to_position(text, hi.max(lo));
        LspRange { start, end }
    }

    pub fn full_range(&self, text: &str) -> LspRange {
        self.span_to_range(text, 0, text.len())
    }
}

/// Apply LSP `contentChanges` (supporting both incremental UTF-16 ranges and full replacement).
pub fn apply_content_changes(text: &mut String, changes: &[JsonValue]) {
    for change in changes {
        let Some(new_text) = change.get_str("text") else {
            continue;
        };
        if let Some(range_val) = change.get("range")
            && !range_val.is_null()
            && let Some(range) = LspRange::from_json(range_val)
        {
            let index = LineIndex::new(text);
            let start = index.position_to_offset(text, range.start);
            let end = index.position_to_offset(text, range.end).max(start);
            text.replace_range(start..end, new_text);
        } else {
            text.clear();
            text.push_str(new_text);
        }
    }
}

/// Convert a `file://` URI into a filesystem `PathBuf`.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path_part = if let Some(stripped) = rest.strip_prefix("localhost/") {
        format!("/{stripped}")
    } else if rest.starts_with('/') {
        rest.to_owned()
    } else {
        return None;
    };
    let decoded = percent_decode(&path_part)?;
    // Handle Windows-like `/C:/...` just in case, while keeping Unix `/path/...` intact.
    if decoded.len() >= 3
        && decoded.as_bytes()[0] == b'/'
        && decoded.as_bytes()[1].is_ascii_alphabetic()
        && decoded.as_bytes()[2] == b':'
    {
        Some(PathBuf::from(&decoded[1..]))
    } else {
        Some(PathBuf::from(decoded))
    }
}

/// Convert a filesystem `Path` into a normalized `file://` URI.
pub fn path_to_uri(path: &Path) -> String {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let raw = abs.to_string_lossy();
    let mut out = String::from("file://");
    if !raw.starts_with('/') {
        out.push('/');
    }
    for &b in raw.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = *bytes.get(i + 1)?;
            let lo = *bytes.get(i + 2)?;
            let val = (hex_val(hi)? << 4) | hex_val(lo)?;
            out.push(val);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_positions_and_incremental_edits_handle_multibyte_text() {
        let mut text = "-- 中文注释 🚀\nlet x = 1;\n".to_owned();
        let idx = LineIndex::new(&text);
        // '-- ' = 3 UTF-16, '中文注释 ' = 5 UTF-16, '🚀' = 2 UTF-16 -> end of line 0 is col 10
        assert_eq!(
            idx.offset_to_position(&text, text.find('\n').unwrap()),
            LspPosition::new(0, 10)
        );

        let changes = vec![JsonValue::object([
            (
                "range",
                LspRange::new(LspPosition::new(1, 8), LspPosition::new(1, 9)).to_json(),
            ),
            ("text", JsonValue::from("42")),
        ])];
        apply_content_changes(&mut text, &changes);
        assert_eq!(text, "-- 中文注释 🚀\nlet x = 42;\n");
    }

    #[test]
    fn uri_round_trip_handles_spaces() {
        let p = PathBuf::from("/tmp/my project/main.fl");
        let uri = path_to_uri(&p);
        assert_eq!(uri, "file:///tmp/my%20project/main.fl");
        assert_eq!(uri_to_path(&uri), Some(p));
    }
}
