//! Pre-lexing UTF-8 scanner for `@use` macro imports and Mode 4 (`^macro4` / `@builtin.str.raw`) raw-source calls.
use crate::imports::{MacroScope, parse_at_use, scan_ident_at, skip_ws_and_comments};
use crate::materialize::tokens_to_macro_tokens;
use crate::provenance::{ExpansionKind, ExpansionMap, MacroToken, TokenSpacing, is_separated_kind};
use crate::registry::{ExpansionContext, MacroInputMode};
use crate::{MacroBudget, MacroError, MacroErrorKind};
use lex::{LexError, TokenKind};
use rustc_span::{BytePos, Span};
use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;

/// Result of running the Mode 4 & `@use` pre-lexing scanner on a source string.
pub struct RawSourcePassResult {
    pub tokens: Vec<MacroToken>,
    pub lex_errors: Vec<LexError>,
    pub modified: bool,
    pub scope: MacroScope,
}

/// Summary of macro roots referenced in a single source file (used for lazy provider preparation).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReferencedMacros {
    pub has_non_builtin_macros: bool,
    pub roots: BTreeSet<String>,
    pub import_roots: BTreeSet<String>,
}

pub(crate) fn encode_flurry_string_literal(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            '$' => out.push_str("\\$"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7F => {
                let _ = write!(out, "\\x{{{:02X}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn horizontal_indent(line: &str) -> &str {
    let end = line
        .bytes()
        .position(|b| !matches!(b, b' ' | b'\t'))
        .unwrap_or(line.len());
    &line[..end]
}

fn is_horizontal_ws_only(line: &str) -> bool {
    line.bytes().all(|b| matches!(b, b' ' | b'\t'))
}

fn common_ws_prefix<'a>(a: &'a str, b: &str) -> &'a str {
    let common = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    &a[..common]
}

/// Dedent a `@builtin.str.raw_aligned { ... }` body: skip a blank leading line,
/// strip common horizontal indentation across all non-blank lines, and omit
/// horizontal-only indentation on the closing `}` line while keeping the
/// preceding line ending.
pub fn align_raw_string(raw: &str) -> String {
    let mut lines: Vec<(&str, &str)> = Vec::new();
    let bytes = raw.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            lines.push((&raw[start..i], "\r\n"));
            i += 2;
            start = i;
        } else if bytes[i] == b'\n' {
            lines.push((&raw[start..i], "\n"));
            i += 1;
            start = i;
        } else {
            i += 1;
        }
    }
    lines.push((&raw[start..], ""));

    let mut slice = lines.as_slice();
    if let Some(&(first_content, first_ending)) = slice.first()
        && !first_ending.is_empty()
        && is_horizontal_ws_only(first_content)
    {
        slice = &slice[1..];
    }

    if let Some(&(last_content, last_ending)) = slice.last()
        && last_ending.is_empty()
        && is_horizontal_ws_only(last_content)
    {
        slice = &slice[..slice.len() - 1];
    }

    let mut common_indent: Option<&str> = None;
    for &(content, _) in slice {
        if is_horizontal_ws_only(content) {
            continue;
        }
        let indent = horizontal_indent(content);
        common_indent = Some(match common_indent {
            None => indent,
            Some(prev) => common_ws_prefix(prev, indent),
        });
    }
    let common_indent = common_indent.unwrap_or("");

    let mut out = String::with_capacity(raw.len());
    for &(content, ending) in slice {
        if let Some(rest) = content.strip_prefix(common_indent) {
            out.push_str(rest);
        } else if !is_horizontal_ws_only(content) {
            out.push_str(content);
        }
        out.push_str(ending);
    }
    out
}

fn scan_raw_balanced_braces(
    src: &str,
    open_brace: usize,
    base_pos: BytePos,
) -> Result<(usize, usize), MacroError> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    debug_assert_eq!(bytes[open_brace], b'{');
    let content_start = open_brace + 1;
    let mut pos = content_start;
    let mut depth = 1usize;
    while pos < len && depth > 0 {
        match bytes[pos] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((content_start, pos));
                }
            }
            _ => {}
        }
        pos += 1;
    }
    Err(MacroError::new(
        MacroErrorKind::UnterminatedMacroInput,
        "unterminated raw macro input `{ ... }`; expected closing `}`",
        Span::new(
            base_pos + BytePos(open_brace as u32),
            base_pos + BytePos((open_brace + 1).min(len) as u32),
        ),
    ))
}

fn skip_block_comment(src: &str, start: usize) -> usize {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut pos = start + 2;
    let mut depth = 1usize;
    while pos < len && depth > 0 {
        if bytes[pos] == b'{' && bytes.get(pos + 1) == Some(&b'-') {
            depth += 1;
            pos += 2;
        } else if bytes[pos] == b'-' && bytes.get(pos + 1) == Some(&b'}') {
            depth -= 1;
            pos += 2;
        } else {
            pos += 1;
        }
    }
    pos
}

