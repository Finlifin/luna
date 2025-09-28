use diagnostic::{DiagnosticContext, FlurryError};
use rustc_span::{BytePos, Span};
use std::fmt;

/// Global error codes for the lexer
pub const LEX_ERROR_BASE: u32 = 1000;
pub const LEX_UNTERMINATED_STRING: u32 = LEX_ERROR_BASE + 1;
pub const LEX_UNTERMINATED_CHAR: u32 = LEX_ERROR_BASE + 2;
pub const LEX_UNTERMINATED_COMMENT: u32 = LEX_ERROR_BASE + 3;
pub const LEX_UNTERMINATED_MACRO: u32 = LEX_ERROR_BASE + 4;
pub const LEX_INVALID_ESCAPE: u32 = LEX_ERROR_BASE + 5;
pub const LEX_INVALID_NUMBER: u32 = LEX_ERROR_BASE + 6;
pub const LEX_UNEXPECTED_CHAR: u32 = LEX_ERROR_BASE + 7;
pub const LEX_EMPTY_CHAR: u32 = LEX_ERROR_BASE + 8;

/// Lexer error types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LexError {
    /// Unterminated string literal
    UnterminatedString { start: u32, message: String },
    /// Unterminated character literal
    UnterminatedChar { start: u32, message: String },
    /// Unterminated comment
    UnterminatedComment { start: u32, message: String },
    /// Unterminated macro content
    UnterminatedMacro { start: u32, message: String },
    /// Invalid escape sequence
    InvalidEscape {
        start: u32,
        escape_char: char,
        message: String,
    },
    /// Invalid number format
    InvalidNumber { start: u32, message: String },
    /// Unexpected character
    UnexpectedChar {
        position: u32,
        char: char,
        message: String,
    },
    /// Empty character literal
    EmptyChar { start: u32, message: String },
}

impl LexError {
    pub fn message(&self) -> &str {
        match self {
            LexError::UnterminatedString { message, .. } => message,
            LexError::UnterminatedChar { message, .. } => message,
            LexError::UnterminatedComment { message, .. } => message,
            LexError::UnterminatedMacro { message, .. } => message,
            LexError::InvalidEscape { message, .. } => message,
            LexError::InvalidNumber { message, .. } => message,
            LexError::UnexpectedChar { message, .. } => message,
            LexError::EmptyChar { message, .. } => message,
        }
    }

    pub fn start_position(&self) -> u32 {
        match self {
            LexError::UnterminatedString { start, .. } => *start,
            LexError::UnterminatedChar { start, .. } => *start,
            LexError::UnterminatedComment { start, .. } => *start,
            LexError::UnterminatedMacro { start, .. } => *start,
            LexError::InvalidEscape { start, .. } => *start,
            LexError::InvalidNumber { start, .. } => *start,
            LexError::UnexpectedChar { position, .. } => *position,
            LexError::EmptyChar { start, .. } => *start,
        }
    }

