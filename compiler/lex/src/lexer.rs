use super::error::{LexError, LexResult};
use super::token::{Index, Token, TokenKind};
use rustc_span::BytePos;
use std::collections::HashMap;

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
    previous_token: Option<TokenKind>,
    can_end_expression: bool,
    recovery_delimiter: Option<char>,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str, base_pos: BytePos) -> Self {
        Self {
            src,
            cursor: 0,
            keywords: Token::keywords(),
            base_pos,
            remaining: src,
            previous_token: None,
            can_end_expression: false,
            recovery_delimiter: None,
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
        let token = self.next_token()?;
        if !matches!(token.kind, TokenKind::Comment | TokenKind::Eof) {
            // A quote after an operand starts a take view, including one-letter
            // view chains such as value'a'b. At an operand boundary it can
            // start a character literal or a quoted expression/pattern.
            self.can_end_expression = match token.kind {
                TokenKind::Id
                | TokenKind::Int
                | TokenKind::IntBin
                | TokenKind::IntOct
                | TokenKind::IntHex
                | TokenKind::Real
                | TokenKind::RealSci
                | TokenKind::Str
                | TokenKind::Char
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null
                | TokenKind::Undefined
                | TokenKind::Underscore
                | TokenKind::SelfLower
                | TokenKind::SelfCap
                | TokenKind::Itself
                | TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::RBrace
                | TokenKind::MacroContent => true,
                TokenKind::Bang | TokenKind::Question | TokenKind::Hash | TokenKind::Gt => {
                    self.can_end_expression
                }
                TokenKind::Star | TokenKind::Ref | TokenKind::Await => {
                    self.previous_token == Some(TokenKind::Dot)
                }
                _ => false,
            };
            self.previous_token = Some(token.kind);
        }
        Ok(token)
    }

    fn next_token(&mut self) -> LexResult<Token> {
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
                        self.next_token() // Recursively get next token
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
                    _ => Ok(Token::new(TokenKind::Eq, start, self.cursor)),
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
                    _ => Ok(Token::new(TokenKind::Colon, start, self.cursor)),
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
            c if is_identifier_start(c) => self.recognize_identifier(start),

            // 其他字符 - 错误
            _ => Err(LexError::UnexpectedChar {
                position: start as u32,
                char: c,
                message: format!("Unexpected character '{}'", c),
            }),
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
            // 正确处理UTF-8：从字节位置向前查找前一个字符
            // 找到最后一个有效UTF-8字符的开始位置
            let mut char_start = start;
            while char_start > 0 {
                char_start -= 1;
                if self.src.is_char_boundary(char_start) {
                    break;
                }
            }
            if char_start < start {
                self.src[char_start..start]
                    .chars()
                    .next()
                    .map(|c| c.is_whitespace())
                    .unwrap_or(false)
            } else {
                false
            }
        } else {
            false
        };
        let has_space_after = self
            .current_char()
            .map(|c| c.is_whitespace())
            .unwrap_or(false);
        has_space_before && has_space_after
    }

    fn recognize_string(&mut self, start: Index) -> LexResult<Token> {
        self.advance();
        self.recovery_delimiter = Some('"');
        while let Some(c) = self.current_char() {
            match c {
                '"' => {
                    self.advance();
                    self.recovery_delimiter = None;
                    return Ok(Token::new(TokenKind::Str, start, self.cursor));
                }
                '\\' => self.consume_escape(start, '"')?,
                '$' if self.peek_char() == Some('{') => self.consume_interpolation(start)?,
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
        Err(LexError::UnterminatedString {
            start: start as u32,
            message: "Unterminated string literal, reached end of file".to_string(),
        })
    }

    /// Keep a braced interpolation inside its containing string token. The
    /// ordinary token scanner supplies structural boundaries, so braces in
    /// nested strings, characters or comments cannot close the interpolation.
    /// Parsing and evaluation of the embedded expression remain deferred.
    fn consume_interpolation(&mut self, string_start: Index) -> LexResult<()> {
        self.advance(); // $
        self.advance(); // {
        let previous_token = self.previous_token;
        let can_end_expression = self.can_end_expression;
        let recovery_delimiter = self.recovery_delimiter;
        self.previous_token = None;
        self.can_end_expression = false;
        self.recovery_delimiter = None;

        let result = (|| {
            let mut depth = 1;
            loop {
                match self.next()?.kind {
                    TokenKind::LBrace => depth += 1,
                    TokenKind::RBrace => {
                        depth -= 1;
                        if depth == 0 {
                            return Ok(());
                        }
                    }
                    TokenKind::Eof => {
                        return Err(LexError::UnterminatedString {
                            start: string_start as u32,
                            message: "Unterminated string interpolation, expected '}'".to_string(),
                        });
                    }
                    _ => {}
                }
            }
        })();

        self.previous_token = previous_token;
        self.can_end_expression = can_end_expression;
        self.recovery_delimiter = recovery_delimiter;
        result
    }

    fn recognize_quote_or_char_or_macro(&mut self, start: Index) -> LexResult<Token> {
        self.advance();
        if self.previous_token == Some(TokenKind::RBrace) && self.starts_closed_character() {
            // A completed statement ending in } omits its semicolon, so a
            // character can start the next statement here. Keep the complete
            // lexeme: only the parser can tell that {}'a'b in an expression
            // continues with two views instead of a new suffixed character.
            return self.recognize_char_content(start);
        }
        if self.can_end_expression {
            return Ok(Token::new(TokenKind::Quote, start, self.cursor));
        }

        if self.current_char() == Some('\'') {
            self.recovery_delimiter = Some('\'');
            return Err(LexError::EmptyChar {
                start: start as u32,
                message: "Empty character literal".to_string(),
            });
        }
        // Check a complete single scalar before whitespace or quote-form
        // lookahead: ' ', '{', '(' and Unicode letters are all characters.
        if self.current_char() == Some('\\') || self.peek_char() == Some('\'') {
            return self.recognize_char_content(start);
        }

        let next = self.remaining.trim_start();
        if next.chars().next().is_some_and(is_identifier_start)
            && next
                .chars()
                .skip_while(|&c| is_identifier_continue(c))
                .next()
                != Some('\'')
        {
            // Type-pattern expressions also use 'id. A closed identifier
            // such as 'ab' is instead an invalid multi-scalar character.
            return Ok(Token::new(TokenKind::Quote, start, self.cursor));
        }
        match next.chars().next() {
            Some('{') => {
                self.cursor += self.remaining.len() - next.len();
                self.remaining = next;
                self.recognize_macro_content(start)
            }
            Some('(' | '[' | '<') | None => Ok(Token::new(TokenKind::Quote, start, self.cursor)),
            _ => self.recognize_char_content(start),
        }
    }

    fn starts_closed_character(&self) -> bool {
        let mut chars = self.remaining.chars();
        match chars.next() {
            Some('\\') => {
                chars.next(); // The escaped character may itself be a quote.
                chars.take_while(|&c| c != '\n').any(|c| c == '\'')
            }
            Some('\'') | None => false,
            Some(_) => chars.next() == Some('\''),
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

    /// Read one Unicode scalar or one documented escape, followed by a quote.
    fn recognize_char_content(&mut self, start: Index) -> LexResult<Token> {
        self.recovery_delimiter = Some('\'');
        match self.current_char() {
            Some('\\') => self.consume_escape(start, '\'')?,
            Some(_) => {
                self.advance();
            }
            None => {
                return Err(LexError::UnterminatedChar {
                    start: start as u32,
                    message: "Unterminated character literal".to_string(),
                });
            }
        }
        if self.current_char() == Some('\'') {
            self.advance();
            self.recovery_delimiter = None;
            Ok(Token::new(TokenKind::Char, start, self.cursor))
        } else {
            let has_closing_quote = self
                .remaining
                .chars()
                .take_while(|&c| c != '\n')
                .any(|c| c == '\'');
            if has_closing_quote {
                Err(LexError::InvalidChar {
                    start: start as u32,
                    message: "Character literal must contain exactly one Unicode scalar"
                        .to_string(),
                })
            } else {
                Err(LexError::UnterminatedChar {
                    start: start as u32,
                    message: "Unterminated character literal, expected closing quote".to_string(),
                })
            }
        }
    }

    /// Consume the same escape language for strings and characters.
    fn consume_escape(&mut self, literal_start: Index, delimiter: char) -> LexResult<()> {
        let escape_start = self.cursor as u32;
        self.advance(); // backslash
        let Some(escape_char) = self.current_char() else {
            return Err(if delimiter == '"' {
                LexError::UnterminatedString {
                    start: literal_start as u32,
                    message: "Unterminated string literal with escape at end of file".to_string(),
                }
            } else {
                LexError::UnterminatedChar {
                    start: literal_start as u32,
                    message: "Unterminated character literal with escape at end of file"
                        .to_string(),
                }
            });
        };
        match escape_char {
            'n' | 't' | 'r' | '\\' | '\'' | '"' | '0' | '$' => {
                self.advance();
                Ok(())
            }
            'x' => {
                self.advance();
                if self.current_char() != Some('{') {
                    return Err(LexError::InvalidEscape {
                        start: escape_start,
                        escape_char,
                        message: "Unicode escape sequence must use \\x{...}".to_string(),
                    });
                }
                self.advance();
                self.consume_unicode_escape(escape_start)
            }
            _ => Err(LexError::InvalidEscape {
                start: escape_start,
                escape_char,
                message: format!("Invalid escape sequence '\\{}'", escape_char),
            }),
        }
    }

    fn consume_unicode_escape(&mut self, escape_start: u32) -> LexResult<()> {
        let mut digits = 0;
        let mut scalar = 0u32;
        while let Some(c) = self.current_char() {
            if c.is_ascii_hexdigit() {
                digits += 1;
                if digits > 6 {
                    break;
                }
                scalar = scalar * 16 + c.to_digit(16).unwrap();
                self.advance();
            } else {
                break;
            }
        }
        let message = if digits == 0 {
            "Unicode escape must contain 1–6 hexadecimal digits"
        } else if digits > 6 {
            "Unicode escape must contain at most 6 hexadecimal digits"
        } else if self.current_char() != Some('}') {
            "Unicode escape must end with '}' after its hexadecimal digits"
        } else if char::from_u32(scalar).is_none() {
            "Unicode escape must be a Unicode scalar (not a surrogate or above U+10FFFF)"
        } else {
            self.advance();
            return Ok(());
        };
        Err(LexError::InvalidEscape {
            start: escape_start,
            escape_char: 'x',
            message: message.to_string(),
        })
    }

    fn recognize_number(&mut self, start: Index) -> LexResult<Token> {
        if self.current_char() == Some('0') {
            let radix = match self.peek_char() {
                Some('b') => Some((2, TokenKind::IntBin)),
                Some('o') => Some((8, TokenKind::IntOct)),
                Some('x') => Some((16, TokenKind::IntHex)),
                Some('B' | 'O' | 'X') => {
                    return Err(LexError::InvalidNumber {
                        start: start as u32,
                        message: "Radix prefixes must be lowercase: 0b, 0o or 0x".to_string(),
                    });
                }
                _ => None,
            };
            if let Some((radix, kind)) = radix {
                self.advance();
                self.advance();
                if self
                    .current_char()
                    .is_none_or(|c| !c.is_alphanumeric() && c != '_')
                {
                    let prefix = match radix {
                        2 => TokenKind::BitBinPrefix,
                        8 => TokenKind::BitOctPrefix,
                        _ => TokenKind::BitHexPrefix,
                    };
                    return Ok(Token::new(prefix, start, self.cursor));
                }
                self.consume_digits(start, radix)?;
                return Ok(Token::new(kind, start, self.cursor));
            }
        }
        self.consume_digits(start, 10)?;
        let mut kind = TokenKind::Int;
        // Leave projection/range punctuation to the parser (1.foo, 1..2).
        if self.current_char() == Some('.') && self.peek_char().is_some_and(|c| c.is_ascii_digit())
        {
            self.advance();
            self.consume_digits(start, 10)?;
            kind = TokenKind::Real;
        }
        // An arbitrary identifier, including e, E and egg, is a suffix. A
        // digit/sign/underscore after e/E instead begins an exponent attempt.
        if matches!(self.current_char(), Some('e' | 'E'))
            && self
                .peek_char()
                .is_some_and(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '_'))
        {
            if kind == TokenKind::Int {
                return Err(LexError::InvalidNumber {
                    start: start as u32,
                    message: "Scientific notation requires a complete decimal before the exponent, e.g. 1.0e6".to_string(),
                });
            }
            self.advance();
            if matches!(self.current_char(), Some('+' | '-')) {
                self.advance();
            }
            self.consume_digits(start, 10)?;
            kind = TokenKind::RealSci;
        }
        Ok(Token::new(kind, start, self.cursor))
    }

    /// Require digits at both ends; underscores may repeat only between them.
    fn consume_digits(&mut self, start: Index, radix: u32) -> LexResult<()> {
        let mut has_digits = false;
        let mut last_underscore = false;
        let mut hex_case = None;
        while let Some(c) = self.current_char() {
            if c == '_' {
                if !has_digits {
                    return Err(LexError::InvalidNumber {
                        start: start as u32,
                        message: "Number separators must occur between digits".to_string(),
                    });
                }
                last_underscore = true;
                self.advance();
            } else if c.is_ascii() && c.is_digit(radix) {
                if radix == 16 && c.is_ascii_alphabetic() {
                    let uppercase = c.is_ascii_uppercase();
                    if hex_case.is_some_and(|previous| previous != uppercase) {
                        return Err(LexError::InvalidNumber {
                            start: start as u32,
                            message: "Hexadecimal digits must use uniformly lowercase or uppercase letters".to_string(),
                        });
                    }
                    hex_case = Some(uppercase);
                }
                has_digits = true;
                last_underscore = false;
                self.advance();
            } else if c.is_ascii_digit() {
                return Err(LexError::InvalidNumber {
                    start: start as u32,
                    message: format!("Digit '{c}' is invalid in a base-{radix} number"),
                });
            } else {
                break;
            }
        }
        if !has_digits || last_underscore {
            return Err(LexError::InvalidNumber {
                start: start as u32,
                message: if has_digits {
                    "Number separators must occur between digits".to_string()
                } else {
                    format!("Base-{radix} number must contain at least one digit")
                },
            });
        }
        Ok(())
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

        let token_kind = self.keywords.get(text).copied().unwrap_or(TokenKind::Id);

        Ok(Token::new(token_kind, start, self.cursor))
    }

    /// Recover from error by skipping the current character
    pub fn recover_from_error(&mut self) {
        self.can_end_expression = false;
        if let Some(delimiter) = self.recovery_delimiter.take() {
            // An invalid escape must not make the closing quote start a new
            // literal and swallow subsequent tokens during recovery.
            while let Some(c) = self.current_char() {
                self.advance();
                if c == delimiter || c == '\n' {
                    break;
                }
                if c == '\\' {
                    self.advance();
                }
            }
        } else if !self.is_eof() {
            self.advance();
        }
    }

    /// Recognize block comment {- ... -}
    fn recognize_block_comment(&mut self, start: Index) -> LexResult<Token> {
        // Consume '{-'
        self.advance(); // {
        self.advance(); // -
        let mut depth = 1;

        while let Some(c) = self.current_char() {
            if c == '{' && self.peek_char() == Some('-') {
                self.advance();
                self.advance();
                depth += 1;
            } else if c == '-' && self.peek_char() == Some('}') {
                self.advance();
                self.advance();
                depth -= 1;
                if depth == 0 {
                    return Ok(Token::new(TokenKind::Comment, start, self.cursor));
                }
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