fn skip_line_comment(src: &str, start: usize) -> usize {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut pos = start + 2;
    while pos < len && bytes[pos] != b'\n' {
        pos += 1;
    }
    pos
}

fn try_skip_char_literal(src: &str, start: usize) -> Option<usize> {
    let len = src.len();
    let mut pos = start + 1;
    let ch = src.get(pos..)?.chars().next()?;
    if ch == '\n' || ch == '\r' || ch == '\'' {
        return None;
    }
    if ch == '\\' {
        pos += 1;
        let esc = src.get(pos..)?.chars().next()?;
        match esc {
            '\\' | '\'' | '"' | 'n' | 'r' | 't' | '0' | '$' => {
                pos += 1;
            }
            'x' => {
                pos += 1;
                if src.as_bytes().get(pos) != Some(&b'{') {
                    return None;
                }
                pos += 1;
                let hex_start = pos;
                while pos < len && src.as_bytes()[pos] != b'}' {
                    let b = src.as_bytes()[pos];
                    if !b.is_ascii_hexdigit() {
                        return None;
                    }
                    pos += 1;
                }
                if pos == hex_start || pos - hex_start > 6 || src.as_bytes().get(pos) != Some(&b'}')
                {
                    return None;
                }
                pos += 1;
            }
            _ => return None,
        }
    } else {
        pos += ch.len_utf8();
    }
    if src.as_bytes().get(pos) == Some(&b'\'') {
        Some(pos + 1)
    } else {
        None
    }
}

fn skip_string_literal(src: &str, start: usize) -> usize {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut pos = start + 1;
    while pos < len {
        let b = bytes[pos];
        if b == b'"' {
            return pos + 1;
        }
        if b == b'\n' {
            // Unterminated on this line; let normal lexer diagnose it cleanly.
            return pos;
        }
        if b == b'\\' {
            pos += 1;
            if pos < len {
                let ch = src[pos..].chars().next().unwrap();
                pos += ch.len_utf8();
            }
            continue;
        }
        if b == b'$' && bytes.get(pos + 1) == Some(&b'{') {
            pos = skip_string_interpolation(src, pos + 2);
            continue;
        }
        let ch = src[pos..].chars().next().unwrap();
        pos += ch.len_utf8();
    }
    pos
}

fn skip_string_interpolation(src: &str, start: usize) -> usize {
    skip_balanced_flurry_braces(src, start).unwrap_or(src.len())
}

fn skip_balanced_flurry_braces(src: &str, after_open_brace: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut pos = after_open_brace;
    let mut depth = 1usize;
    let mut can_end = false;
    while pos < len && depth > 0 {
        let b = bytes[pos];
        if b == b'-' && bytes.get(pos + 1) == Some(&b'-') {
            pos = skip_line_comment(src, pos);
            continue;
        }
        if b == b'{' && bytes.get(pos + 1) == Some(&b'-') {
            pos = skip_block_comment(src, pos);
            continue;
        }
        if b == b'"' {
            pos = skip_string_literal(src, pos);
            can_end = true;
            continue;
        }
        if b == b'\'' {
            if !can_end {
                if let Some(end) = try_skip_char_literal(src, pos) {
                    pos = end;
                    can_end = true;
                    continue;
                }
            }
            pos += 1;
            can_end = false;
            continue;
        }
        if b == b'{' {
            depth += 1;
            can_end = false;
            pos += 1;
            continue;
        }
        if b == b'}' {
            depth -= 1;
            can_end = true;
            pos += 1;
            if depth == 0 {
                return Some(pos);
            }
            continue;
        }
        if let Some((_, end)) = scan_ident_at(src, pos) {
            can_end = true;
            pos = end;
            continue;
        }
        let ch = src[pos..].chars().next().unwrap();
        can_end = matches!(b, b')' | b']') || b.is_ascii_digit();
        pos += ch.len_utf8();
    }
    None
}

fn skip_ws_and_comments_lossy(src: &str, mut pos: usize) -> usize {
    let bytes = src.as_bytes();
    let len = bytes.len();
    while pos < len {
        let b = bytes[pos];
        if b == b'-' && bytes.get(pos + 1) == Some(&b'-') {
            pos = skip_line_comment(src, pos);
            continue;
        }
        if b == b'{' && bytes.get(pos + 1) == Some(&b'-') {
            pos = skip_block_comment(src, pos);
            continue;
        }
        let ch = src[pos..].chars().next().unwrap();
        if ch.is_whitespace() {
            pos += ch.len_utf8();
            continue;
        }
        break;
    }
    pos
}

