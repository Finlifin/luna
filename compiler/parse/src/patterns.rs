use super::basic::Rule;
use super::error::*;
use super::operators::get_pattern_op_info;
use super::parser::*;
use ast::*;
use lex::TokenKind;

/// 模式选项，包含是否记录调用和优先级
#[derive(Debug, Clone, Copy)]
pub struct PatternOption {
    pub no_extended_call: bool,
    pub precedence: i32,
}

impl PatternOption {
    pub fn new() -> Self {
        Self {
            no_extended_call: false,
            precedence: 0,
        }
    }

    pub fn with_no_extended_call(mut self, no_extended_call: bool) -> Self {
        self.no_extended_call = no_extended_call;
        self
    }

    pub fn with_precedence(mut self, precedence: i32) -> Self {
        self.precedence = precedence;
        self
    }
}

impl Default for PatternOption {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser<'_> {
    #[inline]
    pub fn try_pattern(&mut self) -> ParseResult {
        self.try_pattern_with_option(PatternOption::default())
    }

    pub fn try_pattern_with_option(&mut self, option: PatternOption) -> ParseResult {
        self.scoped(|p| p.try_pattern_pratt(0, option))
    }

    /// Parse a pattern while disabling `extended_application_pattern`.
    pub fn try_pattern_without_extended_call(&mut self) -> ParseResult {
        self.scoped(|p| p.try_pattern_pratt(0, PatternOption::new().with_no_extended_call(true)))
    }

    pub fn try_pattern_pratt(&mut self, min_prec: i32, option: PatternOption) -> ParseResult {
        self.scoped(|p| {
            // 尝试解析前缀模式作为左操作数
            let mut current_left = match p.try_prefix_pattern(option)? {
                0 => return Ok(0),
                node => node,
            };

            // 循环处理操作符和右操作数
            loop {
                let token = p.peek_next_token();
                let op_info = get_pattern_op_info(token.kind);

                // 如果操作符无效或优先级太低，则退出循环
                if op_info.node_kind == NodeKind::Invalid || op_info.prec < min_prec {
                    break;
                }

                match p.try_postfix_pattern(token.kind, current_left, op_info.prec + 1, option) {
                    Ok(node) if node != 0 => {
                        current_left = node;
                    }
                    Err(ParseError::MeetPostExtendedCallStart) => {
                        // 如果遇到 MeetPostExtendedCallStart，跳过后缀模式处理
                        break;
                    }
                    Err(e) => return Err(e),

                    _ => {
                        // 消耗操作符标记
                        p.eat_tokens(1);

                        // 解析右操作数（递归调用，使用更高优先级）
                        let right = match p.try_pattern_pratt(op_info.prec + 1, option)? {
                            0 => {
                                return Err(ParseError::invalid_syntax(
                                    format!(
                                        "Expected a right operand after binary operator `{}`",
                                        token.kind.lexme()
                                    ),
                                    token.kind,
                                    p.next_token_span(),
                                ));
                            }

                            node => node,
                        };

                        // 创建二元操作符节点
                        current_left = NodeBuilder::new(op_info.node_kind, p.current_span())
                            .add_single_child(current_left)
                            .add_single_child(right)
                            .build(&mut p.ast);
                    }
                }
            }

            // 返回最终构建的模式
            Ok(current_left)
        })
    }

