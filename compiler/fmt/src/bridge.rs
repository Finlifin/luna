//! Lexical pre-scanner, `--%` / macro / grammar bridge, and trivia + paren index.

use std::collections::{HashMap, HashSet};

use ast::{Ast, NodeIndex, NodeKind};
use diagnostic::DiagnosticContext;
use lex::{Token, TokenKind};
use parse::{error::ParseError, parser::Parser};
use rustc_span::{BytePos, FileName, SourceMap, source_map::FilePathMapping};

use crate::directives::{DirectiveLine, DirectiveWarning, parse_directive_comment};

/// Kind of comment encountered in source order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentKind {
    /// `-- ...` single-line comment.
    Line,
    /// `{- ... -}` nestable block comment.
    Block,
}

/// A comment token (either `--` line comment or `{- ... -}` block comment) in source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentItem {
    pub kind: CommentKind,
    pub byte_start: usize,
    pub byte_end: usize,
    pub line_start: usize,
    pub line_no: usize,
    pub col_no: usize,
    /// True if non-whitespace code precedes this comment on the same line.
    pub has_code_before_on_line: bool,
    /// True if non-whitespace code follows this comment on the same line (block comments).
    pub has_code_after_on_line: bool,
    /// If this comment is a standalone `--%` directive line, indexes into `directives`.
    pub directive_index: Option<usize>,
}

/// A formatting-disabled region started by `--% .off`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffRegion {
    pub off_directive_idx: usize,
    pub on_directive_idx: Option<usize>,
    /// Start byte of the disabled body (immediately after the `--% .off` line).
    pub body_start: usize,
    /// End byte of the disabled body (start of `--% .on` line, closing `}`, or EOF).
    pub body_end: usize,
}

/// Bridged Flurry construct not yet modeled directly by `compiler/parse`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgedConstruct {
    /// `define Name(...) { ... }` template macro definition.
    TemplateMacroDef {
        start: usize,
        end: usize,
        kw_start: usize,
        brace_start: usize,
    },
    /// `@path { ... }` construction macro call (including `@builtin.str.raw { ... }`).
    BraceMacroCall {
        start: usize,
        end: usize,
        path: String,
        brace_start: usize,
    },
    /// `@path(args...)` or bare `@path` construction macro call.
    ParenMacroCall {
        start: usize,
        end: usize,
        path: String,
        /// Byte range of inner `( ... )` arguments (excluding `(` and `)`), if parenthesized.
        args_inner: Option<(usize, usize)>,
    },
    /// `$id(...)`, `$id { ... }`, `$id[...]`, or `$id` template macro invocation.
    DollarMacroCall { start: usize, end: usize },
    /// `{{ ... }}` token buffer expression.
    TokenBuffer { start: usize, end: usize },
    /// Code skipped via `--% .skip` that could not be parsed by current `compiler/parse`.
    SkippedUnparsed { start: usize, end: usize },
}

impl BridgedConstruct {
    pub fn span_bytes(&self) -> (usize, usize) {
        match *self {
            Self::TemplateMacroDef { start, end, .. }
            | Self::BraceMacroCall { start, end, .. }
            | Self::ParenMacroCall { start, end, .. }
            | Self::DollarMacroCall { start, end }
            | Self::TokenBuffer { start, end }
            | Self::SkippedUnparsed { start, end } => (start, end),
        }
    }
}

/// Which `@define_macroN` attribute replaced a macro definition prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefineMacroKind {
    Macro1,
    Macro2,
}

impl DefineMacroKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Macro1 => "@define_macro1",
            Self::Macro2 => "@define_macro2",
        }
    }
}

/// Metadata linking shadow-buffer AST nodes back to original Flurry syntax.
#[derive(Debug, Default, Clone)]
pub struct BridgeMap {
    /// Keyed by start byte offset of the placeholder node in source.
    pub constructs_by_start: HashMap<usize, BridgedConstruct>,
    /// Byte offsets where `@use` was bridged as `use `.
    pub at_use_offsets: HashSet<usize>,
    /// Byte offsets where `@define_macro1` / `@define_macro2` was bridged as `^__fmt_dmN`.
    pub define_macro_offsets: HashMap<usize, DefineMacroKind>,
    /// Byte offsets where `predicate` was bridged as `fn       `.
    pub predicate_kw_offsets: HashSet<usize>,
    /// Byte offsets where `outcomes` was bridged as `requires`.
    pub outcomes_kw_offsets: HashSet<usize>,
    /// Byte offsets where `==>` was bridged as `===`.
    pub implies_offsets: HashSet<usize>,
    /// Byte offsets where `<:` was bridged as `:-`.
    pub subtype_offsets: HashSet<usize>,
}

/// Complete pre-scan + parse result for a single Flurry source buffer.
pub struct ParsedSource {
    pub original_src: String,
    pub shadow_src: String,
    pub line_starts: Vec<usize>,
    pub comments: Vec<CommentItem>,
    pub directives: Vec<DirectiveLine>,
    pub warnings: Vec<DirectiveWarning>,
    pub off_regions: Vec<OffRegion>,
    pub bridge: BridgeMap,
    pub sig_tokens: Vec<Token>,
    pub ast: Ast,
    pub base_pos: BytePos,
}

/// Failure when lexing or parsing input source.
#[derive(Debug, Clone)]
pub struct ParseFailure {
    pub rendered_diagnostics: String,
}

impl ParsedSource {
    /// Convert a `rustc_span::Span` from `self.ast` to `(lo, hi)` byte offsets in `original_src`,
    /// expanding any bridged placeholder to its full original `(start, end)` range.
    pub fn raw_byte_range(&self, node: NodeIndex) -> (usize, usize) {
        let Some(span) = self.ast.get_span(node) else {
            return (0, 0);
        };
        let lo = span.lo().0.saturating_sub(self.base_pos.0) as usize;
        let mut hi = span.hi().0.saturating_sub(self.base_pos.0) as usize;
        if let Some(bridged) = self.bridge.constructs_by_start.get(&lo) {
            hi = hi.max(bridged.span_bytes().1);
        }
        // `Parser` constructs `NodeKind::Pub` and `NodeKind::Private` with `p.current_span()`
        // evaluated BEFORE parsing the inner item, so extend `hi` through outer item wrappers.
        let mut cur = node;
        while let Some(kind) = self.ast.get_node_kind(cur) {
            let ch = self.ast.get_children(cur);
            let next = match kind {
                NodeKind::Pub
                | NodeKind::Private
                | NodeKind::FunctionTemplateDef
                | NodeKind::InlineStatement => ch.first().copied(),
                NodeKind::Attribute | NodeKind::AttributeSetTrue => ch.get(1).copied(),
                _ => None,
            };
            let Some(child) = next.filter(|&c| c != 0) else {
                break;
            };
            if let Some(c_span) = self.ast.get_span(child) {
                let c_lo = c_span.lo().0.saturating_sub(self.base_pos.0) as usize;
                let c_hi = c_span.hi().0.saturating_sub(self.base_pos.0) as usize;
                hi = hi.max(c_hi);
                if let Some(bridged) = self.bridge.constructs_by_start.get(&c_lo) {
                    hi = hi.max(bridged.span_bytes().1);
                }
            }
            cur = child;
        }
        (lo, hi)
    }