/// If `after_define` immediately follows a top-level/statement `define` keyword, skip a valid
/// template macro header + body `{ ... }` so unexpanded template bodies are not pre-expanded.
fn try_skip_define_template_body(src: &str, after_define: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let pos = skip_ws_and_comments_lossy(src, after_define);
    let (_, after_name) = scan_ident_at(src, pos)?;
    let mut cursor = skip_ws_and_comments_lossy(src, after_name);
    match *bytes.get(cursor)? {
        b'(' => {
            cursor += 1;
            let mut paren_depth = 1usize;
            while cursor < bytes.len() && paren_depth > 0 {
                cursor = skip_ws_and_comments_lossy(src, cursor);
                if cursor >= bytes.len() {
                    return None;
                }
                match bytes[cursor] {
                    b'(' => paren_depth += 1,
                    b')' => paren_depth -= 1,
                    b'{' => return None,
                    _ => {}
                }
                cursor += src[cursor..].chars().next()?.len_utf8();
            }
            if paren_depth != 0 {
                return None;
            }
            cursor = skip_ws_and_comments_lossy(src, cursor);
        }
        b'0'..=b'9' => {
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            cursor = skip_ws_and_comments_lossy(src, cursor);
            // Skip any trailing repetition markers or count separators before `{` without swallowing `;`.
            while cursor < bytes.len() && bytes[cursor] != b'{' {
                if bytes[cursor] == b';' || bytes[cursor] == b'\n' {
                    return None;
                }
                cursor += src[cursor..].chars().next()?.len_utf8();
                cursor = skip_ws_and_comments_lossy(src, cursor);
            }
        }
        _ => return None,
    }
    if bytes.get(cursor) != Some(&b'{') {
        return None;
    }
    skip_balanced_flurry_braces(src, cursor + 1)
}

fn skip_token_buffer_literal(src: &str, start: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    if start + 4 > len || bytes[start] != b'{' || bytes[start + 1] != b'{' {
        return None;
    }
    let mut pos = start + 2;
    let mut depth = 0usize;
    let mut can_end = false;
    while pos < len {
        let b = bytes[pos];
        if b == b'-' && bytes.get(pos + 1) == Some(&b'-') {
            pos = skip_line_comment(src, pos);
            continue;
        }
        if b == b'{' && bytes.get(pos + 1) == Some(&b'-') {
            pos = skip_block_comment(src, pos);
            continue;
        }
        if b == b'"' {
            pos = skip_string_literal(src, pos);
            can_end = true;
            continue;
        }
        if b == b'\'' {
            if !can_end {
                if let Some(end) = try_skip_char_literal(src, pos) {
                    pos = end;
                    can_end = true;
                    continue;
                }
            }
            pos += 1;
            can_end = false;
            continue;
        }
        if depth == 0 && b == b'}' && bytes.get(pos + 1) == Some(&b'}') {
            return Some(pos + 2);
        }
        if b == b'{' {
            depth += 1;
            can_end = false;
            pos += 1;
            continue;
        }
        if b == b'}' {
            depth = depth.saturating_sub(1);
            can_end = true;
            pos += 1;
            continue;
        }
        let ch = src[pos..].chars().next().unwrap();
        can_end = matches!(b, b')' | b']') || ch.is_alphanumeric() || b == b'_';
        pos += ch.len_utf8();
    }
    None
}

fn token_buffer_disallowed_after_word(word: Option<&str>) -> bool {
    matches!(
        word,
        Some(
            "match"
                | "do"
                | "when"
                | "else"
                | "struct"
                | "enum"
                | "union"
                | "trait"
                | "impl"
                | "extend"
                | "mod"
                | "unsafe"
                | "async"
                | "comptime"
        )
    )
}

fn word_can_end_expr(word: &str) -> bool {
    matches!(
        word,
        "true" | "false" | "null" | "undefined" | "self" | "Self" | "Itself" | "_"
    ) || !lex::Token::keywords().contains_key(word)
}

fn spacing_for_gap(gap: &str) -> TokenSpacing {
    if gap.is_empty() {
        TokenSpacing::Joint
    } else if gap.contains('\n') {
        TokenSpacing::Newline
    } else {
        TokenSpacing::Alone
    }
}

fn scan_call_path(
    src: &str,
    first_id: String,
    mut pos: usize,
    base_pos: BytePos,
) -> Result<(Vec<String>, usize), MacroError> {
    let bytes = src.as_bytes();
    let mut segments = vec![first_id];
    loop {
        let after_ws = skip_ws_and_comments(src, pos, base_pos)?;
        if bytes.get(after_ws) != Some(&b'.') {
            break;
        }
        let after_dot = skip_ws_and_comments(src, after_ws + 1, base_pos)?;
        let Some((seg, seg_end)) = scan_ident_at(src, after_dot) else {
            break;
        };
        segments.push(seg);
        pos = seg_end;
    }
    Ok((segments, pos))
}

