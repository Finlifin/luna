use rustc_span::BytePos;

use super::error::*;
use ast::*;
use diagnostic::{DiagnosticContext, FlurryError};
use lex::{Symbol, Token, TokenKind};

// hand-write peg parser
pub struct Parser<'a> {
    pub(crate) source_map: &'a rustc_span::SourceMap,
    pub(crate) tokens: Vec<Token>,
    /// Map from token index to interned [`Symbol`]; only identifier tokens
    /// have entries.
    pub(crate) symbols: std::collections::HashMap<usize, Symbol>,
    pub(crate) ast: Ast,
    pub(crate) cursor: usize,
    pub(crate) cursor_stack: Vec<usize>,
    /// Each entry belongs to a quote's runtime subtree. Splice operands are
    /// parsed outside that subtree and may construct their own quote.
    pub(crate) quote_holes: Vec<Option<rustc_span::Span>>,
    pub start_pos: BytePos,

    errors: Vec<ParseError>,
}

impl<'a> Parser<'a> {
    pub fn new(
        source_map: &'a rustc_span::SourceMap,
        tokens: Vec<Token>,
        symbols: std::collections::HashMap<usize, Symbol>,
        start_pos: BytePos,
    ) -> Parser<'a> {
        // Trivia has no grammar role. Reindex symbols along with tokens so
        // identifiers after a block comment keep their original spelling.
        let mut significant = Vec::with_capacity(tokens.len());
        let mut significant_symbols = std::collections::HashMap::new();
        for (old_index, token) in tokens.into_iter().enumerate() {
            if token.kind == TokenKind::Comment {
                continue;
            }
            if let Some(&symbol) = symbols.get(&old_index) {
                significant_symbols.insert(significant.len(), symbol);
            }
            significant.push(token);
        }
        let mut result = Parser {
            source_map,
            tokens: significant,
            symbols: significant_symbols,
            cursor: 0,
            cursor_stack: Vec::new(),
            quote_holes: Vec::new(),
            errors: Vec::new(),
            ast: Ast::new(start_pos),
            start_pos,
        };
        result.enter();
        result
    }

    /// Report whether this file parsed, independently of earlier diagnostics.
    pub fn parse(&mut self, diag_ctx: &DiagnosticContext) -> bool {
        match self.try_file_scope() {
            Ok(node_index) => {
                self.ast.root = node_index;
                true
            }
            Err(error) => {
                // Handle parse error
                let base_pos = self
                    .source_map
                    .lookup_source_file(self.current_span().lo())
                    .start_pos;
                error.emit(diag_ctx, base_pos);
                false
            }
        }
    }

    fn parse_error(&mut self, error: ParseError) {
        self.errors.push(error);
    }

    fn enter(&mut self) {
        // push the current cursor to the stack
        self.cursor_stack.push(self.cursor);
    }

    fn exit(&mut self) {
        // pop the cursor stack
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
            return Ok(Default::default()); // Skip this scope if the guardian is present
        }

        self.enter();
        let result = f(self);
        self.exit();
        result
    }

    pub fn finalize(self) -> Ast {
        self.ast
    }

    /// Check if the next few tokens match the expected token kinds
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

    /// Consume a token if it matches the expected kind
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

    /// Consume multiple tokens (unchecked)
    pub fn eat_tokens(&mut self, amount: usize) {
        self.cursor += amount;
    }

    /// Get the next token and advance cursor
    pub fn next_token(&mut self) -> Token {
        if self.cursor + 1 >= self.tokens.len() {
            Token::new(TokenKind::Eof, 0, 0)
        } else {
            let token = self.tokens[self.cursor + 1];
            self.cursor += 1;
            token
        }
    }

    /// Peek at the next token without consuming it
    pub fn peek_next_token(&self) -> Token {
        if self.cursor + 1 >= self.tokens.len() {
            Token::new(TokenKind::Eof, 0, 0)
        } else {
            self.tokens[self.cursor + 1]
        }
    }

    /// Get the current token without advancing
    pub fn current_token(&self) -> Token {
        if self.cursor >= self.tokens.len() {
            Token::new(TokenKind::Eof, 0, 0)
        } else {
            self.tokens[self.cursor]
        }
    }

    /// Get a token at a specific index
    pub fn get_token(&self, index: usize) -> Token {
        if index >= self.tokens.len() {
            Token::new(TokenKind::Eof, 0, 0)
        } else {
            self.tokens[index]
        }
    }

    /// Get the previous token (relative to current cursor position)
    pub fn previous_token(&self) -> Token {
        if self.cursor == 0 {
            Token::new(TokenKind::Eof, 0, 0)
        } else {
            self.tokens[self.cursor - 1]
        }
    }

    /// Get the span from the cursor stack top to cursor + 1
    pub fn current_span(&self) -> rustc_span::Span {
        if let Some(&start) = self.cursor_stack.last() {
            // Convert token positions to byte positions relative to start_pos
            let start_offset = if start < self.tokens.len() {
                self.tokens[start].from
            } else {
                0
            };
            let end_offset = if self.cursor < self.tokens.len() {
                self.tokens[self.cursor].to
            } else {
                start_offset
            };

            rustc_span::Span::new(
                self.start_pos + rustc_span::BytePos(start_offset as u32),
                self.start_pos + rustc_span::BytePos(end_offset as u32),
            )
        } else {
            eprintln!("Warning: cursor stack is empty, returning dummy span");
            rustc_span::DUMMY_SP
        }
    }

    pub fn next_token_span(&self) -> rustc_span::Span {
        if self.cursor + 1 < self.tokens.len() {
            let token = &self.tokens[self.cursor + 1];
            rustc_span::Span::new(
                self.start_pos + rustc_span::BytePos(token.from as u32),
                self.start_pos + rustc_span::BytePos(token.to as u32),
            )
        } else {
            rustc_span::DUMMY_SP
        }
    }

    /// Return the pre-interned [`Symbol`] for the next token.
    ///
    /// Meaningful only when the next token has kind [`lex::TokenKind::Id`];
    /// returns the empty-string symbol for non-identifier tokens.
    pub fn next_token_symbol(&self) -> Symbol {
        self.symbols
            .get(&(self.cursor + 1))
            .copied()
            .unwrap_or_else(|| Symbol::invalid())
    }

    pub fn current_degree(&self) -> usize {
        if let Some(&start) = self.cursor_stack.last() {
            self.cursor - start
        } else {
            0
        }
    }

    /// Get the source text of a token.
    pub fn token_text(&self, token: &Token) -> String {
        let lo = self.start_pos + BytePos(token.from as u32);
        let sf = self.source_map.lookup_source_file(lo);
        match &sf.src {
            Some(content) => {
                let byte_start = (lo.0 - sf.start_pos.0) as usize;
                let byte_end = byte_start + (token.to - token.from);
                content[byte_start..byte_end].to_string()
            }
            None => String::new(),
        }
    }
}

pub type ParseResult = Result<NodeIndex, ParseError>;
