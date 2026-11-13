use crate::error::{ParseError, ParseWarning};
use ast::{Ast, NodeIndex};
use diagnostic::{DiagnosticContext, FlurryError};
use lex::{Symbol, Token, TokenKind};
use rustc_span::{BytePos, SourceMap, Span};
use std::collections::{HashMap, HashSet};

pub type Cursor = usize;
pub type Symbols = HashMap<usize, Symbol>;

pub struct Parser<'a> {
    pub(crate) source_map: &'a SourceMap,
    /// Token stream (comment tokens filtered out, with a sentinel start token at index 0).
    pub(crate) tokens: Vec<Token>,
    /// Pre-interned identifier symbols keyed by original token index (+1 for sentinel).
    pub(crate) symbols: Symbols,
    /// Current token index in `self.tokens`.
    pub(crate) cursor: Cursor,
    /// Stack of start cursors for computing `current_span()`.
    pub(crate) cursor_stack: Vec<Cursor>,
    /// Active quote hole stack (each entry tracks the single permitted runtime splice in a quote).
    pub(crate) quote_holes: Vec<Option<Span>>,
    /// Collected syntax errors during recovery-enabled parsing.
    pub(crate) errors: Vec<ParseError>,
    /// Collected non-fatal syntax warnings (such as parenthesization hints).
    pub(crate) warnings: Vec<ParseWarning>,
    /// AST node indices that were explicitly wrapped in `(...)` by the user.
    pub(crate) parenthesized_nodes: HashSet<NodeIndex>,
    /// AST node indices that already received a parenthesization warning.
    pub(crate) warned_nodes: HashSet<NodeIndex>,
    /// Whether multi-error panic-mode recovery is active.
    pub(crate) recovery_enabled: bool,
    /// Nesting depth of `< ... >` parameter/argument lists (`> 0` means `>` closes `<`).
    pub(crate) angle_bracket_depth: usize,
    /// Arena AST under construction.
    pub ast: Ast,
    /// Byte start offset of the current source file in `SourceMap`.
    pub start_pos: BytePos,
}

impl<'a> Parser<'a> {
    pub fn new(
        source_map: &'a SourceMap,
        tokens: Vec<Token>,
        symbols: HashMap<usize, Symbol>,
        start_pos: BytePos,
    ) -> Self {
        let mut new_tokens = Vec::with_capacity(tokens.len());
        let mut new_symbols = HashMap::with_capacity(symbols.len());

        // `lex::lex` already prepends `TokenKind::Sof` at index 0 and appends
        // `TokenKind::Eof` at the end; strip comment trivia while reindexing symbols.
        for (old_idx, token) in tokens.into_iter().enumerate() {
            if token.kind == TokenKind::Comment {
                continue;
            }
            let new_idx = new_tokens.len();
            new_tokens.push(token);
            if let Some(&sym) = symbols.get(&old_idx) {
                new_symbols.insert(new_idx, sym);
            }
        }

        let mut result = Self {
            source_map,
            tokens: new_tokens,
            symbols: new_symbols,
            cursor: 0,
            cursor_stack: Vec::with_capacity(64),
            quote_holes: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            parenthesized_nodes: HashSet::new(),
            warned_nodes: HashSet::new(),
            recovery_enabled: false,
            angle_bracket_depth: 0,
            ast: Ast::new(start_pos),
            start_pos,
        };
        result.enter();
        result
    }

    /// Parse the entire file with multi-error recovery, emitting errors and
    /// warnings into `diag_ctx`. Returns `true` iff zero syntax errors occurred.
    pub fn parse(&mut self, diag_ctx: &DiagnosticContext) -> bool {
        let prev_recovery = self.recovery_enabled;
        self.recovery_enabled = true;
        match self.try_file_scope() {
            Ok(root) => {
                self.ast.root = root;
            }
            Err(error) => {
                self.record_error(error);
            }
        }
        self.recovery_enabled = prev_recovery;

        for error in &self.errors {
            error.emit(diag_ctx, self.start_pos);
        }
        for warning in &self.warnings {
            warning.emit(diag_ctx, self.start_pos);
        }

        self.errors.is_empty()
    }

    pub fn errors(&self) -> &[ParseError] {
        &self.errors
    }

