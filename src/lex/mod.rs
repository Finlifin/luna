pub mod error;
pub mod lexer;
pub mod token;

pub use error::{LexError, LexResult};
pub use lexer::Lexer;
use rustc_span::BytePos;
pub use token::{Index, Token, TokenKind};

pub fn lex(src: &str, base_pos: BytePos) -> (Vec<Token>, Vec<LexError>) {
    let mut lexer = Lexer::new(src, base_pos);
    let mut tokens = Vec::new();
    let mut errors = Vec::new();

    tokens.push(Token::new(TokenKind::Sof, 0, 0));

    loop {
        let next = lexer.next();
        match next {
            Ok(t) => {
                tokens.push(t);
                if matches!(t.kind, TokenKind::Eof) {
                    break; // 结束解析
                }
            }
            Err(e) => {
                errors.push(e);
                lexer.recover_from_error();

                // 继续处理下一个token
                continue;
            }
        }
    }

    (tokens, errors)
}
