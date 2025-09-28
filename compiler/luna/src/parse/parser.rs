use rustc_span::BytePos;

use crate::diagnostic::{DiagnosticContext, FlurryError};
use crate::lex::{Token, TokenKind};
use crate::parse::ast::*;
use crate::parse::error::*;

// hand-write peg parser
pub struct Parser<'a> {
    pub(crate) source_map: &'a rustc_span::SourceMap,
    pub(crate) tokens: Vec<Token>,
    pub(crate) ast: Ast,
    pub(crate) cursor: usize,
    pub(crate) cursor_stack: Vec<usize>,
    pub start_pos: BytePos,

    errors: Vec<ParseError>,
}

impl Parser<'_> {
    pub fn new(
        source_map: &rustc_span::SourceMap,
        tokens: Vec<Token>,
        start_pos: BytePos,
    ) -> Parser {
        let mut result = Parser {
            source_map,
            tokens,
            cursor: 0,
            cursor_stack: Vec::new(),
            errors: Vec::new(),
            ast: Ast::new(),
            start_pos,
        };
        result.enter();
        result
    }

    pub fn parse(&mut self, diag_ctx: &DiagnosticContext) {
        match self.try_file_scope() {
            Ok(node_index) => {
                self.ast.root = node_index;
            }
            Err(error) => {
                // Handle parse error
                let base_pos = self
                    .source_map
                    .lookup_source_file(self.current_span().lo())
                    .start_pos;
                error.emit(diag_ctx, base_pos);
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
            println!("Warning: cursor stack is empty, returning dummy span");
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

    pub fn current_degree(&self) -> usize {
        if let Some(&start) = self.cursor_stack.last() {
            self.cursor - start
        } else {
            0
        }
    }
}

pub type ParseResult = Result<NodeIndex, ParseError>;
