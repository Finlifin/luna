use crate::lex::TokenKind;
use crate::parse::ast::*;
use crate::parse::error::*;
use crate::parse::parser::*;

impl Parser<'_> {
    pub fn try_atomic(&mut self) -> ParseResult {
        self.scoped(|p| {
            let next = p.peek_next_token();
            use TokenKind::*;
            let mut not_matched = false;
            let result = match next.kind {
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
                let mut matched_any_rule = false;
                'inner: for rule in rules {
                    let node: NodeIndex = match (rule.parser)(p) {
                        Ok(0) => continue 'inner,
                        Ok(node) => node,
                        Err(err) => return Err(err),
                    };

                    matched_any_rule = true;
                    nodes.push(node);
                    if rule.ends_with_semicolon() {
                        // TODO: 如果规则以分号结尾, 有些特殊规则需要处理
                        if !p.eat_token(rule.separator) {
                            if p.node_ends_with_right_brace(node) {
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
                    // 没有匹配到任何规则, 退出循环
                    break;
                }
            }

            Ok(nodes)
        })
    }

    // TODO: 非常低效
    fn node_ends_with_right_brace(&self, node: NodeIndex) -> bool {
        if let Some(span) = self.ast.get_span(node) {
            let source_file = self.source_map.lookup_source_file(span.lo());

            let source_content = match &source_file.src {
                Some(content) => content.as_str(),
                None => {
                    eprintln!("Error: Source file content not available");
                    return false;
                }
            };
            // Check if the last character of the node's span is a right brace
            if let Some(last_char) =
                source_content.get(span.hi().0 as usize - 1..span.hi().0 as usize)
            {
                return last_char == "}";
            } else {
                eprintln!("Error: Node span is out of bounds in source content");
            }
        }
        false
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
                return Err(ParseError::unexpected_token(
                    bracket.1,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
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
                    "Expected an expression after `.`".to_string(),
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

    pub fn try_property_assign(&mut self) -> ParseResult {
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

            Ok(
                NodeBuilder::new(NodeKind::PropertyAssignment, p.current_span())
                    .add_single_child(id)
                    .add_single_child(expr)
                    .build(&mut p.ast),
            )
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
