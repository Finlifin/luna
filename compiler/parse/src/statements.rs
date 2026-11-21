use super::basic::Rule;
use super::error::*;
use super::parser::*;
use ast::*;
use lex::TokenKind;

impl Parser<'_> {
    pub fn try_statement_or_definition(&mut self) -> ParseResult {
        self.scoped(|p| {
            // An unnamed callable signature is a type expression, including
            // when it is the final expression of a block.
            if p.peek(&[TokenKind::Fn, TokenKind::LParen])
                || p.peek(&[TokenKind::Comptime, TokenKind::Fn, TokenKind::LParen])
            {
                return p.try_expr_statement();
            }
            let token = p.peek_next_token();
            match token.kind {
                // statements
                TokenKind::Let | TokenKind::Var => p.try_let_statement(),
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
                TokenKind::LBrace => {
                    if p.is_token_buffer_start() {
                        p.try_expr_statement()
                    } else {
                        p.try_block()
                    }
                }
                TokenKind::Inline
                    if [
                        TokenKind::Effect,
                        TokenKind::Fn,
                        TokenKind::Case,
                        TokenKind::Unsafe,
                        TokenKind::Spec,
                        TokenKind::Verified,
                        TokenKind::Atomic,
                    ]
                    .iter()
                    .any(|next| p.peek(&[TokenKind::Inline, *next])) =>
                {
                    p.try_keyword_modified_definition()
                }
                TokenKind::Inline => p.try_inline_statement(),

                TokenKind::Pub => {
                    let kw_span = p.next_token_span();
                    p.eat_tokens(1);
                    let inner = p.try_statement_or_definition()?;
                    if inner == 0 {
                        return Err(ParseError::invalid_syntax(
                            "expected a declaration after `pub`".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        )
                        .with_secondary_label(kw_span, "`pub` visibility modifier starts here"));
                    }
                    Ok(NodeBuilder::new(NodeKind::Pub, p.current_span())
                        .add_single_child(inner)
                        .build(&mut p.ast))
                }

                TokenKind::Private => {
                    let kw_span = p.next_token_span();
                    p.eat_tokens(1);
                    let inner = p.try_statement_or_definition()?;
                    if inner == 0 {
                        return Err(ParseError::invalid_syntax(
                            "expected a declaration after `private`".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        )
                        .with_secondary_label(
                            kw_span,
                            "`private` visibility modifier starts here",
                        ));
                    }
                    Ok(NodeBuilder::new(NodeKind::Private, p.current_span())
                        .add_single_child(inner)
                        .build(&mut p.ast))
                }

                // Async/Unsafe: if followed by '{', it's a block expression;
                // otherwise it's a keyword modifier on a definition.
                TokenKind::Unsafe | TokenKind::Async | TokenKind::Comptime => {
                    if p.peek(&[token.kind, TokenKind::LBrace]) {
                        p.try_expr_statement()
                    } else {
                        p.try_keyword_modified_definition()
                    }
                }

                // definitions
                TokenKind::Struct => p.try_struct(),
                TokenKind::Enum => p.try_enum(),
                TokenKind::Union => p.try_union(),
                TokenKind::Trait => p.try_trait(),
                TokenKind::Impl => p.try_implementation(),
                TokenKind::Extend => p.try_extension(),
                TokenKind::Derive => p.try_derivation(),
                TokenKind::Fn => p.try_function(),
                TokenKind::Mod => p.try_module(),
                TokenKind::Effect => p.try_effect(),
                TokenKind::Typealias => p.try_typealias(),
                TokenKind::Newtype => p.try_newtype(),
                TokenKind::Static => p.try_static_def(),
                TokenKind::Case => p.try_case(),
                TokenKind::Test => p.try_test(),
                TokenKind::Template => p.try_test_template(),

                // keyword-modifier definitions (these start definitions)
                TokenKind::Pure
                | TokenKind::KwQuote
                | TokenKind::Spec
                | TokenKind::Verified
                | TokenKind::Atomic => p.try_keyword_modified_definition(),

                // verification statements
                TokenKind::Axiom => p.try_predicate_statement(
                    Rule::semicolon("predicate expression", |p| p.try_expr()),
                    TokenKind::Axiom,
                    NodeKind::Axiom,
                    "Expected predicate expression after `axiom`".to_string(),
                ),
                TokenKind::Ensures => p.try_predicate_statement(
                    Rule::semicolon("ensures expression", |p| p.try_expr()),
                    TokenKind::Ensures,
                    NodeKind::Ensures,
                    "Expected ensures expression after `ensures`".to_string(),
                ),
                TokenKind::Requires => p.try_predicate_statement(
                    Rule::semicolon("requires expression", |p| p.try_expr()),
                    TokenKind::Requires,
                    NodeKind::Requires,
                    "Expected requires expression after `requires`".to_string(),
                ),
                TokenKind::Asserts => p.try_predicate_statement(
                    Rule::semicolon("asserts expression", |p| p.try_expr()),
                    TokenKind::Asserts,
                    NodeKind::Asserts,
                    "Expected asserts expression after `asserts`".to_string(),
                ),
                TokenKind::Assumes => p.try_predicate_statement(
                    Rule::semicolon("assumes expression", |p| p.try_expr()),
                    TokenKind::Assumes,
                    NodeKind::Assumes,
                    "Expected assumes expression after `assumes`".to_string(),
                ),
                TokenKind::Invariant => p.try_predicate_statement(
                    Rule::semicolon("invariant expression", |p| p.try_expr()),
                    TokenKind::Invariant,
                    NodeKind::Invariant,
                    "Expected invariant expression after `invariant`".to_string(),
                ),
                TokenKind::Decreases => p.try_predicate_statement(
                    Rule::semicolon("decreases expression", |p| p.try_expr()),
                    TokenKind::Decreases,
                    NodeKind::Decreases,
                    "Expected decreases expression after `decreases`".to_string(),
                ),

                _ => p.try_expr_statement(),
            }
        })
    }

    // Predicate nodes share the clause layout (optional label, predicate).
    // Statements currently have no label syntax, so retain an explicit empty
    // slot rather than exposing the next arena allocation as a second child.
    fn try_predicate_statement(
        &mut self,
        rule: Rule,
        prefix: TokenKind,
        kind: NodeKind,
        info: String,
    ) -> ParseResult {
        self.scoped_with_expected_prefix(&[prefix], |p| {
            p.eat_tokens(1);
            let predicate = (rule.parser)(p)?;
            if predicate == 0 {
                return Err(ParseError::invalid_syntax(
                    info,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            Ok(NodeBuilder::new(kind, p.current_span())
                .add_single_child(0)
                .add_single_child(predicate)
                .build(&mut p.ast))
        })
    }

    /// inline statement_expr
    pub(super) fn try_inline_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Inline.as_ref(), |p| {
            p.eat_tokens(1); // consume 'inline'
            let inner = p.try_statement_or_definition()?;
            if inner == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected a statement expression after `inline`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            Ok(
                NodeBuilder::new(NodeKind::InlineStatement, p.current_span())
                    .add_single_child(inner)
                    .build(&mut p.ast),
            )
        })
    }

    pub fn try_let_statement(&mut self) -> ParseResult {
        self.scoped(|p| {
            let mut mutable = p.peek(TokenKind::Var.as_ref());
            if !mutable && !p.peek(TokenKind::Let.as_ref()) {
                return Ok(0);
            }
            let kw_span = p.next_token_span();
            let kw_name = if mutable { "var" } else { "let" };
            p.eat_tokens(1);

            // Detect `let mut x = ...` (Rust habit -> `var x = ...`)
            if !mutable
                && p.peek_next_token().kind == TokenKind::Id
                && p.token_content(&p.peek_next_token()).as_deref() == Some("mut")
                && matches!(
                    p.peek_nth_token(2).kind,
                    TokenKind::Id
                        | TokenKind::LParen
                        | TokenKind::LBrace
                        | TokenKind::LBracket
                        | TokenKind::Underscore
                )
            {
                let mut_span = p.next_token_span();
                let next_name = p
                    .token_content(&p.peek_nth_token(2))
                    .unwrap_or_else(|| "name".to_string());
                let err = ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_let_mut",
                    "mutable bindings in Flurry are declared with `var` rather than `let mut`",
                    rustc_span::Span::new(kw_span.lo(), mut_span.hi()),
                )
                .with_primary_label("replace `let mut` with `var`")
                .with_help(format!(
                    "Write `var {next_name} = ...;` instead of `let mut {next_name} = ...;`."
                ));
                if p.recovery_enabled {
                    p.record_error(err);
                    p.eat_tokens(1); // consume `mut` and continue checking the rest of the binding!
                    mutable = true;
                } else {
                    return Err(err);
                }
            }

            let pattern = p.try_pattern()?;
            if pattern == 0 {
                return Err(ParseError::invalid_syntax(
                    format!("expected a binding pattern after `{kw_name}`"),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(kw_span, format!("`{kw_name}` binding starts here")));
            }

            let mut expr: u32 = 0;
            if p.eat_token(TokenKind::Colon) {
                expr = p.try_expr()?;
                if expr == 0 {
                    return Err(ParseError::invalid_syntax(
                        "expected a type expression after `:`".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
            }

            if !p.eat_token(TokenKind::Eq) {
                return Err(ParseError::invalid_syntax(
                    format!("{kw_name} declaration requires an initializer after `=`"),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(kw_span, format!("`{kw_name}` binding declared here"))
                .with_help(format!(
                    "Initialize the binding right away (e.g. `{kw_name} {} = value;`).",
                    p.snippet_or(pattern, "name")
                )));
            }
            let eq_span = p.prev_token_span();
            let init = p.try_expr()?;
            if init == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected an initializer expression after '='".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(
                    eq_span,
                    "`=` expects an initializer expression on its right",
                ));
            }

            let otherwise = if p.eat_token(TokenKind::Else) {
                let block = p.try_block()?;
                if block == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a block after let else".into(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                block
            } else {
                0
            };

            Ok(NodeBuilder::new(
                if mutable {
                    NodeKind::VarDecl
                } else {
                    NodeKind::LetDecl
                },
                p.current_span(),
            )
            .add_single_child(pattern)
            .add_single_child(expr)
            .add_single_child(init)
            .add_single_child(otherwise)
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

            let otherwise = if p.eat_token(TokenKind::Else) {
                let block = p.try_block()?;
                if block == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected a block after const else".into(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                block
            } else {
                0
            };
            Ok(NodeBuilder::new(NodeKind::ConstDecl, p.current_span())
                .add_single_child(pattern)
                .add_single_child(expr)
                .add_single_child(init)
                .add_single_child(otherwise)
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
        self.try_path_component(false)
    }

    fn try_path_component(&mut self, member: bool) -> ParseResult {
        self.scoped(|p| {
            let mut anchors = Vec::new();
            while matches!(p.peek_next_token().kind, TokenKind::At | TokenKind::Dot) {
                anchors.push(p.peek_next_token().kind);
                p.eat_tokens(1);
            }
            let mut left = if member || anchors.last() == Some(&TokenKind::Dot) {
                p.try_member_id()?
            } else {
                p.try_id()?
            };
            if left == 0 {
                return Ok(0);
            }

            while p.eat_token(TokenKind::Dot) {
                let next = p.peek_next_token();
                match next.kind {
                    TokenKind::Id | TokenKind::Default => {
                        let id = p.try_member_id()?;
                        left = NodeBuilder::new(NodeKind::ProjectionPath, p.current_span())
                            .add_single_child(left)
                            .add_single_child(id)
                            .build(&mut p.ast);
                    }
                    TokenKind::Star | TokenKind::SeparatedStar => {
                        p.eat_tokens(1);
                        left = NodeBuilder::new(NodeKind::ProjectionAllPath, p.current_span())
                            .add_single_child(left)
                            .build(&mut p.ast);
                    }
                    TokenKind::LBrace => {
                        p.eat_tokens(1);
                        let mut items = vec![];
                        while !p.peek(TokenKind::RBrace.as_ref()) {
                            let item = p.try_path_component(true)?;
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
                        left = NodeBuilder::new(NodeKind::ProjectionMultiPath, p.current_span())
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

            if p.peek(TokenKind::ColonColon.as_ref()) {
                let left_snip = p.snippet_or(left, "module");
                let right_snip = p
                    .token_content(&p.peek_nth_token(2))
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| "member".to_string());
                return Err(ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_colon_colon_use",
                    "Flurry `use` paths are separated by `.` rather than `::`",
                    p.next_token_span(),
                )
                .with_found(TokenKind::ColonColon)
                .with_primary_label("replace `::` with `.`")
                .with_help(format!("Write `use {left_snip}.{right_snip};` with `.`.")));
            }

            if p.eat_token(TokenKind::As) {
                let alias = p.try_id()?;
                if alias == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected an identifier after import as".into(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                left = NodeBuilder::new(NodeKind::PathAsBind, p.current_span())
                    .add_single_child(left)
                    .add_single_child(alias)
                    .build(&mut p.ast);
            }
            for anchor in anchors.into_iter().rev() {
                let kind = if anchor == TokenKind::At {
                    NodeKind::PackagePath
                } else {
                    NodeKind::SuperPath
                };
                left = NodeBuilder::new(kind, p.current_span())
                    .add_single_child(left)
                    .build(&mut p.ast);
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
            let kw_span = p.next_token_span();
            p.eat_tokens(1);
            let expr = p.try_expr_without_extended_call()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a condition expression after `if`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(kw_span, "`if` starts here"));
            }

            if p.eat_token(TokenKind::Is) {
                return p.try_if_is_match(expr);
            }

            if let Some(err) = p.diagnose_control_header_brace_conflict("if", kw_span, expr) {
                return Err(err);
            }

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "expected `{` after `if` condition to start its body block".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(kw_span, "`if` starts here"));
            }

            let body = p.try_block()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a `{ ... }` block after `if` condition".to_string(),
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
                            "expected `{` or `if` after `else`".to_string(),
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
            if p.peek(TokenKind::Do.as_ref()) {
                return p.try_if_match(expr);
            }

            let pattern = p.parse_is_match_pattern_checking_do("if", expr)?;
            if pattern == 0 {
                return p.try_if_match(expr);
            }

            let body = p.try_block()?;

            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a `{ ... }` block after `do`".to_string(),
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
                            "expected `{` or `if` after `else`".to_string(),
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
                &[Rule::comma("case arm", |p| p.try_case_arm())],
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
            let kw_span = p.next_token_span();
            p.eat_tokens(1);

            // Detect `while 'outer cond` -> `while :outer cond`
            if p.peek(&[TokenKind::Quote, TokenKind::Id]) {
                let q_span = p.next_token_span();
                let id_tok = p.peek_nth_token(2);
                let lbl = p
                    .token_content(&id_tok)
                    .unwrap_or_else(|| "label".to_string());
                return Err(ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_while_quote_label",
                    "loop labels in Flurry use `:label` rather than `'label`",
                    rustc_span::Span::new(q_span.lo(), p.token_span(id_tok).hi()),
                )
                .with_primary_label(format!("write `:{lbl}` here"))
                .with_help(format!("Write `while :{lbl} condition {{ ... }}`.")));
            }

            let label = if p.eat_token(TokenKind::Colon) {
                let l = p.try_id()?;
                if l == 0 {
                    return Err(ParseError::invalid_syntax(
                        "expected a loop label identifier after `:`".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                l
            } else {
                0
            };

            let expr = p.try_expr_without_extended_call()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a condition expression after `while`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(kw_span, "`while` starts here"));
            }

            if p.eat_token(TokenKind::Is) {
                return p.try_while_is_match(label, expr);
            }

            if let Some(err) = p.diagnose_control_header_brace_conflict("while", kw_span, expr) {
                return Err(err);
            }

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "expected `{` after `while` condition to start its body block".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(kw_span, "`while` starts here"));
            }

            let body = p.try_block()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a `{ ... }` block after `while` condition".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::WhileStatement, p.current_span())
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
            if p.peek(TokenKind::Do.as_ref()) {
                return p.try_while_match(label, expr);
            }

            let pattern = p.parse_is_match_pattern_checking_do("while", expr)?;
            if pattern == 0 {
                return p.try_while_match(label, expr);
            }

            let body = p.try_block()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a `{ ... }` block after `do`".to_string(),
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
                &[Rule::comma("case arm", |p| p.try_case_arm())],
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
            let kw_span = p.next_token_span();
            p.eat_tokens(1);

            let label = if p.eat_token(TokenKind::Colon) {
                let l = p.try_id()?;
                if l == 0 {
                    return Err(ParseError::invalid_syntax(
                        "expected a loop label identifier after `:`".to_string(),
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
                    "expected a loop binding pattern after `for`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(kw_span, "`for` starts here"));
            }

            if !p.eat_token(TokenKind::In) {
                return Err(ParseError::unexpected_token(
                    TokenKind::In,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let expr = p.try_expr_without_extended_call()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected an iterable expression after `in`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            if let Some(err) = p.diagnose_control_header_brace_conflict("for", kw_span, expr) {
                return Err(err);
            }

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "expected `{` after `for ... in ...` to start its body block".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(kw_span, "`for` starts here"));
            }

            let body = p.try_block()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a `{ ... }` block for `for` loop body".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::ForStatement, p.current_span())
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
                    "expected `{` after `when` to start condition arms".to_string(),
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
            let condition = if p.eat_token(TokenKind::Else) {
                if !p.eat_token(TokenKind::FatArrow) {
                    return Err(ParseError::unexpected_token(
                        TokenKind::FatArrow,
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    )
                    .with_help("Write `else => { ... }` for the fallback arm in `when`."));
                }
                0
            } else {
                let cond = p.parse_arm_head_checking_fat_arrow(false, "when arm")?;
                if cond == 0 {
                    return Ok(0);
                }
                cond
            };

            let body = p.try_block_or_statement()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a statement, expression, or `{ ... }` block after `=>`".to_string(),
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

    pub fn try_case_arm(&mut self) -> ParseResult {
        self.scoped(|p| {
            let pattern = p.parse_arm_head_checking_fat_arrow(true, "match arm")?;
            if pattern == 0 {
                return Ok(0);
            }

            let body = p.try_block_or_statement()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a statement, expression, or `{ ... }` block after `=>`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::CaseArm, p.current_span())
                .add_single_child(pattern)
                .add_single_child(body)
                .build(&mut p.ast))
        })
    }

    pub fn try_catch_arm(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Catch.as_ref(), |p| {
            let catch_span = p.next_token_span();
            p.eat_tokens(1);

            let id = p.parse_arm_head_checking_fat_arrow(true, "catch arm")?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a pattern after `catch`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(catch_span, "`catch` arm starts here"));
            }

            let body = p.try_block_or_statement()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a statement, expression, or `{ ... }` block after `=>`".to_string(),
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
        self.try_block_with_rules(&[Rule::semicolon("statement or definition", |p| {
            p.try_statement_or_definition()
        })])
    }

    pub(crate) fn try_block_with_rules(&mut self, rules: &[Rule]) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::LBrace.as_ref(), |p| {
            let nodes = p.try_multi_with_bracket(rules, (TokenKind::LBrace, TokenKind::RBrace))?;
            Ok(NodeBuilder::new(NodeKind::Block, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast))
        })
    }

    pub fn try_block_or_statement(&mut self) -> ParseResult {
        self.scoped(|p| {
            if p.peek(TokenKind::LBrace.as_ref()) && !p.is_token_buffer_start() {
                p.try_block()
            } else {
                p.try_statement_or_definition()
            }
        })
    }

    /// Parse a definition prefixed by keyword modifiers (pure, comptime, inline, unsafe, etc.)
    /// These become AttributeSetTrue nodes wrapping the inner definition.
    pub fn try_keyword_modified_definition(&mut self) -> ParseResult {
        // This paired declaration modifier records interface phase identity,
        // independently of the keyword modifiers on its member functions.
        if self.peek(&[TokenKind::Pure, TokenKind::Comptime, TokenKind::Trait]) {
            return self.scoped(|p| {
                p.eat_tokens(2);
                let inner = p.try_trait()?;
                let sym = lex::Symbol::intern("__flurry_kw_comptime_only_trait");
                let (hi, lo) = sym.to_raw_parts();
                let marker = NodeBuilder::new(NodeKind::Id, p.current_span())
                    .add_single_child(hi)
                    .add_single_child(lo)
                    .build(&mut p.ast);
                Ok(
                    NodeBuilder::new(NodeKind::AttributeSetTrue, p.current_span())
                        .add_single_child(marker)
                        .add_single_child(inner)
                        .build(&mut p.ast),
                )
            });
        }
        if self.peek(&[
            TokenKind::Pure,
            TokenKind::Comptime,
            TokenKind::Fn,
            TokenKind::Id,
            TokenKind::Lt,
        ]) {
            return self.try_normal_form_head_definition();
        }
        self.scoped(|p| {
            let token = p.peek_next_token();
            let attr_name = match token.kind {
                TokenKind::Pure => "__flurry_kw_pure",
                TokenKind::KwQuote => "__flurry_kw_quote",
                TokenKind::Comptime => "__flurry_kw_comptime",
                TokenKind::Inline => "__flurry_kw_inline",
                TokenKind::Unsafe => "__flurry_kw_unsafe",
                TokenKind::Spec => "__flurry_kw_spec",
                TokenKind::Verified => "__flurry_kw_verified",
                TokenKind::Atomic => "__flurry_kw_atomic",
                TokenKind::Async => "__flurry_kw_async",
                _ => return Ok(0),
            };
            p.eat_tokens(1);

            // Create a synthetic Id node carrying the pre-interned attribute name.
            let sym = lex::Symbol::intern(attr_name);
            let (hi, lo) = sym.to_raw_parts();
            let attr_id = NodeBuilder::new(NodeKind::Id, p.current_span())
                .add_single_child(hi)
                .add_single_child(lo)
                .build(&mut p.ast);

            // Parse the inner definition or another keyword modifier
            let inner = p.try_statement_or_definition()?;
            let mut declaration = inner;
            while p.ast.get_node_kind(declaration) == Some(NodeKind::AttributeSetTrue) {
                declaration = p.ast.get_children(declaration)[1];
            }
            if p.ast.get_node_kind(declaration) == Some(NodeKind::TraitDef) {
                return Err(ParseError::invalid_syntax(
                    "trait modifiers must use the paired `pure comptime` prefix".into(),
                    token.kind,
                    p.current_span(),
                ));
            }
            if inner == 0 {
                return Err(ParseError::invalid_syntax(
                    format!(
                        "expected a definition after `{}`",
                        token.kind.lexme().trim()
                    ),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(
                NodeBuilder::new(NodeKind::AttributeSetTrue, p.current_span())
                    .add_single_child(attr_id)
                    .add_single_child(inner)
                    .build(&mut p.ast),
            )
        })
    }

    // return expr? (while expr)?
    pub fn try_return_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Return.as_ref(), |p| {
            p.eat_tokens(1);
            let expr = p.try_expr()?;
            let guard = p.try_while_guard()?;
            Ok(
                NodeBuilder::new(NodeKind::ReturnStatement, p.current_span())
                    .add_single_child(expr)
                    .add_single_child(guard)
                    .build(&mut p.ast),
            )
        })
    }

    // resume expr? (while expr)?
    pub fn try_resume_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Resume.as_ref(), |p| {
            p.eat_tokens(1);
            let expr = p.try_expr()?;
            let guard = p.try_while_guard()?;
            Ok(
                NodeBuilder::new(NodeKind::ResumeStatement, p.current_span())
                    .add_single_child(expr)
                    .add_single_child(guard)
                    .build(&mut p.ast),
            )
        })
    }

    // break id? (while expr)?
    pub fn try_break_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Break.as_ref(), |p| {
            p.eat_tokens(1);
            if p.peek(&[TokenKind::Colon, TokenKind::Id])
                || p.peek(&[TokenKind::Quote, TokenKind::Id])
            {
                let prefix_tok = p.peek_next_token();
                let id_tok = p.peek_nth_token(2);
                let lbl = p
                    .token_content(&id_tok)
                    .unwrap_or_else(|| "label".to_string());
                let pfx = prefix_tok.kind.lexme().trim();
                let code = if prefix_tok.kind == TokenKind::Colon {
                    PARSE_SEPARATOR
                } else {
                    PARSE_FOREIGN_HABIT
                };
                return Err(ParseError::custom(
                    code,
                    "jump_label_prefix",
                    format!(
                        "`break` references a loop label directly as `break {lbl}` without `{pfx}`"
                    ),
                    rustc_span::Span::new(
                        p.token_span(prefix_tok).lo(),
                        p.token_span(id_tok).hi(),
                    ),
                )
                .with_primary_label(format!("remove `{pfx}` before `{lbl}`"))
                .with_note("`:label` is used on loop headers (`while :label ...`), while `break` and `continue` take the bare label name.")
                .with_help(format!("Write `break {lbl};`.")));
            }
            let label = p.try_id()?;
            let guard = p.try_while_guard()?;
            Ok(NodeBuilder::new(NodeKind::BreakStatement, p.current_span())
                .add_single_child(label)
                .add_single_child(guard)
                .build(&mut p.ast))
        })
    }

    // continue id? (while expr)?
    pub fn try_continue_statement(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Continue.as_ref(), |p| {
            p.eat_tokens(1);
            if p.peek(&[TokenKind::Colon, TokenKind::Id])
                || p.peek(&[TokenKind::Quote, TokenKind::Id])
            {
                let prefix_tok = p.peek_next_token();
                let id_tok = p.peek_nth_token(2);
                let lbl = p
                    .token_content(&id_tok)
                    .unwrap_or_else(|| "label".to_string());
                let pfx = prefix_tok.kind.lexme().trim();
                let code = if prefix_tok.kind == TokenKind::Colon {
                    PARSE_SEPARATOR
                } else {
                    PARSE_FOREIGN_HABIT
                };
                return Err(ParseError::custom(
                    code,
                    "jump_label_prefix",
                    format!(
                        "`continue` references a loop label directly as `continue {lbl}` without `{pfx}`"
                    ),
                    rustc_span::Span::new(
                        p.token_span(prefix_tok).lo(),
                        p.token_span(id_tok).hi(),
                    ),
                )
                .with_primary_label(format!("remove `{pfx}` before `{lbl}`"))
                .with_note("`:label` is used on loop headers (`while :label ...`), while `break` and `continue` take the bare label name.")
                .with_help(format!("Write `continue {lbl};`.")));
            }
            let label = p.try_id()?;
            let guard = p.try_while_guard()?;
            Ok(
                NodeBuilder::new(NodeKind::ContinueStatement, p.current_span())
                    .add_single_child(label)
                    .add_single_child(guard)
                    .build(&mut p.ast),
            )
        })
    }

    /// Parse optional `while expr` guard
    fn try_while_guard(&mut self) -> ParseResult {
        if self.eat_token(TokenKind::While) {
            let guard = self.try_expr()?;
            if guard == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a condition expression after `while`".to_string(),
                    self.peek_next_token().kind,
                    self.next_token_span(),
                ));
            }
            Ok(guard)
        } else {
            Ok(0)
        }
    }

    // expr
    // expr = expr
    // expr += expr
    // expr -= expr
    // expr *= expr
    // expr /= expr
    // expr %= expr
    pub fn try_expr_statement(&mut self) -> ParseResult {
        self.scoped(|p| {
            let expr = p.try_expr()?;
            if expr == 0 {
                return Ok(0); // No expression, no statement
            }

            let assignment = match p.peek_next_token().kind {
                TokenKind::Eq => Some(NodeKind::Assign),
                TokenKind::PlusEq => Some(NodeKind::AddAssign),
                TokenKind::MinusEq => Some(NodeKind::SubAssign),
                TokenKind::StarEq => Some(NodeKind::MulAssign),
                TokenKind::SlashEq => Some(NodeKind::DivAssign),
                TokenKind::PercentEq => Some(NodeKind::RemAssign),
                _ => None,
            };
            if let Some(kind) = assignment {
                let operator = p.peek_next_token().kind;
                let op_span = p.next_token_span();
                p.eat_tokens(1);
                let init = p.try_expr()?;
                if init == 0 {
                    return Err(ParseError::invalid_syntax(
                        format!("Expected an expression after '{}'", operator.lexme()),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    )
                    .with_secondary_label(
                        op_span,
                        "assignment operator expects a value on its right",
                    ));
                }
                let assign_span = p.current_span();
                if kind == NodeKind::Assign {
                    p.check_deref_assignment_habit(expr, assign_span);
                }
                return Ok(NodeBuilder::new(kind, assign_span)
                    .add_single_child(expr)
                    .add_single_child(init)
                    .build(&mut p.ast));
            }

            if p.peek_next_token().kind == TokenKind::RBrace {
                return Ok(expr);
            }

            Ok(NodeBuilder::new(NodeKind::ExprStatement, p.current_span())
                .add_single_child(expr)
                .build(&mut p.ast))
        })
    }

    pub fn try_file_scope(&mut self) -> ParseResult {
        self.scoped(|p| {
            let nodes = p.try_multi_until(
                &[
                    Rule::comma("configuration", |p| p.try_configuration()),
                    Rule::comma("property", |p| p.try_property()),
                    Rule::semicolon("statement or definition", |p| {
                        p.try_statement_or_definition()
                    }),
                ],
                Some((TokenKind::Eof, None, rustc_span::DUMMY_SP)),
            )?;
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
