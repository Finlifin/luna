use super::basic::Rule;
use super::error::*;
use super::parser::*;
use ast::*;
use lex::TokenKind;

impl Parser<'_> {
    // ── Shared declarative-form helpers ──────────────────────────────────
    // These are used by both `try_param`, `try_clause`, and `try_param_type`.

    /// Parse `...id : type` and build a node of `kind` (a, b).
    fn parse_vararg_decl(&mut self, kind: NodeKind) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Dot, TokenKind::Dot, TokenKind::Dot], |p| {
            p.eat_tokens(3); // consume '...'
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
            let ty = p.try_expr_without_extended_call()?;
            if ty == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a type after `:`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }
            Ok(NodeBuilder::new(kind, p.current_span())
                .add_single_child(id)
                .add_single_child(ty)
                .build(&mut p.ast))
        })
    }

    /// Parse `.id : type = default` and build a node of `kind` (a, b, c).
    fn parse_optional_decl(&mut self, kind: NodeKind) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Dot, TokenKind::Id, TokenKind::Colon], |p| {
            p.eat_tokens(1); // eat '.'
            let id = p.try_id()?;
            p.eat_tokens(1); // eat ':'
            let ty = p.try_expr_without_extended_call()?;
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
            let default_value = p.try_expr_without_extended_call()?;
            if default_value == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a default value after `=`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }
            Ok(NodeBuilder::new(kind, p.current_span())
                .add_single_child(id)
                .add_single_child(ty)
                .add_single_child(default_value)
                .build(&mut p.ast))
        })
    }

    /// Parse `id : type` and build a node of `kind` (a, b).
    fn parse_type_bound_decl(&mut self, kind: NodeKind) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Id, TokenKind::Colon], |p| {
            let id = p.try_id()?;
            p.eat_tokens(1); // eat ':'
            let ty = p.try_expr_without_extended_call()?;
            if ty == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a type after `:`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }
            Ok(NodeBuilder::new(kind, p.current_span())
                .add_single_child(id)
                .add_single_child(ty)
                .build(&mut p.ast))
        })
    }

    /// Parse `id :- trait_bound` and build a node of `kind` (a, b).
    fn parse_trait_bound_decl(&mut self, kind: NodeKind) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Id, TokenKind::ColonMinus], |p| {
            let id = p.try_id()?;
            p.eat_tokens(1); // eat ':-'
            let trait_bound = p.try_expr_without_extended_call()?;
            if trait_bound == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a trait bound after `:-`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }
            Ok(NodeBuilder::new(kind, p.current_span())
                .add_single_child(id)
                .add_single_child(trait_bound)
                .build(&mut p.ast))
        })
    }

    /// Parse a wrapper keyword followed by an inner item, building a unary node.
    fn parse_wrapper_param<F>(&mut self, kind: NodeKind, inner_parser: F) -> ParseResult
    where
        F: FnOnce(&mut Self) -> ParseResult,
    {
        self.scoped(|p| {
            p.eat_tokens(1); // consume the wrapper keyword
            let inner = inner_parser(p)?;
            if inner == 0 {
                return Err(ParseError::invalid_syntax(
                    format!(
                        "Expected a parameter after `{}`",
                        p.current_token().kind.lexme()
                    ),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }
            Ok(NodeBuilder::new(kind, p.current_span())
                .add_single_child(inner)
                .build(&mut p.ast))
        })
    }

    // ── Parameter parsers ────────────────────────────────────────────────

    /// parameter -> trivial_param | type_bound_param | optional_param | vararg_param
    ///            | self_param | self_ref_param | comptime_param | error_param
    ///            | catch_param | lambda_param | implicit_param | quote_param | attr_param
    pub fn try_param(&mut self) -> ParseResult {
        self.scoped(|p| {
            // Wrapper parameter prefixes: comptime/error/catch/lambda/implicit/quote
            match p.peek_next_token().kind {
                TokenKind::Comptime => {
                    return p.parse_wrapper_param(NodeKind::ComptimeParam, |p| p.try_param());
                }
                TokenKind::Error => {
                    return p.parse_wrapper_param(NodeKind::ErrorParam, |p| p.try_param());
                }
                TokenKind::Catch => {
                    return p.parse_wrapper_param(NodeKind::CatchParam, |p| p.try_param());
                }
                TokenKind::Lambda => {
                    return p.parse_wrapper_param(NodeKind::LambdaParam, |p| p.try_param());
                }
                TokenKind::Implicit => {
                    return p.parse_wrapper_param(NodeKind::ImplicitParam, |p| p.try_param());
                }
                TokenKind::KwQuote => {
                    return p.parse_wrapper_param(NodeKind::QuoteParam, |p| p.try_param());
                }
                _ => (),
            }

            // ^expr parameter (attribute parameter)
            if p.peek(TokenKind::Caret.as_ref()) {
                p.eat_tokens(1); // consume '^'
                let attr_expr = p.try_expr_without_extended_call()?;
                if attr_expr == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected attribute expression after `^`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                let inner = p.try_param()?;
                if inner == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a parameter after attribute expression".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                return Ok(NodeBuilder::new(NodeKind::AttrParam, p.current_span())
                    .add_single_child(attr_expr)
                    .add_single_child(inner)
                    .build(&mut p.ast));
            }

            // ...id : type (vararg parameter)
            let vararg = p.parse_vararg_decl(NodeKind::VarargParam)?;
            if vararg != 0 {
                return Ok(vararg);
            }

            // .id : type = default (optional parameter)
            let optional = p.parse_optional_decl(NodeKind::OptionalParam)?;
            if optional != 0 {
                return Ok(optional);
            }

            // id : type (type bound parameter)
            let type_bound = p.parse_type_bound_decl(NodeKind::TypeBoundParam)?;
            if type_bound != 0 {
                return Ok(type_bound);
            }

            // id :- trait_bound (trait bound parameter)
            let trait_bound = p.parse_trait_bound_decl(NodeKind::TraitBoundParam)?;
            if trait_bound != 0 {
                return Ok(trait_bound);
            }

            // *self
            if p.peek(&[TokenKind::Star, TokenKind::SelfLower]) {
                p.eat_tokens(2);
                return Ok(
                    NodeBuilder::new(NodeKind::SelfRefParam, p.current_span()).build(&mut p.ast)
                );
            }

            // *itself
            if p.peek(&[TokenKind::Star, TokenKind::Itself]) {
                p.eat_tokens(2);
                return Ok(
                    NodeBuilder::new(NodeKind::ItselfRefParam, p.current_span()).build(&mut p.ast)
                );
            }

            // self
            if p.peek(TokenKind::SelfLower.as_ref()) {
                p.eat_tokens(1);
                return Ok(
                    NodeBuilder::new(NodeKind::SelfParam, p.current_span()).build(&mut p.ast)
                );
            }

            // itself
            if p.peek(TokenKind::Itself.as_ref()) {
                p.eat_tokens(1);
                return Ok(
                    NodeBuilder::new(NodeKind::ItselfParam, p.current_span()).build(&mut p.ast)
                );
            }

            // trivial parameter: just an id
            Ok(p.try_id()?)
        })
    }

    /// Type-level parameter (used by typealias and newtype).
    /// Excludes self/self_ref — only type-parametric forms.
    pub fn try_type_param(&mut self) -> ParseResult {
        self.scoped(|p| {
            // Wrapper parameter prefixes
            match p.peek_next_token().kind {
                TokenKind::Comptime => {
                    return p.parse_wrapper_param(NodeKind::ComptimeParam, |p| p.try_type_param());
                }
                TokenKind::Error => {
                    return p.parse_wrapper_param(NodeKind::ErrorParam, |p| p.try_type_param());
                }
                TokenKind::Catch => {
                    return p.parse_wrapper_param(NodeKind::CatchParam, |p| p.try_type_param());
                }
                TokenKind::Lambda => {
                    return p.parse_wrapper_param(NodeKind::LambdaParam, |p| p.try_type_param());
                }
                TokenKind::Implicit => {
                    return p.parse_wrapper_param(NodeKind::ImplicitParam, |p| p.try_type_param());
                }
                TokenKind::KwQuote => {
                    return p.parse_wrapper_param(NodeKind::QuoteParam, |p| p.try_type_param());
                }
                _ => (),
            }

            // ^expr parameter (attribute parameter)
            if p.peek(TokenKind::Caret.as_ref()) {
                p.eat_tokens(1); // consume '^'
                let attr_expr = p.try_expr_without_extended_call()?;
                if attr_expr == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected attribute expression after `^`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                let inner = p.try_type_param()?;
                if inner == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a parameter after attribute expression".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                return Ok(NodeBuilder::new(NodeKind::AttrParam, p.current_span())
                    .add_single_child(attr_expr)
                    .add_single_child(inner)
                    .build(&mut p.ast));
            }

            // ...id : type
            let vararg = p.parse_vararg_decl(NodeKind::VarargParam)?;
            if vararg != 0 {
                return Ok(vararg);
            }

            // .id : type = default
            let optional = p.parse_optional_decl(NodeKind::OptionalParam)?;
            if optional != 0 {
                return Ok(optional);
            }

            // id : type
            let type_bound = p.parse_type_bound_decl(NodeKind::TypeBoundParam)?;
            if type_bound != 0 {
                return Ok(type_bound);
            }

            // id :- trait_bound
            let trait_bound = p.parse_trait_bound_decl(NodeKind::TraitBoundParam)?;
            if trait_bound != 0 {
                return Ok(trait_bound);
            }

            // trivial: just an id
            Ok(p.try_id()?)
        })
    }

    /// Parameter type (used in fn_type).
    /// parameter_type -> itself | *itself | comptime/error/catch/lambda/implicit/quote/assoc param_type
    ///                 | ...id:type | .id:type | id:type | id:-trait | expr
    pub fn try_param_type(&mut self) -> ParseResult {
        self.scoped(|p| {
            // Wrapper prefixes (recursive)
            match p.peek_next_token().kind {
                TokenKind::Comptime => {
                    return p.parse_wrapper_param(NodeKind::ComptimeParam, |p| p.try_param_type());
                }
                TokenKind::Error => {
                    return p.parse_wrapper_param(NodeKind::ErrorParam, |p| p.try_param_type());
                }
                TokenKind::Catch => {
                    return p.parse_wrapper_param(NodeKind::CatchParam, |p| p.try_param_type());
                }
                TokenKind::Lambda => {
                    return p.parse_wrapper_param(NodeKind::LambdaParam, |p| p.try_param_type());
                }
                TokenKind::Implicit => {
                    return p.parse_wrapper_param(NodeKind::ImplicitParam, |p| p.try_param_type());
                }
                TokenKind::KwQuote => {
                    return p.parse_wrapper_param(NodeKind::QuoteParam, |p| p.try_param_type());
                }
                TokenKind::Assoc => {
                    return p.parse_wrapper_param(NodeKind::AssocParam, |p| p.try_param_type());
                }
                _ => (),
            }

            // *itself
            if p.peek(&[TokenKind::Star, TokenKind::Itself]) {
                p.eat_tokens(2);
                return Ok(
                    NodeBuilder::new(NodeKind::ItselfRefParam, p.current_span()).build(&mut p.ast)
                );
            }

            // itself
            if p.peek(TokenKind::Itself.as_ref()) {
                p.eat_tokens(1);
                return Ok(
                    NodeBuilder::new(NodeKind::ItselfParam, p.current_span()).build(&mut p.ast)
                );
            }

            // ...id : type (vararg)
            let vararg = p.parse_vararg_decl(NodeKind::VarargDeclClause)?;
            if vararg != 0 {
                return Ok(vararg);
            }

            // .id : type (optional, no default in param_type context)
            // Uses decl_clause rules: .decl_clause = just `.` + a decl_clause form
            if p.peek(&[TokenKind::Dot, TokenKind::Id, TokenKind::Colon]) {
                p.eat_tokens(1); // eat '.'
                let inner = p.try_clause()?;
                if inner == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a declaration clause after `.`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                return Ok(inner); // The clause itself carries the info
            }

            // id : type (type bound decl clause)
            let type_bound = p.parse_type_bound_decl(NodeKind::TypeBoundDeclClause)?;
            if type_bound != 0 {
                return Ok(type_bound);
            }

            // id :- trait_bound (trait bound decl clause)
            let trait_bound = p.parse_trait_bound_decl(NodeKind::TraitBoundDeclClause)?;
            if trait_bound != 0 {
                return Ok(trait_bound);
            }

            // Fallback: trivial_param_type → just an expression (type)
            p.try_expr_without_extended_call()
        })
    }

    /// fn_type -> pure? comptime? inline? (unsafe | spec | verified)? (extern "ABI")? fn(parameter_type*)
    /// Creates FnType node: flags_u32, abi_node, N(parameter_types).
    /// The entry point in expressions is the `fn` token (or modifier keywords before `fn`).
    pub fn try_fn_type(&mut self) -> ParseResult {
        self.scoped(|p| {
            let mut flags: u32 = 0;
            let mut abi_node: NodeIndex = 0;

            // Parse modifier keywords
            loop {
                match p.peek_next_token().kind {
                    TokenKind::Pure => {
                        flags |= ast::FN_MOD_PURE;
                        p.eat_tokens(1);
                    }
                    TokenKind::Comptime => {
                        flags |= ast::FN_MOD_COMPTIME;
                        p.eat_tokens(1);
                    }
                    TokenKind::Inline => {
                        flags |= ast::FN_MOD_INLINE;
                        p.eat_tokens(1);
                    }
                    TokenKind::Unsafe => {
                        flags |= ast::FN_MOD_UNSAFE;
                        p.eat_tokens(1);
                    }
                    TokenKind::Spec => {
                        flags |= ast::FN_MOD_SPEC;
                        p.eat_tokens(1);
                    }
                    TokenKind::Verified => {
                        flags |= ast::FN_MOD_VERIFIED;
                        p.eat_tokens(1);
                    }
                    TokenKind::Extern => {
                        flags |= ast::FN_MOD_EXTERN;
                        p.eat_tokens(1);
                        // Check for optional ABI string
                        if p.peek(TokenKind::Str.as_ref()) {
                            abi_node = NodeBuilder::new(NodeKind::Str, p.next_token_span())
                                .build(&mut p.ast);
                            p.eat_tokens(1);
                        }
                    }
                    _ => break,
                }
            }

            // Expect 'fn'
            if !p.eat_token(TokenKind::Fn) {
                return Err(ParseError::invalid_syntax(
                    "Expected `fn` after function type modifiers".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            // Parse parameter types: (parameter_type*)
            let params = p.try_multi_with_bracket(
                &[Rule::comma("parameter type", |p| p.try_param_type())],
                (TokenKind::LParen, TokenKind::RParen),
            )?;

            Ok(NodeBuilder::new(NodeKind::FnType, p.current_span())
                .add_single_child(flags) // raw u32 modifier bitmask
                .add_single_child(abi_node)
                .add_multiple_children(params)
                .build(&mut p.ast))
        })
    }

    fn parse_predicate_clause(&mut self, keyword: TokenKind) -> ParseResult {
        self.eat_tokens(1); // eat the keyword
        let label_id = if self.eat_token(TokenKind::Colon) {
            let id = self.try_id()?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an identifier after `:`".to_string(),
                    TokenKind::Id,
                    self.current_span(),
                ));
            }
            id
        } else {
            0
        };

        let predicate = self.try_expr_without_extended_call()?;
        if predicate == 0 {
            return Err(ParseError::invalid_syntax(
                format!(
                    "Expected a predicate expression after `{}`",
                    keyword.lexme()
                ),
                self.peek_next_token().kind,
                self.current_span(),
            ));
        }

        Ok(NodeBuilder::new(
            match keyword {
                TokenKind::Requires => NodeKind::Requires,
                TokenKind::Ensures => NodeKind::Ensures,
                TokenKind::Decreases => NodeKind::Decreases,
                // TokenKind::Outcomes => NodeKind::Outcomes,
                _ => unreachable!(),
            },
            self.current_span(),
        )
        .add_single_child(label_id)
        .add_single_child(predicate)
        .build(&mut self.ast))
    }

    /// Parse a single clause in a `where` block.
    /// clause -> decl_clause | verification_clause | outcome_clause
    pub fn try_clause(&mut self) -> ParseResult {
        self.scoped(|p| {
            // Verification clauses
            match p.peek_next_token().kind {
                TokenKind::Requires => return p.parse_predicate_clause(TokenKind::Requires),
                TokenKind::Ensures => return p.parse_predicate_clause(TokenKind::Ensures),
                TokenKind::Decreases => return p.parse_predicate_clause(TokenKind::Decreases),
                _ => (),
            }

            // quote decl_clause
            if p.peek(TokenKind::KwQuote.as_ref()) {
                return p.parse_wrapper_param(NodeKind::QuoteDeclClause, |p| p.try_clause());
            }

            // ...id : type (vararg declaration clause)
            let vararg = p.parse_vararg_decl(NodeKind::VarargDeclClause)?;
            if vararg != 0 {
                return Ok(vararg);
            }

            // .id : type = default (optional declaration clause)
            let optional = p.parse_optional_decl(NodeKind::OptionalDeclClause)?;
            if optional != 0 {
                return Ok(optional);
            }

            // id : type (type bound declaration clause)
            let type_bound = p.parse_type_bound_decl(NodeKind::TypeBoundDeclClause)?;
            if type_bound != 0 {
                return Ok(type_bound);
            }

            // id :- trait_bound (trait bound declaration clause)
            let trait_bound = p.parse_trait_bound_decl(NodeKind::TraitBoundDeclClause)?;
            if trait_bound != 0 {
                return Ok(trait_bound);
            }

            // id (trivial type declaration clause)
            let id = p.try_id()?;
            if id == 0 {
                return Ok(0);
            }
            Ok(NodeBuilder::new(NodeKind::TypeDeclClause, p.current_span())
                .add_single_child(id)
                .build(&mut p.ast))
        })
    }

    // where clauses
    pub fn try_clauses(&mut self) -> Result<Vec<NodeIndex>, ParseError> {
        self.scoped_with_expected_prefix(TokenKind::Where.as_ref(), |p| {
            p.eat_tokens(1); // eat the 'where'
            let clauses = p.try_multi(&[Rule::comma("clause", |p| p.try_clause())])?;
            if clauses.is_empty() {
                return Err(ParseError::invalid_syntax(
                    "Expected at least one clause after `where`".to_string(),
                    TokenKind::Id,
                    p.current_span(),
                ));
            }
            Ok(clauses)
        })
    }

    // -> result_id: type_expr
    // -> type_expr
    pub fn try_return_type(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Arrow.as_ref(), |p| {
            p.eat_tokens(1); // eat the '->'
            if p.peek(&[TokenKind::Id, TokenKind::Colon]) {
                let id = p.try_id()?;
                p.eat_tokens(1); // eat the colon
                let ty = p.try_expr_without_extended_call()?;
                if ty == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a type after `:`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                return Ok(NodeBuilder::new(NodeKind::ResultWithId, p.current_span())
                    .add_single_child(id)
                    .add_single_child(ty)
                    .build(&mut p.ast));
            }

            let ty = p.try_expr_without_extended_call()?;
            if ty == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a type after `->`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }
            Ok(ty)
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
                    Rule::semicolon("statement or definition", |p| {
                        p.try_statement_or_definition()
                    }),
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
                let value = p.try_expr_without_extended_call()?;
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
                    Rule::semicolon("statement or definition", |p| {
                        p.try_statement_or_definition()
                    }),
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
            // id: pattern (pattern enum variant)
            if p.peek(&[TokenKind::Id, TokenKind::Colon]) {
                return p.parse_pattern_enum_variant();
            }
            // id = expr (expr enum variant)
            if p.peek(&[TokenKind::Id, TokenKind::Eq]) {
                return p.parse_expr_enum_variant();
            }
            // id.{ enum_variant* } (sub-enum)
            if p.peek(&[TokenKind::Id, TokenKind::Dot, TokenKind::LBrace]) {
                return p.parse_enum_variant_with_sub_enum();
            }
            // id { struct_field* } (struct variant)
            if p.peek(&[TokenKind::Id, TokenKind::LBrace]) {
                return p.parse_enum_variant_with_struct();
            }
            // id (expr*) (tuple variant)
            if p.peek(&[TokenKind::Id, TokenKind::LParen]) {
                return p.parse_enum_variant_with_tuple();
            }
            // trivial variant: just id
            p.try_id()
        })
    }

    // pattern_enum_variant -> id: pattern
    fn parse_pattern_enum_variant(&mut self) -> ParseResult {
        let id = self.try_id()?;
        self.eat_tokens(1); // eat ':'
        let pattern = self.try_pattern()?;
        if pattern == 0 {
            return Err(ParseError::invalid_syntax(
                "Expected a pattern after `:` in enum variant".to_string(),
                self.peek_next_token().kind,
                self.current_span(),
            ));
        }

        Ok(
            NodeBuilder::new(NodeKind::PatternEnumVariant, self.current_span())
                .add_single_child(id)
                .add_single_child(pattern)
                .build(&mut self.ast),
        )
    }

    // expr_enum_variant -> id = expr
    fn parse_expr_enum_variant(&mut self) -> ParseResult {
        let id = self.try_id()?;
        self.eat_tokens(1); // eat '='
        let expr = self.try_expr_without_extended_call()?;
        if expr == 0 {
            return Err(ParseError::invalid_syntax(
                "Expected an expression after `=` in enum variant".to_string(),
                self.peek_next_token().kind,
                self.current_span(),
            ));
        }

        Ok(
            NodeBuilder::new(NodeKind::ExprEnumVariant, self.current_span())
                .add_single_child(id)
                .add_single_child(expr)
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
            NodeBuilder::new(NodeKind::SubEnumEnumVariant, self.current_span())
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
            NodeBuilder::new(NodeKind::StructEnumVariant, self.current_span())
                .add_single_child(id)
                .add_multiple_children(fields)
                .build(&mut self.ast),
        )
    }

    fn parse_enum_variant_with_tuple(&mut self) -> ParseResult {
        let id = self.try_id()?;
        self.eat_tokens(1); // eat the left parenthesis
        let fields = self.try_multi(&[Rule::comma("tuple field", |p| {
            p.try_expr_without_extended_call()
        })])?;

        if !self.eat_token(TokenKind::RParen) {
            return Err(ParseError::unexpected_token(
                TokenKind::RParen,
                self.peek_next_token().kind,
                self.current_span(),
            ));
        }

        Ok(
            NodeBuilder::new(NodeKind::TupleEnumVariant, self.current_span())
                .add_single_child(id)
                .add_multiple_children(fields)
                .build(&mut self.ast),
        )
    }

    // trait id (:- expr)? clauses? { (assoc_decl | definition | statement)* }
    pub fn try_trait(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Trait.as_ref(), |p| {
            p.eat_tokens(1);
            let id = p.try_id()?;

            // Optional super trait bound: `:- expr`
            let super_trait = if p.eat_token(TokenKind::ColonMinus) {
                let st = p.try_expr_without_extended_call()?;
                if st == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a super trait after `:-`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                st
            } else {
                0
            };

            let clauses = p.try_clauses()?;

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "Expected a block after `trait` declaration".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let nodes = p.try_multi_with_bracket(
                &[
                    Rule::semicolon("associated declaration", |p| p.try_assoc_decl()),
                    Rule::comma("property", |p| p.try_property()),
                    Rule::semicolon("statement or definition", |p| {
                        p.try_statement_or_definition()
                    }),
                ],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast);

            Ok(NodeBuilder::new(NodeKind::TraitDef, p.current_span())
                .add_single_child(id)
                .add_single_child(super_trait)
                .add_multiple_children(clauses)
                .add_single_child(block)
                .build(&mut p.ast))
        })
    }

    // assoc id(<parameter*>)?: expr (= default)? clauses?
    pub fn try_assoc_decl(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Assoc.as_ref(), |p| {
            p.eat_tokens(1); // consume 'assoc'
            let id = p.try_id()?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an identifier after `assoc`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            // Optional type parameters: <param*>
            let params = p.try_multi_with_bracket(
                &[Rule::comma("parameter", |p| p.try_param())],
                (TokenKind::Lt, TokenKind::Gt),
            )?;

            // : type
            if !p.eat_token(TokenKind::Colon) {
                return Err(ParseError::unexpected_token(
                    TokenKind::Colon,
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }
            let type_expr = p.try_expr_without_extended_call()?;
            if type_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a type after `:` in `assoc` declaration".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            // Optional default: = expr
            let default_expr = if p.eat_token(TokenKind::Eq) {
                let d = p.try_expr_without_extended_call()?;
                if d == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a default expression after `=`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                d
            } else {
                0
            };

            let clauses = p.try_clauses()?;

            Ok(NodeBuilder::new(NodeKind::AssocDecl, p.current_span())
                .add_single_child(id)
                .add_multiple_children(params)
                .add_single_child(type_expr)
                .add_single_child(default_expr)
                .add_multiple_children(clauses)
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

            let return_type = p.try_return_type()?;

            let handles = if p.eat_token(TokenKind::Handles) {
                let eff = p.try_expr_without_extended_call()?;
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
                Ok(NodeBuilder::new(NodeKind::Function, p.current_span())
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
                        Rule::semicolon("statement or definition", |p| {
                            p.try_statement_or_definition()
                        }),
                    ],
                    (TokenKind::LBrace, TokenKind::RBrace),
                )?;

                let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                    .add_multiple_children(nodes)
                    .build(&mut p.ast);

                Ok(NodeBuilder::new(NodeKind::Function, p.current_span())
                    .add_single_child(id)
                    .add_multiple_children(params)
                    .add_single_child(return_type)
                    .add_single_child(handles)
                    .add_multiple_children(clauses)
                    .add_single_child(block)
                    .build(&mut p.ast))
            } else if p.peek(TokenKind::Semi.as_ref()) {
                Ok(NodeBuilder::new(NodeKind::Function, p.current_span())
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

    // mod id { (definition | statement)* }
    pub fn try_module(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Mod.as_ref(), |p| {
            p.eat_tokens(1);
            let id = p.try_id()?;

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "Expected a block after `mod` declaration".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let nodes = p.try_multi_with_bracket(
                &[
                    Rule::comma("property", |p| p.try_property()),
                    Rule::semicolon("statement or definition", |p| {
                        p.try_statement_or_definition()
                    }),
                ],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast);

            Ok(NodeBuilder::new(NodeKind::ModuleDef, p.current_span())
                .add_single_child(id)
                .add_single_child(block)
                .build(&mut p.ast))
        })
    }

    pub fn try_implementation(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Impl.as_ref(), |p| {
            p.eat_tokens(1);
            let expr = p.try_expr_without_extended_call()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a type or trait after `impl`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            if p.eat_token(TokenKind::For) {
                // impl expr for expr clauses? { (assoc_decl | definition | statement)* }
                let ty = p.try_expr_without_extended_call()?;
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
                        "Expected a block after `impl ... for` declaration".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }

                let nodes = p.try_multi_with_bracket(
                    &[
                        Rule::semicolon("associated declaration", |p| p.try_assoc_decl()),
                        Rule::comma("property", |p| p.try_property()),
                        Rule::semicolon("statement or definition", |p| {
                            p.try_statement_or_definition()
                        }),
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
                // impl expr clauses? { (definition | statement)* }
                let clauses = p.try_clauses()?;
                if !p.peek(TokenKind::LBrace.as_ref()) {
                    return Err(ParseError::invalid_syntax(
                        "Expected a block after `impl` declaration".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }

                let nodes = p.try_multi_with_bracket(
                    &[
                        Rule::comma("property", |p| p.try_property()),
                        Rule::semicolon("statement or definition", |p| {
                            p.try_statement_or_definition()
                        }),
                    ],
                    (TokenKind::LBrace, TokenKind::RBrace),
                )?;

                let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                    .add_multiple_children(nodes)
                    .build(&mut p.ast);

                Ok(NodeBuilder::new(NodeKind::ImplDef, p.current_span())
                    .add_single_child(expr)
                    .add_multiple_children(clauses)
                    .add_single_child(block)
                    .build(&mut p.ast))
            }
        })
    }

    pub fn try_extension(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Extend.as_ref(), |p| {
            p.eat_tokens(1);

            let expr = p.try_expr_without_extended_call()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a type or trait after `extend`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            if p.eat_token(TokenKind::For) {
                // extend expr for expr clauses? { (assoc_decl | definition | statement)* }
                let ty = p.try_expr_without_extended_call()?;
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
                        "Expected a block after `extend ... for` declaration".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }

                let nodes = p.try_multi_with_bracket(
                    &[
                        Rule::semicolon("associated declaration", |p| p.try_assoc_decl()),
                        Rule::comma("property", |p| p.try_property()),
                        Rule::semicolon("statement or definition", |p| {
                            p.try_statement_or_definition()
                        }),
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
                // extend expr clauses? { (definition | statement)* }
                let clauses = p.try_clauses()?;
                if !p.peek(TokenKind::LBrace.as_ref()) {
                    return Err(ParseError::invalid_syntax(
                        "Expected a block after `extend` declaration".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }

                let nodes = p.try_multi_with_bracket(
                    &[
                        Rule::comma("property", |p| p.try_property()),
                        Rule::semicolon("statement or definition", |p| {
                            p.try_statement_or_definition()
                        }),
                    ],
                    (TokenKind::LBrace, TokenKind::RBrace),
                )?;

                let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                    .add_multiple_children(nodes)
                    .build(&mut p.ast);

                Ok(NodeBuilder::new(NodeKind::ExtendDef, p.current_span())
                    .add_single_child(expr)
                    .add_multiple_children(clauses)
                    .add_single_child(block)
                    .build(&mut p.ast))
            }
        })
    }

    // derive expr for expr clauses?
    pub fn try_derivation(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Derive.as_ref(), |p| {
            p.eat_tokens(1); // eat the 'derive'

            let trait_expr = p.try_expr_without_extended_call()?;
            if trait_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a trait expression after `derive`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            if !p.eat_token(TokenKind::For) {
                return Err(ParseError::unexpected_token(
                    TokenKind::For,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let target = p.try_expr_without_extended_call()?;
            if target == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a target type after `for` in derive".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let clauses = p.try_clauses()?;

            Ok(NodeBuilder::new(NodeKind::DeriveDef, p.current_span())
                .add_single_child(trait_expr)
                .add_single_child(target)
                .add_multiple_children(clauses)
                .build(&mut p.ast))
        })
    }

    // async? effect id (parameter*) (-> result)? clauses? (block | (= expr))
    pub fn try_effect(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Effect.as_ref(), |p| {
            p.eat_tokens(1); // eat the 'effect'
            let id = p.try_id()?;
            if !p.peek(TokenKind::LParen.as_ref()) {
                return Err(ParseError::unexpected_token(
                    TokenKind::LParen,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            let params = p.try_multi_with_bracket(
                &[Rule::comma("parameter", |p| p.try_param())],
                (TokenKind::LParen, TokenKind::RParen),
            )?;

            let return_type = p.try_return_type()?;

            let clauses = p.try_clauses()?;

            // Optional body: block | (= expr)
            let body = if p.peek(TokenKind::LBrace.as_ref()) {
                let nodes = p.try_multi_with_bracket(
                    &[
                        Rule::comma("property", |p| p.try_property()),
                        Rule::semicolon("statement or definition", |p| {
                            p.try_statement_or_definition()
                        }),
                    ],
                    (TokenKind::LBrace, TokenKind::RBrace),
                )?;
                NodeBuilder::new(NodeKind::Block, p.current_span())
                    .add_multiple_children(nodes)
                    .build(&mut p.ast)
            } else if p.eat_token(TokenKind::Eq) {
                let expr = p.try_expr()?;
                if expr == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an expression after `=`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                expr
            } else {
                0
            };

            Ok(
                NodeBuilder::new(NodeKind::AlgebraicEffect, p.current_span())
                    .add_single_child(id)
                    .add_multiple_children(params)
                    .add_single_child(return_type)
                    .add_multiple_children(clauses)
                    .add_single_child(body)
                    .build(&mut p.ast),
            )
        })
    }

    // union id clauses? { (property | union_variant | definition | statement)* }
    pub fn try_union(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Union.as_ref(), |p| {
            p.eat_tokens(1); // eat 'union'
            let id = p.try_id()?;
            let clauses = p.try_clauses()?;

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "Expected a block after `union` declaration".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let nodes = p.try_multi_with_bracket(
                &[
                    Rule::comma("union variant", |p| p.try_union_variant()),
                    Rule::comma("property", |p| p.try_property()),
                    Rule::semicolon("statement or definition", |p| {
                        p.try_statement_or_definition()
                    }),
                ],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast);

            Ok(NodeBuilder::new(NodeKind::UnionDef, p.current_span())
                .add_single_child(id)
                .add_multiple_children(clauses)
                .add_single_child(block)
                .build(&mut p.ast))
        })
    }

    // union_variant -> id : expr
    fn try_union_variant(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Id, TokenKind::Colon], |p| {
            let id = p.try_id()?;
            p.eat_tokens(1); // eat ':'
            let ty = p.try_expr_without_extended_call()?;
            if ty == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a type after `:` in union variant".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }
            Ok(NodeBuilder::new(NodeKind::UnionVariant, p.current_span())
                .add_single_child(id)
                .add_single_child(ty)
                .build(&mut p.ast))
        })
    }

    // const id (: expr)? = expr
    pub fn try_const_def(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Const.as_ref(), |p| {
            p.eat_tokens(1); // eat 'const'
            let id = p.try_id()?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an identifier after `const`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            let ty = if p.eat_token(TokenKind::Colon) {
                let t = p.try_expr_without_extended_call()?;
                if t == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a type after `:`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                t
            } else {
                0
            };

            if !p.eat_token(TokenKind::Eq) {
                return Err(ParseError::unexpected_token(
                    TokenKind::Eq,
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            let init = p.try_expr()?;
            if init == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an expression after `=` in const definition".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::ConstDef, p.current_span())
                .add_single_child(id)
                .add_single_child(ty)
                .add_single_child(init)
                .build(&mut p.ast))
        })
    }

    // typealias id ( <param*> )? = type_expr
    pub fn try_typealias(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Typealias.as_ref(), |p| {
            p.eat_tokens(1);
            let id = p.try_id()?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected identifier after 'typealias'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let params = p.try_multi_with_bracket(
                &[Rule::comma("type parameter", |p| p.try_type_param())],
                (TokenKind::Lt, TokenKind::Gt),
            )?;

            if !p.eat_token(TokenKind::Eq) {
                return Err(ParseError::unexpected_token(
                    TokenKind::Eq,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            let type_expr = p.try_expr_without_extended_call()?;
            if type_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected type expression after '='".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::TypealiasDef, p.current_span())
                .add_single_child(id)
                .add_multiple_children(params)
                .add_single_child(type_expr)
                .build(&mut p.ast))
        })
    }

    // newtype id ( <param*> )? = type_expr
    pub fn try_newtype(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Newtype.as_ref(), |p| {
            p.eat_tokens(1);
            let id = p.try_id()?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected identifier after 'newtype'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let params = p.try_multi_with_bracket(
                &[Rule::comma("type parameter", |p| p.try_type_param())],
                (TokenKind::Lt, TokenKind::Gt),
            )?;

            if !p.eat_token(TokenKind::Eq) {
                return Err(ParseError::unexpected_token(
                    TokenKind::Eq,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            let type_expr = p.try_expr_without_extended_call()?;
            if type_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected type expression after '='".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::NewtypeDef, p.current_span())
                .add_single_child(id)
                .add_multiple_children(params)
                .add_single_child(type_expr)
                .build(&mut p.ast))
        })
    }

    // test id? block
    pub fn try_test(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Test.as_ref(), |p| {
            p.eat_tokens(1); // eat the 'test'
            let id = p.try_id()?;
            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "expected a block after `test` declaration".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            let nodes = p.try_multi_with_bracket(
                &[
                    Rule::comma("property", |p| p.try_property()),
                    Rule::semicolon("statement or definition", |p| {
                        p.try_statement_or_definition()
                    }),
                ],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;
            let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast);
            Ok(NodeBuilder::new(NodeKind::TestDef, p.current_span())
                .add_single_child(id)
                .add_single_child(block)
                .build(&mut p.ast))
        })
    }

    // case id ((param*))? -> return_type clauses? ((= expr) | block)
    pub fn try_case(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Case.as_ref(), |p| {
            p.eat_tokens(1); // eat the 'case'
            let id = p.try_id()?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an identifier after `case`".to_string(),
                    TokenKind::Id,
                    p.current_span(),
                ));
            }

            let params = p.try_multi_with_bracket(
                &[Rule::comma("parameter", |p| p.try_param())],
                (TokenKind::LParen, TokenKind::RParen),
            )?;

            let return_type = p.try_return_type()?;
            if return_type == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a return type after `->`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }

            let clauses = p.try_clauses()?;

            if p.eat_token(TokenKind::Eq) {
                let expr = p.try_expr()?;
                if expr == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an expression after `=`".to_string(),
                        p.peek_next_token().kind,
                        p.current_span(),
                    ));
                }
                Ok(NodeBuilder::new(NodeKind::CaseDef, p.current_span())
                    .add_single_child(id)
                    .add_multiple_children(params)
                    .add_single_child(return_type)
                    .add_multiple_children(clauses)
                    .add_single_child(expr)
                    .build(&mut p.ast))
            } else if p.peek(TokenKind::LBrace.as_ref()) {
                let nodes = p.try_multi_with_bracket(
                    &[
                        Rule::comma("property", |p| p.try_property()),
                        Rule::semicolon("statement or definition", |p| {
                            p.try_statement_or_definition()
                        }),
                    ],
                    (TokenKind::LBrace, TokenKind::RBrace),
                )?;

                let block = NodeBuilder::new(NodeKind::Block, p.current_span())
                    .add_multiple_children(nodes)
                    .build(&mut p.ast);

                Ok(NodeBuilder::new(NodeKind::CaseDef, p.current_span())
                    .add_single_child(id)
                    .add_multiple_children(params)
                    .add_single_child(return_type)
                    .add_multiple_children(clauses)
                    .add_single_child(block)
                    .build(&mut p.ast))
            } else {
                Err(ParseError::invalid_syntax(
                    "Expected `=` followed by an expression or a block after case declaration"
                        .to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ))
            }
        })
    }
}
