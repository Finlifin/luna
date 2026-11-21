//! Pre-lexing UTF-8 scanner for `@use` macro imports and Mode 4 (`^macro4` / `@builtin.str.raw`) raw-source calls.
use crate::imports::{MacroScope, parse_at_use, scan_ident_at, skip_ws_and_comments};
use crate::materialize::tokens_to_macro_tokens;
use crate::provenance::{ExpansionKind, ExpansionMap, MacroToken, TokenSpacing, is_separated_kind};
use crate::registry::{ExpansionContext, MacroInputMode};
use crate::{MacroBudget, MacroError, MacroErrorKind};
use lex::{LexError, TokenKind};
use rustc_span::{BytePos, Span};
use std::collections::HashSet;
use std::fmt::Write as _;

/// Result of running the Mode 4 & `@use` pre-lexing scanner on a source string.
pub struct RawSourcePassResult {
    pub tokens: Vec<MacroToken>,
    pub lex_errors: Vec<LexError>,
    pub modified: bool,
}

fn encode_flurry_string_literal(raw: &str) -> String {
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
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut pos = start;
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
    pos
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

/// Process `@use` macro imports and Mode 4 (`@builtin.str.raw` / `^macro4`) calls before Flurry lexing.
pub fn process_raw_source_and_imports(
    src: &str,
    base_pos: BytePos,
    ctx: &mut ExpansionContext<'_>,
    provenance: &mut ExpansionMap,
    budget: &mut MacroBudget,
) -> Result<RawSourcePassResult, MacroError> {
    if !src.contains('@') {
        return Ok(RawSourcePassResult {
            tokens: Vec::new(),
            lex_errors: Vec::new(),
            modified: false,
        });
    }

    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut scope = MacroScope::new();
    let mut builder = OutputBuilder::new(src, base_pos);

    let mut pos = 0usize;
    let mut slice_start = 0usize;
    let mut modified = false;
    let mut can_end_expr = false;
    let mut prev_was_dot = false;
    let mut prev_word: Option<String> = None;
    let mut in_use_stmt = false;
    let mut use_brace_depth = 0usize;

    while pos < len {
        let b = bytes[pos];

        // 1. Comments
        if b == b'-' && bytes.get(pos + 1) == Some(&b'-') {
            pos = skip_line_comment(src, pos);
            continue;
        }
        if b == b'{' && bytes.get(pos + 1) == Some(&b'-') {
            pos = skip_block_comment(src, pos);
            continue;
        }

        // 2. String literals (with ${...} interpolation)
        if b == b'"' {
            pos = skip_string_literal(src, pos);
            can_end_expr = true;
            prev_was_dot = false;
            prev_word = None;
            continue;
        }

        // 3. Character literals vs take-views (`value'old'tag`)
        if b == b'\'' {
            if !can_end_expr {
                if let Some(end) = try_skip_char_literal(src, pos) {
                    pos = end;
                    can_end_expr = true;
                    prev_was_dot = false;
                    prev_word = None;
                    continue;
                }
            }
            pos += 1;
            can_end_expr = false;
            prev_was_dot = false;
            prev_word = None;
            continue;
        }

        // 4. Token buffer literal `{{ ... }}` (not `match {{...}}`)
        if b == b'{'
            && bytes.get(pos + 1) == Some(&b'{')
            && !token_buffer_disallowed_after_word(prev_word.as_deref())
        {
            if let Some(end) = skip_token_buffer_literal(src, pos) {
                pos = end;
                can_end_expr = true;
                prev_was_dot = false;
                prev_word = None;
                continue;
            }
        }

        // 5. Construction macro `@...` or `@use ...` outside ordinary `use @pkg` paths
        if b == b'@' && !in_use_stmt {
            if let Some((first_seg, after_first)) = scan_ident_at(src, pos + 1) {
                if first_seg == "use" {
                    let after_use = parse_at_use(src, pos, base_pos, &mut scope, ctx)?;
                    builder.flush_slice(slice_start, pos, Some(pos), true);
                    builder.push_stripped();
                    modified = true;
                    slice_start = after_use;
                    pos = after_use;
                    can_end_expr = false;
                    prev_was_dot = false;
                    prev_word = None;
                    continue;
                }

                if first_seg == "define_macro1" || first_seg == "define_macro2" {
                    pos = after_first;
                    can_end_expr = false;
                    prev_was_dot = false;
                    prev_word = None;
                    continue;
                }

                let (path_segments, path_end) =
                    scan_call_path(src, first_seg, after_first, base_pos)?;
                let path_span = Span::new(
                    base_pos + BytePos(pos as u32),
                    base_pos + BytePos(path_end as u32),
                );
                let desc = scope.resolve_call_path(&path_segments, path_span, ctx)?;

                match desc.mode {
                    MacroInputMode::BuiltinRawStr | MacroInputMode::RawSource => {
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
                            MacroInputMode::BuiltinRawStr => {
                                budget
                                    .consume_tokens(1)
                                    .map_err(|e| e.with_span(call_span))?;
                                let encoded = encode_flurry_string_literal(raw_body);
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
                            MacroInputMode::Tokens => unreachable!(),
                        };

                        builder.flush_slice(slice_start, pos, Some(pos), false);
                        builder.push_replacement(replacement, pos, call_end);
                        modified = true;
                        slice_start = call_end;
                        pos = call_end;
                        can_end_expr = true;
                        prev_was_dot = false;
                        prev_word = None;
                        continue;
                    }
                    MacroInputMode::Tokens => {
                        pos = path_end;
                        can_end_expr = true;
                        prev_was_dot = false;
                        prev_word = None;
                        continue;
                    }
                }
            }
        }

        // 6. Track delimiters, operators, and `use ...` state
        if b == b'.' {
            prev_was_dot = true;
            can_end_expr = false;
            prev_word = None;
            pos += 1;
            continue;
        }

        if b == b'{' {
            if in_use_stmt {
                use_brace_depth += 1;
            }
            can_end_expr = false;
            prev_was_dot = false;
            prev_word = None;
            pos += 1;
            continue;
        }

        if b == b'}' {
            if in_use_stmt && use_brace_depth > 0 {
                use_brace_depth -= 1;
                if use_brace_depth == 0 {
                    in_use_stmt = false;
                }
            }
            can_end_expr = true;
            prev_was_dot = false;
            prev_word = None;
            pos += 1;
            continue;
        }

        if b == b';' {
            if in_use_stmt && use_brace_depth == 0 {
                in_use_stmt = false;
            }
            can_end_expr = false;
            prev_was_dot = false;
            prev_word = None;
            pos += 1;
            continue;
        }

        if let Some((word, end)) = scan_ident_at(src, pos) {
            if word == "use" && !in_use_stmt {
                in_use_stmt = true;
                use_brace_depth = 0;
                can_end_expr = false;
            } else if prev_was_dot && word == "await" {
                can_end_expr = true;
            } else if word == "ref" {
                can_end_expr = prev_was_dot;
            } else {
                can_end_expr = word_can_end_expr(&word);
            }
            prev_was_dot = false;
            prev_word = Some(word);
            pos = end;
            continue;
        }

        if b == b'*' && prev_was_dot {
            can_end_expr = true;
            prev_was_dot = false;
            prev_word = None;
            pos += 1;
            continue;
        }

        if matches!(b, b'!' | b'?' | b'#' | b'>') {
            prev_was_dot = false;
            prev_word = None;
            pos += 1;
            continue;
        }

        if matches!(b, b')' | b']') || b.is_ascii_digit() {
            can_end_expr = true;
            prev_was_dot = false;
            prev_word = None;
            pos += 1;
            continue;
        }

        if bytes[pos].is_ascii_whitespace()
            || src[pos..].chars().next().is_some_and(char::is_whitespace)
        {
            let ch = src[pos..].chars().next().unwrap();
            pos += ch.len_utf8();
            continue;
        }

        let ch = src[pos..].chars().next().unwrap();
        can_end_expr = false;
        prev_was_dot = false;
        prev_word = None;
        pos += ch.len_utf8();
    }

    if !modified {
        return Ok(RawSourcePassResult {
            tokens: Vec::new(),
            lex_errors: Vec::new(),
            modified: false,
        });
    }

    builder.flush_slice(slice_start, len, None, false);
    Ok(RawSourcePassResult {
        tokens: builder.out_tokens,
        lex_errors: builder.lex_errors,
        modified: true,
    })
}
