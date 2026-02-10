use super::error::*;
use super::parser::*;
use ast::*;
use lex::TokenKind;

impl Parser<'_> {
    // Try parse a single attribute prefix: `^ expr`.
    // Returns 0 (and consumes nothing) if the next token is not `^`.
    pub fn try_attribute_prefix(&mut self) -> ParseResult {
        self.scoped(|p| {
            if p.peek_next_token().kind != TokenKind::Caret {
                return Ok(0);
            }
            // consume '^'
            p.eat_tokens(1);
            let expr = p.try_expr()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected attribute expression after `^`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
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

    // TODO: GPT写的, 拉胯, 没有用scoped
    fn wrap_with_attributes(&mut self, mut target: NodeIndex, attrs: &[NodeIndex]) -> NodeIndex {
        // Writing order: first parsed attribute should be the outermost wrapper.
        // attrs collected in textual order; we fold from last to first to keep that property.
        for &attr_expr in attrs.iter().rev() {
            let span_a = self.ast.get_span(attr_expr).unwrap_or(rustc_span::DUMMY_SP);
            let span_t = self.ast.get_span(target).unwrap_or(rustc_span::DUMMY_SP);
            let lo = std::cmp::min(span_a.lo(), span_t.lo());
            let hi = std::cmp::max(span_a.hi(), span_t.hi());
            let span = rustc_span::Span::new(lo, hi);
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
                Int => Ok(NodeBuilder::new(NodeKind::Int, p.next_token_span()).build(&mut p.ast)),
                Real => Ok(NodeBuilder::new(NodeKind::Real, p.next_token_span()).build(&mut p.ast)),
                Str => Ok(NodeBuilder::new(NodeKind::Str, p.next_token_span()).build(&mut p.ast)),
                Char => Ok(NodeBuilder::new(NodeKind::Char, p.next_token_span()).build(&mut p.ast)),
                Id => Ok(NodeBuilder::new(NodeKind::Id, p.next_token_span()).build(&mut p.ast)),
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

    pub fn try_id(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Id.as_ref(), |p| {
            let result = Ok(NodeBuilder::new(NodeKind::Id, p.next_token_span()).build(&mut p.ast));
            p.eat_tokens(1);
            result
        })
    }

    // symbol -> . id
    pub fn try_symbol(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Dot, TokenKind::Id], |p| {
            let dot_span = p.next_token_span(); // 获取点号的span
            p.eat_tokens(1);
            let id = p.try_id()?;

            // 计算从点号开始到id结束的span
            let id_span = p.ast.get_span(id).unwrap_or(rustc_span::DUMMY_SP);
            let symbol_span = rustc_span::Span::new(dot_span.lo(), id_span.hi());

            Ok(NodeBuilder::new(NodeKind::Symbol, symbol_span)
                .add_single_child(id)
                .build(&mut p.ast))
        })
    }

    // rules是有序的, 从最左边的规则开始尝试, 选中第一个成功的
    pub fn try_multi(&mut self, rules: &[Rule]) -> Result<Vec<NodeIndex>, ParseError> {
        self.scoped(|p| {
            let mut nodes = Vec::new();
            'outer: loop {
                // New: attempt attribute chain at start of each element parse round.
                let attrs = p.try_attribute_prefix_chain()?; // may be empty, consumption rolled back automatically
                let mut matched_any_rule = false;
                'inner: for rule in rules {
                    let node: NodeIndex = match (rule.parser)(p) {
                        Ok(0) => continue 'inner,
                        Ok(node) => node,
                        Err(err) => return Err(err),
                    };

                    matched_any_rule = true;
                    let final_node = if !attrs.is_empty() {
                        p.wrap_with_attributes(node, &attrs)
                    } else {
                        node
                    };
                    nodes.push(final_node);
                    if rule.ends_with_semicolon() {
                        // TODO: 如果规则以分号结尾, 有些特殊规则需要处理
                        if !p.eat_token(rule.separator) {
                            if p.node_ends_with_right_brace() {
                                continue 'outer;
                            } else {
                                break 'outer;
                            }
                        } else {
                            continue 'outer; // 如果匹配到分隔符, 继续外层循环
                        }
                    } else {
                        if !p.eat_token(rule.separator) {
                            break 'outer; // 如果没有匹配到分隔符, 退出外层循环
                        } else {
                            continue 'outer; // 如果匹配到分隔符, 继续外层循环
                        }
                    }
                }

                if !matched_any_rule {
                    // If we had attributes but no rule matched, this is an error (attributes consumed tokens)
                    // Detect by re-parsing single '^' lookahead: simpler: attrs empty means safe break.
                    // Here we only know attrs from this round are empty (rolled back). To catch the case where
                    // user wrote '^' then nothing legal, the attribute prefix parser would have errored already.
                    // 没有匹配到任何规则, 退出循环
                    break;
                }
            }

            Ok(nodes)
        })
    }

    // 检查上一个 token 是否为右大括号
    fn node_ends_with_right_brace(&self) -> bool {
        self.current_token().kind == TokenKind::RBrace
    }

    pub fn try_multi_with_bracket(
        &mut self,
        rules: &[Rule],
        bracket: (TokenKind, TokenKind),
    ) -> Result<Vec<NodeIndex>, ParseError> {
        self.scoped_with_expected_prefix(&[bracket.0], |p| {
            p.eat_tokens(1); // 吃掉左括号
            let nodes = match p.try_multi(rules) {
                Ok(nodes) => nodes,
                Err(err) => return Err(err),
            };
            if !p.eat_token(bracket.1) {
                let expected = rules
                    .iter()
                    .map(|rule| rule.name)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(ParseError::InvalidSyntax {
                    message: format!("Expected {} or `{}`", expected, bracket.1.lexme()),
                    found: p.next_token().kind,
                    span: p.current_span(),
                });
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
            p.eat_tokens(1); // 吃掉前缀标记
            let node = (rule.parser)(p)?;
            if node == 0 {
                return Err(ParseError::invalid_syntax(
                    info,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            Ok(NodeBuilder::new(result_kind, p.current_span())
                .add_single_child(node)
                .build(&mut p.ast))
        })
    }

    /// property -> .id expr
    pub fn try_property(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Dot, TokenKind::Id], |p| {
            p.eat_tokens(1); // 吃掉点号
            let id = p.try_id()?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an identifier after `.`".to_string(),
                    TokenKind::Id,
                    p.current_span(),
                ));
            }
            let expr = p.try_expr()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an expression after property name in `.id expr`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
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
            p.eat_tokens(3); // consume '...'
            let expr = p.try_expr()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an expression after `...`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
            }
            Ok(NodeBuilder::new(NodeKind::ExtendArg, p.current_span())
                .add_single_child(expr)
                .build(&mut p.ast))
        })
    }

    /// optional_arg -> .id = expr
    pub fn try_optional_arg(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Dot, TokenKind::Id, TokenKind::Eq], |p| {
            p.eat_tokens(1); // 吃掉点号
            let id = p.try_id()?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an identifier after `.`".to_string(),
                    TokenKind::Id,
                    p.current_span(),
                ));
            }
            if !p.eat_token(TokenKind::Eq) {
                return Err(ParseError::unexpected_token(
                    TokenKind::Eq,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            let expr = p.try_expr()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an expression after `=`".to_string(),
                    p.peek_next_token().kind,
                    p.current_span(),
                ));
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

    // 分隔符, `,` 或 `;`
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
