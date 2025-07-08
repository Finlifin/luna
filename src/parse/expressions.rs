use crate::lex::TokenKind;
use crate::parse::ast::*;
use crate::parse::basic::Rule;
use crate::parse::error::*;
use crate::parse::parser::*;

impl Parser<'_> {
    pub fn try_expr(&mut self) -> ParseResult {
        self.scoped(|p| p.try_prefix_expr())
    }

    pub fn try_prefix_expr(&mut self) -> ParseResult {
        self.scoped(|p| {
            let next = p.peek_next_token();
            match next.kind {
                // for those atomic expressions
                TokenKind::Int | TokenKind::Real | TokenKind::Str | TokenKind::Char | TokenKind::Id | TokenKind::False | TokenKind::True
                | TokenKind::SelfCap | TokenKind::SelfLower | TokenKind::Null => {
                    p.try_atomic()
                }
                
                TokenKind::LBracket => {
                    let nodes = p.try_multi_with_bracket(
                        &[Rule::comma("expr", |p| p.try_expr())],
                        (TokenKind::LBracket, TokenKind::RBracket),
                    );
                    match nodes {
                        Ok(nodes) => Ok(NodeBuilder::new(NodeKind::ListOf, p.current_span())
                            .add_multiple_children(nodes)
                            .build(&mut p.ast)),
                        Err(e) => Err(e),
                    }
                }

                _ => Ok(0),
            }
        })
    }
}
