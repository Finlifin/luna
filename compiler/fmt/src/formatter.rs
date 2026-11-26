//! Core `Formatter` state, token-slot navigation, container layout, and directive execution.

use std::cmp::Ordering;

use ast::{NodeIndex, NodeKind};
use lex::TokenKind;

use crate::bridge::{BridgedConstruct, ParseFailure, ParsedSource, parse_and_bridge};
use crate::comments::{CommentCursor, measure_leading_ws};
use crate::config::FormatConfig;
use crate::directives::{DirectiveWarning, ScopedConfigStack};
use crate::doc::{Doc, print_doc};
use crate::verify::verify_formatted_output;

/// Output of a successful formatting run.
#[derive(Debug, Clone)]
pub struct FormatResult {
    pub formatted: String,
    pub warnings: Vec<DirectiveWarning>,
}

/// Failure during source formatting.
#[derive(Debug, Clone)]
pub enum FormatError {
    Parse(ParseFailure),
    Verification(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(p) => f.write_str(&p.rendered_diagnostics),
            Self::Verification(msg) => write!(f, "formatter verification error: {msg}"),
        }
    }
}

impl std::error::Error for FormatError {}

/// Format a Flurry source string using `config`.
/// When `emit_diagnostics` is true, lex/parse errors are printed via `DiagnosticContext`.
pub fn format_source(
    src: &str,
    display_name: &str,
    config: &FormatConfig,
    emit_diagnostics: bool,
) -> Result<FormatResult, FormatError> {
    let parsed =
        parse_and_bridge(src, display_name, emit_diagnostics).map_err(FormatError::Parse)?;
    let warnings = parsed.warnings.clone();
    let newline = config.newline_style.resolve(src);

    // Empty or whitespace-only file without comments -> empty string.
    let root_slots = parsed.ast.get_children(parsed.ast.root);
    let root_items = if root_slots.is_empty() {
        &[][..]
    } else {
        parsed
            .ast
            .get_multi_child_slice(root_slots[0])
            .unwrap_or(&[])
    };
    if root_items.is_empty() && parsed.comments.is_empty() {
        return Ok(FormatResult {
            formatted: String::new(),
            warnings,
        });
    }

    let mut fmt = Formatter::new(&parsed, config.clone());
    let doc = fmt.format_file_scope();
    let rendered = print_doc(&doc, config.max_width, newline);

    verify_formatted_output(&parsed, &rendered, display_name, config)
        .map_err(FormatError::Verification)?;

    Ok(FormatResult {
        formatted: rendered,
        warnings,
    })
}

/// Inclusive range `[first ..= last]` in `ParsedSource::sig_tokens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenSlot {
    pub first: usize,
    pub last: usize,
}

impl TokenSlot {
    pub fn new(first: usize, last: usize) -> Self {
        Self { first, last }
    }

    pub fn from_node(parsed: &ParsedSource, node: NodeIndex, floor: usize) -> Self {
        let (f, l) = parsed.exact_token_bounds(node, floor.max(1));
        Self { first: f, last: l }
    }

    /// Unwrap explicit `( ... )` parenthesis pairs wrapping `node` inside `self`.
    pub fn unwrap_parens(self, parsed: &ParsedSource, node: NodeIndex) -> (usize, Self) {
        let (count, inner_f, inner_l) = parsed.count_outer_parens(node, self.first, self.last);
        (
            count,
            Self {
                first: inner_f,
                last: inner_l,
            },
        )
    }

    /// Resolve the balanced token slot `[start_tok ..= end_tok]` for `child` inside `self`,
    /// including any explicit outer `(...)` around `child`.
    pub fn child_slot_from(
        self,
        parsed: &ParsedSource,
        start_tok: usize,
        child: NodeIndex,
    ) -> Self {
        let max_real = parsed.sig_tokens.len().saturating_sub(2);
        if max_real == 0 || child == 0 {
            return Self::new(0, 0);
        }
        let start = start_tok.clamp(1, self.last.max(1).min(max_real));
        let (_, raw_last) = parsed.exact_token_bounds(child, start);
        let mut end = raw_last.clamp(start, self.last.max(start).min(max_real));

        // Compute delimiter balance across `start ..= end`; if `child` was wrapped in outer
        // `(...)`, advance `end` through matching closing delimiters.
        let mut depth = 0isize;
        for idx in start..=end {
            match parsed.sig_tokens[idx].kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth = (depth - 1).max(0);
                }
                _ => {}
            }
        }
        let limit = self.last.min(max_real);
        while depth > 0 && end < limit {
            end += 1;
            match parsed.sig_tokens[end].kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => depth -= 1,
                _ => {}
            }
        }
        // Trim any leading punctuation that precedes `child` when `start` was an estimate.
        let mut actual_start = start;
        let (child_raw_first, _) = parsed.exact_token_bounds(child, start);
        while actual_start < child_raw_first
            && parsed.sig_tokens[actual_start].kind != TokenKind::LParen
        {
            actual_start += 1;
        }
        Self::new(actual_start, end)
    }
}

/// Syntactic context for `NodeKind::ExprAsPattern`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprPatternMode {
    /// Standalone `< expr >` pattern (requires whitespace on both sides of `<` and `>`).
    Standalone,
    /// `<expr>` segment inside `0b...` / `0o...` / `0x...` bitvec pattern (compact).
    BitVecSegment,
    /// Bare `op` head of `op(args) => ...` inside an `EffectElimination` (`#`) arm.
    BareEffectOp,
}

/// Kind of container sequence for blank-line, comma, and semicolon rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    /// Top-level file scope.
    FileScope,
    /// `mod name { ... }` body.
    ModuleBody,
    /// `struct` / `enum` / `union` body (configs/properties + fields/variants + methods/stmts).
    AdtBody,
    /// `trait` / `impl` / `extend` / `derive fn` body.
    TraitOrImplBody,
    /// Value/statement `{ ... }` block (`fn`, `if`, `else`, `while`, `for`, `do`, `match` arm, etc.).
    ValueBlock,
}