    /// Resolve the exact `(first_tok_idx, last_tok_idx)` in `self.sig_tokens` for `node`,
    /// given that `node` cannot start before `floor_tok_idx`.
    ///
    /// Compensates for `Parser::current_span()` recording the preceding token's `from` on
    /// scoped nodes while preserving exact explicit spans.
    pub fn exact_token_bounds(&self, node: NodeIndex, floor_tok_idx: usize) -> (usize, usize) {
        let (raw_lo, raw_hi) = self.raw_byte_range(node);
        let max_real = self.sig_tokens.len().saturating_sub(2);
        if max_real == 0 {
            return (0, 0);
        }
        let floor = floor_tok_idx.clamp(1, max_real);

        // Find last token whose `to <= raw_hi` (or covers `raw_hi` for split TakeView chars).
        let mut last_tok = floor;
        let idx_hi = self.sig_tokens.partition_point(|t| {
            t.kind == TokenKind::Sof || (t.from < raw_hi && t.kind != TokenKind::Eof)
        });
        if idx_hi > 1 {
            let candidate = idx_hi - 1;
            if candidate >= floor {
                last_tok = candidate;
            }
        }

        // Handle leaf nodes whose exact token count is fixed when `current_span` was used.
        if let Some(kind) = self.ast.get_node_kind(node) {
            match kind {
                NodeKind::Id
                | NodeKind::Int
                | NodeKind::Real
                | NodeKind::Str
                | NodeKind::Char
                | NodeKind::Bool
                | NodeKind::Null
                | NodeKind::Undefined
                | NodeKind::SelfLower
                | NodeKind::SelfCap
                | NodeKind::Wildcard
                | NodeKind::SelfParam
                | NodeKind::ItselfParam => {
                    return (last_tok, last_tok);
                }
                NodeKind::Unit
                | NodeKind::RangeFull
                | NodeKind::SelfRefParam
                | NodeKind::ItselfRefParam => {
                    let first = last_tok.saturating_sub(1).max(floor);
                    return (first, last_tok);
                }
                NodeKind::Attribute => {
                    // `Attribute` wraps `^ attr_expr target`; include the leading `^` token.
                    let children = self.ast.get_children(node);
                    if !children.is_empty() {
                        let (attr_first, _) = self.exact_token_bounds(children[0], floor);
                        if attr_first > floor
                            && self.sig_tokens[attr_first - 1].kind == TokenKind::Caret
                        {
                            return (attr_first - 1, last_tok);
                        }
                    }
                }
                NodeKind::IfIsMatch | NodeKind::IfMatch => {
                    // `try_if_is_match` / `try_if_match` open a nested `scoped` after `if expr is`;
                    // recover the leading `if` token right before `expr` (`children[0]`).
                    let children = self.ast.get_children(node);
                    if !children.is_empty() {
                        let (expr_first, _) = self.exact_token_bounds(children[0], floor);
                        let mut k = expr_first;
                        while k > floor {
                            k -= 1;
                            if self.sig_tokens[k].kind == TokenKind::If {
                                return (k, last_tok);
                            }
                            if self.sig_tokens[k].kind != TokenKind::LParen {
                                break;
                            }
                        }
                    }
                }
                NodeKind::WhileIsMatch | NodeKind::WhileMatch => {
                    // `try_while_is_match` / `try_while_match` open a nested `scoped` after `while (:lbl)? expr is`;
                    // recover the leading `while` token before `expr` (`children[1]`).
                    let children = self.ast.get_children(node);
                    if children.len() >= 2 {
                        let (expr_first, _) = self.exact_token_bounds(children[1], floor);
                        let mut k = expr_first;
                        while k > floor {
                            k -= 1;
                            if self.sig_tokens[k].kind == TokenKind::While {
                                return (k, last_tok);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Locate token at `raw_lo`.
        let mut first_tok = self
            .sig_tokens
            .partition_point(|t| {
                t.kind == TokenKind::Sof || (t.to <= raw_lo && t.kind != TokenKind::Eof)
            })
            .clamp(floor, last_tok);

        // If `raw_lo` matched the start of token `t` ONLY because `Parser::current_span()`
        // recorded the token *before* this node (`start` on `cursor_stack`), detect whether
        // token `first_tok` can genuinely start `node`.
        if first_tok < last_tok
            && self.sig_tokens[first_tok].from == raw_lo
            && !token_can_start_node(
                self.ast.get_node_kind(node),
                self.sig_tokens[first_tok].kind,
            )
        {
            first_tok += 1;
        }

        (first_tok, last_tok)
    }

    /// Exact `(lo, hi)` byte range in `original_src` for `node` given `floor_tok_idx`.
    pub fn exact_byte_range(&self, node: NodeIndex, floor_tok_idx: usize) -> (usize, usize) {
        if let Some(NodeKind::TakeView) = self.ast.get_node_kind(node) {
            // `value'a'b` may split a single `Char` token across nested `TakeView` nodes;
            // its explicit AST span is already exact.
            let (raw_lo, raw_hi) = self.raw_byte_range(node);
            return (raw_lo, raw_hi);
        }
        let (first_tok, last_tok) = self.exact_token_bounds(node, floor_tok_idx);
        if first_tok == 0 || last_tok == 0 || first_tok >= self.sig_tokens.len() {
            return self.raw_byte_range(node);
        }
        let lo = self.sig_tokens[first_tok].from;
        let mut hi = self.sig_tokens[last_tok].to;
        if let Some(bridged) = self.bridge.constructs_by_start.get(&lo) {
            hi = hi.max(bridged.span_bytes().1);
        }
        (lo, hi)
    }

    /// Count explicit user-written `( ... )` parenthesis pairs wrapping `node` inside
    /// the enclosing token slot `[slot_first_tok ..= slot_last_tok]`.
    pub fn count_outer_parens(
        &self,
        node: NodeIndex,
        slot_first_tok: usize,
        slot_last_tok: usize,
    ) -> (usize, usize, usize) {
        if slot_first_tok == 0
            || slot_last_tok == 0
            || slot_first_tok > slot_last_tok
            || slot_last_tok >= self.sig_tokens.len()
        {
            let (f, l) = self.exact_token_bounds(node, slot_first_tok.max(1));
            return (0, f, l);
        }
        let mut left = slot_first_tok;
        let mut right = slot_last_tok;
        let mut count = 0usize;

        while left < right
            && self.sig_tokens[left].kind == TokenKind::LParen
            && self.sig_tokens[right].kind == TokenKind::RParen
            && matching_paren_at(&self.sig_tokens, left) == Some(right)
        {
            // Stop unwrapping if the `( ... )` IS the node itself (`Unit` or `Tuple` or `TuplePattern`).
            let (inner_f, inner_l) = self.exact_token_bounds(node, left);
            if matches!(
                self.ast.get_node_kind(node),
                Some(NodeKind::Unit | NodeKind::Tuple | NodeKind::TuplePattern)
            ) && inner_f == left
                && inner_l == right
            {
                break;
            }
            count += 1;
            left += 1;
            right -= 1;
        }
        (count, left, right)
    }

    /// Find a token of `target_kind` at delimiter depth 0 within `[slot_first ..= slot_last]`.
    pub fn find_depth_zero_token(
        &self,
        slot_first: usize,
        slot_last: usize,
        pred: impl Fn(TokenKind) -> bool,
        reverse: bool,
    ) -> Option<usize> {
        if slot_first == 0 || slot_first > slot_last || slot_last >= self.sig_tokens.len() {
            return None;
        }
        let mut depth = 0isize;
        let mut matches_at_zero = Vec::new();
        for idx in slot_first..=slot_last {
            let kind = self.sig_tokens[idx].kind;
            match kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                    if depth == 0 && pred(kind) {
                        matches_at_zero.push(idx);
                    }
                    depth += 1;
                }
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth = (depth - 1).max(0);
                    if depth == 0 && pred(kind) {
                        matches_at_zero.push(idx);
                    }
                }
                _ => {
                    if depth == 0 && pred(kind) {
                        matches_at_zero.push(idx);
                    }
                }
            }
        }
        if reverse {
            matches_at_zero.pop()
        } else {
            matches_at_zero.first().copied()
        }
    }

    /// 1-based line number for a byte offset.
    pub fn line_of_byte(&self, byte_pos: usize) -> usize {
        self.line_starts.partition_point(|&start| start <= byte_pos)
    }

    /// 0-based column in spaces (expanding `\t` by `tab_spaces`) at `byte_pos` on its line.
    pub fn visual_column(&self, byte_pos: usize, tab_spaces: usize) -> usize {
        let line_idx = self.line_of_byte(byte_pos).saturating_sub(1);
        let line_start = self.line_starts.get(line_idx).copied().unwrap_or(0);
        let slice = self
            .original_src
            .get(line_start..byte_pos.min(self.original_src.len()))
            .unwrap_or("");
        let mut col = 0usize;
        for ch in slice.chars() {
            if ch == '\t' {
                col += tab_spaces - (col % tab_spaces);
            } else {
                col += 1;
            }
        }
        col
    }

    /// Count blank lines (lines containing only horizontal whitespace) strictly between
    /// `end_a` and `start_b` when there are no comments in `end_a..start_b`.
    pub fn blank_lines_in_gap(&self, end_a: usize, start_b: usize) -> usize {
        if start_b <= end_a || end_a > self.original_src.len() {
            return 0;
        }
        let slice = &self.original_src[end_a..start_b.min(self.original_src.len())];
        let newline_count = slice.bytes().filter(|&b| b == b'\n').count();
        newline_count.saturating_sub(1)
    }

    /// Check if `[lo..hi]` overlaps any `--% .off` disabled region.
    pub fn enclosing_off_region(&self, lo: usize, hi: usize) -> Option<&OffRegion> {
        self.off_regions
            .iter()
            .find(|r| lo >= r.body_start && hi <= r.body_end && r.body_start < r.body_end)
    }
}

fn matching_paren_at(tokens: &[Token], open_idx: usize) -> Option<usize> {
    if tokens.get(open_idx)?.kind != TokenKind::LParen {
        return None;
    }
    let mut depth = 0usize;
    for (idx, tok) in tokens.iter().enumerate().skip(open_idx) {
        match tok.kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            TokenKind::Eof => break,
            _ => {}
        }
    }
    None
}

fn token_can_start_node(node_kind: Option<NodeKind>, tok: TokenKind) -> bool {
    // Punctuation tokens that only terminate or separate an earlier sibling can never start a node.
    if matches!(
        tok,
        TokenKind::Semi
            | TokenKind::Comma
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::RBrace
            | TokenKind::FatArrow
            | TokenKind::Arrow
            | TokenKind::Eq
            | TokenKind::PlusEq
            | TokenKind::MinusEq
            | TokenKind::StarEq
            | TokenKind::SlashEq
            | TokenKind::PercentEq
            | TokenKind::Colon
            | TokenKind::ColonMinus
    ) {
        return false;
    }
    // Check specific node kinds whose leading token is unambiguous.
    match node_kind {
        Some(
            NodeKind::Block | NodeKind::Object | NodeKind::TokenBuffer | NodeKind::StructPattern,
        ) => tok == TokenKind::LBrace,
        Some(NodeKind::ListOf | NodeKind::ListPattern) => tok == TokenKind::LBracket,
        Some(NodeKind::Tuple | NodeKind::TuplePattern | NodeKind::Unit) => tok == TokenKind::LParen,
        Some(NodeKind::StructDef) => tok == TokenKind::Struct,
        Some(NodeKind::EnumDef) => tok == TokenKind::Enum,
        Some(NodeKind::UnionDef) => tok == TokenKind::Union,
        Some(NodeKind::TraitDef) => tok == TokenKind::Trait,
        Some(NodeKind::ImplDef | NodeKind::ImplTraitDef) => tok == TokenKind::Impl,
        Some(NodeKind::ExtendDef | NodeKind::ExtendTraitDef) => tok == TokenKind::Extend,
        Some(NodeKind::DeriveDef | NodeKind::DeriverDef) => tok == TokenKind::Derive,
        Some(NodeKind::LetDecl) => tok == TokenKind::Let,
        Some(NodeKind::VarDecl) => tok == TokenKind::Var,
        Some(NodeKind::ConstDecl | NodeKind::ConstDef) => tok == TokenKind::Const,
        Some(NodeKind::StaticDef) => tok == TokenKind::Static,
        Some(NodeKind::UseStatement) => tok == TokenKind::Use,
        Some(NodeKind::ModuleDef | NodeKind::ModStatement) => tok == TokenKind::Mod,
        Some(NodeKind::TypealiasDef) => tok == TokenKind::Typealias,
        Some(NodeKind::NewtypeDef) => tok == TokenKind::Newtype,
        Some(NodeKind::TestDef | NodeKind::TestTemplateInstance) => tok == TokenKind::Test,
        Some(NodeKind::FunctionTemplateDef | NodeKind::TestTemplateDef) => {
            tok == TokenKind::Template
        }
        Some(NodeKind::IfStatement | NodeKind::IfIsMatch | NodeKind::IfMatch) => {
            tok == TokenKind::If
        }
        Some(NodeKind::WhenStatement) => tok == TokenKind::When,
        Some(NodeKind::WhileStatement | NodeKind::WhileIsMatch | NodeKind::WhileMatch) => {
            tok == TokenKind::While
        }
        Some(NodeKind::ForStatement) => tok == TokenKind::For,
        Some(NodeKind::ReturnStatement) => tok == TokenKind::Return,
        Some(NodeKind::ResumeStatement) => tok == TokenKind::Resume,
        Some(NodeKind::BreakStatement) => tok == TokenKind::Break,
        Some(NodeKind::ContinueStatement) => tok == TokenKind::Continue,
        Some(NodeKind::Pub) => tok == TokenKind::Pub,
        Some(NodeKind::Private) => tok == TokenKind::Private,
        Some(
            NodeKind::Configuration
            | NodeKind::OptionalParam
            | NodeKind::OptionalDeclClause
            | NodeKind::OptionalArg
            | NodeKind::Symbol,
        ) => tok == TokenKind::Dot,
        _ => true,
    }
}

/// Parse and index `src` (named `display_name` for diagnostics).
/// When `emit_diagnostics` is true, lex/parse errors are rendered through `DiagnosticContext` to stderr.
pub fn parse_and_bridge(
    src: &str,
    display_name: &str,
    emit_diagnostics: bool,
) -> Result<ParsedSource, ParseFailure> {
    let scan = pre_scan_source(src);
    let mut shadow_bytes = scan.shadow_bytes;
    let mut bridge = scan.bridge;

    // Attempt 1: parse with `.off` regions left unmasked (so valid Flurry code inside `.off`
    // builds normal AST nodes while `.off` regions still preserve verbatim formatting).
    match try_lex_and_parse(&shadow_bytes, display_name, false) {
        Ok((shadow_src, sig_tokens, ast, base_pos)) => Ok(ParsedSource {
            original_src: src.to_string(),
            shadow_src,
            line_starts: scan.line_starts,
            comments: scan.comments,
            directives: scan.directives,
            warnings: scan.warnings,
            off_regions: scan.off_regions,
            bridge,
            sig_tokens,
            ast,
            base_pos,
        }),
        Err(first_err_offset) => {
            // Check whether the error occurred inside an `--% .off` region or immediately after
            // a `--% .skip` directive! If so, mask those protected ranges and retry.
            let mut masked_any = false;
            for region in &scan.off_regions {
                if region.body_start < region.body_end {
                    blank_preserving_newlines(
                        &mut shadow_bytes,
                        region.body_start,
                        region.body_end,
                    );
                    masked_any = true;
                }
            }
            if !masked_any {
                if let Some(skip_construct) = try_mask_skipped_target(
                    src,
                    &mut shadow_bytes,
                    &scan.directives,
                    first_err_offset,
                ) {
                    let (s, _) = skip_construct.span_bytes();
                    bridge.constructs_by_start.insert(s, skip_construct);
                    masked_any = true;
                }
            }

            if masked_any {
                if let Ok((shadow_src, sig_tokens, ast, base_pos)) =
                    try_lex_and_parse(&shadow_bytes, display_name, false)
                {
                    return Ok(ParsedSource {
                        original_src: src.to_string(),
                        shadow_src,
                        line_starts: scan.line_starts,
                        comments: scan.comments,
                        directives: scan.directives,
                        warnings: scan.warnings,
                        off_regions: scan.off_regions,
                        bridge,
                        sig_tokens,
                        ast,
                        base_pos,
                    });
                }
            }

            // Genuine syntax/lex error outside `.off` / `.skip`: emit diagnostics against original
            // line/byte positions and fail without modifying the file.
            let rendered =
                emit_lex_or_parse_failure(&shadow_bytes, src, display_name, emit_diagnostics);
            Err(ParseFailure {
                rendered_diagnostics: rendered,
            })
        }
    }
}

fn try_lex_and_parse(
    shadow_bytes: &[u8],
    display_name: &str,
    _emit: bool,
) -> Result<(String, Vec<Token>, Ast, BytePos), usize> {
    let shadow_src = std::str::from_utf8(shadow_bytes)
        .map_err(|e| e.valid_up_to())?
        .to_string();
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(
        FileName::Custom(display_name.to_string()),
        shadow_src.clone(),
    );
    let (tokens, symbols, lex_errors) = lex::lex(&shadow_src, file.start_pos);
    if let Some(first_lex_err) = lex_errors.first() {
        return Err(first_lex_err.start_position() as usize);
    }
    let sig_tokens: Vec<Token> = tokens
        .iter()
        .copied()
        .filter(|t| t.kind != TokenKind::Comment)
        .collect();
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    match parser.try_file_scope() {
        Ok(root) => {
            let mut ast = parser.finalize();
            ast.root = root;
            Ok((shadow_src, sig_tokens, ast, file.start_pos))
        }
        Err(parse_err) => {
            let sp = parse_err.to_span();
            let off = sp.lo().0.saturating_sub(file.start_pos.0) as usize;
            Err(off)
        }
    }
}

fn emit_lex_or_parse_failure(
    shadow_bytes: &[u8],
    original_src: &str,
    display_name: &str,
    emit_to_stderr: bool,
) -> String {
    let shadow_str = std::str::from_utf8(shadow_bytes).unwrap_or(original_src);
    let map = SourceMap::new(FilePathMapping::empty());
    // Register `original_src` in SourceMap so ariadne displays original user code!
    let orig_file = map.new_source_file(
        FileName::Custom(display_name.to_string()),
        original_src.to_string(),
    );
    let (tokens, symbols, lex_errors) = lex::lex(shadow_str, orig_file.start_pos);
    if !lex_errors.is_empty() {
        if emit_to_stderr {
            let diag_ctx = DiagnosticContext::new(&map);
            for err in &lex_errors {
                use diagnostic::FlurryError as _;
                err.emit(&diag_ctx, orig_file.start_pos);
            }
            return diag_ctx.failure_summary("lexing");
        }
        return format!("lexical error in `{display_name}`: {:?}", lex_errors[0]);
    }
    let mut parser = Parser::new(&map, tokens, symbols, orig_file.start_pos);
    match parser.try_file_scope() {
        Ok(_) => String::new(),
        Err(err) => {
            if emit_to_stderr {
                let diag_ctx = DiagnosticContext::new(&map);
                use diagnostic::FlurryError as _;
                err.emit(&diag_ctx, orig_file.start_pos);
                diag_ctx.failure_summary("parsing")
            } else {
                format!(
                    "syntax error in `{display_name}`: {}",
                    format_parse_error_plain(&map, orig_file.start_pos, &err)
                )
            }
        }
    }
}

fn format_parse_error_plain(map: &SourceMap, _base: BytePos, err: &ParseError) -> String {
    let sp = err.to_span();
    if sp != rustc_span::DUMMY_SP {
        let loc = map.lookup_char_pos(sp.lo());
        format!("{}:{}: {}", loc.line, loc.col.0 + 1, err.message())
    } else {
        err.message().to_string()
    }
}

struct PreScanResult {
    shadow_bytes: Vec<u8>,
    line_starts: Vec<usize>,
    comments: Vec<CommentItem>,
    directives: Vec<DirectiveLine>,
    warnings: Vec<DirectiveWarning>,
    off_regions: Vec<OffRegion>,
    bridge: BridgeMap,
}

fn compute_line_starts(src: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (idx, b) in src.bytes().enumerate() {
        if b == b'\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn pre_scan_source(src: &str) -> PreScanResult {
    let line_starts = compute_line_starts(src);
    let mut shadow = src.as_bytes().to_vec();
    let mut comments = Vec::new();
    let mut directives: Vec<DirectiveLine> = Vec::new();
    let mut warnings = Vec::new();
    let mut off_regions: Vec<OffRegion> = Vec::new();
    let mut bridge = BridgeMap::default();

    // Active `--% .off` state: `(directive_idx, brace_depth_at_off, body_start_byte)`.
    let mut active_off: Option<(usize, usize, usize)> = None;

    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut brace_depth = 0usize;
    let mut can_end_expr = false;
    let mut prev_was_rbrace = false;
    let mut prev_sig_word: Option<&str> = None;
    let mut in_use_stmt = false;
    let mut use_brace_depth = 0usize;

    while i < len {
        let b = bytes[i];

        // Skip horizontal/vertical whitespace.
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Line comment: `-- ...`
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            let comment_start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            let mut comment_end = i;
            if comment_end > comment_start && bytes[comment_end - 1] == b'\r' {
                comment_end -= 1;
            }
            let line_end_with_newline = if i < len && bytes[i] == b'\n' {
                i + 1
            } else {
                i
            };
            let line_idx = line_starts
                .partition_point(|&s| s <= comment_start)
                .saturating_sub(1);
            let line_start = line_starts[line_idx];
            let prefix_on_line = &src[line_start..comment_start];
            let has_code_before = prefix_on_line.chars().any(|c| !matches!(c, ' ' | '\t'));
            let col_no = prefix_on_line.chars().count() + 1;
            let raw_comment = &src[comment_start..comment_end];

            let mut directive_index = None;
            if !has_code_before && raw_comment.starts_with("--%") {
                let (actions, normalized, warn) =
                    parse_directive_comment(raw_comment, line_idx + 1, col_no);
                let is_valid = warn.is_none();
                if let Some(w) = warn {
                    warnings.push(w);
                }
                let d_idx = directives.len();
                let dir = DirectiveLine {
                    line_start,
                    comment_start,
                    comment_end,
                    line_end_with_newline,
                    line_no: line_idx + 1,
                    col_no,
                    brace_depth,
                    raw_text: raw_comment.to_string(),
                    normalized_text: normalized,
                    actions,
                    is_valid,
                };
                if dir.has_off() && active_off.is_none() {
                    active_off = Some((d_idx, brace_depth, line_end_with_newline));
                } else if dir.has_on() {
                    if let Some((off_idx, _, body_start)) = active_off.take() {
                        off_regions.push(OffRegion {
                            off_directive_idx: off_idx,
                            on_directive_idx: Some(d_idx),
                            body_start,
                            body_end: line_start,
                        });
                    }
                }
                directives.push(dir);
                directive_index = Some(d_idx);
            }

            comments.push(CommentItem {
                kind: CommentKind::Line,
                byte_start: comment_start,
                byte_end: comment_end,
                line_start,
                line_no: line_idx + 1,
                col_no,
                has_code_before_on_line: has_code_before,
                has_code_after_on_line: false,
                directive_index,
            });
            continue;
        }

        // Block comment: `{- ... -}` (nested).
        if b == b'{' && i + 1 < len && bytes[i + 1] == b'-' {
            let comment_start = i;
            let line_idx = line_starts
                .partition_point(|&s| s <= comment_start)
                .saturating_sub(1);
            let line_start = line_starts[line_idx];
            let prefix_on_line = &src[line_start..comment_start];
            let has_code_before = prefix_on_line.chars().any(|c| !matches!(c, ' ' | '\t'));
            let col_no = prefix_on_line.chars().count() + 1;

            i += 2;
            let mut depth = 1usize;
            while i < len && depth > 0 {
                if bytes[i] == b'{' && i + 1 < len && bytes[i + 1] == b'-' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'-' && i + 1 < len && bytes[i + 1] == b'}' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let comment_end = i;
            let mut rest_idx = comment_end;
            let mut has_code_after = false;
            while rest_idx < len && bytes[rest_idx] != b'\n' && bytes[rest_idx] != b'\r' {
                if bytes[rest_idx] == b'-' && rest_idx + 1 < len && bytes[rest_idx + 1] == b'-' {
                    break;
                }
                if !matches!(bytes[rest_idx], b' ' | b'\t') {
                    has_code_after = true;
                    break;
                }
                rest_idx += 1;
            }
            comments.push(CommentItem {
                kind: CommentKind::Block,
                byte_start: comment_start,
                byte_end: comment_end,
                line_start,
                line_no: line_idx + 1,
                col_no,
                has_code_before_on_line: has_code_before,
                has_code_after_on_line: has_code_after,
                directive_index: None,
            });
            continue;
        }

        // String literal `"..."` (with `${ ... }` interpolation).
        if b == b'"' {
            i = skip_flurry_string(bytes, i);
            can_end_expr = true;
            prev_was_rbrace = false;
            prev_sig_word = None;
            continue;
        }

        // Quote, take-view, `'c'` char literal, or `'{ ... }` macro content.
        if b == b'\'' {
            let next_i = skip_quote_or_char(src, i, can_end_expr, prev_was_rbrace);
            can_end_expr = next_i > i + 1;
            prev_was_rbrace = false;
            prev_sig_word = None;
            i = next_i;
            continue;
        }

        // Check if an unclosed `--% .off` region is closed by exiting its enclosing `{}` block!
        if b == b'}' {
            if let Some((off_idx, off_depth, body_start)) = active_off {
                if brace_depth == off_depth && off_depth > 0 {
                    active_off = None;
                    let line_idx = line_starts.partition_point(|&s| s <= i).saturating_sub(1);
                    let close_line_start = line_starts[line_idx];
                    let prefix = &src[close_line_start..i];
                    let body_end = if prefix.chars().all(|c| matches!(c, ' ' | '\t')) {
                        close_line_start
                    } else {
                        i
                    };
                    off_regions.push(OffRegion {
                        off_directive_idx: off_idx,
                        on_directive_idx: None,
                        body_start,
                        body_end: body_end.max(body_start),
                    });
                }
            }
        }

        // Do not apply grammar bridging inside an active `--% .off` region unless needed;
        // wait: if valid bridged constructs appear inside `.off`, bridging them in `shadow`
        // still keeps `original_src` untouched and helps Attempt 1 succeed!

        // 1. Implies operator `==>` -> `===` (3 bytes -> 3 bytes)
        if b == b'=' && i + 2 < len && bytes[i + 1] == b'=' && bytes[i + 2] == b'>' {
            shadow[i + 2] = b'=';
            bridge.implies_offsets.insert(i);
            i += 3;
            can_end_expr = false;
            prev_was_rbrace = false;
            prev_sig_word = None;
            continue;
        }

        // 2. Subtype operator `<:` -> `:-` (2 bytes -> 2 bytes)
        if b == b'<' && i + 1 < len && bytes[i + 1] == b':' {
            shadow[i] = b':';
            shadow[i + 1] = b'-';
            bridge.subtype_offsets.insert(i);
            i += 2;
            can_end_expr = false;
            prev_was_rbrace = false;
            prev_sig_word = None;
            continue;
        }

        // 3. Token buffer `{{ ... }}` (contiguous `{{` and `}}`)
        if b == b'{' && i + 1 < len && bytes[i + 1] == b'{' {
            if let Some(end) = scan_double_brace_token_buffer(src, i) {
                // Replace `{{ ... }}` (`i..end`, >= 4 bytes) with `do` + spaces/newlines + `{}`.
                shadow[i] = b'd';
                shadow[i + 1] = b'o';
                blank_preserving_newlines(&mut shadow, i + 2, end - 2);
                shadow[end - 2] = b'{';
                shadow[end - 1] = b'}';
                bridge
                    .constructs_by_start
                    .insert(i, BridgedConstruct::TokenBuffer { start: i, end });
                i = end;
                can_end_expr = true;
                prev_was_rbrace = true;
                prev_sig_word = None;
                continue;
            }
        }

        // 4. Construction macro `@...` (outside `use` paths)
        if b == b'@' && !in_use_stmt {
            if let Some((next_pos, ended_with_rbrace)) =
                bridge_at_construct(src, i, &mut shadow, &mut bridge, &mut in_use_stmt)
            {
                i = next_pos;
                can_end_expr = ended_with_rbrace;
                prev_was_rbrace = ended_with_rbrace;
                prev_sig_word = None;
                continue;
            }
        }

        // 5. Template macro call `$id...`
        if b == b'$' {
            if let Some((next_pos, ended_with_rbrace)) =
                bridge_dollar_construct(src, i, &mut shadow, &mut bridge)
            {
                i = next_pos;
                can_end_expr = ended_with_rbrace;
                prev_was_rbrace = ended_with_rbrace;
                prev_sig_word = None;
                continue;
            }
        }

        // Track `{`, `}`, `;` for `brace_depth` and `in_use_stmt`.
        if b == b'{' {
            brace_depth += 1;
            if in_use_stmt {
                use_brace_depth += 1;
            }
            can_end_expr = false;
            prev_was_rbrace = false;
            prev_sig_word = None;
            i += 1;
            continue;
        }
        if b == b'}' {
            brace_depth = brace_depth.saturating_sub(1);
            if in_use_stmt && use_brace_depth > 0 {
                use_brace_depth -= 1;
                if use_brace_depth == 0 {
                    // Check if `use a.{b, c}` ends without `;`.
                    let rest = skip_ws_and_comments(src, i + 1);
                    if rest >= len || !src[rest..].starts_with('.') {
                        in_use_stmt = false;
                    }
                }
            }
            can_end_expr = true;
            prev_was_rbrace = true;
            prev_sig_word = None;
            i += 1;
            continue;
        }
        if b == b';' {
            in_use_stmt = false;
            use_brace_depth = 0;
            can_end_expr = false;
            prev_was_rbrace = false;
            prev_sig_word = None;
            i += 1;
            continue;
        }

        // Identifiers / keywords
        if let Some(ch) = src[i..].chars().next() {
            if is_ident_start(ch) {
                let word_start = i;
                i += ch.len_utf8();
                while let Some(c2) = src[i..].chars().next() {
                    if is_ident_continue(c2) {
                        i += c2.len_utf8();
                    } else {
                        break;
                    }
                }
                let word = &src[word_start..i];

                if word == "use" && prev_sig_word != Some(".") {
                    // Distinguish `use path` import from `fn foo use tmpl` or `.use(...)`.
                    if !matches!(prev_sig_word, Some(")") | Some("fn") | Some("test"))
                        && !can_end_expr
                    {
                        in_use_stmt = true;
                        use_brace_depth = 0;
                    }
                } else if word == "define" && prev_sig_word != Some(".") {
                    // Template macro definition: `define Name ... { ... }`
                    if let Some(end) =
                        try_bridge_define_macro_def(src, word_start, i, &mut shadow, &mut bridge)
                    {
                        i = end;
                        can_end_expr = true;
                        prev_was_rbrace = true;
                        prev_sig_word = None;
                        continue;
                    }
                } else if word == "predicate" && prev_sig_word != Some(".") {
                    // `predicate Name(` -> `fn        Name(` (9 bytes -> 9 bytes)
                    let after_kw = skip_ws_and_comments(src, i);
                    if let Some(id_end) = scan_ident(src, after_kw) {
                        let after_id = skip_ws_and_comments(src, id_end);
                        if after_id < len && bytes[after_id] == b'(' {
                            shadow[word_start..word_start + 9].copy_from_slice(b"fn       ");
                            bridge.predicate_kw_offsets.insert(word_start);
                        }
                    }
                } else if word == "outcomes" && prev_sig_word != Some(".") {
                    // `outcomes (: id)? [` -> `requires (: id)? [` (8 bytes -> 8 bytes)
                    let mut p = skip_ws_and_comments(src, i);
                    if p < len && bytes[p] == b':' {
                        p = skip_ws_and_comments(src, p + 1);
                        if let Some(id_end) = scan_ident(src, p) {
                            p = skip_ws_and_comments(src, id_end);
                        }
                    }
                    if p < len && bytes[p] == b'[' {
                        shadow[word_start..word_start + 8].copy_from_slice(b"requires");
                        bridge.outcomes_kw_offsets.insert(word_start);
                    }
                }

                can_end_expr = matches!(
                    word,
                    "true" | "false" | "null" | "undefined" | "self" | "Self" | "itself" | "_"
                ) || !is_keyword(word);
                prev_was_rbrace = false;
                prev_sig_word = Some(word);
                continue;
            }

            // Numbers
            if ch.is_ascii_digit() {
                i += 1;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                can_end_expr = true;
                prev_was_rbrace = false;
                prev_sig_word = None;
                continue;
            }

            // Other punctuation
            can_end_expr = matches!(ch, ')' | ']');
            prev_was_rbrace = false;
            prev_sig_word = match ch {
                '.' => Some("."),
                ')' => Some(")"),
                _ => None,
            };
            i += ch.len_utf8();
        } else {
            break;
        }
    }

    // Close any unclosed `--% .off` at EOF.
    if let Some((off_idx, _, body_start)) = active_off.take() {
        off_regions.push(OffRegion {
            off_directive_idx: off_idx,
            on_directive_idx: None,
            body_start,
            body_end: len,
        });
    }

    PreScanResult {
        shadow_bytes: shadow,
        line_starts,
        comments,
        directives,
        warnings,
        off_regions,
        bridge,
    }
}

fn blank_preserving_newlines(buf: &mut [u8], start: usize, end: usize) {
    let end = end.min(buf.len());
    for b in &mut buf[start.min(end)..end] {
        if *b != b'\n' && *b != b'\r' {
            *b = b' ';
        }
    }
}

fn bridge_at_construct(
    src: &str,
    at_pos: usize,
    shadow: &mut [u8],
    bridge: &mut BridgeMap,
    in_use_stmt: &mut bool,
) -> Option<(usize, bool)> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let after_at = at_pos + 1;
    let first_id_end = scan_ident(src, after_at)?;
    let first_seg = &src[after_at..first_id_end];

    // `@use path...` -> `use  path...` (4 bytes -> 4 bytes)
    if first_seg == "use" {
        shadow[at_pos..at_pos + 4].copy_from_slice(b"use ");
        bridge.at_use_offsets.insert(at_pos);
        *in_use_stmt = true;
        return Some((first_id_end, false));
    }

    // `@define_macro1` / `@define_macro2` -> `^__fmt_dm1    ` (14 bytes -> 14 bytes)
    if first_seg == "define_macro1" && first_id_end == at_pos + 14 {
        shadow[at_pos..at_pos + 14].copy_from_slice(b"^__fmt_dm1    ");
        bridge
            .define_macro_offsets
            .insert(at_pos, DefineMacroKind::Macro1);
        return Some((first_id_end, false));
    }
    if first_seg == "define_macro2" && first_id_end == at_pos + 14 {
        shadow[at_pos..at_pos + 14].copy_from_slice(b"^__fmt_dm2    ");
        bridge
            .define_macro_offsets
            .insert(at_pos, DefineMacroKind::Macro2);
        return Some((first_id_end, false));
    }

    // Scan dotted path `@a.b.c`
    let mut path_end = first_id_end;
    let mut path = first_seg.to_string();
    loop {
        let p = skip_ws_and_comments(src, path_end);
        if p < len && bytes[p] == b'.' {
            let after_dot = skip_ws_and_comments(src, p + 1);
            if let Some(seg_end) = scan_ident(src, after_dot) {
                path.push('.');
                path.push_str(&src[after_dot..seg_end]);
                path_end = seg_end;
                continue;
            }
        }
        break;
    }

    let after_path = skip_ws_and_comments(src, path_end);
    if after_path < len && bytes[after_path] == b'{' {
        // `@path { ... }`: for `.raw` (`@builtin.str.raw { ... }`), scan raw balanced `{}`!
        let is_raw = path == "raw" || path.ends_with(".raw");
        let close_end = if is_raw {
            scan_raw_balanced_braces(bytes, after_path)?
        } else {
            scan_balanced_delim_flurry(src, after_path, b'{', b'}')
                .or_else(|| scan_raw_balanced_braces(bytes, after_path))?
        };
        // Replace `at_pos..close_end` (>= 4 bytes) with `do` + spaces/newlines + `{ ... }`.
        shadow[at_pos] = b'd';
        shadow[at_pos + 1] = b'o';
        blank_preserving_newlines(shadow, at_pos + 2, after_path);
        shadow[after_path] = b'{';
        blank_preserving_newlines(shadow, after_path + 1, close_end - 1);
        shadow[close_end - 1] = b'}';
        bridge.constructs_by_start.insert(
            at_pos,
            BridgedConstruct::BraceMacroCall {
                start: at_pos,
                end: close_end,
                path,
                brace_start: after_path,
            },
        );
        return Some((close_end, true));
    }

    if after_path < len && bytes[after_path] == b'(' {
        let close_end = scan_balanced_delim_flurry(src, after_path, b'(', b')')?;
        shadow[at_pos] = b'_';
        shadow[at_pos + 1] = b'f';
        blank_preserving_newlines(shadow, at_pos + 2, close_end);
        bridge.constructs_by_start.insert(
            at_pos,
            BridgedConstruct::ParenMacroCall {
                start: at_pos,
                end: close_end,
                path,
                args_inner: Some((after_path + 1, close_end - 1)),
            },
        );
        return Some((close_end, false));
    }

    // Bare `@path`
    shadow[at_pos] = b'_';
    shadow[at_pos + 1] = b'f';
    blank_preserving_newlines(shadow, at_pos + 2, path_end);
    bridge.constructs_by_start.insert(
        at_pos,
        BridgedConstruct::ParenMacroCall {
            start: at_pos,
            end: path_end,
            path,
            args_inner: None,
        },
    );
    Some((path_end, false))
}

fn bridge_dollar_construct(
    src: &str,
    dollar_pos: usize,
    shadow: &mut [u8],
    bridge: &mut BridgeMap,
) -> Option<(usize, bool)> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let id_end = scan_ident(src, dollar_pos + 1)?;
    let after_id = skip_ws_and_comments(src, id_end);
    if after_id < len && bytes[after_id] == b'{' {
        let close_end = scan_balanced_delim_flurry(src, after_id, b'{', b'}')
            .or_else(|| scan_raw_balanced_braces(bytes, after_id))?;
        shadow[dollar_pos] = b'd';
        shadow[dollar_pos + 1] = b'o';
        blank_preserving_newlines(shadow, dollar_pos + 2, after_id);
        shadow[after_id] = b'{';
        blank_preserving_newlines(shadow, after_id + 1, close_end - 1);
        shadow[close_end - 1] = b'}';
        bridge.constructs_by_start.insert(
            dollar_pos,
            BridgedConstruct::DollarMacroCall {
                start: dollar_pos,
                end: close_end,
            },
        );
        return Some((close_end, true));
    }
    let end = if id_end < len && bytes[id_end] == b'(' {
        scan_balanced_delim_flurry(src, id_end, b'(', b')').unwrap_or(id_end)
    } else if id_end < len && bytes[id_end] == b'[' {
        scan_balanced_delim_flurry(src, id_end, b'[', b']').unwrap_or(id_end)
    } else {
        id_end
    };
    shadow[dollar_pos] = b'_';
    shadow[dollar_pos + 1] = b'f';
    blank_preserving_newlines(shadow, dollar_pos + 2, end);
    bridge.constructs_by_start.insert(
        dollar_pos,
        BridgedConstruct::DollarMacroCall {
            start: dollar_pos,
            end,
        },
    );
    Some((end, false))
}

fn try_bridge_define_macro_def(
    src: &str,
    kw_start: usize,
    kw_end: usize,
    shadow: &mut [u8],
    bridge: &mut BridgeMap,
) -> Option<usize> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let name_pos = skip_ws_and_comments(src, kw_end);
    let mut cursor = scan_ident(src, name_pos)?;
    cursor = skip_ws_and_comments(src, cursor);
    if cursor < len && bytes[cursor] == b'(' {
        cursor = scan_balanced_delim_flurry(src, cursor, b'(', b')')?;
        cursor = skip_ws_and_comments(src, cursor);
    }
    if cursor >= len || bytes[cursor] != b'{' {
        return None;
    }
    let brace_start = cursor;
    let end = scan_balanced_delim_flurry(src, brace_start, b'{', b'}')
        .or_else(|| scan_raw_balanced_braces(bytes, brace_start))?;

