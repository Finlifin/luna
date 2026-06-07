pub mod error;
pub mod lexer;
pub mod token;

use std::collections::HashMap;

pub use error::{LexError, LexResult};
pub use lexer::Lexer;
use rustc_span::BytePos;
pub use symbol::Symbol;
pub use token::{Index, Token, TokenKind};

/// Lex `src` into tokens, an index-to-symbol map, and errors.
///
/// `symbols` maps the token index to its [`Symbol`]; only identifier tokens
/// have entries.  Use `symbols.get(&i)` to look up token `i`.
pub fn lex(src: &str, base_pos: BytePos) -> (Vec<Token>, HashMap<usize, Symbol>, Vec<LexError>) {
    let mut lexer = Lexer::new(src, base_pos);
    let mut tokens = Vec::new();
    let mut symbols: HashMap<usize, Symbol> = HashMap::new();
    let mut errors = Vec::new();

    // SOF sentinel – not an identifier.
    tokens.push(Token::new(TokenKind::Sof, 0, 0));

    loop {
        let next = lexer.next();
        match next {
            Ok(t) => {
                let idx = tokens.len();
                if matches!(t.kind, TokenKind::Id) {
                    symbols.insert(idx, Symbol::intern(&src[t.from as usize..t.to as usize]));
                }
                let is_eof = matches!(t.kind, TokenKind::Eof);
                tokens.push(t);
                if is_eof {
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

    (tokens, symbols, errors)
}