struct OutputBuilder<'a> {
    src: &'a str,
    base_pos: BytePos,
    out_tokens: Vec<MacroToken>,
    lex_errors: Vec<LexError>,
    pending_replacement_tail: Option<usize>,
    pending_stripped_cut: bool,
    seen_tokens: bool,
    keywords_set: HashSet<&'static str>,
}

impl<'a> OutputBuilder<'a> {
    fn new(src: &'a str, base_pos: BytePos) -> Self {
        Self {
            src,
            base_pos,
            out_tokens: Vec::new(),
            lex_errors: Vec::new(),
            pending_replacement_tail: None,
            pending_stripped_cut: false,
            seen_tokens: false,
            keywords_set: lex::Token::keywords().keys().copied().collect(),
        }
    }

    fn flush_slice(
        &mut self,
        start: usize,
        end: usize,
        next_token_start_override: Option<usize>,
        force_non_joint_after: bool,
    ) {
        if start > end {
            return;
        }
        let slice = &self.src[start..end];
        let slice_base = self.base_pos + BytePos(start as u32);
        let (lexed, _, mut errs) = lex::lex(slice, slice_base);
        self.lex_errors.append(&mut errs);

        let mut slice_tokens = tokens_to_macro_tokens(&lexed, slice, slice_base);
        if let Some(first) = slice_tokens.first() {
            let first_offset = first.span.lo().0.saturating_sub(self.base_pos.0) as usize;
            if let Some(repl_end) = self.pending_replacement_tail.take() {
                if let Some(prev) = self.out_tokens.last_mut() {
                    let gap = self.src.get(repl_end..first_offset).unwrap_or(" ");
                    prev.spacing = if is_separated_kind(prev.kind) {
                        TokenSpacing::Separated
                    } else {
                        spacing_for_gap(gap)
                    };
                }
            } else if self.pending_stripped_cut {
                self.pending_stripped_cut = false;
                if let Some(prev) = self.out_tokens.last_mut() {
                    let prev_end = prev.span.hi().0.saturating_sub(self.base_pos.0) as usize;
                    let gap = self.src.get(prev_end..first_offset).unwrap_or(" ");
                    prev.spacing = if is_separated_kind(prev.kind) {
                        TokenSpacing::Separated
                    } else if gap.contains('\n') {
                        TokenSpacing::Newline
                    } else {
                        TokenSpacing::Alone
                    };
                }
            }
        }

        if let Some(last) = slice_tokens.last_mut() {
            if let Some(next_start) = next_token_start_override {
                let last_end = last.span.hi().0.saturating_sub(self.base_pos.0) as usize;
                let gap = self.src.get(last_end..next_start).unwrap_or(" ");
                last.spacing = if is_separated_kind(last.kind) {
                    TokenSpacing::Separated
                } else if force_non_joint_after {
                    if gap.contains('\n') {
                        TokenSpacing::Newline
                    } else {
                        TokenSpacing::Alone
                    }
                } else {
                    spacing_for_gap(gap)
                };
            }
        }

        if !slice_tokens.is_empty() {
            self.seen_tokens = true;
            self.pending_replacement_tail = None;
            self.pending_stripped_cut = false;
        }
        self.out_tokens.extend(slice_tokens);
    }

    fn push_replacement(
        &mut self,
        mut tokens: Vec<MacroToken>,
        action_start: usize,
        action_end: usize,
    ) {
        if tokens.is_empty() {
            self.pending_stripped_cut = true;
            return;
        }
        if let Some(repl_end) = self.pending_replacement_tail.take() {
            if let Some(prev) = self.out_tokens.last_mut() {
                let gap = self.src.get(repl_end..action_start).unwrap_or(" ");
                prev.spacing = if is_separated_kind(prev.kind) {
                    TokenSpacing::Separated
                } else {
                    spacing_for_gap(gap)
                };
            }
        } else if self.pending_stripped_cut {
            self.pending_stripped_cut = false;
            if let Some(prev) = self.out_tokens.last_mut() {
                let prev_end = prev.span.hi().0.saturating_sub(self.base_pos.0) as usize;
                let gap = self.src.get(prev_end..action_start).unwrap_or(" ");
                prev.spacing = if is_separated_kind(prev.kind) {
                    TokenSpacing::Separated
                } else if gap.contains('\n') {
                    TokenSpacing::Newline
                } else {
                    TokenSpacing::Alone
                };
            }
        }
        if let Some(last) = tokens.last_mut() {
            if last.spacing != TokenSpacing::Separated {
                last.spacing = TokenSpacing::Newline;
            }
        }
        self.seen_tokens = true;
        self.out_tokens.extend(tokens);
        self.pending_replacement_tail = Some(action_end);
    }