    // `define` is 6 bytes; `fn f()` is 6 bytes!
    shadow[kw_start..kw_start + 6].copy_from_slice(b"fn f()");
    blank_preserving_newlines(shadow, kw_start + 6, brace_start);
    shadow[brace_start] = b'{';
    blank_preserving_newlines(shadow, brace_start + 1, end - 1);
    shadow[end - 1] = b'}';
    bridge.constructs_by_start.insert(
        kw_start,
        BridgedConstruct::TemplateMacroDef {
            start: kw_start,
            end,
            kw_start,
            brace_start,
        },
    );
    Some(end)
}

fn scan_double_brace_token_buffer(src: &str, start: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    if start + 4 > len || bytes[start] != b'{' || bytes[start + 1] != b'{' {
        return None;
    }
    let mut i = start + 2;
    let mut inner_brace_depth = 0usize;
    while i < len {
        let b = bytes[i];
        if b == b'"' {
            i = skip_flurry_string(bytes, i);
            continue;
        }
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'{' && i + 1 < len && bytes[i + 1] == b'-' {
            i += 2;
            let mut d = 1usize;
            while i < len && d > 0 {
                if bytes[i] == b'{' && i + 1 < len && bytes[i + 1] == b'-' {
                    d += 1;
                    i += 2;
                } else if bytes[i] == b'-' && i + 1 < len && bytes[i + 1] == b'}' {
                    d -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if b == b'}' && inner_brace_depth == 0 && i + 1 < len && bytes[i + 1] == b'}' {
            return Some(i + 2);
        }
        if b == b'{' {
            inner_brace_depth += 1;
        } else if b == b'}' && inner_brace_depth > 0 {
            inner_brace_depth -= 1;
        }
        i += 1;
    }
    None
}

fn try_mask_skipped_target(
    src: &str,
    shadow: &mut [u8],
    directives: &[DirectiveLine],
    err_offset: usize,
) -> Option<BridgedConstruct> {
    // Find a `--% .skip` directive immediately preceding `err_offset` (only whitespace/comments between).
    let dir = directives
        .iter()
        .rev()
        .find(|d| d.skip_flag() == Some(true) && d.line_end_with_newline <= err_offset + 64)?;
    let target_start = skip_ws_and_comments(src, dir.line_end_with_newline);
    if err_offset < target_start {
        return None;
    }
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = target_start;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut target_end = None;

    while i < len {
        let b = bytes[i];
        if b == b'"' {
            i = skip_flurry_string(bytes, i);
            continue;
        }
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        match b {
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'{' => brace_depth += 1,
            b'}' => {
                if brace_depth == 1 && paren_depth == 0 && bracket_depth == 0 {
                    target_end = Some(i + 1);
                    break;
                }
                if brace_depth == 0 {
                    break;
                }
                brace_depth -= 1;
            }
            b';' if brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 => {
                target_end = Some(i);
                break;
            }
            _ => {}
        }
        i += 1;
    }

    let end = target_end?;
    if end.saturating_sub(target_start) < 2 {
        return None;
    }
    if bytes[end - 1] == b'}' && end - target_start >= 4 {
        shadow[target_start] = b'd';
        shadow[target_start + 1] = b'o';
        shadow[target_start + 2] = b'{';
        blank_preserving_newlines(shadow, target_start + 3, end - 1);
        shadow[end - 1] = b'}';
    } else {
        shadow[target_start] = b'_';
        shadow[target_start + 1] = b'f';
        blank_preserving_newlines(shadow, target_start + 2, end);
    }
    Some(BridgedConstruct::SkippedUnparsed {
        start: target_start,
        end,
    })
}

fn scan_raw_balanced_braces(bytes: &[u8], open_brace: usize) -> Option<usize> {
    if bytes.get(open_brace) != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    for (idx, &b) in bytes.iter().enumerate().skip(open_brace) {
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(idx + 1);
            }
        }
    }
    None
}

fn scan_balanced_delim_flurry(
    src: &str,
    open_pos: usize,
    open_b: u8,
    close_b: u8,
) -> Option<usize> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    if bytes.get(open_pos) != Some(&open_b) {
        return None;
    }
    let mut i = open_pos + 1;
    let mut depth = 1usize;
    while i < len && depth > 0 {
        let b = bytes[i];
        if b == b'"' {
            i = skip_flurry_string(bytes, i);
            continue;
        }
        if b == b'\'' {
            i = skip_quote_or_char(src, i, false, false);
            continue;
        }
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'{' && i + 1 < len && bytes[i + 1] == b'-' {
            i += 2;
            let mut cd = 1usize;
            while i < len && cd > 0 {
                if bytes[i] == b'{' && i + 1 < len && bytes[i + 1] == b'-' {
                    cd += 1;
                    i += 2;
                } else if bytes[i] == b'-' && i + 1 < len && bytes[i + 1] == b'}' {
                    cd -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if b == open_b {
            depth += 1;
        } else if b == close_b {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

fn skip_flurry_string(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start + 1;
    while i < len {
        match bytes[i] {
            b'\\' => {
                i += 2;
            }
            b'$' if i + 1 < len && bytes[i + 1] == b'{' => {
                i += 2;
                let mut brace_depth = 1usize;
                while i < len && brace_depth > 0 {
                    match bytes[i] {
                        b'"' => {
                            i = skip_flurry_string(bytes, i);
                            continue;
                        }
                        b'{' => brace_depth += 1,
                        b'}' => brace_depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
            }
            b'"' => return i + 1,
            b'\n' => return i,
            _ => i += 1,
        }
    }
    len
}

fn skip_quote_or_char(src: &str, start: usize, can_end_expr: bool, prev_was_rbrace: bool) -> usize {
    let bytes = src.as_bytes();
    let len = bytes.len();
    if start + 1 >= len {
        return len;
    }
    if prev_was_rbrace && starts_closed_char(&src[start + 1..]) {
        return consume_closed_char(&src[start + 1..], start + 1);
    }
    if can_end_expr {
        // Take-view `expr'id`
        return start + 1;
    }
    if starts_closed_char(&src[start + 1..]) {
        return consume_closed_char(&src[start + 1..], start + 1);
    }
    start + 1
}

fn starts_closed_char(after_quote: &str) -> bool {
    let mut chars = after_quote.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first == '\'' || first == '\n' || first == '\r' {
        return false;
    }
    if first == '\\' {
        let Some(esc) = chars.next() else {
            return false;
        };
        if esc == 'x' {
            if chars.next() != Some('{') {
                return false;
            }
            for c in chars.by_ref() {
                if c == '}' {
                    return chars.next() == Some('\'');
                }
            }
            return false;
        }
        return chars.next() == Some('\'');
    }
    chars.next() == Some('\'')
}

fn consume_closed_char(after_quote: &str, base: usize) -> usize {
    let mut idx = 0usize;
    let mut chars = after_quote.char_indices();
    if let Some((_, first)) = chars.next() {
        if first == '\\' {
            for (pos, c) in chars {
                if c == '\'' {
                    idx = pos + 1;
                    break;
                }
            }
        } else if let Some((pos, '\'')) = chars.next() {
            idx = pos + 1;
        }
    }
    if idx == 0 { base } else { base + idx }
}

fn skip_ws_and_comments(src: &str, mut i: usize) -> usize {
    let bytes = src.as_bytes();
    let len = bytes.len();
    while i < len {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i] == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'{' && i + 1 < len && bytes[i + 1] == b'-' {
            i += 2;
            let mut d = 1usize;
            while i < len && d > 0 {
                if bytes[i] == b'{' && i + 1 < len && bytes[i + 1] == b'-' {
                    d += 1;
                    i += 2;
                } else if bytes[i] == b'-' && i + 1 < len && bytes[i + 1] == b'}' {
                    d -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        break;
    }
    i
}

fn scan_ident(src: &str, pos: usize) -> Option<usize> {
    let rest = src.get(pos..)?;
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if !is_ident_start(first) {
        return None;
    }
    let mut end = pos + first.len_utf8();
    for (off, ch) in chars {
        if is_ident_continue(ch) {
            end = pos + off + ch.len_utf8();
        } else {
            break;
        }
    }
    Some(end)
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic()
        || c == '_'
        || matches!(
            c,
            '\u{1F000}'..='\u{1F9FF}' | '\u{2600}'..='\u{26FF}' | '\u{2700}'..='\u{27BF}'
        )
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || is_ident_start(c)
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "let"
            | "var"
            | "const"
            | "fn"
            | "struct"
            | "enum"
            | "union"
            | "trait"
            | "impl"
            | "extend"
            | "derive"
            | "case"
            | "typealias"
            | "newtype"
            | "mod"
            | "use"
            | "pub"
            | "private"
            | "if"
            | "else"
            | "when"
            | "while"
            | "for"
            | "in"
            | "do"
            | "match"
            | "matches"
            | "is"
            | "as"
            | "and"
            | "or"
            | "not"
            | "return"
            | "resume"
            | "break"
            | "continue"
            | "where"
            | "requires"
            | "ensures"
            | "decreases"
            | "asserts"
            | "assumes"
            | "axiom"
            | "invariant"
            | "pure"
            | "comptime"
            | "inline"
            | "unsafe"
            | "spec"
            | "verified"
            | "atomic"
            | "async"
            | "effect"
            | "error"
            | "catch"
            | "lambda"
            | "implicit"
            | "quote"
            | "assoc"
            | "template"
            | "test"
            | "forall"
            | "exists"
            | "dyn"
            | "lift"
            | "refines"
            | "defer"
            | "handles"
            | "ref"
            | "await"
    )
}
