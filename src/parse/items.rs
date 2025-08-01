use crate::lex::TokenKind;
use crate::parse::ast::*;
use crate::parse::basic::Rule;
use crate::parse::error::*;
use crate::parse::parser::*;

impl Parser<'_> {
    // Wow, a long function
    pub fn try_param(&mut self) -> ParseResult {
        self.scoped(|p| {
            if p.peek(&[TokenKind::Dot, TokenKind::Dot, TokenKind::Dot]) {
                // This is a rest parameter
                p.eat_tokens(3);
                let id = p.try_id()?;
                if id == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an identifier after `...`".to_string(),
                        TokenKind::Id,
                        p.current_span(),
                    ));
                }
                if !p.eat_token(TokenKind::Colon) {
                    return Err(ParseError::unexpected_token(
                        TokenKind::Colon,
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                let ty = p.try_expr_without_object_call()?;
                if ty == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a type after `:`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                return Ok(NodeBuilder::new(NodeKind::ParamRestBind, p.current_span())
                    .add_single_child(id)
                    .add_single_child(ty)
                    .build(&mut p.ast));
            }

            if p.peek(TokenKind::Dot.as_ref()) {
                // This is a optional parameter
                p.eat_tokens(1);
                let id = p.try_id()?;
                if id == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an identifier after `.`".to_string(),
                        TokenKind::Id,
                        p.current_span(),
                    ));
                }
                p.eat_tokens(1); // eat the colon

                let ty = p.try_expr_without_object_call()?;
                if ty == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a type after `:`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }

                if !p.eat_token(TokenKind::Eq) {
                    return Err(ParseError::unexpected_token(
                        TokenKind::Eq,
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                let default_value = p.try_expr_without_object_call()?;
                if default_value == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a default value after `=`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                return Ok(NodeBuilder::new(NodeKind::ParamOptional, p.current_span())
                    .add_single_child(id)
                    .add_single_child(ty)
                    .add_single_child(default_value)
                    .build(&mut p.ast));
            }

            if p.peek(&[TokenKind::Id, TokenKind::Colon]) {
                // This is a normal parameter with type annotation
                let id = p.try_id()?;
                p.eat_tokens(1);

                let ty = p.try_expr_without_object_call()?;
                if ty == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a type after `:`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }

                return Ok(NodeBuilder::new(NodeKind::ParamTyped, p.current_span())
                    .add_single_child(id)
                    .add_single_child(ty)
                    .build(&mut p.ast));
            }

            if p.peek(&[TokenKind::Id, TokenKind::ColonMinus]) {
                // This is a parameter with trait bound instead of type
                let id = p.try_id()?;
                p.eat_tokens(1);
                let trait_bound = p.try_expr_without_object_call()?;
                if trait_bound == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a trait bound expression after `:-`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                return Ok(
                    NodeBuilder::new(NodeKind::ParamTraitBound, p.current_span())
                        .add_single_child(id)
                        .add_single_child(trait_bound)
                        .build(&mut p.ast),
                );
            }

            if p.peek(&[TokenKind::Star, TokenKind::SelfLower]) {
                p.eat_tokens(2);
                return Ok(
                    NodeBuilder::new(NodeKind::ParamSelfRef, p.current_span()).build(&mut p.ast)
                );
            }

            if p.peek(&[TokenKind::Star, TokenKind::Itself]) {
                p.eat_tokens(2);
                return Ok(
                    NodeBuilder::new(NodeKind::ParamItself, p.current_span()).build(&mut p.ast)
                );
            }

            if p.peek(TokenKind::SelfLower.as_ref()) {
                p.eat_tokens(1);
                return Ok(
                    NodeBuilder::new(NodeKind::ParamSelf, p.current_span()).build(&mut p.ast)
                );
            }

            if p.peek(TokenKind::Itself.as_ref()) {
                p.eat_tokens(1);
                return Ok(
                    NodeBuilder::new(NodeKind::ParamItself, p.current_span()).build(&mut p.ast)
                );
            }

            Ok(p.try_id()?)
        })
    }

    // T
    // T:- trait_bound_expr
    // id: type_expr
    // .id: type_expr = default_value_expr
    pub fn try_clause(&mut self) -> ParseResult {
        self.scoped(|p| {
            if p.peek(&[TokenKind::Id, TokenKind::Colon]) {
                let id = p.try_id()?;
                p.eat_tokens(1); // eat the colon
                let ty = p.try_expr_without_object_call()?;
                if ty == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a type after `:`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                return Ok(NodeBuilder::new(NodeKind::ClauseDecl, p.current_span())
                    .add_single_child(id)
                    .add_single_child(ty)
                    .build(&mut p.ast));
            }

            if p.peek(&[TokenKind::Id, TokenKind::ColonMinus]) {
                let id = p.try_id()?;
                p.eat_tokens(1); // eat the colon-minus
                let trait_bound = p.try_expr_without_object_call()?;
                if trait_bound == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a trait bound expression after `:-`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                return Ok(
                    NodeBuilder::new(NodeKind::ClauseTraitBoundDecl, p.current_span())
                        .add_single_child(id)
                        .add_single_child(trait_bound)
                        .build(&mut p.ast),
                );
            }

            if p.peek(&[TokenKind::Dot, TokenKind::Id, TokenKind::Colon]) {
                p.eat_tokens(1);
                let id = p.try_id()?;
                p.eat_tokens(1);

                let ty = p.try_expr_without_object_call()?;
                if ty == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a type after `:`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                if !p.eat_token(TokenKind::Eq) {
                    return Err(ParseError::unexpected_token(
                        TokenKind::Eq,
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                let default_value = p.try_expr_without_object_call()?;
                if default_value == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a default value after `=`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                return Ok(
                    NodeBuilder::new(NodeKind::ClauseOptionalDecl, p.current_span())
                        .add_single_child(id)
                        .add_single_child(ty)
                        .add_single_child(default_value)
                        .build(&mut p.ast),
                );
            }

            let id = p.try_id()?;
            if id == 0 {
                return Ok(0);
            }
            Ok(NodeBuilder::new(NodeKind::ClauseTypeDecl, p.current_span())
                .add_single_child(id)
                .build(&mut p.ast))
        })
    }

    // where clauses
    pub fn try_clauses(&mut self) -> Result<Vec<NodeIndex>, ParseError> {
        self.scoped_with_expected_prefix(TokenKind::Where.as_ref(), |p| {
            p.eat_tokens(1); // eat the 'where'
            p.try_multi(&[Rule::comma("clause", |p| p.try_clause())])
        })
    }

    pub fn try_item(&mut self) -> ParseResult {
        self.scoped(|p| {
            let next = p.peek_next_token();
            return match next.kind {
                TokenKind::Struct => p.try_struct(),
                TokenKind::Enum => p.try_enum(),
                TokenKind::Trait => p.try_trait(),
                TokenKind::Impl => p.try_implementation(),
                TokenKind::Extend => p.try_extension(),
                TokenKind::Fn => p.try_function(),
                TokenKind::Mod => p.try_module(),
                // TokenKind::Effect => p.try_effect(),
                // TokenKind::Typealias => p.try_typealias(),
                // TokenKind::Newtype => p.try_newtype(),
                // TokenKind::Handles => p.try_handles(),
                _ => Ok(0),
            };
        })
    }

    // struct id? clauses? block
    pub fn try_struct(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Struct.as_ref(), |p| {
            p.eat_tokens(1); // eat the 'struct'
            let id = p.try_id()?;
            let clauses = p.try_clauses()?;

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "expected a block after `struct` declaration".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let nodes = p.try_multi_with_bracket(
                &[
                    Rule::comma("struct field", |p| p.try_struct_field()),
                    Rule::comma("property", |p| p.try_property()),
                    Rule::semicolon("item", |p| p.try_item()),
                    Rule::semicolon("statement", |p| p.try_statement()),
                ],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast);

            Ok(NodeBuilder::new(NodeKind::StructDef, p.current_span())
                .add_single_child(id)
                .add_multiple_children(clauses)
                .add_single_child(block)
                .build(&mut p.ast))
        })
    }

    // id: type_expr (= default_value_expr)?
    fn try_struct_field(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Id, TokenKind::Colon], |p| {
            let id = p.try_id()?;

            if !p.eat_token(TokenKind::Colon) {
                return Err(ParseError::unexpected_token(
                    TokenKind::Colon,
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            let ty = p.try_expr()?;
            if ty == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a type after `:`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            let default_value = if p.eat_token(TokenKind::Eq) {
                let value = p.try_expr_without_object_call()?;
                if value == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a default value after `=`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                value
            } else {
                0
            };

            Ok(NodeBuilder::new(NodeKind::StructField, p.current_span())
                .add_single_child(id)
                .add_single_child(ty)
                .add_single_child(default_value)
                .build(&mut p.ast))
        })
    }

    // enum id? clauses? block
    pub fn try_enum(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Enum.as_ref(), |p| {
            p.eat_tokens(1); // eat the 'enum'
            let id = p.try_id()?;
            let clauses = p.try_clauses()?;

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "expected a block after `enum` declaration".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let nodes = p.try_multi_with_bracket(
                &[
                    Rule::comma("enum variant", |p| p.try_enum_variant()),
                    Rule::comma("property", |p| p.try_property()),
                    Rule::semicolon("item", |p| p.try_item()),
                    Rule::semicolon("statement", |p| p.try_statement()),
                ],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast);

            Ok(NodeBuilder::new(NodeKind::EnumDef, p.current_span())
                .add_single_child(id)
                .add_multiple_children(clauses)
                .add_single_child(block)
                .build(&mut p.ast))
        })
    }

    pub fn try_enum_variant(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Id.as_ref(), |p| {
            if p.peek(&[TokenKind::Id, TokenKind::Eq]) {
                return p.parse_enum_variant_with_pattern();
            }
            if p.peek(&[TokenKind::Id, TokenKind::Dot, TokenKind::LBrace]) {
                return p.parse_enum_variant_with_sub_enum();
            }
            if p.peek(&[TokenKind::Id, TokenKind::LBrace]) {
                return p.parse_enum_variant_with_struct();
            }
            if p.peek(&[TokenKind::Id, TokenKind::LParen]) {
                return p.parse_enum_variant_with_tuple();
            }

            p.try_id()
        })
    }

    fn parse_enum_variant_with_pattern(&mut self) -> ParseResult {
        let id = self.try_id()?;
        self.eat_tokens(1);
        let pattern = self.try_pattern()?;
        if pattern == 0 {
            return Err(ParseError::invalid_syntax(
                "Expected a pattern after `=`".to_string(),
                self.peek_next_token().kind,
                self.current_span(),
            ));
        }

        Ok(
            NodeBuilder::new(NodeKind::EnumVariantWithPattern, self.current_span())
                .add_single_child(id)
                .add_single_child(pattern)
                .build(&mut self.ast),
        )
    }

    fn parse_enum_variant_with_sub_enum(&mut self) -> ParseResult {
        let id = self.try_id()?;
        self.eat_tokens(1); // eat the dot
        let body = self.try_multi_with_bracket(
            &[Rule::comma("enum variant", |p| p.try_enum_variant())],
            (TokenKind::LBrace, TokenKind::RBrace),
        )?;

        Ok(
            NodeBuilder::new(NodeKind::EnumVariantWithSubEnum, self.current_span())
                .add_single_child(id)
                .add_multiple_children(body)
                .build(&mut self.ast),
        )
    }

    fn parse_enum_variant_with_struct(&mut self) -> ParseResult {
        let id = self.try_id()?;
        self.eat_tokens(1); // eat the left brace
        let fields = self.try_multi(&[Rule::comma("struct field", |p| p.try_struct_field())])?;

        if !self.eat_token(TokenKind::RBrace) {
            return Err(ParseError::unexpected_token(
                TokenKind::RBrace,
                self.peek_next_token().kind,
                self.current_span(),
            ));
        }

        Ok(
            NodeBuilder::new(NodeKind::EnumVariantWithStruct, self.current_span())
                .add_single_child(id)
                .add_multiple_children(fields)
                .build(&mut self.ast),
        )
    }

    fn parse_enum_variant_with_tuple(&mut self) -> ParseResult {
        let id = self.try_id()?;
        self.eat_tokens(1); // eat the left parenthesis
        let fields = self.try_multi(&[Rule::comma("tuple field", |p| {
            p.try_expr_without_object_call()
        })])?;

        if !self.eat_token(TokenKind::RParen) {
            return Err(ParseError::unexpected_token(
                TokenKind::RParen,
                self.peek_next_token().kind,
                self.current_span(),
            ));
        }

        Ok(
            NodeBuilder::new(NodeKind::EnumVariantWithTuple, self.current_span())
                .add_single_child(id)
                .add_multiple_children(fields)
                .build(&mut self.ast),
        )
    }

    pub fn try_trait(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Trait.as_ref(), |p| {
            p.eat_tokens(1);
            let id = p.try_id()?;
            let clauses = p.try_clauses()?;

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "expected a block after `trait` declaration".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let nodes = p.try_multi_with_bracket(
                &[
                    Rule::comma("property", |p| p.try_property()),
                    Rule::semicolon("item", |p| p.try_item()),
                    Rule::semicolon("statement", |p| p.try_statement()),
                ],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast);

            Ok(NodeBuilder::new(NodeKind::TraitDef, p.current_span())
                .add_single_child(id)
                .add_multiple_children(clauses)
                .add_single_child(block)
                .build(&mut p.ast))
        })
    }

    // fn id? ( params ) (-> return_type)? (handles eff)? clauses?  (block | = expr)
    pub fn try_function(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Fn.as_ref(), |p| {
            p.eat_tokens(1);
            let id = p.try_id()?;
            if !p.peek(TokenKind::LParen.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "expected `(` after function name".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            let params = p.try_multi_with_bracket(
                &[Rule::comma("parameter", |p| p.try_param())],
                (TokenKind::LParen, TokenKind::RParen),
            )?;

            let return_type = if p.eat_token(TokenKind::Arrow) {
                let ty = p.try_expr_without_object_call()?;
                if ty == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a return type after `->`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                ty
            } else {
                0
            };

            let handles = if p.eat_token(TokenKind::Handles) {
                let eff = p.try_expr_without_object_call()?;
                if eff == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an effect expression after `handles`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                eff
            } else {
                0
            };

            let clauses = p.try_clauses()?;

            if !p.peek(TokenKind::LBrace.as_ref())
                && !p.peek(TokenKind::Eq.as_ref())
                && !p.peek(TokenKind::Semi.as_ref())
            {
                return Err(ParseError::invalid_syntax(
                    "expected a block or `=` after function declaration".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            if p.peek(TokenKind::Eq.as_ref()) {
                p.eat_tokens(1);
                let expr = p.try_expr()?;
                if expr == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an expression after `=`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                Ok(NodeBuilder::new(NodeKind::FunctionDef, p.current_span())
                    .add_single_child(id)
                    .add_multiple_children(params)
                    .add_single_child(return_type)
                    .add_single_child(handles)
                    .add_multiple_children(clauses)
                    .add_single_child(expr)
                    .build(&mut p.ast))
            } else if p.peek(TokenKind::LBrace.as_ref()) {
                let nodes = p.try_multi_with_bracket(
                    &[
                        Rule::comma("property", |p| p.try_property()),
                        Rule::semicolon("item", |p| p.try_item()),
                        Rule::semicolon("statement", |p| p.try_statement()),
                    ],
                    (TokenKind::LBrace, TokenKind::RBrace),
                )?;

                let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                    .add_multiple_children(nodes)
                    .build(&mut p.ast);

                Ok(NodeBuilder::new(NodeKind::FunctionDef, p.current_span())
                    .add_single_child(id)
                    .add_multiple_children(params)
                    .add_single_child(return_type)
                    .add_single_child(handles)
                    .add_multiple_children(clauses)
                    .add_single_child(block)
                    .build(&mut p.ast))
            } else if p.peek(TokenKind::Semi.as_ref()) {
                Ok(NodeBuilder::new(NodeKind::FunctionDef, p.current_span())
                    .add_single_child(id)
                    .add_multiple_children(params)
                    .add_single_child(return_type)
                    .add_single_child(handles)
                    .add_multiple_children(clauses)
                    .add_single_child(0)
                    .build(&mut p.ast))
            } else {
                Err(ParseError::invalid_syntax(
                    "expected a block or `=` after function declaration".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ))
            }
        })
    }

    // mod id? clauses? block
    pub fn try_module(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Mod.as_ref(), |p| {
            p.eat_tokens(1);
            let id = p.try_id()?;
            let clauses = p.try_clauses()?;

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "expected a block after `mod` declaration".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let nodes = p.try_multi_with_bracket(
                &[
                    Rule::comma("property", |p| p.try_property()),
                    Rule::semicolon("item", |p| p.try_item()),
                    Rule::semicolon("statement", |p| p.try_statement()),
                ],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast);

            Ok(NodeBuilder::new(NodeKind::ModuleDef, p.current_span())
                .add_single_child(id)
                .add_multiple_children(clauses)
                .add_single_child(block)
                .build(&mut p.ast))
        })
    }

    pub fn try_implementation(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Impl.as_ref(), |p| {
            p.eat_tokens(1);
            let expr = p.try_expr_without_object_call()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a type or trait after `impl`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            if p.eat_token(TokenKind::For) {
                let ty = p.try_expr_without_object_call()?;
                if ty == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a type after `for`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                let clauses = p.try_clauses()?;

                if !p.peek(TokenKind::LBrace.as_ref()) {
                    return Err(ParseError::invalid_syntax(
                        "expected a block after `impl` declaration".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }

                let nodes = p.try_multi_with_bracket(
                    &[
                        Rule::comma("property", |p| p.try_property()),
                        Rule::semicolon("item", |p| p.try_item()),
                        Rule::semicolon("statement", |p| p.try_statement()),
                    ],
                    (TokenKind::LBrace, TokenKind::RBrace),
                )?;

                let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                    .add_multiple_children(nodes)
                    .build(&mut p.ast);

                Ok(NodeBuilder::new(NodeKind::ImplTraitDef, p.current_span())
                    .add_single_child(expr)
                    .add_single_child(ty)
                    .add_multiple_children(clauses)
                    .add_single_child(block)
                    .build(&mut p.ast))
            } else {
                let clauses = p.try_clauses()?;
                if !p.peek(TokenKind::LBrace.as_ref()) {
                    return Err(ParseError::invalid_syntax(
                        "expected a block after `impl` declaration".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }

                let nodes = p.try_multi_with_bracket(
                    &[
                        Rule::comma("property", |p| p.try_property()),
                        Rule::semicolon("item", |p| p.try_item()),
                        Rule::semicolon("statement", |p| p.try_statement()),
                    ],
                    (TokenKind::LBrace, TokenKind::RBrace),
                )?;

                let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                    .add_multiple_children(nodes)
                    .build(&mut p.ast);

                Ok(NodeBuilder::new(NodeKind::ImplTraitDef, p.current_span())
                    .add_single_child(expr)
                    .add_multiple_children(clauses)
                    .add_single_child(block)
                    .build(&mut p.ast))
            }
        })
    }

    pub fn try_extension(&mut self) -> ParseResult {
        // just like implementation, but for extensions
        self.scoped_with_expected_prefix(TokenKind::Extend.as_ref(), |p| {
            p.eat_tokens(1);

            let expr = p.try_expr_without_object_call()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a type or trait after `extend`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            if p.eat_token(TokenKind::For) {
                let ty = p.try_expr_without_object_call()?;
                if ty == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a type after `for`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                let clauses = p.try_clauses()?;

                if !p.peek(TokenKind::LBrace.as_ref()) {
                    return Err(ParseError::invalid_syntax(
                        "expected a block after `extend` declaration".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }

                let nodes = p.try_multi_with_bracket(
                    &[
                        Rule::comma("property", |p| p.try_property()),
                        Rule::semicolon("item", |p| p.try_item()),
                        Rule::semicolon("statement", |p| p.try_statement()),
                    ],
                    (TokenKind::LBrace, TokenKind::RBrace),
                )?;

                let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                    .add_multiple_children(nodes)
                    .build(&mut p.ast);

                Ok(NodeBuilder::new(NodeKind::ExtendTraitDef, p.current_span())
                    .add_single_child(expr)
                    .add_single_child(ty)
                    .add_multiple_children(clauses)
                    .add_single_child(block)
                    .build(&mut p.ast))
            } else {
                let clauses = p.try_clauses()?;
                if !p.peek(TokenKind::LBrace.as_ref()) {
                    return Err(ParseError::invalid_syntax(
                        "expected a block after `extend` declaration".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }

                let nodes = p.try_multi_with_bracket(
                    &[
                        Rule::comma("property", |p| p.try_property()),
                        Rule::semicolon("item", |p| p.try_item()),
                        Rule::semicolon("statement", |p| p.try_statement()),
                    ],
                    (TokenKind::LBrace, TokenKind::RBrace),
                )?;

                let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                    .add_multiple_children(nodes)
                    .build(&mut p.ast);

                Ok(NodeBuilder::new(NodeKind::ExtendTraitDef, p.current_span())
                    .add_single_child(expr)
                    .add_multiple_children(clauses)
                    .add_single_child(block)
                    .build(&mut p.ast))
            }
        })
    }
}