    fn push_stripped(&mut self) {
        self.pending_stripped_cut = true;
    }

    #[allow(dead_code)]
    fn is_keyword(&self, word: &str) -> bool {
        self.keywords_set.contains(word)
    }
}

struct RawScanCursor<'a> {
    src: &'a str,
    bytes: &'a [u8],
    len: usize,
    pos: usize,
    can_end_expr: bool,
    prev_was_dot: bool,
    prev_word: Option<String>,
    in_use_stmt: bool,
    use_brace_depth: usize,
    use_can_end: bool,
    skip_define_bodies: bool,
}

impl<'a> RawScanCursor<'a> {
    fn new(src: &'a str, skip_define_bodies: bool) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            len: src.len(),
            pos: 0,
            can_end_expr: false,
            prev_was_dot: false,
            prev_word: None,
            in_use_stmt: false,
            use_brace_depth: 0,
            use_can_end: false,
            skip_define_bodies,
        }
    }

    fn jump(
        &mut self,
        pos: usize,
        can_end_expr: bool,
        prev_was_dot: bool,
        prev_word: Option<String>,
    ) {
        self.pos = pos;
        self.can_end_expr = can_end_expr;
        self.prev_was_dot = prev_was_dot;
        self.prev_word = prev_word;
    }

    /// Advance through comments, strings, char literals, take-views, `{{ ... }}`,
    /// ordinary `use` paths, and (when configured) `define` bodies until the next `@` token.
    fn next_at(&mut self) -> Option<usize> {
        while self.pos < self.len {
            let b = self.bytes[self.pos];

            if b == b'-' && self.bytes.get(self.pos + 1) == Some(&b'-') {
                self.pos = skip_line_comment(self.src, self.pos);
                continue;
            }
            if b == b'{' && self.bytes.get(self.pos + 1) == Some(&b'-') {
                self.pos = skip_block_comment(self.src, self.pos);
                continue;
            }

            if b == b'"' {
                self.pos = skip_string_literal(self.src, self.pos);
                self.can_end_expr = true;
                self.prev_was_dot = false;
                self.prev_word = None;
                continue;
            }

            if b == b'\'' {
                if !self.can_end_expr {
                    if let Some(end) = try_skip_char_literal(self.src, self.pos) {
                        self.pos = end;
                        self.can_end_expr = true;
                        self.prev_was_dot = false;
                        self.prev_word = None;
                        continue;
                    }
                }
                self.pos += 1;
                self.can_end_expr = false;
                self.prev_was_dot = false;
                self.prev_word = None;
                continue;
            }

            if b == b'{'
                && self.bytes.get(self.pos + 1) == Some(&b'{')
                && !token_buffer_disallowed_after_word(self.prev_word.as_deref())
            {
                if let Some(end) = skip_token_buffer_literal(self.src, self.pos) {
                    self.pos = end;
                    self.can_end_expr = true;
                    self.prev_was_dot = false;
                    self.prev_word = None;
                    continue;
                }
            }

            if b == b'@' && !self.in_use_stmt {
                return Some(self.pos);
            }

            if b == b'.' {
                self.prev_was_dot = true;
                self.can_end_expr = false;
                self.use_can_end = false;
                self.prev_word = None;
                self.pos += 1;
                continue;
            }

            if b == b'{' {
                if self.in_use_stmt {
                    self.use_brace_depth += 1;
                    self.use_can_end = false;
                }
                self.can_end_expr = false;
                self.prev_was_dot = false;
                self.prev_word = None;
                self.pos += 1;
                continue;
            }

            if b == b'}' {
                if self.in_use_stmt && self.use_brace_depth > 0 {
                    self.use_brace_depth -= 1;
                    if self.use_brace_depth == 0 {
                        self.in_use_stmt = false;
                        self.use_can_end = false;
                    }
                }
                self.can_end_expr = true;
                self.prev_was_dot = false;
                self.prev_word = None;
                self.pos += 1;
                continue;
            }

            if b == b';' {
                if self.in_use_stmt && self.use_brace_depth == 0 {
                    self.in_use_stmt = false;
                    self.use_can_end = false;
                }
                self.can_end_expr = false;
                self.prev_was_dot = false;
                self.prev_word = None;
                self.pos += 1;
                continue;
            }

            if let Some((word, end)) = scan_ident_at(self.src, self.pos) {
                if self.skip_define_bodies
                    && word == "define"
                    && !self.prev_was_dot
                    && !self.in_use_stmt
                {
                    if let Some(after_define) = try_skip_define_template_body(self.src, end) {
                        self.jump(after_define, false, false, None);
                        continue;
                    }
                }
                if word == "use" && !self.in_use_stmt && !self.prev_was_dot {
                    self.in_use_stmt = true;
                    self.use_brace_depth = 0;
                    self.use_can_end = false;
                    self.can_end_expr = false;
                } else {
                    if self.in_use_stmt {
                        self.use_can_end = word != "as";
                    }
                    if self.prev_was_dot && word == "await" {
                        self.can_end_expr = true;
                    } else if word == "ref" {
                        self.can_end_expr = self.prev_was_dot;
                    } else {
                        self.can_end_expr = word_can_end_expr(&word);
                    }
                }
                self.prev_was_dot = false;
                self.prev_word = Some(word);
                self.pos = end;
                continue;
            }

            if b == b'*' && self.prev_was_dot {
                if self.in_use_stmt {
                    self.use_can_end = true;
                }
                self.can_end_expr = true;
                self.prev_was_dot = false;
                self.prev_word = None;
                self.pos += 1;
                continue;
            }

            if b == b',' && self.in_use_stmt {
                self.use_can_end = false;
                self.can_end_expr = false;
                self.prev_was_dot = false;
                self.prev_word = None;
                self.pos += 1;
                continue;
            }

            if matches!(b, b'!' | b'?' | b'#' | b'>') {
                self.prev_was_dot = false;
                self.prev_word = None;
                self.pos += 1;
                continue;
            }

            if matches!(b, b')' | b']') || b.is_ascii_digit() {
                self.can_end_expr = true;
                self.prev_was_dot = false;
                self.prev_word = None;
                self.pos += 1;
                continue;
            }

            let ch = self.src[self.pos..].chars().next().unwrap();
            if ch == '\n' && self.in_use_stmt && self.use_brace_depth == 0 && self.use_can_end {
                self.in_use_stmt = false;
                self.use_can_end = false;
            }
            if ch.is_whitespace() {
                self.pos += ch.len_utf8();
                continue;
            }

            self.can_end_expr = false;
            self.prev_was_dot = false;
            self.prev_word = None;
            self.pos += ch.len_utf8();
        }
        None
    }
}

