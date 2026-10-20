//! CMTT delimiters do not reuse comparison or normal-form application parsing.
use crate::{
    error::*,
    expressions::ExprOption,
    parser::{ParseResult, Parser},
};
use ast::{NodeBuilder, NodeIndex, NodeKind};
use lex::TokenKind as T;

impl Parser<'_> {
    pub(crate) fn try_contextual_expr_type(&mut self) -> ParseResult {
        self.eat_tokens(1);
        // Expression contexts start with `name:` or `|-`. Otherwise the
        // first expression is the subject of a pattern interface.
        if self.peek_next_token().kind != T::Pipe && !self.peek(&[T::Id, T::Colon]) {
            return self.contextual_pattern_type();
        }
        let mut bindings = Vec::new();
        while self.peek_next_token().kind != T::Pipe {
            let name = self.try_id()?;
            if name == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected runtime binding or `|-` in contextual code type".into(),
                    self.peek_next_token().kind,
                    self.next_token_span(),
                ));
            }
            self.code_token(T::Colon)?;
            let ty = self.try_expr_with_option(ExprOption {
                stop_at_pipe: true,
                ..Default::default()
            })?;
            if ty == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected runtime binding type".into(),
                    self.peek_next_token().kind,
                    self.next_token_span(),
                ));
            }
            bindings.push(
                NodeBuilder::new(NodeKind::CodeContextBinding, self.current_span())
                    .add_single_child(name)
                    .add_single_child(ty)
                    .build(&mut self.ast),
            );
            if self.peek_next_token().kind != T::Pipe {
                self.code_token(T::Comma)?;
            }
        }
        self.code_token(T::Pipe)?;
        if !self.eat_token(T::Minus) {
            self.code_token(T::SeparatedMinus)?;
        }
        let result = self.try_expr_with_option(ExprOption {
            stop_at_pattern_gt: true,
            ..Default::default()
        })?;
        if result == 0 {
            return Err(ParseError::invalid_syntax(
                "Expected result type after `|-`".into(),
                self.peek_next_token().kind,
                self.next_token_span(),
            ));
        }
        if !self.eat_token(T::Gt) {
            self.code_token(T::SeparatedGt)?;
        }
        Ok(
            NodeBuilder::new(NodeKind::ContextualExprType, self.current_span())
                .add_single_child(result)
                .add_multiple_children(bindings)
                .build(&mut self.ast),
        )
    }

    fn contextual_pattern_type(&mut self) -> ParseResult {
        let subject = self.try_expr_with_option(ExprOption {
            stop_at_pattern_context: true,
            ..Default::default()
        })?;
        if subject == 0 {
            return Err(ParseError::invalid_syntax(
                "Expected subject type in pattern code interface".into(),
                self.peek_next_token().kind,
                self.next_token_span(),
            ));
        }
        if !self.eat_token(T::Minus) {
            self.code_token(T::SeparatedMinus)?;
        }
        if self.eat_token(T::PipeGt) {
            return Ok(
                NodeBuilder::new(NodeKind::ContextualPatternType, self.current_span())
                    .add_single_child(subject)
                    .add_multiple_children(Vec::new())
                    .build(&mut self.ast),
            );
        }
        self.code_token(T::Pipe)?;
        let mut bindings = Vec::new();
        while !matches!(self.peek_next_token().kind, T::Gt | T::SeparatedGt) {
            let name = self.try_id()?;
            if name == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected success binding in pattern code interface".into(),
                    self.peek_next_token().kind,
                    self.next_token_span(),
                ));
            }
            self.code_token(T::Colon)?;
            let ty = self.try_expr_with_option(ExprOption {
                stop_at_pattern_gt: true,
                ..Default::default()
            })?;
            if ty == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected success binding type".into(),
                    self.peek_next_token().kind,
                    self.next_token_span(),
                ));
            }
            bindings.push(
                NodeBuilder::new(NodeKind::CodeContextBinding, self.current_span())
                    .add_single_child(name)
                    .add_single_child(ty)
                    .build(&mut self.ast),
            );
            if !matches!(self.peek_next_token().kind, T::Gt | T::SeparatedGt) {
                self.code_token(T::Comma)?;
            }
        }
        if !self.eat_token(T::Gt) {
            self.code_token(T::SeparatedGt)?;
        }
        Ok(
            NodeBuilder::new(NodeKind::ContextualPatternType, self.current_span())
                .add_single_child(subject)
                .add_multiple_children(bindings)
                .build(&mut self.ast),
        )
    }

    fn code_token(&mut self, token: T) -> Result<(), ParseError> {
        if self.eat_token(token) {
            Ok(())
        } else {
            Err(ParseError::unexpected_token(
                token,
                self.peek_next_token().kind,
                self.next_token_span(),
            ))
        }
    }

    fn code_substitutions(&mut self) -> Result<Vec<NodeIndex>, ParseError> {
        let mut entries = Vec::new();
        if !self.eat_token(T::LBracket) {
            return Ok(entries);
        }
        if self.eat_token(T::RBracket) {
            return Ok(entries);
        }
        loop {
            let name = self.try_id()?;
            if name == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a code interface name".into(),
                    self.peek_next_token().kind,
                    self.next_token_span(),
                ));
            }
            self.code_token(T::Eq)?;
            let value = self.try_id()?;
            if value == 0 {
                return Err(ParseError::invalid_syntax(
                    "Code substitution requires a binding identifier".into(),
                    self.peek_next_token().kind,
                    self.next_token_span(),
                ));
            }
            entries.push(
                NodeBuilder::new(NodeKind::CodeSubstitution, self.current_span())
                    .add_single_child(name)
                    .add_single_child(value)
                    .build(&mut self.ast),
            );
            if self.eat_token(T::RBracket) {
                break;
            }
            self.code_token(T::Comma)?;
            if self.eat_token(T::RBracket) {
                break;
            }
        }
        Ok(entries)
    }

    pub(crate) fn try_code_quote(&mut self) -> ParseResult {
        self.eat_tokens(1);
        let substitutions = self.code_substitutions()?;
        if matches!(self.peek_next_token().kind, T::Lt | T::SeparatedLt) {
            self.eat_tokens(1);
            self.quote_holes.push(None);
            let pattern = self.try_pattern();
            self.quote_holes.pop();
            let pattern = pattern?;
            if pattern == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected runtime pattern in quote".into(),
                    self.peek_next_token().kind,
                    self.next_token_span(),
                ));
            }
            if !self.eat_token(T::Gt) {
                self.code_token(T::SeparatedGt)?;
            }
            return Ok(
                NodeBuilder::new(NodeKind::QuotePattern, self.current_span())
                    .add_single_child(pattern)
                    .add_multiple_children(substitutions)
                    .build(&mut self.ast),
            );
        }
        self.code_token(T::LParen)?;
        self.quote_holes.push(None);
        let body = self.try_expr();
        self.quote_holes.pop();
        let body = body?;
        if body == 0 {
            return Err(ParseError::invalid_syntax(
                "Expected runtime expression in quote".into(),
                self.peek_next_token().kind,
                self.next_token_span(),
            ));
        }
        self.code_token(T::RParen)?;
        Ok(NodeBuilder::new(NodeKind::QuoteExpr, self.current_span())
            .add_single_child(body)
            .add_multiple_children(substitutions)
            .build(&mut self.ast))
    }

    pub(crate) fn try_code_splice(&mut self) -> ParseResult {
        self.code_splice(NodeKind::SpliceExpr)
    }

    pub(crate) fn try_pattern_splice(&mut self) -> ParseResult {
        self.code_splice(NodeKind::SplicePattern)
    }

    fn code_splice(&mut self, kind: NodeKind) -> ParseResult {
        let span = self.next_token_span();
        if let Some(hole) = self.quote_holes.last_mut() {
            if hole.is_some() {
                return Err(ParseError::invalid_syntax(
                    "A quote permits at most one splice in its runtime body".into(),
                    T::Tilde,
                    span,
                ));
            }
            *hole = Some(span);
        }
        self.eat_tokens(1);
        let substitutions = self.code_substitutions()?;
        if !self.eat_token(T::Lt) {
            self.code_token(T::SeparatedLt)?;
        }
        let surrounding = std::mem::take(&mut self.quote_holes);
        let operand = self.try_expr_with_option(ExprOption {
            stop_at_pattern_gt: true,
            ..Default::default()
        });
        self.quote_holes = surrounding;
        let operand = operand?;
        if operand == 0 {
            return Err(ParseError::invalid_syntax(
                "Expected comptime code operand in splice".into(),
                self.peek_next_token().kind,
                self.next_token_span(),
            ));
        }
        if !self.eat_token(T::Gt) {
            self.code_token(T::SeparatedGt)?;
        }
        Ok(NodeBuilder::new(kind, self.current_span())
            .add_single_child(operand)
            .add_multiple_children(substitutions)
            .build(&mut self.ast))
    }
}
