use crate::lex::TokenKind;
use crate::parse::ast::*;
use crate::parse::error::*;
use crate::parse::parser::*;

impl Parser<'_> {
    pub fn try_atomic(&mut self) -> ParseResult {
        self.scoped(|p| {
            let next = p.peek_next_token();
            use TokenKind::*;
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

                _ => Ok(0),
            };
            p.eat_tokens(1);
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
            p.eat_tokens(1);
            let id = p.try_id()?;
            Ok(NodeBuilder::new(NodeKind::Symbol, p.current_span())
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
                            break 'outer;
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
}

pub struct Rule {
    pub name: &'static str,
    pub parser: Box<dyn Fn(&mut Parser) -> ParseResult>,
    pub degree: (),

    // 分隔符, 例如: `,` 或 `;`
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

    pub fn ends_with_semicolon(&self) -> bool {
        self.separator == TokenKind::Semi
    }
}