fn collect_at_use_scan_targets(
    src: &str,
    mut pos: usize,
    leaves: &mut HashSet<String>,
    has_glob: &mut bool,
) -> usize {
    let bytes = src.as_bytes();
    let len = bytes.len();
    loop {
        pos = skip_ws_and_comments_lossy(src, pos);
        if pos >= len {
            break;
        }
        if bytes[pos] == b'*' {
            *has_glob = true;
            pos += 1;
            break;
        }
        if bytes[pos] == b'{' {
            pos += 1;
            loop {
                pos = skip_ws_and_comments_lossy(src, pos);
                if pos >= len {
                    return pos;
                }
                if bytes[pos] == b'}' {
                    pos += 1;
                    break;
                }
                pos = collect_at_use_scan_targets(src, pos, leaves, has_glob);
                pos = skip_ws_and_comments_lossy(src, pos);
                if bytes.get(pos) == Some(&b',') {
                    pos += 1;
                    continue;
                }
                if bytes.get(pos) == Some(&b'}') {
                    pos += 1;
                    break;
                }
                break;
            }
            break;
        }
        let Some((seg, seg_end)) = scan_ident_at(src, pos) else {
            break;
        };
        let after_seg = skip_ws_and_comments_lossy(src, seg_end);
        if bytes.get(after_seg) == Some(&b'.') {
            pos = after_seg + 1;
            continue;
        }
        if let Some((kw, kw_end)) = scan_ident_at(src, after_seg) {
            if kw == "as" {
                let alias_pos = skip_ws_and_comments_lossy(src, kw_end);
                if let Some((alias, alias_end)) = scan_ident_at(src, alias_pos) {
                    leaves.insert(alias);
                    pos = alias_end;
                    break;
                }
            }
        }
        leaves.insert(seg);
        pos = seg_end;
        break;
    }
    pos
}

fn collect_path_segments_lossy(
    src: &str,
    after_first: usize,
    first_seg: String,
) -> (Vec<String>, usize) {
    let mut segments = vec![first_seg];
    let mut scan_end = after_first;
    loop {
        let ws_end = skip_ws_and_comments_lossy(src, scan_end);
        if src.as_bytes().get(ws_end) != Some(&b'.') {
            break;
        }
        let next_pos = skip_ws_and_comments_lossy(src, ws_end + 1);
        let Some((seg, seg_end)) = scan_ident_at(src, next_pos) else {
            break;
        };
        segments.push(seg);
        scan_end = seg_end;
    }
    (segments, scan_end)
}

fn is_meta_macros_import_path(src: &str, root_pos: usize) -> bool {
    let Some((root_seg, root_end)) = scan_ident_at(src, root_pos) else {
        return false;
    };
    if root_seg != "meta" {
        return false;
    }
    let ws1 = skip_ws_and_comments_lossy(src, root_end);
    if src.as_bytes().get(ws1) != Some(&b'.') {
        return false;
    }
    let second_pos = skip_ws_and_comments_lossy(src, ws1 + 1);
    matches!(scan_ident_at(src, second_pos), Some((ref seg, _)) if seg == "macros")
}