/// Member group category inside mixed containers (`struct`, `enum`, `union`, `mod`, `trait`, `impl`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberCategory {
    ConfigOrProperty,
    FieldOrVariant,
    AssocItem,
    UseImport,
    StatementOrDef,
}

/// Stateful AST -> `Doc` formatter.
pub struct Formatter<'a> {
    pub parsed: &'a ParsedSource,
    pub comments: CommentCursor<'a>,
    pub cfg_stack: ScopedConfigStack,
    pub indent_spaces: usize,
    pub brace_depth: usize,
}

impl<'a> Formatter<'a> {
    pub fn new(parsed: &'a ParsedSource, root_cfg: FormatConfig) -> Self {
        let tab_spaces = root_cfg.tab_spaces;
        Self {
            parsed,
            comments: CommentCursor::new(parsed, tab_spaces),
            cfg_stack: ScopedConfigStack::new(root_cfg),
            indent_spaces: 0,
            brace_depth: 0,
        }
    }

    pub fn cfg(&self) -> FormatConfig {
        self.cfg_stack.current().clone()
    }

    pub fn tab_spaces(&self) -> usize {
        self.cfg_stack.current().tab_spaces
    }

    /// Decode an `Id` node to its identifier spelling.
    pub fn decode_id(&self, node: NodeIndex, floor_tok: usize) -> String {
        if self.parsed.ast.get_node_kind(node) == Some(NodeKind::Id) {
            let slots = self.parsed.ast.get_children(node);
            if slots.len() == 2 {
                // SAFETY: `Id` slots were produced by `Symbol::to_raw_parts` in this process.
                let sym = unsafe { symbol::Symbol::from_raw_parts(slots[0], slots[1]) };
                return sym.as_str().to_string();
            }
        }
        self.literal_slice(node, floor_tok).to_string()
    }

