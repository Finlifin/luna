use super::basic::Rule;
use super::error::*;
use super::parser::*;
use ast::*;
use lex::TokenKind;

impl Parser<'_> {
    pub fn try_statement_or_definition(&mut self) -> ParseResult {
        self.scoped(|p| {
            let token = p.peek_next_token();
            match token.kind {
                TokenKind::Let => p.try_let_statement(),
                TokenKind::Const => p.try_const_statement(),
                TokenKind::If => p.try_if_statement(),
                TokenKind::Use => p.try_use_statement(),
                TokenKind::When => p.try_when_statement(),
                TokenKind::While => p.try_while_loop(),
                TokenKind::For => p.try_for_loop(),
                TokenKind::Return => p.try_return_statement(),
                TokenKind::Resume => p.try_resume_statement(),
                TokenKind::Break => p.try_break_statement(),
                TokenKind::Continue => p.try_continue_statement(),
                TokenKind::LBrace => p.try_block(),

                TokenKind::Struct => p.try_struct(),
                TokenKind::Enum => p.try_enum(),
                TokenKind::Trait => p.try_trait(),
                TokenKind::Impl => p.try_implementation(),
                TokenKind::Extend => p.try_extension(),
                TokenKind::Derive => p.try_derivation(),
                TokenKind::Fn => p.try_function(),
                TokenKind::Mod => p.try_module(),
                TokenKind::Effect => p.try_effect(),
                TokenKind::Typealias => p.try_typealias(),
                TokenKind::Newtype => p.try_newtype(),
                TokenKind::Case => p.try_case(),
                TokenKind::Test => p.try_test(),
                // TokenKind::Lemma
                // TokenKind::Predicate
                // TokenKind::Union
                // TokenKind::Static
                TokenKind::Axiom => p.try_unary(
                    Rule::semicolon("predicate expression", |p| p.try_expr()),
                    TokenKind::Axiom,
                    NodeKind::Axiom,
                    "Expected predicate expression after `axiom`".to_string(),
                ),
                TokenKind::Ensures => p.try_unary(
                    Rule::semicolon("ensures expression", |p| p.try_expr()),
                    TokenKind::Ensures,
                    NodeKind::Ensures,
                    "Expected ensures expression after `ensures`".to_string(),
                ),
                TokenKind::Requires => p.try_unary(
                    Rule::semicolon("requires expression", |p| p.try_expr()),
                    TokenKind::Requires,
                    NodeKind::Requires,
                    "Expected requires expression after `requires`".to_string(),
                ),
                TokenKind::Asserts => p.try_unary(
                    Rule::semicolon("asserts expression", |p| p.try_expr()),
                    TokenKind::Asserts,
                    NodeKind::Asserts,
                    "Expected asserts expression after `asserts`".to_string(),
                ),
                TokenKind::Assumes => p.try_unary(
                    Rule::semicolon("assumes expression", |p| p.try_expr()),
                    TokenKind::Assumes,
                    NodeKind::Assumes,
                    "Expected assumes expression after `assumes`".to_string(),
                ),
                TokenKind::Invariant => p.try_unary(
                    Rule::semicolon("invariant expression", |p| p.try_expr()),
                    TokenKind::Invariant,
                    NodeKind::Invariant,
                    "Expected invariant expression after `invariant`".to_string(),
                ),
                TokenKind::Decreases => p.try_unary(
                    Rule::semicolon("decreases expression", |p| p.try_expr()),
                    TokenKind::Decreases,
                    NodeKind::Decreases,
                    "Expected decreases expression after `decreases`".to_string(),
                ),

                _ => p.try_expr_statement(),
            }
        })
    }

    pub fn try_let_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Let.as_ref(), |p| {
            p.eat_tokens(1);
            let pattern = p.try_pattern()?;
            if pattern == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected pattern after 'let'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let mut expr: u32 = 0;
            if p.eat_token(TokenKind::Colon) {
                expr = p.try_expr()?;
                if expr == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a type expression after ':'".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
            }

            let mut init: u32 = 0;
            if p.eat_token(TokenKind::Eq) {
                init = p.try_expr()?;
                if init == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an initializer expression after '='".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
            }

            Ok(NodeBuilder::new(NodeKind::LetDecl, p.current_span())
                .add_single_child(pattern)
                .add_single_child(expr)
                .add_single_child(init)
                .build(&mut p.ast))
        })
    }

    pub fn try_const_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Const.as_ref(), |p| {
            p.eat_tokens(1);
            let pattern = p.try_pattern()?;
            if pattern == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected pattern after 'const'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let mut expr: u32 = 0;
            if p.eat_token(TokenKind::Colon) {
                expr = p.try_expr()?;
                if expr == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a type expression after ':'".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
            }

            let mut init: u32 = 0;
            if p.eat_token(TokenKind::Eq) {
                init = p.try_expr()?;
                if init == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an initializer expression after '='".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
            }

            Ok(NodeBuilder::new(NodeKind::ConstDecl, p.current_span())
                .add_single_child(pattern)
                .add_single_child(expr)
                .add_single_child(init)
                .build(&mut p.ast))
        })
    }

    pub fn try_use_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Use.as_ref(), |p| {
            p.eat_tokens(1);
            let path = p.try_path()?;
            if path == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a path after 'use'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::UseStatement, p.current_span())
                .add_single_child(path)
                .build(&mut p.ast))
        })
    }

    pub fn try_path(&mut self) -> ParseResult {
        self.scoped(|p| {
            let next = p.peek_next_token();
            let mut left = p.try_prefix_path()?;

            while p.eat_token(TokenKind::Dot) {
                let next = p.peek_next_token();
                match next.kind {
                    TokenKind::Id => {
                        let id = p.try_id()?;
                        left = NodeBuilder::new(NodeKind::PathSelect, p.current_span())
                            .add_single_child(left)
                            .add_single_child(id)
                            .build(&mut p.ast);
                    }
                    TokenKind::Star | TokenKind::SeparatedStar => {
                        p.eat_tokens(1);
                        left = NodeBuilder::new(NodeKind::PathSelectAll, p.current_span())
                            .add_single_child(left)
                            .build(&mut p.ast);
                    }
                    TokenKind::LBrace => {
                        p.eat_tokens(1);
                        let mut items = vec![];
                        while !p.peek(TokenKind::RBrace.as_ref()) {
                            let item = p.try_path()?;
                            if item == 0 {
                                return Err(ParseError::invalid_syntax(
                                    "Expected pathes inside `{}`".to_string(),
                                    p.peek_next_token().kind,
                                    p.next_token_span(),
                                ));
                            }
                            items.push(item);
                            if !p.eat_token(TokenKind::Comma) {
                                if !p.peek(TokenKind::RBrace.as_ref()) {
                                    return Err(ParseError::unexpected_token(
                                        TokenKind::Comma,
                                        p.peek_next_token().kind,
                                        p.next_token_span(),
                                    ));
                                }
                                break;
                            }
                        }
                        if !p.eat_token(TokenKind::RBrace) {
                            return Err(ParseError::unexpected_token(
                                TokenKind::RBrace,
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            ));
                        }
                        left = NodeBuilder::new(NodeKind::PathSelectMulti, p.current_span())
                            .add_single_child(left)
                            .add_multiple_children(items)
                            .build(&mut p.ast);
                    }
                    _ => {
                        return Err(ParseError::invalid_syntax(
                            "Expected an identifier or `{` or `*` or `as` after '.'".to_string(),
                            next.kind,
                            p.next_token_span(),
                        ));
                    }
                }
            }

            Ok(left)
        })
    }

    pub fn try_prefix_path(&mut self) -> ParseResult {
        self.scoped(|p| {
            let next = p.peek_next_token();
            match next.kind {
                TokenKind::Id => p.try_id(),
                TokenKind::Dot => {
                    p.eat_tokens(1);
                    let sub_path = p.try_prefix_path()?;
                    if sub_path == 0 {
                        return Err(ParseError::invalid_syntax(
                            "Expected a sub-path after '.'".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    Ok(NodeBuilder::new(NodeKind::SuperPath, p.current_span())
                        .add_single_child(sub_path)
                        .build(&mut p.ast))
                }
                TokenKind::At => {
                    p.eat_tokens(1);
                    let sub_path = p.try_prefix_path()?;
                    if sub_path == 0 {
                        return Err(ParseError::invalid_syntax(
                            "Expected a sub-path after '@'".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    Ok(NodeBuilder::new(NodeKind::PackagePath, p.current_span())
                        .add_single_child(sub_path)
                        .build(&mut p.ast))
                }
                _ => Ok(0),
            }
        })
    }

    pub fn try_if_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::If.as_ref(), |p| {
            p.eat_tokens(1);
            let expr = p.try_expr_without_object_call()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a condition expression after 'if'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            if p.eat_token(TokenKind::Is) {
                return p.try_if_is_match(expr);
            }

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "Expected '{' after 'if' to start a block of statements".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let body = p.try_block()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a block of statements after 'if'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let else_ = if p.eat_token(TokenKind::Else) {
                if p.peek(TokenKind::If.as_ref()) {
                    // If the next token is 'if', we treat it as an else-if
                    p.try_if_statement()?
                } else {
                    if !p.peek(TokenKind::LBrace.as_ref()) {
                        return Err(ParseError::invalid_syntax(
                            "Expected '{' after 'else' to start a block of statements".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    p.try_block()?
                }
            } else {
                0
            };

            Ok(NodeBuilder::new(NodeKind::IfStatement, p.current_span())
                .add_single_child(expr)
                .add_single_child(body)
                .add_single_child(else_)
                .build(&mut p.ast))
        })
    }

    // if expr is pattern do block else?
    //         ^
    fn try_if_is_match(&mut self, expr: NodeIndex) -> ParseResult {
        self.scoped(|p| {
            let pattern = p.try_pattern()?;
            if pattern == 0 {
                return p.try_if_match(expr);
            }

            if !p.eat_token(TokenKind::Do) {
                return Err(ParseError::unexpected_token(
                    TokenKind::Do,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let body = p.try_block()?;

            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a block of statements after 'do'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let else_ = if p.eat_token(TokenKind::Else) {
                if p.peek(TokenKind::If.as_ref()) {
                    // If the next token is 'if', we treat it as an else-if
                    p.try_if_statement()?
                } else {
                    if !p.peek(TokenKind::LBrace.as_ref()) {
                        return Err(ParseError::invalid_syntax(
                            "Expected '{' after 'else' to start a block of statements".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    p.try_block()?
                }
            } else {
                0
            };

            Ok(NodeBuilder::new(NodeKind::IfIsMatch, p.current_span())
                .add_single_child(expr)
                .add_single_child(pattern)
                .add_single_child(body)
                .add_single_child(else_)
                .build(&mut p.ast))
        })
    }

    // if expr is do block
    //         ^
    fn try_if_match(&mut self, expr: NodeIndex) -> ParseResult {
        self.scoped(|p| {
            if !p.eat_token(TokenKind::Do) {
                return Err(ParseError::unexpected_token(
                    TokenKind::Do,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let arms = p.try_multi_with_bracket(
                &[Rule::semicolon("pattern arm", |p| p.try_pattern_arm())],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            Ok(NodeBuilder::new(NodeKind::IfMatch, p.current_span())
                .add_single_child(expr)
                .add_multiple_children(arms)
                .build(&mut p.ast))
        })
    }

    // while (: label)? expr block
    pub fn try_while_loop(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::While.as_ref(), |p| {
            p.eat_tokens(1);

            let label = if p.eat_token(TokenKind::Colon) {
                let l = p.try_id()?;
                if l == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a label after ':'".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                l
            } else {
                0
            };

            let expr = p.try_expr_without_object_call()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a condition expression after 'while'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            if p.eat_token(TokenKind::Is) {
                return p.try_while_is_match(label, expr);
            }

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "Expected '{' after 'while' to start a block of statements".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let body = p.try_block()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a block of statements after 'while'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::WhileLoop, p.current_span())
                .add_single_child(label)
                .add_single_child(expr)
                .add_single_child(body)
                .build(&mut p.ast))
        })
    }

    // while (: label)? expr is pattern do block
    //                       ^
    pub fn try_while_is_match(&mut self, label: NodeIndex, expr: NodeIndex) -> ParseResult {
        self.scoped(|p| {
            let pattern = p.try_pattern()?;
            if pattern == 0 {
                return p.try_while_match(label, expr);
            }

            if !p.eat_token(TokenKind::Do) {
                return Err(ParseError::unexpected_token(
                    TokenKind::Do,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let body = p.try_block()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a block of statements after 'do'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::WhileIsMatch, p.current_span())
                .add_single_child(label)
                .add_single_child(expr)
                .add_single_child(pattern)
                .add_single_child(body)
                .build(&mut p.ast))
        })
    }

    // while (: label)? expr is do { arms }
    //                          ^
    fn try_while_match(&mut self, label: NodeIndex, expr: NodeIndex) -> ParseResult {
        self.scoped(|p| {
            if !p.eat_token(TokenKind::Do) {
                return Err(ParseError::unexpected_token(
                    TokenKind::Do,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let arms = p.try_multi_with_bracket(
                &[Rule::semicolon("pattern arm", |p| p.try_pattern_arm())],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            Ok(NodeBuilder::new(NodeKind::WhileMatch, p.current_span())
                .add_single_child(label)
                .add_single_child(expr)
                .add_multiple_children(arms)
                .build(&mut p.ast))
        })
    }

    // for (: label)? pattern in expr block
    pub fn try_for_loop(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::For.as_ref(), |p| {
            p.eat_tokens(1);

            let label = if p.eat_token(TokenKind::Colon) {
                let l = p.try_id()?;
                if l == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a label after ':'".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                l
            } else {
                0
            };

            let pattern = p.try_pattern()?;
            if pattern == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a pattern".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            if !p.eat_token(TokenKind::In) {
                return Err(ParseError::unexpected_token(
                    TokenKind::In,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let expr = p.try_expr_without_object_call()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an iterable expression after 'in'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "Expected '{' to start a block of statements".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let body = p.try_block()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a block of statements".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::ForLoop, p.current_span())
                .add_single_child(label)
                .add_single_child(pattern)
                .add_single_child(expr)
                .add_single_child(body)
                .build(&mut p.ast))
        })
    }

    pub fn try_when_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::When.as_ref(), |p| {
            p.eat_tokens(1);
            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "Expected '{' after 'when' to start a block of condition arms".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let arms = p.try_multi_with_bracket(
                &[Rule::comma("condition arm", |p| p.try_condition_arm())],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            Ok(NodeBuilder::new(NodeKind::WhenStatement, p.current_span())
                .add_multiple_children(arms)
                .build(&mut p.ast))
        })
    }

    pub fn try_condition_arm(&mut self) -> ParseResult {
        self.scoped(|p| {
            let condition = p.try_expr()?;
            if (condition == 0) && !p.eat_token(TokenKind::Else) {
                return Ok(0);
            }

            if !p.eat_token(TokenKind::FatArrow) {
                return Err(ParseError::unexpected_token(
                    TokenKind::FatArrow,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let body = p.try_block_or_statement()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a statement, expr or block after '=>'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::ConditionArm, p.current_span())
                .add_single_child(condition)
                .add_single_child(body)
                .build(&mut p.ast))
        })
    }

    pub fn try_pattern_arm(&mut self) -> ParseResult {
        self.scoped(|p| {
            let pattern = p.try_pattern()?;
            if pattern == 0 {
                return Ok(0);
            }

            if !p.eat_token(TokenKind::FatArrow) {
                return Err(ParseError::unexpected_token(
                    TokenKind::FatArrow,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let body = p.try_block_or_statement()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a statement, expr or block after '=>'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::PatternArm, p.current_span())
                .add_single_child(pattern)
                .add_single_child(body)
                .build(&mut p.ast))
        })
    }

    pub fn try_catch_arm(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Catch.as_ref(), |p| {
            p.eat_tokens(1);

            let id = p.try_id()?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an identifier after 'catch'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            if !p.eat_token(TokenKind::FatArrow) {
                return Err(ParseError::unexpected_token(
                    TokenKind::FatArrow,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let body = p.try_block_or_statement()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a statement, expr or block after '=>'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::CatchArm, p.current_span())
                .add_single_child(id)
                .add_single_child(body)
                .build(&mut p.ast))
        })
    }

    pub fn try_block(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::LBrace.as_ref(), |p| {
            let nodes = p.try_multi_with_bracket(
                &[Rule::semicolon("statement or definition", |p| {
                    p.try_statement_or_definition()
                })],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            Ok(NodeBuilder::new(NodeKind::Block, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast))
        })
    }

    pub fn try_block_or_statement(&mut self) -> ParseResult {
        self.scoped(|p| {
            if p.peek(TokenKind::LBrace.as_ref()) {
                p.try_block()
            } else {
                p.try_statement_or_definition()
            }
        })
    }

    pub fn try_return_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Return.as_ref(), |p| {
            p.eat_tokens(1);
            let expr = p.try_expr()?;

            let guard = if p.eat_token(TokenKind::If) {
                match p.try_expr() {
                    Ok(0) => {
                        return Err(ParseError::invalid_syntax(
                            "Expected a condition expression after 'if'".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    Ok(node) => node,
                    Err(err) => {
                        return Err(err);
                    }
                }
            } else {
                0
            };

            Ok(
                NodeBuilder::new(NodeKind::ReturnStatement, p.current_span())
                    .add_single_child(expr)
                    .add_single_child(guard)
                    .build(&mut p.ast),
            )
        })
    }

    pub fn try_resume_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Resume.as_ref(), |p| {
            p.eat_tokens(1);
            let expr = p.try_expr()?;

            let guard = if p.eat_token(TokenKind::If) {
                match p.try_expr() {
                    Ok(0) => {
                        return Err(ParseError::invalid_syntax(
                            "Expected a condition expression after 'if'".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    Ok(node) => node,
                    Err(err) => {
                        return Err(err);
                    }
                }
            } else {
                0
            };

            Ok(
                NodeBuilder::new(NodeKind::ResumeStatement, p.current_span())
                    .add_single_child(expr)
                    .add_single_child(guard)
                    .build(&mut p.ast),
            )
        })
    }

    pub fn try_break_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Break.as_ref(), |p| {
            p.eat_tokens(1);

            let label = p.try_id()?;

            let guard = if p.eat_token(TokenKind::If) {
                match p.try_expr() {
                    Ok(0) => {
                        return Err(ParseError::invalid_syntax(
                            "Expected a condition expression after 'if'".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    Ok(node) => node,
                    Err(err) => {
                        return Err(err);
                    }
                }
            } else {
                0
            };

            Ok(NodeBuilder::new(NodeKind::BreakStatement, p.current_span())
                .add_single_child(label)
                .add_single_child(guard)
                .build(&mut p.ast))
        })
    }

    pub fn try_continue_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Continue.as_ref(), |p| {
            p.eat_tokens(1);

            let label = p.try_id()?;

            let guard = if p.eat_token(TokenKind::If) {
                match p.try_expr() {
                    Ok(0) => {
                        return Err(ParseError::invalid_syntax(
                            "Expected a condition expression after 'if'".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    Ok(node) => node,
                    Err(err) => {
                        return Err(err);
                    }
                }
            } else {
                0
            };

            Ok(
                NodeBuilder::new(NodeKind::ContinueStatement, p.current_span())
                    .add_single_child(label)
                    .add_single_child(guard)
                    .build(&mut p.ast),
            )
        })
    }

    // expr
    // expr = expr
    // expr += expr
    // expr -= expr
    // expr *= expr
    // expr /= expr
    pub fn try_expr_statement(&mut self) -> ParseResult {
        self.scoped(|p| {
            let expr = p.try_expr()?;
            if expr == 0 {
                return Ok(0); // No expression, no statement
            }

            if p.eat_token(TokenKind::Eq) {
                let init = p.try_expr()?;
                if init == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an initializer expression after '='".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                return Ok(NodeBuilder::new(NodeKind::Assign, p.current_span())
                    .add_single_child(expr)
                    .add_single_child(init)
                    .build(&mut p.ast));
            } else if p.eat_token(TokenKind::PlusEq) {
                let init = p.try_expr()?;
                if init == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an expression after '+='".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                return Ok(NodeBuilder::new(NodeKind::AddAssign, p.current_span())
                    .add_single_child(expr)
                    .add_single_child(init)
                    .build(&mut p.ast));
            } else if p.eat_token(TokenKind::MinusEq) {
                let init = p.try_expr()?;
                if init == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an expression after '-='".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                return Ok(NodeBuilder::new(NodeKind::SubAssign, p.current_span())
                    .add_single_child(expr)
                    .add_single_child(init)
                    .build(&mut p.ast));
            } else if p.eat_token(TokenKind::StarEq) {
                let init = p.try_expr()?;
                if init == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an expression after '*='".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                return Ok(NodeBuilder::new(NodeKind::MulAssign, p.current_span())
                    .add_single_child(expr)
                    .add_single_child(init)
                    .build(&mut p.ast));
            } else if p.eat_token(TokenKind::SlashEq) {
                let init = p.try_expr()?;
                if init == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an expression after '/='".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                return Ok(NodeBuilder::new(NodeKind::DivAssign, p.current_span())
                    .add_single_child(expr)
                    .add_single_child(init)
                    .build(&mut p.ast));
            }

            Ok(NodeBuilder::new(NodeKind::ExprStatement, p.current_span())
                .add_single_child(expr)
                .build(&mut p.ast))
        })
    }

    pub fn try_file_scope(&mut self) -> ParseResult {
        self.scoped(|p| {
            let nodes = p.try_multi(&[
                Rule::comma("property", |p| p.try_property()),
                Rule::semicolon("statement or definition", |p| {
                    p.try_statement_or_definition()
                }),
            ])?;
            if p.peek_next_token().kind != TokenKind::Eof {
                return Err(ParseError::unexpected_token(
                    TokenKind::Eof,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            Ok(NodeBuilder::new(NodeKind::FileScope, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast))
        })
    }
}