/// Fast read-only pre-scan collecting macro package/script roots referenced in a source string.
pub fn scan_referenced_macro_roots(src: &str) -> ReferencedMacros {
    let mut out = ReferencedMacros::default();
    if !src.contains('@') {
        return out;
    }

    let mut builtin_leaves = HashSet::new();
    let mut has_builtin_glob = false;
    let mut has_meta_macros_glob = false;
    let mut non_builtin_leaves = HashSet::new();
    let mut has_non_builtin_glob = false;

    let mut cursor = RawScanCursor::new(src, false);
    while let Some(at_pos) = cursor.next_at() {
        let Some((first_seg, after_first)) = scan_ident_at(src, at_pos + 1) else {
            cursor.jump(at_pos + 1, false, false, None);
            continue;
        };

        if first_seg == "use" {
            let root_pos = skip_ws_and_comments_lossy(src, after_first);
            if let Some((root_seg, _)) = scan_ident_at(src, root_pos) {
                let mut leaves = HashSet::new();
                let mut glob = false;
                let end_pos = collect_at_use_scan_targets(src, root_pos, &mut leaves, &mut glob);
                let after_ws = skip_ws_and_comments_lossy(src, end_pos);
                let final_pos = if src.as_bytes().get(after_ws) == Some(&b';') {
                    after_ws + 1
                } else {
                    end_pos
                };
                if root_seg == "builtin" {
                    builtin_leaves.extend(leaves);
                    has_builtin_glob |= glob;
                } else if is_meta_macros_import_path(src, root_pos) {
                    builtin_leaves.extend(leaves);
                    has_meta_macros_glob |= glob;
                } else {
                    out.has_non_builtin_macros = true;
                    out.roots.insert(root_seg.clone());
                    out.import_roots.insert(root_seg);
                    non_builtin_leaves.extend(leaves);
                    has_non_builtin_glob |= glob;
                }
                cursor.jump(final_pos, false, false, None);
            } else {
                cursor.jump(after_first, false, false, None);
            }
            continue;
        }

        let (segments, scan_end) = collect_path_segments_lossy(src, after_first, first_seg);
        let seg0 = segments[0].as_str();
        let seg_count = segments.len();

        let is_builtin_qualified = seg0 == "builtin"
            || matches!(
                segments
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .as_slice(),
                ["meta", "macros", "define_macro1" | "define_macro2"]
            );
        if is_builtin_qualified || matches!(seg0, "define_macro1" | "define_macro2") {
            cursor.jump(scan_end, true, false, None);
            continue;
        }

        let is_builtin_imported_leaf = seg_count == 1
            && (builtin_leaves.contains(seg0)
                || (has_builtin_glob
                    && matches!(seg0, "raw" | "raw_aligned")
                    && !non_builtin_leaves.contains(seg0)
                    && !has_non_builtin_glob)
                || (has_meta_macros_glob
                    && matches!(seg0, "define_macro1" | "define_macro2")
                    && !non_builtin_leaves.contains(seg0)
                    && !has_non_builtin_glob));
        if is_builtin_imported_leaf {
            cursor.jump(scan_end, true, false, None);
            continue;
        }

        out.has_non_builtin_macros = true;
        if !(seg_count == 1 && non_builtin_leaves.contains(seg0)) {
            out.roots.insert(segments[0].clone());
        }
        cursor.jump(scan_end, true, false, None);
    }

    out
}