    pub fn to_span(&self, base_pos: BytePos) -> Span {
        let start = self.start_position();
        let end = match self {
            // 对于未终止的字符串，只高亮开始的引号
            LexError::UnterminatedString { .. } => start + 1,
            // 对于未终止的字符，只高亮开始的引号
            LexError::UnterminatedChar { .. } => start + 1,
            // 对于空字符，高亮整个 ''
            LexError::EmptyChar { .. } => start + 2,
            // 对于无效转义，高亮转义序列 (如 \q 为 2 个字符)
            LexError::InvalidEscape { .. } => start + 2,
            // 对于无效数字，只高亮开始位置
            LexError::InvalidNumber { .. } => start + 1,
            // 对于意外字符，高亮整个字符（考虑Unicode）
            LexError::UnexpectedChar { .. } => start + 1,
            // 对于未终止的注释，高亮注释开始
            LexError::UnterminatedComment { .. } => start + 2,
            // 对于未终止的宏，高亮宏开始 '{
            LexError::UnterminatedMacro { .. } => start + 2,
        };
        Span::new(BytePos(base_pos.0 + start), BytePos(base_pos.0 + end))
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for LexError {}

impl FlurryError for LexError {
    fn error_code(&self) -> u32 {
        match self {
            LexError::UnterminatedString { .. } => LEX_UNTERMINATED_STRING,
            LexError::UnterminatedChar { .. } => LEX_UNTERMINATED_CHAR,
            LexError::UnterminatedComment { .. } => LEX_UNTERMINATED_COMMENT,
            LexError::UnterminatedMacro { .. } => LEX_UNTERMINATED_MACRO,
            LexError::InvalidEscape { .. } => LEX_INVALID_ESCAPE,
            LexError::InvalidNumber { .. } => LEX_INVALID_NUMBER,
            LexError::UnexpectedChar { .. } => LEX_UNEXPECTED_CHAR,
            LexError::EmptyChar { .. } => LEX_EMPTY_CHAR,
        }
    }

    fn error_name(&self) -> &'static str {
        match self {
            LexError::UnterminatedString { .. } => "unterminated_string",
            LexError::UnterminatedChar { .. } => "unterminated_char",
            LexError::UnterminatedComment { .. } => "unterminated_comment",
            LexError::UnterminatedMacro { .. } => "unterminated_macro",
            LexError::InvalidEscape { .. } => "invalid_escape",
            LexError::InvalidNumber { .. } => "invalid_number",
            LexError::UnexpectedChar { .. } => "unexpected_char",
            LexError::EmptyChar { .. } => "empty_char",
        }
    }

    fn emit(&self, diag_ctx: &DiagnosticContext, base_pos: rustc_span::BytePos) {
        let span = self.to_span(base_pos);
        let error_code = self.error_code();

        match self {
            LexError::UnterminatedString { .. } => {
                diag_ctx
                    .error(self.message().to_string())
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, "String literal is not terminated".to_string())
                    .with_help(
                        "Add a closing quote (\") to terminate the string literal".to_string(),
                    )
                    .emit(diag_ctx);
            }
            LexError::UnterminatedChar { .. } => {
                diag_ctx
                    .error(self.message().to_string())
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, "Character literal is not terminated".to_string())
                    .with_help(
                        "Add a closing single quote (') to terminate the character literal"
                            .to_string(),
                    )
                    .emit(diag_ctx);
            }
            LexError::EmptyChar { .. } => {
                diag_ctx
                    .error(self.message().to_string())
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, "Character literal cannot be empty".to_string())
                    .with_help(
                        "Add a character between the single quotes, e.g., 'a' or '\\n'".to_string(),
                    )
                    .emit(diag_ctx);
            }
            LexError::InvalidEscape { escape_char, .. } => {
                diag_ctx.error(self.message().to_string())
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, format!("Invalid escape sequence '\\{}'", escape_char))
                    .with_help("Valid escape sequences are: \\n, \\t, \\r, \\\\, \\', \\\", \\0, \\a, \\b, \\f, \\v, \\x{...}, \\u{...}".to_string())
                    .emit(diag_ctx);
            }
            LexError::InvalidNumber { .. } => {
                diag_ctx
                    .error(self.message().to_string())
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, "Invalid number format".to_string())
                    .with_help(
                        "Make sure decimal numbers have digits after the decimal point".to_string(),
                    )
                    .emit(diag_ctx);
            }
            LexError::UnexpectedChar { char, .. } => {
                diag_ctx
                    .error(self.message().to_string())
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, format!("Unexpected character '{}'", char))
                    .with_help("Remove or replace this character with a valid token".to_string())
                    .emit(diag_ctx);
            }
            LexError::UnterminatedComment { .. } => {
                diag_ctx
                    .error(self.message().to_string())
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, "Comment is not terminated".to_string())
                    .with_help("Add a comment terminator".to_string())
                    .emit(diag_ctx);
            }
            LexError::UnterminatedMacro { .. } => {
                diag_ctx
                    .error(self.message().to_string())
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, "Macro content is not terminated".to_string())
                    .with_help("Add a closing brace '}' to terminate the macro content".to_string())
                    .emit(diag_ctx);
            }
        }
    }
}

/// Result type for lexer operations
pub type LexResult<T> = Result<T, LexError>;