    pub fn warnings(&self) -> &[ParseWarning] {
        &self.warnings
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub(crate) fn record_error(&mut self, error: ParseError) {
        if error.is_internal_sentinel() {
            return;
        }
        let error = error.with_fallback_span(self.eof_span());
        let span = error.to_span();
        // Avoid piling duplicate diagnostics onto the exact same byte span.
        if self.errors.iter().any(|existing| {
            existing.to_span() == span && existing.error_code() == error.error_code()
        }) {
            return;
        }
        self.errors.push(error);
    }

    pub(crate) fn record_warning(&mut self, warning: ParseWarning) {
        if self
            .warnings
            .iter()
            .any(|existing| existing.span == warning.span && existing.code == warning.code)
        {
            return;
        }
        self.warnings.push(warning);
    }

    pub(crate) fn mark_parenthesized(&mut self, node: NodeIndex) {
        if node != 0 {
            self.parenthesized_nodes.insert(node);
        }
    }

    pub fn is_parenthesized(&self, node: NodeIndex) -> bool {
        self.parenthesized_nodes.contains(&node)
    }

    /// Run a lookahead / diagnostic probe and roll back all parser, AST, and
    /// diagnostic state afterwards.
    pub(crate) fn speculative<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved_cursor = self.cursor;
        let saved_stack_len = self.cursor_stack.len();
        let saved_quote_holes = self.quote_holes.clone();
        let saved_errors_len = self.errors.len();
        let saved_warnings_len = self.warnings.len();
        let saved_parens = self.parenthesized_nodes.clone();
        let saved_warned = self.warned_nodes.clone();
        let saved_recovery = self.recovery_enabled;
        let saved_angle = self.angle_bracket_depth;
        let saved_nodes_len = self.ast.nodes.len();
        let saved_spans_len = self.ast.spans.len();
        let saved_cstart_len = self.ast.children_start.len();
        let saved_clen_len = self.ast.children_len.len();
        let saved_children_len = self.ast.children.len();
        let saved_root = self.ast.root;

        self.recovery_enabled = false;
        let out = f(self);

        self.cursor = saved_cursor;
        self.cursor_stack.truncate(saved_stack_len);
        self.quote_holes = saved_quote_holes;
        self.errors.truncate(saved_errors_len);
        self.warnings.truncate(saved_warnings_len);
        self.parenthesized_nodes = saved_parens;
        self.warned_nodes = saved_warned;
        self.recovery_enabled = saved_recovery;
        self.angle_bracket_depth = saved_angle;
        self.ast.nodes.truncate(saved_nodes_len);
        self.ast.spans.truncate(saved_spans_len);
        self.ast.children_start.truncate(saved_cstart_len);
        self.ast.children_len.truncate(saved_clen_len);
        self.ast.children.truncate(saved_children_len);
        self.ast.root = saved_root;

        out
    }

    fn enter(&mut self) {
        self.cursor_stack.push(self.cursor);
    }

    fn exit(&mut self) {
        self.cursor_stack.pop();
    }

    pub fn scoped<T, F: FnOnce(&mut Self) -> Result<T, ParseError>>(
        &mut self,
        f: F,
    ) -> Result<T, ParseError> {
        self.enter();
        let result = f(self);
        self.exit();
        result
    }

    pub fn scoped_with_expected_prefix<
        T: Default,
        F: FnOnce(&mut Self) -> Result<T, ParseError>,
    >(
        &mut self,
        prefix: &[TokenKind],
        f: F,
    ) -> Result<T, ParseError> {
        if !self.peek(prefix) {
            return Ok(Default::default());
        }

        self.enter();
        let result = f(self);
        self.exit();
        result
    }

    pub fn finalize(self) -> Ast {
        self.ast
    }

    /// Check if the next few tokens match the expected token kinds.
    pub fn peek(&self, expected: &[TokenKind]) -> bool {
        if self.cursor + 1 + expected.len() >= self.tokens.len() {
            return false;
        }

        for (i, &expected_kind) in expected.iter().enumerate() {
            if self.tokens[self.cursor + i + 1].kind != expected_kind {
                return false;
            }
        }

        true
    }

