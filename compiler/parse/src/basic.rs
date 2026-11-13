use super::error::*;
use super::parser::*;
use ast::*;
use lex::TokenKind;
use rustc_span::{DUMMY_SP, Span};

impl Parser<'_> {
    // Try parse a single attribute prefix: `^ expr`.
    // Returns 0 (and consumes nothing) if the next token is not `^`.
    pub fn try_attribute_prefix(&mut self) -> ParseResult {
        self.scoped(|p| {
            if p.peek_next_token().kind != TokenKind::Caret {
                return Ok(0);
            }
            let caret_span = p.next_token_span();
            p.eat_tokens(1);
            let expression_start = p.next_token_span().lo();
            // A following named input belongs to the attributed element,
            // not to the attribute's postfix projection chain.
            let mut option = super::expressions::ExprOption::default();
            option.stop_at_named_parameter = true;
            let expr = p.try_expr_with_option(option)?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected an attribute expression after `^`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(caret_span, "attribute prefix `^` starts here"));
            }
            // Postfix parsing may assign the root a suffix-only span. An
            // attribute must retain the entire expression for deferred name
            // resolution, including its callee/path prefix.
            p.ast.spans[expr as usize] = Span::new(expression_start, p.current_span().hi());

            if let Some(conflict) =
                p.check_non_block_extended_call_conflict(expr, "a prefix `^attribute`", None)
            {
                return Err(conflict);
            }

            Ok(expr)
        })
    }

    // Try parse a chain of attribute prefixes. Stops when no further `^`.
    // Returns an empty Vec if none found (and consumes nothing).
    pub fn try_attribute_prefix_chain(&mut self) -> Result<Vec<NodeIndex>, ParseError> {
        self.scoped(|p| {
            let mut attrs = Vec::new();
            loop {
                let attr = p.try_attribute_prefix()?;
                if attr == 0 {
                    break;
                }
                attrs.push(attr);
            }
            Ok(attrs)
        })
    }

    pub(crate) fn wrap_with_attributes(
        &mut self,
        mut target: NodeIndex,
        attrs: &[NodeIndex],
    ) -> NodeIndex {
        // Writing order: first parsed attribute should be the outermost wrapper.
        for &attr_expr in attrs.iter().rev() {
            let span_a = self.ast.get_span(attr_expr).unwrap_or(DUMMY_SP);
            let span_t = self.ast.get_span(target).unwrap_or(DUMMY_SP);
            let lo = std::cmp::min(span_a.lo(), span_t.lo());
            let hi = std::cmp::max(span_a.hi(), span_t.hi());
            let span = Span::new(lo, hi);
            target = NodeBuilder::new(NodeKind::Attribute, span)
                .add_single_child(attr_expr)
                .add_single_child(target)
                .build(&mut self.ast);
        }
        target
    }

    pub fn try_atomic(&mut self) -> ParseResult {
        self.scoped(|p| {
            let next = p.peek_next_token();
            use TokenKind::*;
            let mut not_matched = false;
            let result = match next.kind {
                Underscore => {
                    Ok(NodeBuilder::new(NodeKind::Wildcard, p.next_token_span()).build(&mut p.ast))
                }
                Int | IntBin | IntOct | IntHex => {
                    Ok(NodeBuilder::new(NodeKind::Int, p.next_token_span()).build(&mut p.ast))
                }
                Real | RealSci => {
                    Ok(NodeBuilder::new(NodeKind::Real, p.next_token_span()).build(&mut p.ast))
                }
                Str => return p.try_string_literal(),
                Char => Ok(NodeBuilder::new(NodeKind::Char, p.next_token_span()).build(&mut p.ast)),
                Id => {
                    let sym = p.next_token_symbol();
                    let (hi, lo) = sym.to_raw_parts();
                    Ok(NodeBuilder::new(NodeKind::Id, p.next_token_span())
                        .add_single_child(hi)
                        .add_single_child(lo)
                        .build(&mut p.ast))
                }
                False => {
                    Ok(NodeBuilder::new(NodeKind::Bool, p.next_token_span()).build(&mut p.ast))
                }
                True => Ok(NodeBuilder::new(NodeKind::Bool, p.next_token_span()).build(&mut p.ast)),

                SelfCap => {
                    Ok(NodeBuilder::new(NodeKind::SelfCap, p.next_token_span()).build(&mut p.ast))
                }
                SelfLower => Ok(
                    NodeBuilder::new(NodeKind::SelfLower, p.next_token_span()).build(&mut p.ast)
                ),
                Null => Ok(NodeBuilder::new(NodeKind::Null, p.next_token_span()).build(&mut p.ast)),
                Undefined => Ok(
                    NodeBuilder::new(NodeKind::Undefined, p.next_token_span()).build(&mut p.ast)
                ),

                _ => {
                    not_matched = true;
                    Ok(0)
                }
            };
            if !not_matched {
                p.eat_tokens(1);
            }
            result
        })
    }

    /// Keep individual token spans for decoding; comments never become text.
    fn try_string_literal(&mut self) -> ParseResult {
        let first_span = self.next_token_span();
        let mut last_span = first_span;
        let mut last_end = self.peek_next_token().to;
        let mut leaves = Vec::new();
        loop {
            let token = self.peek_next_token();
            if token.kind != TokenKind::Str {
                break;
            }
            if !leaves.is_empty() {
                let gap = lex::Token::new(TokenKind::Str, last_end, token.from);
                if !self.token_text(&gap).chars().all(char::is_whitespace) {
                    break;
                }
            }
            last_span = self.next_token_span();
            leaves.push(NodeBuilder::new(NodeKind::Str, last_span).build(&mut self.ast));
            last_end = token.to;
            self.eat_tokens(1);
        }
        if leaves.len() == 1 {
            Ok(leaves[0])
        } else {
            Ok(NodeBuilder::new(
                NodeKind::StringConcat,
                Span::new(first_span.lo(), last_span.hi()),
            )
            .add_multiple_children(leaves)
            .build(&mut self.ast))
        }
    }

    pub fn try_id(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Id.as_ref(), |p| {
            let sym = p.next_token_symbol();
            let (hi, lo) = sym.to_raw_parts();
            let result = Ok(NodeBuilder::new(NodeKind::Id, p.next_token_span())
                .add_single_child(hi)
                .add_single_child(lo)
                .build(&mut p.ast));
            p.eat_tokens(1);
            result
        })
    }

    /// Reserved spellings accepted only as qualified member components.
    pub(crate) fn try_member_id(&mut self) -> ParseResult {
        if !self.peek(TokenKind::Default.as_ref()) {
            return self.try_id();
        }
        let (hi, lo) = lex::Symbol::intern("default").to_raw_parts();
        let id = NodeBuilder::new(NodeKind::Id, self.next_token_span())
            .add_single_child(hi)
            .add_single_child(lo)
            .build(&mut self.ast);
        self.eat_tokens(1);
        Ok(id)
    }

    // symbol -> . id
    pub fn try_symbol(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Dot, TokenKind::Id], |p| {
            let dot_span = p.next_token_span();
            p.eat_tokens(1);
            let id = p.try_id()?;

            let id_span = p.ast.get_span(id).unwrap_or(DUMMY_SP);
            let symbol_span = Span::new(dot_span.lo(), id_span.hi());

            Ok(NodeBuilder::new(NodeKind::Symbol, symbol_span)
                .add_single_child(id)
                .build(&mut p.ast))
        })
    }

    pub fn try_multi(&mut self, rules: &[Rule]) -> Result<Vec<NodeIndex>, ParseError> {
        self.try_multi_until(rules, None)
    }

    /// Parse repeated `rules` up to an optional known closing boundary
    /// `(close_kind, open_kind_opt, open_span)`.
    pub(crate) fn try_multi_until(
        &mut self,
        rules: &[Rule],
        closing: Option<(TokenKind, Option<TokenKind>, Span)>,
    ) -> Result<Vec<NodeIndex>, ParseError> {
        let has_semi_rule = rules.iter().any(|r| r.ends_with_semicolon());
        let has_comma_rule = rules.iter().any(|r| !r.ends_with_semicolon());
        let has_param_rule = rules.iter().any(|r| r.name.contains("parameter"));

        self.scoped(|p| {
            let mut nodes = Vec::new();
            'outer: loop {
                let iter_start_cursor = p.cursor;
                if let Some((close_kind, _, _)) = closing {
                    let next_kind = p.peek_next_token().kind;
                    if next_kind == close_kind || p.is_eof() {
                        break 'outer;
                    }
                    // Unmatched enclosing `}` inside `(...)` or `[...]`: stop here so
                    // `try_multi_with_bracket` reports the unclosed delimiter and leaves `}`
                    // for the enclosing block!
                    if close_kind != TokenKind::RBrace && next_kind == TokenKind::RBrace {
                        break 'outer;
                    }
                }

                // Check statement-start heuristics in statement/item containers
                if closing.is_some()
                    && has_semi_rule
                    && let Some(err) = p.check_statement_start_heuristics()
                {
                    if p.recovery_enabled {
                        let sync_close = closing.and_then(|(ck, _, _)| {
                            (ck != TokenKind::Eof).then_some(ck)
                        });
                        p.record_error(err);
                        p.synchronize_in_block_or_file(sync_close);
                        continue 'outer;
                    }
                    return Err(err);
                }

                // Attempt attribute chain at start of each element parse round.
                let attrs = match p.try_attribute_prefix_chain() {
                    Ok(attrs) => attrs,
                    Err(err) => {
                        if closing.is_some() && p.recovery_enabled {
                            p.record_error(err);
                            Self::recover_in_multi(p, has_semi_rule, closing);
                            continue 'outer;
                        }
                        return Err(err);
                    }
                };

                'inner: for rule in rules {
                    let node: NodeIndex = match (rule.parser)(p) {
                        Ok(0) => continue 'inner,
                        Ok(node) => node,
                        Err(err) => {
                            if closing.is_some() && p.recovery_enabled {
                                p.record_error(err);
                                Self::recover_in_multi(p, has_semi_rule, closing);
                                continue 'outer;
                            }
                            return Err(err);
                        }
                    };

                    let final_node = if !attrs.is_empty() {
                        p.wrap_with_attributes(node, &attrs)
                    } else {
                        node
                    };
                    nodes.push(final_node);

                    if rule.ends_with_semicolon() {
                        if p.eat_token(rule.separator) || p.node_ends_with_right_brace() {
                            continue 'outer;
                        }
                        let Some((close_kind, open_kind, open_span)) = closing else {
                            break 'outer;
                        };
                        let next_tok = p.peek_next_token();
                        if next_tok.kind == close_kind || p.is_eof() {
                            break 'outer;
                        }
                        if let Some(comment_err) = p.check_c_style_comment_tokens() {
                            if p.recovery_enabled {
                                p.record_error(comment_err);
                                Self::recover_in_multi(p, true, closing);
                                continue 'outer;
                            }
                            return Err(comment_err);
                        }
                        // Comma written after statement where `;` is required
                        if next_tok.kind == TokenKind::Comma && !has_comma_rule {
                            let err = ParseError::custom(
                                PARSE_SEPARATOR,
                                "comma_instead_of_semicolon",
                                "statements in a block are separated by `;` rather than `,`",
                                p.next_token_span(),
                            )
                            .with_found(TokenKind::Comma)
                            .with_primary_label("replace `,` with `;`")
                            .with_help("Use `;` to end statements inside `{ ... }`.");
                            if p.recovery_enabled {
                                p.eat_tokens(1);
                                p.record_error(err);
                                continue 'outer;
                            }
                            return Err(err);
                        }
                        // Missing `;` before the next statement/item!
                        if Self::is_item_or_stmt_sync_keyword(next_tok.kind)
                            || !p.tokens_on_same_line(p.current_token(), next_tok)
                        {
                            let err = ParseError::custom(
                                PARSE_SEPARATOR,
                                "missing_semicolon",
                                "missing `;` at the end of this statement",
                                p.prev_token_span(),
                            )
                            .with_found(next_tok.kind)
                            .with_primary_label("expected `;` right after this")
                            .with_secondary_label(
                                p.next_token_span(),
                                format!("before {} here", describe_token(next_tok.kind)),
                            )
                            .with_help(
                                "End statements and declarations with `;` unless they finish with `}`.",
                            );
                            if p.recovery_enabled {
                                p.record_error(err);
                                continue 'outer;
                            }
                            return Err(err);
                        }
                        // Same-line trailing token after a completed statement
                        let mut err = ParseError::custom(
                            PARSE_SEPARATOR,
                            "unexpected_after_statement",
                            format!(
                                "expected `;` after statement, but ran into {}",
                                describe_token(next_tok.kind)
                            ),
                            p.next_token_span(),
                        )
                        .with_found(next_tok.kind)
                        .with_primary_label(format!(
                            "expected `;`, found {}",
                            describe_token(next_tok.kind)
                        ));
                        if let Some(ok) = open_kind {
                            err = err.with_secondary_label(
                                open_span,
                                format!("inside `{}` opened here", ok.lexme().trim()),
                            );
                        }
                        if p.recovery_enabled {
                            p.record_error(err);
                            Self::recover_in_multi(p, true, closing);
                            continue 'outer;
                        }
                        return Err(err);
                    } else {
                        if p.eat_token(rule.separator) {
                            continue 'outer;
                        }
                        let Some((close_kind, open_kind, open_span)) = closing else {
                            break 'outer;
                        };
                        let next_tok = p.peek_next_token();
                        if next_tok.kind == close_kind || p.is_eof() {
                            break 'outer;
                        }
                        if let Some(comment_err) = p.check_c_style_comment_tokens() {
                            if p.recovery_enabled {
                                p.record_error(comment_err);
                                Self::recover_in_multi(p, has_semi_rule, closing);
                                continue 'outer;
                            }
                            return Err(comment_err);
                        }
                        // `;` written instead of `,` after a comma-separated rule (such as
                        // `struct` fields `x: i32; y: i32` or `fn` params `a: i32; b: i32`).
                        // When inside `(...)` or `[...]` and `;` is immediately followed by `}`
                        // or EOF (e.g. `let xs = [1, 2, 3;\n}`), `;` is the outer statement
                        // terminator after an unclosed bracket, so let `try_multi_with_bracket`
                        // report the unclosed bracket instead!
                        if next_tok.kind == TokenKind::Semi
                            && (close_kind == TokenKind::RBrace
                                || !matches!(
                                    p.peek_nth_token(2).kind,
                                    TokenKind::RBrace | TokenKind::Eof
                                ))
                        {
                            let err = ParseError::custom(
                                PARSE_SEPARATOR,
                                "semicolon_instead_of_comma",
                                format!(
                                    "{}s here are separated by `,` rather than `;`",
                                    rule.name
                                ),
                                p.next_token_span(),
                            )
                            .with_found(TokenKind::Semi)
                            .with_primary_label("replace `;` with `,`")
                            .with_help(format!("Separate each {} with `,`.", rule.name));
                            if p.recovery_enabled {
                                p.eat_tokens(1);
                                p.record_error(err);
                                continue 'outer;
                            }
                            return Err(err);
                        }
                        // `.name = value` written inside `{ ... }` instead of `name: value` or `.name value`!
                        if close_kind == TokenKind::RBrace
                            && next_tok.kind == TokenKind::Eq
                            && p.ast.get_node_kind(node) == Some(NodeKind::Symbol)
                        {
                            let sym_snip = p.snippet_or(node, ".name");
                            let bare = sym_snip.strip_prefix('.').unwrap_or("name");
                            let val_snip = p
                                .token_content(&p.peek_nth_token(2))
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| "value".to_string());
                            let sym_span = p.ast.get_span(node).unwrap_or_else(|| p.next_token_span());
                            let err = ParseError::custom(
                                PARSE_SEPARATOR,
                                "named_arg_inside_braces",
                                format!("unexpected `=` after `{sym_snip}` inside braces"),
                                p.next_token_span(),
                            )
                            .with_found(TokenKind::Eq)
                            .with_primary_label("`.name = value` is for `(...)` call arguments")
                            .with_secondary_label(sym_span, format!("`{sym_snip}` starts here"))
                            .with_help(format!(
                                "For an object or extended-call property, write `{bare}: {val_snip}`; for a configuration entry, omit `=` and write `{sym_snip} {val_snip}`."
                            ));
                            if p.recovery_enabled {
                                p.record_error(err);
                                p.synchronize_in_comma_list(close_kind);
                                continue 'outer;
                            }
                            return Err(err);
                        }
                        if close_kind != TokenKind::RBrace || !has_semi_rule {
                            if close_kind != TokenKind::RBrace
                                && next_tok.kind == TokenKind::Semi
                                && matches!(
                                    p.peek_nth_token(2).kind,
                                    TokenKind::RBrace | TokenKind::Eof
                                )
                            {
                                break 'outer;
                            }
                            let close_str = close_kind.lexme().trim();
                            let mut err = ParseError::custom(
                                PARSE_SEPARATOR,
                                "missing_comma",
                                format!(
                                    "expected `,` or `{close_str}` after {}, but ran into {}",
                                    rule.name,
                                    describe_token(next_tok.kind)
                                ),
                                p.next_token_span(),
                            )
                            .with_found(next_tok.kind)
                            .with_primary_label(format!("expected `,` or `{close_str}` here"));
                            if let Some(ok) = open_kind {
                                err = err.with_secondary_label(
                                    open_span,
                                    format!("this `{}` was opened here", ok.lexme().trim()),
                                );
                            }
                            if p.recovery_enabled {
                                p.record_error(err);
                                p.synchronize_in_comma_list(close_kind);
                                if p.cursor == iter_start_cursor {
                                    break 'outer;
                                }
                                continue 'outer;
                            }
                            return Err(err);
                        }
                        break 'outer;
                    }
                }

                if !attrs.is_empty() {
                    let err = ParseError::invalid_syntax(
                        "expected a declaration, statement, or item after attribute `^...`"
                            .to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    );
                    if closing.is_some() && p.recovery_enabled {
                        p.record_error(err);
                        Self::recover_in_multi(p, has_semi_rule, closing);
                        continue 'outer;
                    }
                    return Err(err);
                }

                let Some((close_kind, open_kind, open_span)) = closing else {
                    break 'outer;
                };
                if p.peek_next_token().kind == close_kind || p.is_eof() {
                    break 'outer;
                }

                if has_param_rule
                    && let Some(param_err) = p.check_param_heuristics()
                {
                    if p.recovery_enabled {
                        p.record_error(param_err);
                        p.synchronize_in_comma_list(close_kind);
                        continue 'outer;
                    }
                    return Err(param_err);
                }

                if let Some(comment_err) = p.check_c_style_comment_tokens() {
                    if p.recovery_enabled {
                        p.record_error(comment_err);
                        Self::recover_in_multi(p, has_semi_rule, closing);
                        continue 'outer;
                    }
                    return Err(comment_err);
                }

                if p.recovery_enabled {
                    let expected = rules
                        .iter()
                        .map(|rule| rule.name)
                        .collect::<Vec<_>>()
                        .join(", ");
                    let found = p.peek_next_token().kind;
                    let mut err = if close_kind == TokenKind::Eof {
                        ParseError::invalid_syntax(
                            format!(
                                "expected {expected}, but ran into {}",
                                describe_token(found)
                            ),
                            found,
                            p.next_token_span(),
                        )
                    } else {
                        let close_str = close_kind.lexme().trim();
                        ParseError::invalid_syntax(
                            format!(
                                "expected {expected} or `{close_str}`, but ran into {}",
                                describe_token(found)
                            ),
                            found,
                            p.next_token_span(),
                        )
                    };
                    if let Some(ok) = open_kind {
                        err = err.with_secondary_label(
                            open_span,
                            format!("inside `{}` opened here", ok.lexme().trim()),
                        );
                    }
                    p.record_error(err);
                    Self::recover_in_multi(p, has_semi_rule, closing);
                    if p.cursor == iter_start_cursor {
                        if close_kind != TokenKind::RBrace
                            && p.peek_next_token().kind == TokenKind::RBrace
                        {
                            break 'outer;
                        }
                        p.eat_tokens(1);
                    }
                    continue 'outer;
                }

                break 'outer;
            }

            Ok(nodes)
        })
    }

    fn recover_in_multi(
        p: &mut Parser<'_>,
        has_semi_rule: bool,
        closing: Option<(TokenKind, Option<TokenKind>, Span)>,
    ) {
        if let Some((close_kind, _, _)) = closing {
            if has_semi_rule {
                let sync_close = (close_kind != TokenKind::Eof).then_some(close_kind);
                p.synchronize_in_block_or_file(sync_close);
            } else {
                p.synchronize_in_comma_list(close_kind);
            }
        }
    }

    fn node_ends_with_right_brace(&self) -> bool {
        self.current_token().kind == TokenKind::RBrace
    }

    pub fn try_multi_with_bracket(
        &mut self,
        rules: &[Rule],
        bracket: (TokenKind, TokenKind),
    ) -> Result<Vec<NodeIndex>, ParseError> {
        self.scoped_with_expected_prefix(&[bracket.0], |p| {
            let open_span = p.next_token_span();
            p.eat_tokens(1);

            let is_angle = bracket == (TokenKind::Lt, TokenKind::Gt);
            let is_pipe = bracket == (TokenKind::Pipe, TokenKind::Pipe);
            let saved_recovery = p.recovery_enabled;
            if is_angle {
                p.angle_bracket_depth += 1;
            }
            if is_angle || is_pipe {
                p.recovery_enabled = false;
            }
            let multi_res =
                p.try_multi_until(rules, Some((bracket.1, Some(bracket.0), open_span)));
            if is_angle {
                p.angle_bracket_depth = p.angle_bracket_depth.saturating_sub(1);
            }
            p.recovery_enabled = saved_recovery;

            let nodes = multi_res?;

            if !p.eat_token(bracket.1) {
                let expected = rules
                    .iter()
                    .map(|rule| rule.name)
                    .collect::<Vec<_>>()
                    .join(", ");
                let found = p.peek_next_token().kind;
                let open_str = bracket.0.lexme().trim();
                let close_str = bracket.1.lexme().trim();
                let headline = if found == TokenKind::Eof {
                    format!(
                        "reached the end of the file while looking for `{close_str}` to close `{open_str}`"
                    )
                } else {
                    format!(
                        "expected {expected} or closing `{close_str}`, but ran into {}",
                        describe_token(found)
                    )
                };
                return Err(
                    ParseError::custom(
                        PARSE_UNCLOSED_DELIMITER,
                        "unclosed_delimiter",
                        headline,
                        p.next_token_span(),
                    )
                    .with_found(found)
                    .with_primary_label(format!(
                        "expected `{close_str}`, found {}",
                        describe_token(found)
                    ))
                    .with_secondary_label(
                        open_span,
                        format!("this `{open_str}` was opened here"),
                    ),
                );
            }
            Ok(nodes)
        })
    }

    pub fn try_unary(
        &mut self,
        rule: Rule,
        prefix: TokenKind,
        result_kind: NodeKind,
        info: String,
    ) -> ParseResult {
        self.scoped_with_expected_prefix(&[prefix], |p| {
            let prefix_span = p.next_token_span();
            p.eat_tokens(1);
            let node = (rule.parser)(p)?;
            if node == 0 {
                return Err(ParseError::invalid_syntax(
                    info,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(
                    prefix_span,
                    format!("`{}` starts here", prefix.lexme().trim()),
                ));
            }
            Ok(NodeBuilder::new(result_kind, p.current_span())
                .add_single_child(node)
                .build(&mut p.ast))
        })
    }

    /// Configuration belongs to the enclosing definition, not the next member.
    pub fn try_configuration(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Dot, TokenKind::Id], |p| {
            let dot_span = p.next_token_span();
            p.eat_tokens(1);
            let id = p.try_id()?;
            let name = p.snippet_or(id, "name");

            // Heuristic: `.name = value` written inside `{ ... }` instead of `name: value` or `.name value`!
            if p.peek(TokenKind::Eq.as_ref()) {
                let eq_span = p.next_token_span();
                return Err(
                    ParseError::custom(
                        PARSE_SEPARATOR,
                        "named_arg_inside_braces",
                        format!("unexpected `=` after `.{name}` inside braces"),
                        eq_span,
                    )
                    .with_found(TokenKind::Eq)
                    .with_primary_label("`.name = value` is for `(...)` call arguments")
                    .with_secondary_label(dot_span, format!("`.{name}` starts here"))
                    .with_help(format!(
                        "For an object or extended-call property, write `{name}: value`; for a configuration entry, omit `=` and write `.{name} value`."
                    )),
                );
            }

            let value_start = p.next_token_span().lo();
            let value = p.try_expr()?;
            if value == 0 {
                return Err(
                    ParseError::invalid_syntax(
                        format!("expected a value expression after configuration `.{name}`"),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    )
                    .with_secondary_label(dot_span, format!("configuration `.{name}` starts here")),
                );
            }
            p.ast.spans[value as usize] = Span::new(value_start, p.current_span().hi());
            Ok(NodeBuilder::new(NodeKind::Configuration, p.current_span())
                .add_single_child(id)
                .add_single_child(value)
                .build(&mut p.ast))
        })
    }

    /// property -> id: expr
    pub fn try_property(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Id, TokenKind::Colon], |p| {
            let id = p.try_id()?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a property name before `:`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            let colon_span = p.next_token_span();
            p.eat_tokens(1);
            let expr = p.try_expr()?;
            if expr == 0 {
                let prop_name = p.snippet_or(id, "field");
                return Err(ParseError::invalid_syntax(
                    format!("expected a value expression after `{prop_name}:`"),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(colon_span, "property value expected after this `:`"));
            }

            Ok(NodeBuilder::new(NodeKind::Property, p.current_span())
                .add_single_child(id)
                .add_single_child(expr)
                .build(&mut p.ast))
        })
    }

    /// extend_arg -> ... expr
    pub fn try_extend_arg(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Dot, TokenKind::Dot, TokenKind::Dot], |p| {
            let dots_lo = p.next_token_span().lo();
            p.eat_tokens(3);
            let dots_span = Span::new(dots_lo, p.prev_token_span().hi());
            let expr = p.try_expr()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected an expression to expand after `...`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(dots_span, "spread `...` requires an operand"));
            }
            Ok(NodeBuilder::new(NodeKind::ExpandArg, p.current_span())
                .add_single_child(expr)
                .build(&mut p.ast))
        })
    }

    /// optional_arg -> .id = expr
    pub fn try_optional_arg(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Dot, TokenKind::Id, TokenKind::Eq], |p| {
            p.eat_tokens(1);
            let id = p.try_id()?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected an argument name after `.`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            let eq_span = p.next_token_span();
            if !p.eat_token(TokenKind::Eq) {
                return Err(ParseError::unexpected_token(
                    TokenKind::Eq,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            let expr = p.try_expr()?;
            if expr == 0 {
                let arg_name = p.snippet_or(id, "arg");
                return Err(ParseError::invalid_syntax(
                    format!("expected an expression after `.{arg_name} =`"),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(eq_span, "value expected after this `=`"));
            }

            Ok(NodeBuilder::new(NodeKind::OptionalArg, p.current_span())
                .add_single_child(id)
                .add_single_child(expr)
                .build(&mut p.ast))
        })
    }
}

pub struct Rule {
    pub name: &'static str,
    pub parser: Box<dyn Fn(&mut Parser) -> ParseResult>,
    pub degree: (),
    pub separator: TokenKind,
}

impl Rule {
    pub fn comma(
        name: &'static str,
        parser: impl Fn(&mut Parser) -> ParseResult + 'static,
    ) -> Self {
        Self {
            name,
            parser: Box::new(parser),
            degree: (),
            separator: TokenKind::Comma,
        }
    }

    pub fn semicolon(
        name: &'static str,
        parser: impl Fn(&mut Parser) -> ParseResult + 'static,
    ) -> Self {
        Self {
            name,
            parser: Box::new(parser),
            degree: (),
            separator: TokenKind::Semi,
        }
    }

    #[inline]
    pub fn ends_with_semicolon(&self) -> bool {
        self.separator == TokenKind::Semi
    }
}

#[repr(u32)]
pub enum BuiltinAttribute {
    KWPrivate,
    KWComptime,
    KWPure,
    KWSpec,
    KWHandles,
    KWAsync,
    KWUnsafe,
    KWGhost,
}

impl BuiltinAttribute {
    pub fn as_str(&self) -> &'static str {
        match self {
            BuiltinAttribute::KWPrivate => "flurry_kw_private",
            BuiltinAttribute::KWComptime => "flurry_kw_comptime",
            BuiltinAttribute::KWPure => "flurry_kw_pure",
            BuiltinAttribute::KWSpec => "flurry_kw_spec",
            BuiltinAttribute::KWHandles => "flurry_kw_handles",
            BuiltinAttribute::KWAsync => "flurry_kw_async",
            BuiltinAttribute::KWGhost => "flurry_kw_ghost",
            BuiltinAttribute::KWUnsafe => "flurry_kw_unsafe",
        }
    }
}