    /// Exact slice of `original_src` for a leaf/literal node.
    pub fn literal_slice(&self, node: NodeIndex, floor_tok: usize) -> &'a str {
        let (lo, hi) = self.parsed.exact_byte_range(node, floor_tok.max(1));
        self.parsed.original_src.get(lo..hi).unwrap_or("")
    }

    /// Check if `node` (at raw start offset) is a bridged construct.
    pub fn bridged_at(&self, node: NodeIndex, floor_tok: usize) -> Option<&'a BridgedConstruct> {
        let (raw_lo, _) = self.parsed.raw_byte_range(node);
        if let Some(b) = self.parsed.bridge.constructs_by_start.get(&raw_lo) {
            return Some(b);
        }
        let (exact_lo, _) = self.parsed.exact_byte_range(node, floor_tok.max(1));
        self.parsed.bridge.constructs_by_start.get(&exact_lo)
    }

    /// Format a bridged construct (`@path(...)`, `@path { ... }`, `define ... { ... }`, `$id`, `{{ ... }}`).
    pub fn format_bridged(&mut self, bridged: &BridgedConstruct) -> Doc {
        match bridged {
            BridgedConstruct::BraceMacroCall { start, end, .. }
            | BridgedConstruct::DollarMacroCall { start, end }
            | BridgedConstruct::TokenBuffer { start, end } => {
                self.comments
                    .advance_past_verbatim(*end, self.brace_depth, &mut self.cfg_stack);
                let raw = &self.parsed.original_src[*start..*end];
                if raw.contains('\n') {
                    Doc::dedent_to_zero(Doc::verbatim(raw.to_string()))
                } else {
                    Doc::text(raw.to_string())
                }
            }
            BridgedConstruct::TemplateMacroDef {
                start,
                end,
                kw_start,
                brace_start,
            } => {
                // Normalize `define Name(...) ` header spacing while keeping `{ ... }` body verbatim!
                self.comments
                    .advance_past_verbatim(*end, self.brace_depth, &mut self.cfg_stack);
                let prefix = self.parsed.original_src[*start..*kw_start].trim();
                let header = self.parsed.original_src[*kw_start..*brace_start].trim();
                let body = &self.parsed.original_src[*brace_start..*end];
                let mut combined = String::new();
                if !prefix.is_empty() {
                    combined.push_str(prefix);
                    combined.push(' ');
                }
                combined.push_str(header);
                combined.push(' ');
                combined.push_str(body);
                if combined.contains('\n') {
                    Doc::dedent_to_zero(Doc::verbatim(combined))
                } else {
                    Doc::text(combined)
                }
            }
            BridgedConstruct::ParenMacroCall {
                start,
                end,
                path,
                args_inner,
            } => {
                self.comments
                    .advance_past_verbatim(*end, self.brace_depth, &mut self.cfg_stack);
                let Some((arg_lo, arg_hi)) = *args_inner else {
                    return Doc::text(format!("@{path}"));
                };
                let inner_raw = &self.parsed.original_src[arg_lo..arg_hi];
                if inner_raw.trim().is_empty() {
                    return Doc::text(format!("@{path}()"));
                }
                // Try formatting `args_inner` as a valid Flurry call argument list `f(args)`.
                if let Some(formatted_args) = self.try_format_macro_paren_args(path, inner_raw) {
                    return formatted_args;
                }
                let raw = &self.parsed.original_src[*start..*end];
                if raw.contains('\n') {
                    Doc::dedent_to_zero(Doc::verbatim(raw.to_string()))
                } else {
                    Doc::text(raw.to_string())
                }
            }
            BridgedConstruct::SkippedUnparsed { start, end } => {
                self.format_skipped_bytes(*start, *end)
            }
        }
    }

    fn try_format_macro_paren_args(&self, path: &str, inner_raw: &str) -> Option<Doc> {
        let synthetic = format!("__fmt_call({inner_raw});\n");
        let sub_parsed = parse_and_bridge(&synthetic, "<macro-args>", false).ok()?;
        if !sub_parsed.comments.is_empty() {
            // Preserve raw when comments appear inside macro args to avoid shifting offsets.
            return None;
        }
        let root_slots = sub_parsed.ast.get_children(sub_parsed.ast.root);
        let items = sub_parsed.ast.get_multi_child_slice(*root_slots.first()?)?;
        if items.len() != 1 {
            return None;
        }
        let expr_stmt = items[0];
        let call_node = if sub_parsed.ast.get_node_kind(expr_stmt) == Some(NodeKind::ExprStatement)
        {
            sub_parsed.ast.get_children(expr_stmt)[0]
        } else {
            expr_stmt
        };
        if sub_parsed.ast.get_node_kind(call_node) != Some(NodeKind::Application) {
            return None;
        }
        let mut sub_fmt = Formatter::new(&sub_parsed, self.cfg());
        sub_fmt.indent_spaces = self.indent_spaces;
        let call_slot = TokenSlot::from_node(&sub_parsed, call_node, 1);
        let args_doc = sub_fmt.format_call_args_only(call_node, call_slot);
        Some(Doc::concat([Doc::text(format!("@{path}")), args_doc]))
    }

    /// Format `[lo..hi]` under `--% .skip`: align line 1 to current indent and adjust subsequent
    /// lines' outer indent relative to line 1's original visual column.
    pub fn format_skipped_bytes(&mut self, lo: usize, hi: usize) -> Doc {
        self.comments
            .advance_past_verbatim(hi, self.brace_depth, &mut self.cfg_stack);
        let raw = self.parsed.original_src.get(lo..hi).unwrap_or("");
        if !raw.contains('\n') {
            return Doc::text(raw.trim_end_matches([' ', '\t']).to_string());
        }
        let tab_spaces = self.tab_spaces();
        let orig_base_col = self.parsed.visual_column(lo, tab_spaces);
        let target_indent = self.indent_spaces;
        let raw_ranges = self.raw_macro_byte_ranges_in(lo, hi);

        let normalized = raw.replace("\r\n", "\n");
        let lines: Vec<&str> = normalized.split('\n').collect();
        let mut out_lines = Vec::with_capacity(lines.len());
        let mut byte_cursor = lo;

        for (idx, line) in lines.iter().enumerate() {
            let line_byte_start = byte_cursor;
            byte_cursor += line.len() + 1;
            let in_raw_macro = raw_ranges
                .iter()
                .any(|&(rs, re)| line_byte_start > rs && line_byte_start < re);
            if idx == 0 {
                out_lines.push(line.trim_end_matches([' ', '\t']).to_string());
                continue;
            }
            if in_raw_macro {
                out_lines.push((*line).to_string());
                continue;
            }
            let stripped = line.trim_end_matches([' ', '\t']);
            if stripped.is_empty() {
                out_lines.push(String::new());
                continue;
            }
            let (line_col, content_idx) = measure_leading_ws(stripped, tab_spaces);
            let content = &stripped[content_idx..];
            let new_indent = if line_col >= orig_base_col {
                target_indent + (line_col - orig_base_col)
            } else {
                line_col
            };
            let mut rebuilt = String::with_capacity(new_indent + content.len());
            for _ in 0..new_indent {
                rebuilt.push(' ');
            }
            rebuilt.push_str(content);
            out_lines.push(rebuilt);
        }

        // First line uses enclosing `pending_indent`; subsequent lines carry exact absolute spaces!
        let first = out_lines[0].clone();
        let rest = out_lines[1..].join("\n");
        Doc::concat([
            Doc::text(first),
            Doc::dedent_to_zero(Doc::verbatim(format!("\n{rest}"))),
        ])
    }

    /// Format `[lo..hi]` under `--% .preserve`: preserve user line breaks and intra-line alignment
    /// spaces while normalizing outer block indentation and `--` comment spacing.
    pub fn format_preserved_bytes(&mut self, lo: usize, hi: usize) -> Doc {
        let normalize_comments = self.cfg_stack.current().normalize_comments;
        self.comments
            .advance_past_verbatim(hi, self.brace_depth, &mut self.cfg_stack);
        let raw = self.parsed.original_src.get(lo..hi).unwrap_or("");
        let tab_spaces = self.tab_spaces();
        let orig_base_col = self.parsed.visual_column(lo, tab_spaces);
        let target_indent = self.indent_spaces;
        let raw_ranges = self.raw_macro_byte_ranges_in(lo, hi);

        let normalized = raw.replace("\r\n", "\n");
        let lines: Vec<&str> = normalized.split('\n').collect();
        if lines.len() == 1 {
            let line = normalize_line_comment_suffix(
                lines[0].trim_end_matches([' ', '\t']),
                normalize_comments,
            );
            return Doc::text(line);
        }

        let mut out_lines = Vec::with_capacity(lines.len());
        let mut byte_cursor = lo;
        for (idx, line) in lines.iter().enumerate() {
            let line_byte_start = byte_cursor;
            byte_cursor += line.len() + 1;
            let in_raw_macro = raw_ranges
                .iter()
                .any(|&(rs, re)| line_byte_start > rs && line_byte_start < re);
            if in_raw_macro {
                out_lines.push((*line).to_string());
                continue;
            }
            let stripped = line.trim_end_matches([' ', '\t']);
            if idx == 0 {
                out_lines.push(normalize_line_comment_suffix(stripped, normalize_comments));
                continue;
            }
            if stripped.is_empty() {
                out_lines.push(String::new());
                continue;
            }
            let (line_col, content_idx) = measure_leading_ws(stripped, tab_spaces);
            let content =
                normalize_line_comment_suffix(&stripped[content_idx..], normalize_comments);
            let new_indent = if line_col >= orig_base_col {
                target_indent + (line_col - orig_base_col)
            } else {
                line_col
            };
            let mut rebuilt = String::with_capacity(new_indent + content.len());
            for _ in 0..new_indent {
                rebuilt.push(' ');
            }
            rebuilt.push_str(&content);
            out_lines.push(rebuilt);
        }

        let first = out_lines[0].clone();
        let rest = out_lines[1..].join("\n");
        Doc::concat([
            Doc::text(first),
            Doc::dedent_to_zero(Doc::verbatim(format!("\n{rest}"))),
        ])
    }

    fn raw_macro_byte_ranges_in(&self, lo: usize, hi: usize) -> Vec<(usize, usize)> {
        self.parsed
            .bridge
            .constructs_by_start
            .values()
            .filter_map(|b| {
                let (s, e) = b.span_bytes();
                if e > lo && s < hi && matches!(b, BridgedConstruct::BraceMacroCall { .. }) {
                    Some((s, e))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Format top-level `FileScope`.
    pub fn format_file_scope(&mut self) -> Doc {
        let root = self.parsed.ast.root;
        let slots = self.parsed.ast.get_children(root);
        let items = if slots.is_empty() {
            &[][..]
        } else {
            self.parsed
                .ast
                .get_multi_child_slice(slots[0])
                .unwrap_or(&[])
        };
        let max_real = self.parsed.sig_tokens.len().saturating_sub(2);
        let eof_byte = self.parsed.original_src.len();
        self.format_container_elements(items, 1, max_real, 0, eof_byte, ContainerKind::FileScope)
    }

    /// Format a sequence of elements inside a file, module, ADT, trait/impl, or `{ ... }` block.
    pub fn format_container_elements(
        &mut self,
        items: &[NodeIndex],
        container_first_tok: usize,
        container_last_tok: usize,
        open_end_byte: usize,
        close_start_byte: usize,
        kind: ContainerKind,
    ) -> Doc {
        if items.is_empty() {
            let (trailing_comments, _, _) = self.comments.take_leading_before(
                open_end_byte,
                close_start_byte,
                self.indent_spaces,
                self.brace_depth,
                false,
                &mut self.cfg_stack,
            );
            return trim_trailing_hardline(trailing_comments);
        }

        // Precompute exact token bounds & byte ranges for each element monotonically.
        let mut elem_infos = Vec::with_capacity(items.len());
        let mut floor_tok = container_first_tok.max(1);
        for (idx, &node) in items.iter().enumerate() {
            let (raw_f, raw_l) = self.parsed.exact_token_bounds(node, floor_tok);
            // Expand right across balanced `)` if expression statement had outer parens, stopping
            // before `;` or `,`.
            let slot_start = floor_tok;
            let mut f = raw_f;
            while f > slot_start && self.parsed.sig_tokens[f - 1].kind == TokenKind::LParen {
                f -= 1;
            }
            let bound = TokenSlot::new(f, container_last_tok.max(raw_l));
            let full_slot = bound.child_slot_from(self.parsed, f, node);
            let lo = self.parsed.sig_tokens[full_slot.first].from;
            let mut hi = self.parsed.sig_tokens[full_slot.last].to;
            if let Some(b) = self.parsed.bridge.constructs_by_start.get(&lo) {
                hi = hi.max(b.span_bytes().1);
            }
            // Check whether a `,` or `;` immediately follows `full_slot.last`.
            let mut sep_end_byte = hi;
            let mut after_sep_tok = full_slot.last + 1;
            if after_sep_tok < self.parsed.sig_tokens.len()
                && matches!(
                    self.parsed.sig_tokens[after_sep_tok].kind,
                    TokenKind::Semi | TokenKind::Comma
                )
            {
                sep_end_byte = self.parsed.sig_tokens[after_sep_tok].to;
                after_sep_tok += 1;
            }
            let end_line = self.parsed.line_of_byte(sep_end_byte.saturating_sub(1));
            elem_infos.push(ElemInfo {
                node,
                slot: full_slot,
                lo,
                hi,
                sep_end_byte,
                end_line,
            });
            floor_tok = after_sep_tok;
        }

        let mut parts: Vec<Doc> = Vec::new();
        let mut prev_end_byte = open_end_byte;
        let mut prev_category: Option<MemberCategory> = None;
        let mut prev_was_multiline_def = false;
        let mut idx = 0usize;

        while idx < elem_infos.len() {
            let info = elem_infos[idx];

            // Handle `--% .off` region covering one or more elements!
            if let Some(region) = self.parsed.enclosing_off_region(info.lo, info.hi) {
                // First emit leading comments (including the `--% .off` directive line itself!)
                // up to `region.body_start`.
                let (lead, _, _) = self.comments.take_leading_before(
                    prev_end_byte,
                    region.body_start,
                    self.indent_spaces,
                    self.brace_depth,
                    !parts.is_empty(),
                    &mut self.cfg_stack,
                );
                if !matches!(lead, Doc::Nil) {
                    if !parts.is_empty() {
                        parts.push(Doc::HardLine);
                    }
                    parts.push(trim_trailing_hardline(lead));
                }
                let raw_body = &self.parsed.original_src[region.body_start..region.body_end];
                let trimmed_nl = raw_body.strip_suffix('\n').unwrap_or(raw_body);
                let trimmed_nl = trimmed_nl.strip_suffix('\r').unwrap_or(trimmed_nl);
                if !trimmed_nl.is_empty() {
                    if !parts.is_empty() {
                        parts.push(Doc::HardLine);
                    }
                    parts.push(Doc::dedent_to_zero(Doc::verbatim(trimmed_nl.to_string())));
                }
                self.comments.advance_past_verbatim(
                    region.body_end,
                    self.brace_depth,
                    &mut self.cfg_stack,
                );
                prev_end_byte = region.body_end;
                while idx < elem_infos.len() && elem_infos[idx].hi <= region.body_end {
                    prev_category = Some(categorize_member(
                        &self.parsed.ast,
                        elem_infos[idx].node,
                        kind,
                    ));
                    prev_was_multiline_def = false;
                    idx += 1;
                }
                continue;
            }

            // Consume leading comments/directives before `info.lo`.
            let (lead_doc, gap_cursor, dir_flags) = self.comments.take_leading_before(
                prev_end_byte,
                info.lo,
                self.indent_spaces,
                self.brace_depth,
                !parts.is_empty(),
                &mut self.cfg_stack,
            );

            let cur_cat = categorize_member(&self.parsed.ast, info.node, kind);
            let cur_is_multiline_def = is_multiline_item_def(&self.parsed.ast, info.node);

            // Determine separator (`HardLine` vs `BlankLine`) from the previous element.
            if !parts.is_empty() {
                let user_had_blank = self.parsed.blank_lines_in_gap(prev_end_byte, info.lo) > 0
                    || self.parsed.blank_lines_in_gap(gap_cursor, info.lo) > 0;
                let group_transition = match (kind, prev_category, cur_cat) {
                    (
                        ContainerKind::AdtBody
                        | ContainerKind::ModuleBody
                        | ContainerKind::TraitOrImplBody
                        | ContainerKind::FileScope,
                        Some(prev_c),
                        cur_c,
                    ) if prev_c != cur_c => true,
                    _ => false,
                };
                let top_def_spacing = matches!(
                    kind,
                    ContainerKind::FileScope
                        | ContainerKind::ModuleBody
                        | ContainerKind::TraitOrImplBody
                ) && (prev_was_multiline_def || cur_is_multiline_def);

                // Avoid double blank line if `lead_doc` already started with an initial blank.
                let lead_starts_blank = doc_starts_with_line_break(&lead_doc);
                if (user_had_blank || group_transition || top_def_spacing) && !lead_starts_blank {
                    parts.push(Doc::BlankLine);
                } else {
                    parts.push(Doc::HardLine);
                }
            }

            // Check if a contiguous `use` (or `@use`) group starts here when `.reorder_imports` is enabled!
            if self.cfg_stack.current().reorder_imports
                && !dir_flags.skip
                && !dir_flags.preserve
                && is_sortable_use_item(self.parsed, info.node)
            {
                let group_end = self.find_contiguous_use_group_end(&elem_infos, idx);
                if group_end > idx + 1 {
                    if !matches!(lead_doc, Doc::Nil) {
                        parts.push(lead_doc);
                    }
                    let next_after_group = elem_infos.get(group_end).copied();
                    let group_doc = self.format_sorted_use_group(
                        &elem_infos[idx..group_end],
                        next_after_group,
                        kind,
                    );
                    parts.push(group_doc);
                    let last_in_group = elem_infos[group_end - 1];
                    prev_end_byte = last_in_group.sep_end_byte;
                    // Consume any same-line trailing comment on the last item of the group if not yet taken.
                    let _ = last_in_group;
                    prev_category = Some(MemberCategory::UseImport);
                    prev_was_multiline_def = false;
                    idx = group_end;
                    continue;
                }
            }

            if !matches!(lead_doc, Doc::Nil) {
                parts.push(lead_doc);
            }

            let is_last = idx + 1 == elem_infos.len();
            let next_info = elem_infos.get(idx + 1).copied();
            let next_limit_byte = next_info.map_or(close_start_byte, |n| n.lo);

            let elem_doc = if dir_flags.skip {
                // Include original trailing `;` or `,` if needed via normal terminator logic!
                let body = self.format_skipped_bytes(info.lo, info.hi);
                let term = self.element_terminator(
                    info,
                    is_last,
                    next_info,
                    kind,
                    raw_ends_with_rbrace(self.parsed, info.slot),
                );
                Doc::concat([body, term])
            } else if dir_flags.preserve {
                let body = self.format_preserved_bytes(info.lo, info.hi);
                let term = self.element_terminator(
                    info,
                    is_last,
                    next_info,
                    kind,
                    raw_ends_with_rbrace(self.parsed, info.slot),
                );
                Doc::concat([body, term])
            } else {
                let active_mw = self.cfg_stack.current().max_width;
                let (body, ends_with_rbrace) =
                    self.format_container_member(info.node, info.slot, is_last, kind);
                let term =
                    self.element_terminator(info, is_last, next_info, kind, ends_with_rbrace);
                Doc::with_max_width(active_mw, Doc::concat([body, term]))
            };

            let cfg_now = self.cfg();
            let trailing_comment = self.comments.take_same_line_trailing(
                info.end_line,
                next_limit_byte,
                self.indent_spaces,
                &cfg_now,
            );
            parts.push(elem_doc);
            if !matches!(trailing_comment, Doc::Nil) {
                parts.push(trailing_comment);
            }

            prev_end_byte = info.sep_end_byte.max(self.last_consumed_comment_end());
            prev_category = Some(cur_cat);
            prev_was_multiline_def = cur_is_multiline_def;
            idx += 1;
        }

        // Emit any remaining comments before `close_start_byte` (e.g. comments at end of block/file).
        let (end_comments, _, _) = self.comments.take_leading_before(
            prev_end_byte,
            close_start_byte,
            self.indent_spaces,
            self.brace_depth,
            true,
            &mut self.cfg_stack,
        );
        if !matches!(end_comments, Doc::Nil) {
            if !parts.is_empty() {
                let had_blank = self
                    .parsed
                    .blank_lines_in_gap(prev_end_byte, close_start_byte)
                    > 0;
                if had_blank && !doc_starts_with_line_break(&end_comments) {
                    parts.push(Doc::BlankLine);
                } else {
                    parts.push(Doc::HardLine);
                }
            }
            parts.push(trim_trailing_hardline(end_comments));
        }

        Doc::concat(parts)
    }

    fn last_consumed_comment_end(&self) -> usize {
        if self.comments.next_idx == 0 {
            0
        } else {
            self.parsed.comments[self.comments.next_idx - 1].byte_end
        }
    }

    fn element_terminator(
        &self,
        info: ElemInfo,
        is_last: bool,
        next_info: Option<ElemInfo>,
        container_kind: ContainerKind,
        ends_with_rbrace: bool,
    ) -> Doc {
        let base_kind = unwrap_modifiers_kind(&self.parsed.ast, info.node);

        // Comma-separated items (`Configuration`, `Property`, `StructField`, `EnumVariant`, `UnionVariant`):
        if is_comma_rule_member(base_kind, container_kind) {
            // In multi-line containers (`FileScope`, `AdtBody`, `ModuleBody`, `TraitOrImplBody`),
            // vertical trailing-comma rule + `try_multi` parser requirement both give `,`!
            return Doc::str(",");
        }

        // Semicolon-separated items:
        // 1. Inside `ValueBlock`, the final item:
        //    - If `NodeKind::ExprStatement`, user explicitly wrote `;` to suppress return value -> preserve `;`!
        //    - If a bare trailing expression (not a declaration requiring `;`), never add `;`!
        let direct_kind = self.parsed.ast.get_node_kind(info.node);
        if container_kind == ContainerKind::ValueBlock && is_last {
            if direct_kind == Some(NodeKind::ExprStatement) {
                return Doc::str(";");
            }
            if !declaration_requires_semicolon_without_brace(base_kind) {
                return Doc::Nil;
            }
        }

        // 2. If formatted element ends in `}`:
        if ends_with_rbrace {
            // Check greedy continuation ambiguity when an expression-ending statement is followed
            // by another element whose first token would continue the expression across newlines!
            if let Some(next) = next_info {
                if ends_in_open_expr(base_kind, &self.parsed.ast, info.node)
                    && self.starts_with_continuation_token(next)
                {
                    return Doc::str(";");
                }
            }
            return Doc::Nil;
        }

        Doc::str(";")
    }

    fn starts_with_continuation_token(&self, next: ElemInfo) -> bool {
        let Some(tok) = self.parsed.sig_tokens.get(next.slot.first) else {
            return false;
        };
        matches!(
            tok.kind,
            TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
                | TokenKind::Dot
                | TokenKind::Quote
                | TokenKind::Question
                | TokenKind::Bang
                | TokenKind::Hash
                | TokenKind::Match
                | TokenKind::Do
                | TokenKind::Matches
                | TokenKind::Plus
                | TokenKind::SeparatedPlus
                | TokenKind::Minus
                | TokenKind::SeparatedMinus
                | TokenKind::Star
                | TokenKind::SeparatedStar
                | TokenKind::Slash
                | TokenKind::SeparatedSlash
                | TokenKind::Percent
                | TokenKind::SeparatedPercent
                | TokenKind::PlusPlus
                | TokenKind::EqEq
                | TokenKind::BangEq
                | TokenKind::Lt
                | TokenKind::SeparatedLt
                | TokenKind::LtEq
                | TokenKind::Gt
                | TokenKind::SeparatedGt
                | TokenKind::GtEq
                | TokenKind::And
                | TokenKind::Or
                | TokenKind::EqEqEq
                | TokenKind::Arrow
                | TokenKind::Pipe
                | TokenKind::PipeGt
                | TokenKind::Eq
                | TokenKind::PlusEq
                | TokenKind::MinusEq
                | TokenKind::StarEq
                | TokenKind::SlashEq
                | TokenKind::PercentEq
                | TokenKind::Colon
                | TokenKind::ColonMinus
        )
    }

    fn find_contiguous_use_group_end(&self, infos: &[ElemInfo], start_idx: usize) -> usize {
        let first_key = use_group_partition_key(self.parsed, infos[start_idx].node);
        let mut end = start_idx + 1;
        while end < infos.len() {
            let prev = infos[end - 1];
            let cur = infos[end];
            if !is_sortable_use_item(self.parsed, cur.node) {
                break;
            }
            if use_group_partition_key(self.parsed, cur.node) != first_key {
                break;
            }
            if self.parsed.enclosing_off_region(cur.lo, cur.hi).is_some() {
                break;
            }
            // Any blank line or standalone comment/directive between `prev` and `cur` breaks the group!
            if self.parsed.blank_lines_in_gap(prev.sep_end_byte, cur.lo) > 0 {
                break;
            }
            let has_intervening_standalone_comment = self.parsed.comments.iter().any(|c| {
                c.byte_start >= prev.sep_end_byte
                    && c.byte_start < cur.lo
                    && !(c.line_no == prev.end_line && c.has_code_before_on_line)
            });
            if has_intervening_standalone_comment {
                break;
            }
            end += 1;
        }
        end
    }

    fn format_sorted_use_group(
        &mut self,
        group: &[ElemInfo],
        next_after: Option<ElemInfo>,
        container_kind: ContainerKind,
    ) -> Doc {
        struct RenderedUse {
            sort_key: String,
            body: Doc,
            ends_with_rbrace: bool,
            trailing_comment: Doc,
            orig_info: ElemInfo,
        }

        let cfg_now = self.cfg();
        let mut entries = Vec::with_capacity(group.len());
        for (i, &info) in group.iter().enumerate() {
            let limit_byte = group
                .get(i + 1)
                .map(|n| n.lo)
                .or_else(|| next_after.map(|n| n.lo))
                .unwrap_or(self.parsed.original_src.len());
            let (body, ends_with_rbrace, sort_key) =
                self.format_use_item_with_key(info.node, info.slot);
            let trailing_comment = self.comments.take_same_line_trailing(
                info.end_line,
                limit_byte,
                self.indent_spaces,
                &cfg_now,
            );
            entries.push(RenderedUse {
                sort_key,
                body,
                ends_with_rbrace,
                trailing_comment,
                orig_info: info,
            });
        }

        entries.sort_by(|a, b| compare_import_keys(&a.sort_key, &b.sort_key));

        let mut docs = Vec::with_capacity(entries.len() * 2);
        let len = entries.len();
        for (idx, entry) in entries.into_iter().enumerate() {
            if idx > 0 {
                docs.push(Doc::HardLine);
            }
            let is_very_last = idx + 1 == len && next_after.is_none();
            let term = self.element_terminator(
                entry.orig_info,
                is_very_last,
                next_after,
                container_kind,
                entry.ends_with_rbrace,
            );
            docs.push(entry.body);
            docs.push(term);
            if !matches!(entry.trailing_comment, Doc::Nil) {
                docs.push(entry.trailing_comment);
            }
        }
        Doc::concat(docs)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ElemInfo {
    pub node: NodeIndex,
    pub slot: TokenSlot,
    pub lo: usize,
    pub hi: usize,
    pub sep_end_byte: usize,
    pub end_line: usize,
}

fn unwrap_modifiers_kind(ast: &ast::Ast, mut node: NodeIndex) -> Option<NodeKind> {
    loop {
        let kind = ast.get_node_kind(node)?;
        match kind {
            NodeKind::Pub | NodeKind::Private | NodeKind::FunctionTemplateDef => {
                node = *ast.get_children(node).first()?;
            }
            NodeKind::InlineStatement
                if crate::items::wraps_callable_def(
                    ast,
                    ast.get_children(node).first().copied().unwrap_or(0),
                ) =>
            {
                node = *ast.get_children(node).first()?;
            }
            NodeKind::Attribute | NodeKind::AttributeSetTrue => {
                node = *ast.get_children(node).get(1)?;
            }
            other => return Some(other),
        }
    }
}

fn is_comma_rule_member(kind: Option<NodeKind>, container_kind: ContainerKind) -> bool {
    match kind {
        Some(NodeKind::Id) => container_kind == ContainerKind::AdtBody,
        Some(
            NodeKind::Configuration
            | NodeKind::Property
            | NodeKind::StructField
            | NodeKind::PatternEnumVariant
            | NodeKind::ExprEnumVariant
            | NodeKind::TupleEnumVariant
            | NodeKind::StructEnumVariant
            | NodeKind::SubEnumEnumVariant
            | NodeKind::UnionVariant,
        ) => container_kind != ContainerKind::ValueBlock,
        _ => false,
    }
}

fn categorize_member(
    ast: &ast::Ast,
    node: NodeIndex,
    container_kind: ContainerKind,
) -> MemberCategory {
    match unwrap_modifiers_kind(ast, node) {
        Some(NodeKind::Configuration | NodeKind::Property) => MemberCategory::ConfigOrProperty,
        Some(
            NodeKind::StructField
            | NodeKind::PatternEnumVariant
            | NodeKind::ExprEnumVariant
            | NodeKind::TupleEnumVariant
            | NodeKind::StructEnumVariant
            | NodeKind::SubEnumEnumVariant
            | NodeKind::UnionVariant,
        ) => MemberCategory::FieldOrVariant,
        // Note: inside `EnumDef`, a bare `NodeKind::Id` is a unit `EnumVariant`!
        Some(NodeKind::Id) if container_kind == ContainerKind::AdtBody => {
            MemberCategory::FieldOrVariant
        }
        Some(NodeKind::AssocDecl) => MemberCategory::AssocItem,
        Some(NodeKind::UseStatement) => MemberCategory::UseImport,
        _ => MemberCategory::StatementOrDef,
    }
}

fn is_multiline_item_def(ast: &ast::Ast, node: NodeIndex) -> bool {
    let mut cur = node;
    let mut has_attr = false;
    loop {
        let Some(kind) = ast.get_node_kind(cur) else {
            return false;
        };
        match kind {
            NodeKind::Attribute => {
                has_attr = true;
                cur = ast.get_children(cur)[1];
            }
            NodeKind::Pub | NodeKind::Private | NodeKind::FunctionTemplateDef => {
                cur = ast.get_children(cur)[0];
            }
            NodeKind::InlineStatement
                if crate::items::wraps_callable_def(ast, ast.get_children(cur)[0]) =>
            {
                cur = ast.get_children(cur)[0];
            }
            NodeKind::AttributeSetTrue => {
                cur = ast.get_children(cur)[1];
            }
            NodeKind::StructDef
            | NodeKind::EnumDef
            | NodeKind::UnionDef
            | NodeKind::TraitDef
            | NodeKind::ImplDef
            | NodeKind::ImplTraitDef
            | NodeKind::ExtendDef
            | NodeKind::ExtendTraitDef
            | NodeKind::DeriverDef
            | NodeKind::CaseDef
            | NodeKind::ModuleDef
            | NodeKind::EffectEnumDef
            | NodeKind::TestDef
            | NodeKind::TestTemplateDef
            | NodeKind::TestTemplateInstance
            | NodeKind::FunctionTemplateInstance
            | NodeKind::FunctionTemplateInstanceWithParams => return true,
            NodeKind::Function | NodeKind::NormalFormHeadDef => {
                let body = ast.get_children(cur)[4];
                return has_attr || ast.get_node_kind(body) == Some(NodeKind::Block);
            }
            _ => return has_attr,
        }
    }
}

fn declaration_requires_semicolon_without_brace(kind: Option<NodeKind>) -> bool {
    matches!(
        kind,
        Some(
            NodeKind::LetDecl
                | NodeKind::VarDecl
                | NodeKind::ConstDecl
                | NodeKind::ConstDef
                | NodeKind::StaticDef
                | NodeKind::TypealiasDef
                | NodeKind::NewtypeDef
                | NodeKind::UseStatement
                | NodeKind::ModStatement
                | NodeKind::DeriveDef
                | NodeKind::AssocDecl
                | NodeKind::AlgebraicEffect
                | NodeKind::ReturnStatement
                | NodeKind::ResumeStatement
                | NodeKind::BreakStatement
                | NodeKind::ContinueStatement
                | NodeKind::Assign
                | NodeKind::AddAssign
                | NodeKind::SubAssign
                | NodeKind::MulAssign
                | NodeKind::DivAssign
                | NodeKind::RemAssign
                | NodeKind::Asserts
                | NodeKind::Assumes
                | NodeKind::Axiom
                | NodeKind::Invariant
                | NodeKind::Decreases
                | NodeKind::Requires
                | NodeKind::Ensures
        )
    )
}

fn ends_in_open_expr(base_kind: Option<NodeKind>, ast: &ast::Ast, mut node: NodeIndex) -> bool {
    while let Some(k) = ast.get_node_kind(node) {
        match k {
            NodeKind::Pub | NodeKind::Private | NodeKind::FunctionTemplateDef => {
                node = ast.get_children(node)[0];
            }
            NodeKind::Attribute | NodeKind::AttributeSetTrue => {
                node = ast.get_children(node)[1];
            }
            _ => break,
        }
    }
    match base_kind {
        Some(NodeKind::LetDecl | NodeKind::VarDecl | NodeKind::ConstDecl) => {
            // Only open-ended if `else { ... }` block is absent!
            ast.get_children(node).get(3).copied().unwrap_or(0) == 0
        }
        Some(NodeKind::Function | NodeKind::NormalFormHeadDef) => {
            let body = ast.get_children(node).get(4).copied().unwrap_or(0);
            ast.get_node_kind(body) != Some(NodeKind::Block)
        }
        Some(
            NodeKind::ExprStatement
            | NodeKind::ConstDef
            | NodeKind::StaticDef
            | NodeKind::TypealiasDef
            | NodeKind::NewtypeDef
            | NodeKind::Assign
            | NodeKind::AddAssign
            | NodeKind::SubAssign
            | NodeKind::MulAssign
            | NodeKind::DivAssign
            | NodeKind::RemAssign
            | NodeKind::ReturnStatement
            | NodeKind::ResumeStatement
            | NodeKind::BreakStatement
            | NodeKind::ContinueStatement
            | NodeKind::Asserts
            | NodeKind::Assumes
            | NodeKind::Axiom
            | NodeKind::Invariant
            | NodeKind::Decreases
            | NodeKind::Requires
            | NodeKind::Ensures,
        ) => true,
        _ => false,
    }
}

fn raw_ends_with_rbrace(parsed: &ParsedSource, slot: TokenSlot) -> bool {
    parsed
        .sig_tokens
        .get(slot.last)
        .is_some_and(|t| t.kind == TokenKind::RBrace)
}

fn is_sortable_use_item(parsed: &ParsedSource, node: NodeIndex) -> bool {
    let mut cur = node;
    loop {
        let Some(kind) = parsed.ast.get_node_kind(cur) else {
            return false;
        };
        match kind {
            NodeKind::Pub | NodeKind::Private => {
                cur = parsed.ast.get_children(cur)[0];
            }
            // Don't reorder `^attr use ...`!
            NodeKind::Attribute | NodeKind::AttributeSetTrue => return false,
            NodeKind::UseStatement => return true,
            _ => return false,
        }
    }
}

/// Partition key `(vis_tag, is_at_use)` so `use`, `pub use`, `private use`, and `@use` sort within
/// their own visibility/macro kind!
fn use_group_partition_key(parsed: &ParsedSource, mut node: NodeIndex) -> (u8, bool) {
    let mut vis = 0u8;
    while let Some(kind) = parsed.ast.get_node_kind(node) {
        match kind {
            NodeKind::Pub => {
                vis = 1;
                node = parsed.ast.get_children(node)[0];
            }
            NodeKind::Private => {
                vis = 2;
                node = parsed.ast.get_children(node)[0];
            }
            _ => break,
        }
    }
    let (exact_lo, _) = parsed.exact_byte_range(node, 1);
    let is_at_use = parsed.bridge.at_use_offsets.contains(&exact_lo);
    (vis, is_at_use)
}

/// Lexicographical comparison for Flurry `use` paths (`@` package paths, `.` super paths, identifiers, `*`).
pub fn compare_import_keys(a: &str, b: &str) -> Ordering {
    a.cmp(b)
}

fn trim_trailing_hardline(doc: Doc) -> Doc {
    match doc {
        Doc::Concat(mut items) => {
            if matches!(items.last(), Some(Doc::HardLine)) {
                items.pop();
            }
            Doc::concat(items)
        }
        other => other,
    }
}

fn doc_starts_with_line_break(doc: &Doc) -> bool {
    match doc {
        Doc::HardLine | Doc::BlankLine => true,
        Doc::Concat(items) => items.first().is_some_and(doc_starts_with_line_break),
        _ => false,
    }
}

/// Normalize `--foo` -> `-- foo` on the trailing `--` comment of a single line (outside strings/chars).
fn normalize_line_comment_suffix(line: &str, enabled: bool) -> String {
    if !enabled {
        return line.to_string();
    }
    let bytes = line.as_bytes();
    let mut i = 0usize;
    let mut in_str = false;
    let mut in_char = false;
    let mut escaped = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if in_char {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'\'' {
                in_char = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'\'' => in_char = true,
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                let before = &line[..i];
                let after = &line[i + 2..];
                if after.is_empty()
                    || after.starts_with('-')
                    || after.starts_with('%')
                    || after.starts_with(' ')
                {
                    return line.to_string();
                }
                if let Some(after_tab) = after.strip_prefix('\t') {
                    return format!("{before}-- {after_tab}");
                }
                return format!("{before}-- {after}");
            }
            _ => {}
        }
        i += 1;
    }
    line.to_string()
}