    pub fn try_prefix_pattern(&mut self, option: PatternOption) -> ParseResult {
        self.scoped(|p| {
            // Check for bit vector patterns (0b..., 0o..., 0x...)
            // TODO: you need to check two tokens instead of one
            if p.peek_next_token().kind == TokenKind::Int {
                let token = p.peek_next_token();
                let text = p.token_text(&token);
                if text.starts_with("0b") {
                    return p.try_bit_vec_pattern(NodeKind::BitVecBinPattern);
                } else if text.starts_with("0o") {
                    return p.try_bit_vec_pattern(NodeKind::BitVecOctPattern);
                } else if text.starts_with("0x") || text.starts_with("0X") {
                    return p.try_bit_vec_pattern(NodeKind::BitVecHexPattern);
                }
            }

            let token = p.peek_next_token();
            match token.kind {
                // 原子模式
                TokenKind::Int
                | TokenKind::Real
                | TokenKind::Str
                | TokenKind::Char
                | TokenKind::Id
                | TokenKind::False
                | TokenKind::True
                | TokenKind::SelfCap
                | TokenKind::SelfLower
                | TokenKind::Underscore
                | TokenKind::Null => p.try_atomic(),

                TokenKind::Dot => p.try_range_to_or_symbol(option),
                TokenKind::LBracket => p.try_list_pattern(),
                TokenKind::LParen => p.try_tuple_pattern(),
                TokenKind::LBrace => p.try_record_pattern(option),
                TokenKind::SeparatedLt => p.try_pattern_from_expr(),

                TokenKind::Ref => {
                    p.eat_tokens(1);
                    let pattern = match p.try_pattern_with_option(option)? {
                        0 => {
                            return Err(ParseError::invalid_syntax(
                                "Expected a pattern after `ref`".to_string(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            ));
                        }
                        node => node,
                    };
                    Ok(NodeBuilder::new(NodeKind::RefPattern, p.current_span())
                        .add_single_child(pattern)
                        .build(&mut p.ast))
                }

                TokenKind::Async => {
                    p.eat_tokens(1);
                    let pattern = match p.try_pattern_with_option(option)? {
                        0 => {
                            return Err(ParseError::invalid_syntax(
                                "Expected a pattern after `async`".to_string(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            ));
                        }
                        node => node,
                    };
                    Ok(NodeBuilder::new(NodeKind::AsyncPattern, p.current_span())
                        .add_single_child(pattern)
                        .build(&mut p.ast))
                }

                TokenKind::Not => {
                    p.eat_tokens(1);
                    let pattern = match p.try_pattern_with_option(option)? {
                        0 => {
                            return Err(ParseError::invalid_syntax(
                                "Expected a pattern after `not`".to_string(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            ));
                        }
                        node => node,
                    };
                    Ok(NodeBuilder::new(NodeKind::NotPattern, p.current_span())
                        .add_single_child(pattern)
                        .build(&mut p.ast))
                }

                TokenKind::Error => {
                    p.eat_tokens(1);
                    let pattern = match p.try_pattern_with_option(option)? {
                        0 => {
                            return Err(ParseError::invalid_syntax(
                                "Expected a pattern after `error`".to_string(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            ));
                        }
                        node => node,
                    };
                    Ok(NodeBuilder::new(NodeKind::ErrorPattern, p.current_span())
                        .add_single_child(pattern)
                        .build(&mut p.ast))
                }

                TokenKind::Quote => {
                    p.eat_tokens(1);
                    let id = match p.try_id()? {
                        0 => {
                            return Err(ParseError::invalid_syntax(
                                "Expected an identifier after `'`".to_string(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            ));
                        }
                        node => node,
                    };
                    Ok(
                        NodeBuilder::new(NodeKind::TypeBindPattern, p.current_span())
                            .add_single_child(id)
                            .build(&mut p.ast),
                    )
                }

                _ => Ok(0),
            }
        })
    }

    /// 尝试解析后缀模式
    pub fn try_postfix_pattern(
        &mut self,
        token_kind: TokenKind,
        left: NodeIndex,
        min_prec: i32,
        option: PatternOption,
    ) -> ParseResult {
        let option_ = option.clone();

        self.scoped(|p| {
            match token_kind {
                TokenKind::LParen => {
                    // 模式调用
                    let nodes = p.try_multi_with_bracket(
                        &[Rule::comma("pattern", |p| p.try_pattern())],
                        (TokenKind::LParen, TokenKind::RParen),
                    )?;
                    Ok(
                        NodeBuilder::new(NodeKind::ApplicationPattern, p.current_span())
                            .add_single_child(left)
                            .add_multiple_children(nodes)
                            .build(&mut p.ast),
                    )
                }

                TokenKind::Lt => {
                    // 钻石模式调用
                    let nodes = p.try_multi_with_bracket(
                        &[Rule::comma("pattern", |p| p.try_pattern())],
                        (TokenKind::Lt, TokenKind::Gt),
                    )?;
                    Ok(
                        NodeBuilder::new(NodeKind::NormalFormApplicationPattern, p.current_span())
                            .add_single_child(left)
                            .add_multiple_children(nodes)
                            .build(&mut p.ast),
                    )
                }

                TokenKind::Dot => {
                    if p.peek(&[TokenKind::Dot, TokenKind::Star]) {
                        p.eat_tokens(2);
                        Ok(
                            NodeBuilder::new(NodeKind::ProjectionAllPath, p.current_span())
                                .add_single_child(left)
                                .build(&mut p.ast),
                        )
                    } else if p.peek(&[TokenKind::Dot, TokenKind::Dot]) {
                        p.eat_tokens(2);
                        if p.eat_token(TokenKind::Eq) {
                            let end = match p.try_pattern_with_option(option_)? {
                                0 => {
                                    return Err(ParseError::invalid_syntax(
                                        "Expected a pattern after `..=`".to_string(),
                                        p.peek_next_token().kind,
                                        p.next_token_span(),
                                    ));
                                }
                                node => node,
                            };
                            Ok(NodeBuilder::new(
                                NodeKind::RangeFromToInclusivePattern,
                                p.current_span(),
                            )
                            .add_single_child(left)
                            .add_single_child(end)
                            .build(&mut p.ast))
                        } else {
                            match p.try_pattern_with_option(option_)? {
                                0 => Ok(NodeBuilder::new(
                                    NodeKind::RangeFromPattern,
                                    p.current_span(),
                                )
                                .add_single_child(left)
                                .build(&mut p.ast)),
                                node => Ok(NodeBuilder::new(
                                    NodeKind::RangeFromToPattern,
                                    p.current_span(),
                                )
                                .add_single_child(left)
                                .add_single_child(node)
                                .build(&mut p.ast)),
                            }
                        }
                    } else {
                        p.eat_tokens(1);
                        let id = p.try_id()?;
                        if id == 0 {
                            return Err(ParseError::invalid_syntax(
                                "Expected an identifier after `.`".to_string(),
                                TokenKind::Id,
                                p.current_span(),
                            ));
                        }
                        Ok(NodeBuilder::new(NodeKind::Select, p.current_span())
                            .add_single_child(left)
                            .add_single_child(id)
                            .build(&mut p.ast))
                    }
                }

                TokenKind::LBrace => {
                    // 记录调用模式
                    if option.no_extended_call {
                        return Err(ParseError::MeetPostExtendedCallStart);
                    }

                    let nodes = p.try_multi_with_bracket(
                        &[
                            Rule::comma("property pattern", move |p| {
                                p.try_property_pattern(option)
                            }),
                            Rule::comma("id", |p| p.try_id()),
                        ],
                        (TokenKind::LBrace, TokenKind::RBrace),
                    )?;

                    Ok(
                        NodeBuilder::new(NodeKind::ExtendedApplicationPattern, p.current_span())
                            .add_single_child(left)
                            .add_multiple_children(nodes)
                            .build(&mut p.ast),
                    )
                }

                TokenKind::As => {
                    // as 绑定
                    p.eat_tokens(1);
                    let id = match p.try_id()? {
                        0 => {
                            return Err(ParseError::invalid_syntax(
                                "Expected an identifier after `as`".to_string(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            ));
                        }
                        node => node,
                    };
                    Ok(NodeBuilder::new(NodeKind::AsBindPattern, p.current_span())
                        .add_single_child(left)
                        .add_single_child(id)
                        .build(&mut p.ast))
                }

                TokenKind::If => {
                    // if 守卫
                    p.eat_tokens(1);
                    let guard = match p.try_expr_without_extended_call()? {
                        0 => {
                            return Err(ParseError::invalid_syntax(
                                "Expected an expression after `if`".to_string(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            ));
                        }
                        node => node,
                    };
                    Ok(NodeBuilder::new(NodeKind::IfGuardPattern, p.current_span())
                        .add_single_child(left)
                        .add_single_child(guard)
                        .build(&mut p.ast))
                }

                TokenKind::And => {
                    // and-is 模式
                    p.eat_tokens(1);
                    let expr = match p.try_expr()? {
                        0 => {
                            return Err(ParseError::invalid_syntax(
                                "Expected an expression after `and`".to_string(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            ));
                        }
                        node => node,
                    };

                    if !p.eat_token(TokenKind::Is) {
                        return Err(ParseError::invalid_syntax(
                            "Missing 'is' after 'and'".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }

                    let pattern = match p.try_pattern_pratt(min_prec, option)? {
                        0 => {
                            return Err(ParseError::invalid_syntax(
                                "Expected a pattern after `is`".to_string(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            ));
                        }
                        node => node,
                    };

                    Ok(NodeBuilder::new(NodeKind::AndIsPattern, p.current_span())
                        .add_single_child(left)
                        .add_single_child(expr)
                        .add_single_child(pattern)
                        .build(&mut p.ast))
                }

                TokenKind::Question => {
                    // optional some 模式
                    p.eat_tokens(1);
                    Ok(
                        NodeBuilder::new(NodeKind::OptionSomePattern, p.current_span())
                            .add_single_child(left)
                            .build(&mut p.ast),
                    )
                }

                TokenKind::Bang => {
                    // 错误 ok 模式
                    p.eat_tokens(1);
                    Ok(NodeBuilder::new(NodeKind::ErrorOkPattern, p.current_span())
                        .add_single_child(left)
                        .build(&mut p.ast))
                }

                _ => Ok(0),
            }
        })
    }

    /// Parse a bit vector pattern.
    /// bit_vec_*_pattern -> 0b/0o/0x (integer | (id: expr) | <expr>)+
    /// Returns a MultiChildren node whose children are the segments.
    fn try_bit_vec_pattern(&mut self, kind: NodeKind) -> ParseResult {
        self.scoped(|p| {
            let mut segments = Vec::new();

            // First segment is the initial integer literal (e.g., 0b1110)
            let first = NodeBuilder::new(NodeKind::Int, p.next_token_span()).build(&mut p.ast);
            p.eat_tokens(1);
            segments.push(first);

            // Parse additional segments: integer | (id: expr) | <expr>
            loop {
                let next = p.peek_next_token();
                match next.kind {
                    // Another integer literal segment (e.g., 10 in 0b1110 10)
                    TokenKind::Int => {
                        let seg =
                            NodeBuilder::new(NodeKind::Int, p.next_token_span()).build(&mut p.ast);
                        p.eat_tokens(1);
                        segments.push(seg);
                    }
                    // (id: expr) — named bit field
                    TokenKind::LParen => {
                        let seg = p.scoped(|p2| {
                            p2.eat_tokens(1); // eat '('
                            let id = p2.try_id()?;
                            if id == 0 {
                                return Err(ParseError::invalid_syntax(
                                    "Expected identifier in bit vector field".to_string(),
                                    p2.peek_next_token().kind,
                                    p2.next_token_span(),
                                ));
                            }
                            if !p2.eat_token(TokenKind::Colon) {
                                return Err(ParseError::unexpected_token(
                                    TokenKind::Colon,
                                    p2.peek_next_token().kind,
                                    p2.next_token_span(),
                                ));
                            }
                            let ty = p2.try_expr()?;
                            if ty == 0 {
                                return Err(ParseError::invalid_syntax(
                                    "Expected type expression in bit vector field".to_string(),
                                    p2.peek_next_token().kind,
                                    p2.next_token_span(),
                                ));
                            }
                            if !p2.eat_token(TokenKind::RParen) {
                                return Err(ParseError::unexpected_token(
                                    TokenKind::RParen,
                                    p2.peek_next_token().kind,
                                    p2.next_token_span(),
                                ));
                            }
                            // Store as TypeBoundDeclClause (id: type)
                            Ok(
                                NodeBuilder::new(NodeKind::TypeBoundDeclClause, p2.current_span())
                                    .add_single_child(id)
                                    .add_single_child(ty)
                                    .build(&mut p2.ast),
                            )
                        })?;
                        segments.push(seg);
                    }
                    // <expr> — computed bit field
                    TokenKind::SeparatedLt => {
                        let seg = p.scoped(|p2| {
                            p2.eat_tokens(1); // eat ' < '
                            let expr = p2.try_expr()?;
                            if expr == 0 {
                                return Err(ParseError::invalid_syntax(
                                    "Expected expression in bit vector computed field".to_string(),
                                    p2.peek_next_token().kind,
                                    p2.next_token_span(),
                                ));
                            }
                            if !p2.eat_token(TokenKind::SeparatedGt) {
                                return Err(ParseError::unexpected_token(
                                    TokenKind::SeparatedGt,
                                    p2.peek_next_token().kind,
                                    p2.next_token_span(),
                                ));
                            }
                            // Wrap in ExprAsPattern
                            Ok(NodeBuilder::new(NodeKind::ExprAsPattern, p2.current_span())
                                .add_single_child(expr)
                                .build(&mut p2.ast))
                        })?;
                        segments.push(seg);
                    }
                    _ => break,
                }
            }

            Ok(NodeBuilder::new(kind, p.current_span())
                .add_multiple_children(segments)
                .build(&mut p.ast))
        })
    }

    /// 尝试解析从表达式构建的模式 < expr >
    fn try_pattern_from_expr(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::SeparatedLt], |p| {
            p.eat_tokens(1);

            let expr = match p.try_expr()? {
                0 => {
                    return Err(ParseError::invalid_syntax(
                        "Expected an expression after ` < `".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                node => node,
            };

            if !p.eat_token(TokenKind::SeparatedGt) {
                return Err(ParseError::invalid_syntax(
                    "Expected ` > ` after expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::ExprAsPattern, p.current_span())
                .add_single_child(expr)
                .build(&mut p.ast))
        })
    }

    /// 尝试解析范围到模式或符号 (..pattern 或 ..=pattern)
    fn try_range_to_or_symbol(&mut self, option: PatternOption) -> ParseResult {
        self.scoped(|p| {
            if p.peek(&[TokenKind::Dot, TokenKind::Dot, TokenKind::Eq]) {
                p.eat_tokens(3); // 消耗 "..="
                let end = match p.try_pattern_with_option(option)? {
                    0 => {
                        return Err(ParseError::invalid_syntax(
                            "Expected a pattern after `..=`".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    node => node,
                };
                Ok(
                    NodeBuilder::new(NodeKind::RangeToInclusivePattern, p.current_span())
                        .add_single_child(end)
                        .build(&mut p.ast),
                )
            } else if p.peek(&[TokenKind::Dot, TokenKind::Dot]) {
                p.eat_tokens(2); // 消耗 ".."
                let end = match p.try_pattern_with_option(option)? {
                    0 => {
                        return Err(ParseError::invalid_syntax(
                            "Expected a pattern after `..`".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    node => node,
                };
                Ok(NodeBuilder::new(NodeKind::RangeToPattern, p.current_span())
                    .add_single_child(end)
                    .build(&mut p.ast))
            } else {
                // 尝试解析符号
                p.try_symbol()
            }
        })
    }

    /// 尝试解析属性模式 (id: pattern)
    fn try_property_pattern(&mut self, option: PatternOption) -> ParseResult {
        self.scoped(|p| {
            if !p.peek(&[TokenKind::Id, TokenKind::Colon]) {
                return Ok(0);
            }

            let id = match p.try_id()? {
                0 => {
                    return Err(ParseError::invalid_syntax(
                        "Expected an identifier".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                node => node,
            };

            p.eat_tokens(1); // 消耗冒号

            let pattern = match p.try_pattern_with_option(option)? {
                0 => {
                    return Err(ParseError::invalid_syntax(
                        "Expected a pattern after `:`".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                node => node,
            };

            Ok(
                NodeBuilder::new(NodeKind::PropertyPattern, p.current_span())
                    .add_single_child(id)
                    .add_single_child(pattern)
                    .build(&mut p.ast),
            )
        })
    }

    /// 尝试解析记录模式 {props}
    fn try_record_pattern(&mut self, option: PatternOption) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::LBrace], |p| {
            let nodes = p.try_multi_with_bracket(
                &[
                    Rule::comma("property pattern", move |p| p.try_property_pattern(option)),
                    Rule::comma("id", |p| p.try_id()),
                ],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            Ok(NodeBuilder::new(NodeKind::StructPattern, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast))
        })
    }

    /// Try parse list rest pattern: `...id`
    fn try_list_rest_pattern(&mut self) -> ParseResult {
        self.scoped(|p| {
            if !p.peek(&[TokenKind::Dot, TokenKind::Dot, TokenKind::Dot]) {
                return Ok(0);
            }
            p.eat_tokens(3); // consume '...'
            let id = p.try_id()?;
            if id == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected an identifier after `...` in list rest pattern".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            Ok(
                NodeBuilder::new(NodeKind::ListRestPattern, p.current_span())
                    .add_single_child(id)
                    .build(&mut p.ast),
            )
        })
    }

    /// Try parse list pattern: `[pattern*]`
    fn try_list_pattern(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::LBracket], |p| {
            let nodes = p.try_multi_with_bracket(
                &[
                    Rule::comma("list rest pattern", |p| p.try_list_rest_pattern()),
                    Rule::comma("pattern", |p| p.try_pattern()),
                ],
                (TokenKind::LBracket, TokenKind::RBracket),
            )?;

            Ok(NodeBuilder::new(NodeKind::ListPattern, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast))
        })
    }

    /// 尝试解析元组模式 (items)
    fn try_tuple_pattern(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::LParen], |p| {
            let nodes = p.try_multi_with_bracket(
                &[Rule::comma("pattern", |p| p.try_pattern())],
                (TokenKind::LParen, TokenKind::RParen),
            )?;

            Ok(NodeBuilder::new(NodeKind::TuplePattern, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast))
        })
    }
}