    /// Consume a token if it matches the expected kind.
    pub fn eat_token(&mut self, expected: TokenKind) -> bool {
        if self.cursor + 1 >= self.tokens.len() {
            return false;
        }

        if self.tokens[self.cursor + 1].kind == expected {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    /// Consume multiple tokens (unchecked).
    pub fn eat_tokens(&mut self, amount: usize) {
        self.cursor += amount;
    }

    /// Get the next token and advance cursor.
    pub fn next_token(&mut self) -> Token {
        if self.cursor + 1 >= self.tokens.len() {
            let eof_off = self.eof_offset();
            Token::new(TokenKind::Eof, eof_off, eof_off)
        } else {
            let token = self.tokens[self.cursor + 1];
            self.cursor += 1;
            token
        }
    }

    /// Peek at the next token without advancing cursor.
    pub fn peek_next_token(&self) -> Token {
        if self.cursor + 1 >= self.tokens.len() {
            let eof_off = self.eof_offset();
            Token::new(TokenKind::Eof, eof_off, eof_off)
        } else {
            self.tokens[self.cursor + 1]
        }
    }

    /// Peek `offset` tokens ahead (`offset = 1` is `peek_next_token`, `offset = 2` is the one after).
    pub fn peek_nth_token(&self, offset: usize) -> Token {
        let idx = self.cursor + offset;
        if idx >= self.tokens.len() {
            let eof_off = self.eof_offset();
            Token::new(TokenKind::Eof, eof_off, eof_off)
        } else {
            self.tokens[idx]
        }
    }

    /// Get the current token (at cursor position).
    pub fn current_token(&self) -> Token {
        if self.cursor >= self.tokens.len() {
            let eof_off = self.eof_offset();
            Token::new(TokenKind::Eof, eof_off, eof_off)
        } else {
            self.tokens[self.cursor]
        }
    }

    /// Get token at specific index.
    pub fn get_token(&self, index: usize) -> Token {
        if index >= self.tokens.len() {
            let eof_off = self.eof_offset();
            Token::new(TokenKind::Eof, eof_off, eof_off)
        } else {
            self.tokens[index]
        }
    }

    /// Span of a specific token relative to `self.start_pos`.
    pub fn token_span(&self, token: Token) -> Span {
        Span::new(
            self.start_pos + BytePos(token.from as u32),
            self.start_pos + BytePos(token.to as u32),
        )
    }

    /// Byte offset of EOF within this file.
    pub fn eof_offset(&self) -> usize {
        self.tokens.last().map(|t| t.to).unwrap_or(0)
    }

    /// Accurate zero-width span at the end of the current file (never `DUMMY_SP`).
    pub fn eof_span(&self) -> Span {
        let pos = self.start_pos + BytePos(self.eof_offset() as u32);
        Span::new(pos, pos)
    }

    /// Span of the most recently consumed token (`self.current_token()`).
    pub fn prev_token_span(&self) -> Span {
        if self.cursor == 0 {
            Span::new(self.start_pos, self.start_pos)
        } else {
            self.token_span(self.current_token())
        }
    }

    /// Get the previous token (before cursor).
    pub fn previous_token(&self) -> Token {
        if self.cursor == 0 {
            Token::new(TokenKind::Invalid, 0, 0)
        } else {
            self.tokens[self.cursor - 1]
        }
    }

    /// Get the span from the start of the current scope to the current cursor.
    pub fn current_span(&self) -> Span {
        if let Some(&start) = self.cursor_stack.last() {
            if self.cursor <= start {
                let off = if start < self.tokens.len() {
                    self.tokens[start].to
                } else {
                    self.eof_offset()
                };
                let pos = self.start_pos + BytePos(off as u32);
                return Span::new(pos, pos);
            }

            let start_offset = if start + 1 < self.tokens.len() {
                self.tokens[start + 1].from
            } else {
                self.eof_offset()
            };

            let end_offset = if self.cursor < self.tokens.len() {
                self.tokens[self.cursor].to
            } else {
                self.eof_offset()
            };

            let lo = self.start_pos + BytePos(start_offset as u32);
            let hi = self.start_pos + BytePos(end_offset.max(start_offset) as u32);

            Span::new(lo, hi)
        } else {
            self.eof_span()
        }
    }

    /// Get the span of the next token (falls back to `eof_span()`, never `DUMMY_SP`).
    pub fn next_token_span(&self) -> Span {
        if self.cursor + 1 < self.tokens.len() {
            self.token_span(self.tokens[self.cursor + 1])
        } else {
            self.eof_span()
        }
    }

    /// Return the pre-interned [`Symbol`] for the next token.
    pub fn next_token_symbol(&self) -> Symbol {
        self.symbols
            .get(&(self.cursor + 1))
            .copied()
            .unwrap_or_else(Symbol::invalid)
    }

    pub fn current_degree(&self) -> usize {
        if let Some(&start) = self.cursor_stack.last() {
            self.cursor.saturating_sub(start)
        } else {
            0
        }
    }

    /// Check if we're at the end of tokens.
    pub fn is_eof(&self) -> bool {
        self.cursor + 1 >= self.tokens.len() || self.tokens[self.cursor + 1].kind == TokenKind::Eof
    }

    /// Get the source text of a token.
    pub fn token_text(&self, token: &Token) -> String {
        let lo = self.start_pos + BytePos(token.from as u32);
        let sf = self.source_map.lookup_source_file(lo);
        match &sf.src {
            Some(content) => {
                let byte_start = (lo.0.saturating_sub(sf.start_pos.0)) as usize;
                let byte_end =
                    (byte_start + token.to.saturating_sub(token.from)).min(content.len());
                content.get(byte_start..byte_end).unwrap_or("").to_string()
            }
            None => String::new(),
        }
    }

    /// Get the source content of a token.
    pub fn token_content(&self, token: &Token) -> Option<String> {
        let span = self.token_span(*token);
        self.source_map.span_to_snippet(span).ok()
    }
}

pub type ParseResult = Result<NodeIndex, ParseError>;
