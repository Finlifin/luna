use std::collections::HashMap;
use super::token::{Token, TokenKind, Index};
use super::error::{LexError, LexResult};
use rustc_span::BytePos;

/// Check if a character can be the start of an identifier
fn is_identifier_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || 
    // Support Unicode identifiers, including some common symbols
    matches!(c, 
        // Unicode symbols and emoji
        '\u{1F000}'..='\u{1F9FF}' |  // Various symbol blocks
        '\u{2600}'..='\u{26FF}' |    // Miscellaneous symbols
        '\u{2700}'..='\u{27BF}' |    // Decorative symbols
        // Other common non-alphabetic identifier characters
        '$' | '@'
    )
}

/// Check if a character can be a continuation of an identifier
fn is_identifier_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || is_identifier_start(c)
}

pub struct Lexer<'a> {
    src: &'a str,
    cursor: Index,
    keywords: HashMap<&'static str, TokenKind>,
    base_pos: BytePos,
    remaining: &'a str,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str, base_pos: BytePos) -> Self {
        Self {
            src,
            cursor: 0,
            keywords: Token::keywords(),
            base_pos,
            remaining: src,
        }
    }

    /// Get the current character without moving the cursor
    fn current_char(&self) -> Option<char> {
        self.remaining.chars().next()
    }

    /// Move the cursor to the next character
    fn advance(&mut self) -> Option<char> {
        let ch = self.current_char()?;
        let ch_len = ch.len_utf8();
        self.cursor += ch_len;
        self.remaining = &self.remaining[ch_len..];
        Some(ch)
    }

    /// Peek the next character without moving the cursor
    fn peek_char(&self) -> Option<char> {
        let mut chars = self.remaining.chars();
        chars.next()?; // 跳过当前字符
        chars.next()
    }

    /// Check if the end of file is reached
    fn is_eof(&self) -> bool {
        self.remaining.is_empty()
    }

    pub fn next(&mut self) -> LexResult<Token> {
        // Skip whitespace
        self.skip_whitespace();
        let start = self.cursor;

        if self.is_eof() {
            return Ok(Token::new(TokenKind::Eof, start, start));
        }

        let c = self.current_char().unwrap();
        
        match c {
            // Single character tokens
            '^' => {
                self.advance();
                Ok(Token::new(TokenKind::Caret, start, self.cursor))
            }
            '.' => {
                self.advance();
                Ok(Token::new(TokenKind::Dot, start, self.cursor))
            }
            '@' => {
                self.advance();
                Ok(Token::new(TokenKind::At, start, self.cursor))
            }
            '\\' => {
                self.advance();
                Ok(Token::new(TokenKind::Backslash, start, self.cursor))
            }
            '(' => {
                self.advance();
                Ok(Token::new(TokenKind::LParen, start, self.cursor))
            }
            ')' => {
                self.advance();
                Ok(Token::new(TokenKind::RParen, start, self.cursor))
            }
            '[' => {
                self.advance();
                Ok(Token::new(TokenKind::LBracket, start, self.cursor))
            }
            ']' => {
                self.advance();
                Ok(Token::new(TokenKind::RBracket, start, self.cursor))
            }
            '}' => {
                self.advance();
                Ok(Token::new(TokenKind::RBrace, start, self.cursor))
            }
            ',' => {
                self.advance();
                Ok(Token::new(TokenKind::Comma, start, self.cursor))
            }
            ';' => {
                self.advance();
                Ok(Token::new(TokenKind::Semi, start, self.cursor))
            }
            '#' => {
                self.advance();
                Ok(Token::new(TokenKind::Hash, start, self.cursor))
            }
            '$' => {
                self.advance();
                Ok(Token::new(TokenKind::Dollar, start, self.cursor))
            }
            '_' => {
                self.advance();
                Ok(Token::new(TokenKind::Underscore, start, self.cursor))
            }

            // Complex tokens
            '"' => self.recognize_string(start),
            '\'' => self.recognize_quote_or_char_or_macro(start),
            '0'..='9' => self.recognize_number(start),
            
            // Operators with potential combinations
            '+' => {
                self.advance();
                match self.current_char() {
                    Some('=') => {
                        self.advance();
                        Ok(Token::new(TokenKind::PlusEq, start, self.cursor))
                    }
                    Some('+') => {
                        self.advance();
                        Ok(Token::new(TokenKind::PlusPlus, start, self.cursor))
                    }
                    _ => {
                        if self.is_separated(start) {
                            Ok(Token::new(TokenKind::SeparatedPlus, start, self.cursor))
                        } else {
                            Ok(Token::new(TokenKind::Plus, start, self.cursor))
                        }
                    }
                }
            }
            
            '-' => {
                self.advance();
                match self.current_char() {
                    Some('=') => {
                        self.advance();
                        Ok(Token::new(TokenKind::MinusEq, start, self.cursor))
                    }
                    Some('>') => {
                        self.advance();
                        Ok(Token::new(TokenKind::Arrow, start, self.cursor))
                    }
                    Some('-') => {
                    // Line comment
                        self.skip_line_comment();
                        self.next() // Recursively get next token
                    }
                    _ => {
                        if self.is_separated(start) {
                            Ok(Token::new(TokenKind::SeparatedMinus, start, self.cursor))
                        } else {
                            Ok(Token::new(TokenKind::Minus, start, self.cursor))
                        }
                    }
                }
            }

            '*' => {
                self.advance();
                if let Some('=') = self.current_char() {
                    self.advance();
                    Ok(Token::new(TokenKind::StarEq, start, self.cursor))
                } else if self.is_separated(start) {
                    Ok(Token::new(TokenKind::SeparatedStar, start, self.cursor))
                } else {
                    Ok(Token::new(TokenKind::Star, start, self.cursor))
                }
            }

            '/' => {
                self.advance();
                if let Some('=') = self.current_char() {
                    self.advance();
                    Ok(Token::new(TokenKind::SlashEq, start, self.cursor))
                } else if self.is_separated(start) {
                    Ok(Token::new(TokenKind::SeparatedSlash, start, self.cursor))
                } else {
                    Ok(Token::new(TokenKind::Slash, start, self.cursor))
                }
            }

            '%' => {
                self.advance();
                if let Some('=') = self.current_char() {
                    self.advance();
                    Ok(Token::new(TokenKind::PercentEq, start, self.cursor))
                } else if self.is_separated(start) {
                    Ok(Token::new(TokenKind::SeparatedPercent, start, self.cursor))
                } else {
                    Ok(Token::new(TokenKind::Percent, start, self.cursor))
                }
            }

            '?' => {
                self.advance();
                Ok(Token::new(TokenKind::Question, start, self.cursor))
            }

            '&' => {
                self.advance();
                Ok(Token::new(TokenKind::Ampersand, start, self.cursor))
            }

            '=' => {
                self.advance();
                match self.current_char() {
                    Some('=') => {
                        self.advance();
                        if let Some('=') = self.current_char() {
                            self.advance();
                            Ok(Token::new(TokenKind::EqEqEq, start, self.cursor))
                        } else {
                            Ok(Token::new(TokenKind::EqEq, start, self.cursor))
                        }
                    }
                    Some('>') => {
                        self.advance();
                        Ok(Token::new(TokenKind::FatArrow, start, self.cursor))
                    }
                    _ => Ok(Token::new(TokenKind::Eq, start, self.cursor))
                }
            }

            '!' => {
                self.advance();
                if let Some('=') = self.current_char() {
                    self.advance();
                    Ok(Token::new(TokenKind::BangEq, start, self.cursor))
                } else {
                    Ok(Token::new(TokenKind::Bang, start, self.cursor))
                }
            }

            '<' => {
                self.advance();
                if let Some('=') = self.current_char() {
                    self.advance();
                    Ok(Token::new(TokenKind::LtEq, start, self.cursor))
                } else if self.is_separated(start) {
                    Ok(Token::new(TokenKind::SeparatedLt, start, self.cursor))
                } else {
                    Ok(Token::new(TokenKind::Lt, start, self.cursor))
                }
            }

            '>' => {
                self.advance();
                if let Some('=') = self.current_char() {
                    self.advance();
                    Ok(Token::new(TokenKind::GtEq, start, self.cursor))
                } else if self.is_separated(start) {
                    Ok(Token::new(TokenKind::SeparatedGt, start, self.cursor))
                } else {
                    Ok(Token::new(TokenKind::Gt, start, self.cursor))
                }
            }

            ':' => {
                self.advance();
                match self.current_char() {
                    Some(':') => {
                        self.advance();
                        Ok(Token::new(TokenKind::ColonColon, start, self.cursor))
                    }
                    Some('~') => {
                        self.advance();
                        Ok(Token::new(TokenKind::ColonTilde, start, self.cursor))
                    }
                    Some('-') => {
                        self.advance();
                        Ok(Token::new(TokenKind::ColonMinus, start, self.cursor))
                    }
                    _ => Ok(Token::new(TokenKind::Colon, start, self.cursor))
                }
            }

            '~' => {
                self.advance();
                if let Some('>') = self.current_char() {
                    self.advance();
                    Ok(Token::new(TokenKind::TildeGt, start, self.cursor))
                } else {
                    Ok(Token::new(TokenKind::Tilde, start, self.cursor))
                }
            }

            '|' => {
                self.advance();
                if let Some('>') = self.current_char() {
                    self.advance();
                    Ok(Token::new(TokenKind::PipeGt, start, self.cursor))
                } else {
                    Ok(Token::new(TokenKind::Pipe, start, self.cursor))
                }
            }

            '{' => {
                // 检查是否是块注释 {-
                if let Some('-') = self.peek_char() {
                    self.recognize_block_comment(start)
                } else {
                    self.advance();
                    Ok(Token::new(TokenKind::LBrace, start, self.cursor))
                }
            }

            // 字母、数字、下划线或其他有效 Unicode 标识符字符
            c if is_identifier_start(c) => {
                self.recognize_identifier(start)
            }

            // 其他字符 - 错误
            _ => {
                Err(LexError::UnexpectedChar {
                    position: start as u32,
                    char: c,
                    message: format!("Unexpected character '{}'", c),
                })
            }
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.current_char() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_line_comment(&mut self) {
        while let Some(c) = self.current_char() {
            if c == '\n' {
                break;
            }
            self.advance();
        }
    }

    fn is_separated(&self, start: Index) -> bool {
        let has_space_before = if start > 0 {
            self.src.chars().nth(start - 1).map(|c| c.is_whitespace()).unwrap_or(false)
        } else {
            false
        };
        let has_space_after = self.current_char().map(|c| c.is_whitespace()).unwrap_or(false);
        has_space_before && has_space_after
    }

    fn recognize_string(&mut self, start: Index) -> LexResult<Token> {
        // Move past opening quote
        self.advance();

        while let Some(c) = self.current_char() {
            match c {
                '"' => {
                    // Closing quote found
                    self.advance();
                    return Ok(Token::new(TokenKind::Str, start, self.cursor));
                }
                '\\' => {
                    // Handle escape sequence
                    let escape_start = self.cursor; // 记录转义序列的开始位置
                    self.advance();
                    if let Some(escape_char) = self.current_char() {
                        // Validate escape character
                        match escape_char {
                            // 基本转义字符
                            'n' | 't' | 'r' | '\\' | '\'' | '"' | '0' => {
                                self.advance();
                            }
                            // 更多转义字符
                            'a' | 'b' | 'f' | 'v' => {
                                self.advance();
                            }
                            // Unicode 转义序列 \x{...}
                            'x' => {
                                self.advance();
                                if let Some('{') = self.current_char() {
                                    self.advance();
                                    if let Err(e) = self.consume_unicode_escape(escape_start as u32) {
                                        return Err(e);
                                    }
                                } else {
                                    return Err(LexError::InvalidEscape {
                                        start: escape_start as u32,
                                        escape_char: 'x',
                                        message: "Unicode escape sequence must be in format \\x{...}".to_string(),
                                    });
                                }
                            }
                            // 十六进制转义序列 \xHH
                            'u' => {
                                self.advance();
                                if let Some('{') = self.current_char() {
                                    self.advance();
                                    if let Err(e) = self.consume_unicode_escape(escape_start as u32) {
                                        return Err(e);
                                    }
                                } else {
                                    return Err(LexError::InvalidEscape {
                                        start: escape_start as u32,
                                        escape_char: 'u',
                                        message: "Unicode escape sequence must be in format \\u{...}".to_string(),
                                    });
                                }
                            }
                            _ => {
                                return Err(LexError::InvalidEscape {
                                    start: escape_start as u32, // 使用转义序列的开始位置
                                    escape_char,
                                    message: format!("Invalid escape sequence '\\{}'", escape_char),
                                });
                            }
                        }
                    } else {
                        return Err(LexError::UnterminatedString {
                            start: start as u32,
                            message: "Unterminated string literal with escape at end of file".to_string(),
                        });
                    }
                }
                '\n' => {
                    return Err(LexError::UnterminatedString {
                        start: start as u32,
                        message: "Unterminated string literal, unexpected newline".to_string(),
                    });
                }
                _ => {
                    self.advance();
                }
            }
        }

        // Reached end of file without closing quote
        Err(LexError::UnterminatedString {
            start: start as u32,
            message: "Unterminated string literal, reached end of file".to_string(),
        })
    }

    /// 识别引号、字符字面量或宏内容
    fn recognize_quote_or_char_or_macro(&mut self, start: Index) -> LexResult<Token> {
        // 移动过开始的 '
        self.advance();
        
        // 跳过空格，查看后面的第一个非空字符
        let mut next_non_space_char = None;
        let mut temp_cursor = self.cursor;
        let mut temp_remaining = self.remaining;
        
        while let Some(c) = temp_remaining.chars().next() {
            if c.is_whitespace() {
                let ch_len = c.len_utf8();
                temp_cursor += ch_len;
                temp_remaining = &temp_remaining[ch_len..];
            } else {
                next_non_space_char = Some(c);
                break;
            }
        }
        
        // 根据宏规则：当lexer碰到`'`字符后, 向后看到第一个非空字符为 `{` 的时候, 认为是宏内容
        if let Some('{') = next_non_space_char {
            // 这是宏内容 '{ ... }，更新游标到第一个非空字符
            self.cursor = temp_cursor;
            self.remaining = temp_remaining;
            self.recognize_macro_content(start)
        } else {
            // 不是宏内容，检查是否是字符字面量
            // 只有当紧接着'的字符不是空白且下一个字符是'时，才认为是字符字面量
            match self.current_char() {
                Some('\'') => {
                    // 空字符字面量 ''
                    Err(LexError::EmptyChar {
                        start: start as u32,
                        message: "Empty character literal".to_string(),
                    })
                }
                Some(c) if !c.is_whitespace() && !is_identifier_start(c) => {
                    // 紧接着的非标识符字符，可能是字符字面量（如 'a', '\n' 等）
                    self.recognize_char_content(start)
                }
                _ => {
                    // 其他情况都是单独的引号
                    Ok(Token::new(TokenKind::Quote, start, self.cursor))
                }
            }
        }
    }

    /// 识别宏内容 '{ ... }
    fn recognize_macro_content(&mut self, start: Index) -> LexResult<Token> {
        // 已经消费了 '，现在消费 {
        self.advance();
        let mut brace_count = 1;
        
        while let Some(c) = self.current_char() {
            match c {
                '{' => {
                    brace_count += 1;
                    self.advance();
                }
                '}' => {
                    brace_count -= 1;
                    self.advance();
                    if brace_count == 0 {
                        // 找到了匹配的闭合括号
                        return Ok(Token::new(TokenKind::MacroContent, start, self.cursor));
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }
        
        // 到达文件末尾但括号没有闭合
        Err(LexError::UnterminatedMacro {
            start: start as u32,
            message: "Unterminated macro content, expected closing '}'".to_string(),
        })
    }

    /// 识别字符字面量的内容部分（已经消费了开始的'）
    fn recognize_char_content(&mut self, start: Index) -> LexResult<Token> {
        if let Some(c) = self.current_char() {
            // Handle escape sequences
            if c == '\\' {
                let escape_start = self.cursor;
                self.advance();
                if let Some(escape_char) = self.current_char() {
                    match escape_char {
                        // 基本转义字符
                        'n' | 't' | 'r' | '\\' | '\'' | '"' | '0' => {
                            self.advance();
                        }
                        // 更多转义字符
                        'a' | 'b' | 'f' | 'v' => {
                            self.advance();
                        }
                        // Unicode 转义序列 \x{...} 或 \u{...}
                        'x' | 'u' => {
                            self.advance();
                            if let Some('{') = self.current_char() {
                                self.advance();
                                if let Err(e) = self.consume_unicode_escape(escape_start as u32) {
                                    return Err(e);
                                }
                            } else {
                                return Err(LexError::InvalidEscape {
                                    start: escape_start as u32,
                                    escape_char,
                                    message: format!("Unicode escape sequence must be in format \\{}{{...}}", escape_char),
                                });
                            }
                        }
                        _ => {
                            let error = LexError::InvalidEscape {
                                start: escape_start as u32,
                                escape_char,
                                message: format!("Invalid escape sequence '\\{}'", escape_char),
                            };
                            
                            // 继续消费字符直到找到闭合引号，避免级联错误
                            self.advance();
                            if let Some('\'') = self.current_char() {
                                self.advance();
                            }
                            
                            return Err(error);
                        }
                    }
                } else {
                    return Err(LexError::UnterminatedChar {
                        start: start as u32,
                        message: "Unterminated character literal with escape".to_string(),
                    });
                }
            } else {
                // Regular character
                self.advance();
            }

            // Expect closing quote
            if let Some('\'') = self.current_char() {
                self.advance(); // Move past closing quote
                Ok(Token::new(TokenKind::Char, start, self.cursor))
            } else {
                Err(LexError::UnterminatedChar {
                    start: start as u32,
                    message: "Unterminated character literal, expected closing '".to_string(),
                })
            }
        } else {
            // 只有一个单引号，后面没有内容
            Ok(Token::new(TokenKind::Quote, start, self.cursor))
        }
    }

    /// 消费Unicode转义序列 {hex_digits}
    fn consume_unicode_escape(&mut self, escape_start: u32) -> Result<(), LexError> {
        let mut hex_digits = 0;
        
        // 读取十六进制数字
        while let Some(c) = self.current_char() {
            match c {
                '0'..='9' | 'a'..='f' | 'A'..='F' => {
                    hex_digits += 1;
                    if hex_digits > 6 {
                        return Err(LexError::InvalidEscape {
                            start: escape_start,
                            escape_char: 'x',
                            message: "Unicode escape sequence too long (max 6 hex digits)".to_string(),
                        });
                    }
                    self.advance();
                }
                '}' => {
                    if hex_digits == 0 {
                        return Err(LexError::InvalidEscape {
                            start: escape_start,
                            escape_char: 'x',
                            message: "Unicode escape sequence must contain at least one hex digit".to_string(),
                        });
                    }
                    self.advance(); // 消费 '}'
                    return Ok(());
                }
                _ => {
                    return Err(LexError::InvalidEscape {
                        start: escape_start,
                        escape_char: 'x',
                        message: "Invalid character in Unicode escape sequence".to_string(),
                    });
                }
            }
        }
        
        Err(LexError::UnterminatedChar {
            start: escape_start,
            message: "Unterminated Unicode escape sequence".to_string(),
        })
    }

    fn recognize_number(&mut self, start: Index) -> LexResult<Token> {
        // 检查是否以 0 开头（可能是特殊进制）
        if self.current_char() == Some('0') {
            self.advance();
            match self.current_char() {
                Some('b') | Some('B') => {
                    // 二进制数字
                    self.advance();
                    return self.recognize_binary_number(start);
                }
                Some('o') | Some('O') => {
                    // 八进制数字
                    self.advance();
                    return self.recognize_octal_number(start);
                }
                Some('x') | Some('X') => {
                    // 十六进制数字
                    self.advance();
                    return self.recognize_hex_number(start);
                }
                Some('0'..='9') | Some('_') => {
                    // 继续作为十进制数字处理
                }
                Some('.') => {
                // Floating point number starting with 0.
                    return self.recognize_decimal_number(start, true);
                }
                _ => {
                    // 单独的 0
                    return Ok(Token::new(TokenKind::Int, start, self.cursor));
                }
            }
        }
        
        // Decimal numbers (including those starting with 0 but not special radix)
        self.recognize_decimal_number(start, false)
    }

    fn recognize_decimal_number(&mut self, start: Index, already_has_zero: bool) -> LexResult<Token> {
        let mut has_dot = false;
        let mut has_digits_after_dot = false;
        
        // 如果还没有消费任何数字，先消费第一个数字
        if !already_has_zero {
            self.advance();
        }
        
        // 消费数字和下划线
        while let Some(c) = self.current_char() {
            match c {
                '0'..='9' => {
                    self.advance();
                    if has_dot {
                        has_digits_after_dot = true;
                    }
                }
                '_' => {
                    // 检查下划线周围是否有数字
                    self.advance();
                    if !matches!(self.current_char(), Some('0'..='9')) {
                        return Err(LexError::InvalidNumber {
                            start: start as u32,
                            message: "Invalid underscore position in number".to_string(),
                        });
                    }
                }
                '.' => {
                    if has_dot {
                        // 已经有小数点了
                        break;
                    }
                    has_dot = true;
                    self.advance();
                }
                'e' | 'E' => {
                    // 科学计数法
                    return self.recognize_scientific_notation(start, has_dot);
                }
                _ => break,
            }
        }
        
        if has_dot && !has_digits_after_dot {
            return Err(LexError::InvalidNumber {
                start: start as u32,
                message: "Decimal point must be followed by digits".to_string(),
            });
        }
        
        let token_kind = if has_dot {
            TokenKind::Real
        } else {
            TokenKind::Int
        };
        
        Ok(Token::new(token_kind, start, self.cursor))
    }

    fn recognize_binary_number(&mut self, start: Index) -> LexResult<Token> {
        let mut has_digits = false;
        
        while let Some(c) = self.current_char() {
            match c {
                '0' | '1' => {
                    has_digits = true;
                    self.advance();
                }
                '_' => {
                    self.advance();
                    if !matches!(self.current_char(), Some('0' | '1')) {
                        return Err(LexError::InvalidNumber {
                            start: start as u32,
                            message: "Invalid underscore position in binary number".to_string(),
                        });
                    }
                }
                _ => break,
            }
        }
        
        if !has_digits {
            return Err(LexError::InvalidNumber {
                start: start as u32,
                message: "Binary number must contain at least one digit".to_string(),
            });
        }
        
        Ok(Token::new(TokenKind::IntBin, start, self.cursor))
    }

    fn recognize_octal_number(&mut self, start: Index) -> LexResult<Token> {
        let mut has_digits = false;
        
        while let Some(c) = self.current_char() {
            match c {
                '0'..='7' => {
                    has_digits = true;
                    self.advance();
                }
                '_' => {
                    self.advance();
                    if !matches!(self.current_char(), Some('0'..='7')) {
                        return Err(LexError::InvalidNumber {
                            start: start as u32,
                            message: "Invalid underscore position in octal number".to_string(),
                        });
                    }
                }
                _ => break,
            }
        }
        
        if !has_digits {
            return Err(LexError::InvalidNumber {
                start: start as u32,
                message: "Octal number must contain at least one digit".to_string(),
            });
        }
        
        Ok(Token::new(TokenKind::IntOct, start, self.cursor))
    }

    fn recognize_hex_number(&mut self, start: Index) -> LexResult<Token> {
        let mut has_digits = false;
        
        while let Some(c) = self.current_char() {
            match c {
                '0'..='9' | 'a'..='f' | 'A'..='F' => {
                    has_digits = true;
                    self.advance();
                }
                '_' => {
                    self.advance();
                    if !matches!(self.current_char(), Some('0'..='9' | 'a'..='f' | 'A'..='F')) {
                        return Err(LexError::InvalidNumber {
                            start: start as u32,
                            message: "Invalid underscore position in hexadecimal number".to_string(),
                        });
                    }
                }
                _ => break,
            }
        }
        
        if !has_digits {
            return Err(LexError::InvalidNumber {
                start: start as u32,
                message: "Hexadecimal number must contain at least one digit".to_string(),
            });
        }
        
        Ok(Token::new(TokenKind::IntHex, start, self.cursor))
    }

    fn recognize_scientific_notation(&mut self, start: Index, _has_dot: bool) -> LexResult<Token> {
        // 消费 'e' 或 'E'
        self.advance();
        
        // 可选的符号
        if matches!(self.current_char(), Some('+' | '-')) {
            self.advance();
        }
        
        let mut has_exponent_digits = false;
        
        // 指数部分的数字
        while let Some(c) = self.current_char() {
            match c {
                '0'..='9' => {
                    has_exponent_digits = true;
                    self.advance();
                }
                '_' => {
                    self.advance();
                    if !matches!(self.current_char(), Some('0'..='9')) {
                        return Err(LexError::InvalidNumber {
                            start: start as u32,
                            message: "Invalid underscore position in scientific notation".to_string(),
                        });
                    }
                }
                _ => break,
            }
        }
        
        if !has_exponent_digits {
            return Err(LexError::InvalidNumber {
                start: start as u32,
                message: "Scientific notation must have digits in exponent".to_string(),
            });
        }
        
        Ok(Token::new(TokenKind::RealSci, start, self.cursor))
    }

    fn recognize_identifier(&mut self, start: Index) -> LexResult<Token> {
        // First character already checked to be valid identifier start
        self.advance();

        while let Some(c) = self.current_char() {
            if is_identifier_continue(c) {
                self.advance();
            } else {
                break;
            }
        }

        let text = &self.src[start..self.cursor];

        let token_kind = self.keywords.get(text)
            .copied()
            .unwrap_or(TokenKind::Id);

        Ok(Token::new(token_kind, start, self.cursor))
    }

    /// Recover from error by skipping the current character
    pub fn recover_from_error(&mut self) {
        if !self.is_eof() {
            self.advance();
        }
    }

    /// Recognize block comment {- ... -}
    fn recognize_block_comment(&mut self, start: Index) -> LexResult<Token> {
        // Consume '{-'
        self.advance(); // {
        self.advance(); // -
        
        while let Some(c) = self.current_char() {
            if c == '-' {
                self.advance();
                if let Some('}') = self.current_char() {
                    self.advance(); // Consume '}'
                    return Ok(Token::new(TokenKind::Comment, start, self.cursor));
                }
                // If not -}, keep searching
            } else {
                self.advance();
            }
        }
        
        // Reached end of file but comment not closed
        Err(LexError::UnterminatedComment {
            start: start as u32,
            message: "Unterminated block comment, expected '-}'".to_string(),
        })
    }
}