/// Process `@use` macro imports and Mode 4 (`@builtin.str.raw` / `^macro4`) calls before Flurry lexing.
pub fn process_raw_source_and_imports(
    src: &str,
    base_pos: BytePos,
    ctx: &mut ExpansionContext<'_>,
    provenance: &mut ExpansionMap,
    budget: &mut MacroBudget,
) -> Result<RawSourcePassResult, MacroError> {
    let mut scope = MacroScope::new();
    if !src.contains('@') {
        return Ok(RawSourcePassResult {
            tokens: Vec::new(),
            lex_errors: Vec::new(),
            modified: false,
            scope,
        });
    }

    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut builder = OutputBuilder::new(src, base_pos);
    let mut cursor = RawScanCursor::new(src, true);

    let mut slice_start = 0usize;
    let mut modified = false;

    while let Some(pos) = cursor.next_at() {
        let Some((first_seg, after_first)) = scan_ident_at(src, pos + 1) else {
            cursor.jump(pos + 1, false, false, None);
            continue;
        };

        if first_seg == "use" {
            let after_use = parse_at_use(src, pos, base_pos, &mut scope, ctx)?;
            builder.flush_slice(slice_start, pos, Some(pos), true);
            builder.push_stripped();
            modified = true;
            slice_start = after_use;
            cursor.jump(after_use, false, false, None);
            continue;
        }

        let (path_segments, path_end) = scan_call_path(src, first_seg, after_first, base_pos)?;
        let path_span = Span::new(
            base_pos + BytePos(pos as u32),
            base_pos + BytePos(path_end as u32),
        );
        let desc = scope.resolve_call_path(&path_segments, path_span, ctx)?;

        match desc.mode {
            MacroInputMode::BuiltinRawStr
            | MacroInputMode::BuiltinRawStrAligned
            | MacroInputMode::RawSource => {
                let delim_pos = skip_ws_and_comments(src, path_end, base_pos)?;
                if bytes.get(delim_pos) != Some(&b'{') {
                    let found_end = (delim_pos + 1).min(len);
                    return Err(MacroError::new(
                        MacroErrorKind::DelimiterMismatch,
                        format!(
                            "raw-source macro `@{}` requires `{{ ... }}` brace delimiters",
                            desc.full_path()
                        ),
                        Span::new(
                            base_pos + BytePos(delim_pos as u32),
                            base_pos + BytePos(found_end as u32),
                        ),
                    )
                    .with_label(path_span, "macro called here"));
                }

                let (content_start, close_brace) =
                    scan_raw_balanced_braces(src, delim_pos, base_pos)?;
                let call_end = close_brace + 1;
                let call_span = Span::new(
                    base_pos + BytePos(pos as u32),
                    base_pos + BytePos(call_end as u32),
                );

                budget.consume_step().map_err(|e| e.with_span(call_span))?;
                let exp_id = provenance.record(
                    ExpansionKind::ConstructionSource,
                    desc.full_path(),
                    call_span,
                    desc.def_site,
                    None,
                );
                let raw_body = &src[content_start..close_brace];
                let replacement = match desc.mode {
                    MacroInputMode::BuiltinRawStr | MacroInputMode::BuiltinRawStrAligned => {
                        budget
                            .consume_tokens(1)
                            .map_err(|e| e.with_span(call_span))?;
                        let aligned;
                        let effective = if matches!(desc.mode, MacroInputMode::BuiltinRawStrAligned)
                        {
                            aligned = align_raw_string(raw_body);
                            aligned.as_str()
                        } else {
                            raw_body
                        };
                        let encoded = encode_flurry_string_literal(effective);
                        vec![MacroToken::new(
                            TokenKind::Str,
                            encoded,
                            call_span,
                            TokenSpacing::Alone,
                            Some(exp_id),
                        )]
                    }
                    MacroInputMode::RawSource => {
                        let Some(executor) = ctx.executor.as_deref_mut() else {
                            return Err(MacroError::new(
                                MacroErrorKind::HandlerExecutionError,
                                format!(
                                    "cannot evaluate `^macro4` construction macro `@{}` without a comptime macro executor",
                                    desc.full_path()
                                ),
                                call_span,
                            )
                            .with_chain(provenance, Some(exp_id)));
                        };
                        let mut out = executor
                            .execute_raw_source(desc, raw_body, call_span, budget)
                            .map_err(|e| {
                                e.with_chain_if_empty(provenance, Some(exp_id), call_span)
                            })?;
                        for tok in &mut out {
                            if tok.expansion_id.is_none() {
                                tok.expansion_id = Some(exp_id);
                            }
                            if tok.span == rustc_span::DUMMY_SP {
                                tok.span = call_span;
                            }
                        }
                        out
                    }
                    MacroInputMode::Tokens
                    | MacroInputMode::BuiltinDefineMacro1
                    | MacroInputMode::BuiltinDefineMacro2
                    | MacroInputMode::Ast { .. }
                    | MacroInputMode::Peg { .. } => unreachable!(),
                };

                builder.flush_slice(slice_start, pos, Some(pos), false);
                builder.push_replacement(replacement, pos, call_end);
                modified = true;
                slice_start = call_end;
                cursor.jump(call_end, true, false, None);
            }
            MacroInputMode::Tokens
            | MacroInputMode::BuiltinDefineMacro1
            | MacroInputMode::BuiltinDefineMacro2
            | MacroInputMode::Ast { .. }
            | MacroInputMode::Peg { .. } => {
                cursor.jump(path_end, true, false, None);
            }
        }
    }

    if !modified {
        return Ok(RawSourcePassResult {
            tokens: Vec::new(),
            lex_errors: Vec::new(),
            modified: false,
            scope,
        });
    }

    builder.flush_slice(slice_start, len, None, false);
    Ok(RawSourcePassResult {
        tokens: builder.out_tokens,
        lex_errors: builder.lex_errors,
        modified: true,
        scope,
    })
}
