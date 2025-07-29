use std::option;

use crate::lex::TokenKind;
use crate::parse::ast::*;
use crate::parse::basic::Rule;
use crate::parse::error::*;
use crate::parse::operators::get_pattern_op_info;
use crate::parse::parser::*;

/// 模式选项，包含是否记录调用和优先级
#[derive(Debug, Clone, Copy)]
pub struct PatternOption {
    pub no_object_call: bool,
    pub precedence: i32,
}

impl PatternOption {
    pub fn new() -> Self {
        Self {
            no_object_call: false,
            precedence: 0,
        }
    }

    pub fn with_no_object_call(mut self, no_object_call: bool) -> Self {
        self.no_object_call = no_object_call;
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

    pub fn try_pattern_without_object_call(&mut self) -> ParseResult {
        self.scoped(|p| p.try_pattern_pratt(0, PatternOption::new().with_no_object_call(true)))
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
                    Err(ParseError::MeetPostObjectStart) => {
                        // 如果遇到 MeetPostObjectStart，跳过后缀模式处理
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
            // TODO
            // // 检查是否是位向量模式 (0x, 0o, 0b)
            // if p.check_bit_vec_pattern() {
            //     return p.try_bit_vec_pattern();
            // }

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
                | TokenKind::Null => p.try_atomic(),

                TokenKind::Dot => p.try_range_to_or_symbol(option),
                TokenKind::LBracket => p.try_list_pattern(),
                TokenKind::LParen => p.try_tuple_pattern(),
                TokenKind::LBrace => p.try_record_pattern(option),
                TokenKind::SeparatedLt => p.try_pattern_from_expr(),

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
                    Ok(NodeBuilder::new(NodeKind::PatternAsync, p.current_span())
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
                    Ok(NodeBuilder::new(NodeKind::PatternNot, p.current_span())
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
                    Ok(NodeBuilder::new(NodeKind::PatternError, p.current_span())
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
                        NodeBuilder::new(NodeKind::PatternTypeBind, p.current_span())
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
                    Ok(NodeBuilder::new(NodeKind::PatternCall, p.current_span())
                        .add_single_child(left)
                        .add_multiple_children(nodes)
                        .build(&mut p.ast))
                }

                TokenKind::Lt => {
                    // 钻石模式调用
                    let nodes = p.try_multi_with_bracket(
                        &[Rule::comma("pattern", |p| p.try_pattern())],
                        (TokenKind::Lt, TokenKind::Gt),
                    )?;
                    Ok(
                        NodeBuilder::new(NodeKind::PatternDiamondCall, p.current_span())
                            .add_single_child(left)
                            .add_multiple_children(nodes)
                            .build(&mut p.ast),
                    )
                }

                TokenKind::Dot => {
                    if p.peek(&[TokenKind::Dot, TokenKind::Star]) {
                        p.eat_tokens(2);
                        Ok(NodeBuilder::new(NodeKind::PathSelectAll, p.current_span())
                            .add_single_child(left)
                            .build(&mut p.ast))
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
                                NodeKind::PatternRangeFromToInclusive,
                                p.current_span(),
                            )
                            .add_single_child(left)
                            .add_single_child(end)
                            .build(&mut p.ast))
                        } else {
                            match p.try_pattern_with_option(option_)? {
                                0 => Ok(NodeBuilder::new(
                                    NodeKind::PatternRangeFrom,
                                    p.current_span(),
                                )
                                .add_single_child(left)
                                .build(&mut p.ast)),
                                node => Ok(NodeBuilder::new(
                                    NodeKind::PatternRangeFromTo,
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
                    if option.no_object_call {
                        return Err(ParseError::MeetPostObjectStart);
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
                        NodeBuilder::new(NodeKind::PatternObjectCall, p.current_span())
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
                    Ok(NodeBuilder::new(NodeKind::PatternAsBind, p.current_span())
                        .add_single_child(left)
                        .add_single_child(id)
                        .build(&mut p.ast))
                }

                TokenKind::If => {
                    // if 守卫
                    p.eat_tokens(1);
                    let guard = match p.try_expr_without_object_call()? {
                        0 => {
                            return Err(ParseError::invalid_syntax(
                                "Expected an expression after `if`".to_string(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            ));
                        }
                        node => node,
                    };
                    Ok(NodeBuilder::new(NodeKind::PatternIfGuard, p.current_span())
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

                    Ok(NodeBuilder::new(NodeKind::PatternAndIs, p.current_span())
                        .add_single_child(left)
                        .add_single_child(expr)
                        .add_single_child(pattern)
                        .build(&mut p.ast))
                }

                TokenKind::Question => {
                    // optional some 模式
                    p.eat_tokens(1);
                    Ok(
                        NodeBuilder::new(NodeKind::PatternOptionSome, p.current_span())
                            .add_single_child(left)
                            .build(&mut p.ast),
                    )
                }

                TokenKind::Bang => {
                    // 错误 ok 模式
                    p.eat_tokens(1);
                    Ok(NodeBuilder::new(NodeKind::PatternErrorOk, p.current_span())
                        .add_single_child(left)
                        .build(&mut p.ast))
                }

                _ => Ok(0),
            }
        })
    }

    // /// 检查是否是位向量模式的开始 (0x, 0o, 0b)
    // fn check_bit_vec_pattern(&mut self) -> bool {
    //     if self.peek_next_token().kind == TokenKind::Int {
    //         // 这里需要实现获取token文本的逻辑
    //         // 暂时返回false，因为没有直接的方法获取token文本
    //         false
    //     } else {
    //         false
    //     }
    // }

    // /// 尝试解析位向量模式
    // fn try_bit_vec_pattern(&mut self) -> ParseResult {
    //     self.scoped(|p| {
    //         // 简化版本，暂时不实现位向量模式
    //         Ok(0)
    //     })
    // }

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
                    NodeBuilder::new(NodeKind::PatternRangeToInclusive, p.current_span())
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
                Ok(NodeBuilder::new(NodeKind::PatternRangeTo, p.current_span())
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

            Ok(NodeBuilder::new(NodeKind::PatternRecord, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast))
        })
    }

    /// 尝试解析列表模式 [items]
    fn try_list_pattern(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::LBracket], |p| {
            let nodes = p.try_multi_with_bracket(
                &[Rule::comma("pattern", |p| p.try_pattern())],
                (TokenKind::LBracket, TokenKind::RBracket),
            )?;

            Ok(NodeBuilder::new(NodeKind::PatternList, p.current_span())
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

            Ok(NodeBuilder::new(NodeKind::PatternTuple, p.current_span())
                .add_multiple_children(nodes)
                .build(&mut p.ast))
        })
    }
}
