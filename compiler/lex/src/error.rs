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
pub const LEX_INVALID_CHAR: u32 = LEX_ERROR_BASE + 9;

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
    /// Character literal containing more than one Unicode scalar
    InvalidChar { start: u32, message: String },
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
            LexError::InvalidChar { message, .. } => message,
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
            LexError::InvalidChar { start, .. } => *start,
        }
    }

    pub fn to_span(&self, base_pos: BytePos) -> Span {
        let start = self.start_position();
        let end = match self {
            LexError::UnterminatedString { .. } => start + 1,
            LexError::UnterminatedChar { .. } => start + 1,
            LexError::EmptyChar { .. } => start + 2,
            LexError::InvalidChar { .. } => start + 1,
            LexError::InvalidEscape { escape_char, .. } => {
                start + 1 + escape_char.len_utf8() as u32
            }
            LexError::InvalidNumber { .. } => start + 1,
            LexError::UnexpectedChar { char, .. } => start + char.len_utf8() as u32,
            LexError::UnterminatedComment { .. } => start + 2,
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
            LexError::InvalidChar { .. } => LEX_INVALID_CHAR,
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
            LexError::InvalidChar { .. } => "invalid_char",
        }
    }

    fn emit(&self, diag_ctx: &DiagnosticContext, base_pos: BytePos) {
        let span = self.to_span(base_pos);
        let error_code = self.error_code();

        match self {
            LexError::UnterminatedString { message, .. } => {
                diag_ctx
                    .error(format!(
                        "this string literal is missing its closing `\"` ({message})"
                    ))
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(
                        span,
                        "string literal starts here and never closes".to_string(),
                    )
                    .with_help(
                        "Add a closing `\"` to finish the string, or use `\"\"\"...\"\"\"` for multi-line text."
                            .to_string(),
                    )
                    .emit(diag_ctx);
            }
            LexError::UnterminatedChar { message, .. } => {
                diag_ctx
                    .error(format!(
                        "this character literal is missing its closing `'` ({message})"
                    ))
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, "unclosed character literal".to_string())
                    .with_help(
                        "Close a single-character literal with `'`, or use `\"...\"` for a string."
                            .to_string(),
                    )
                    .emit(diag_ctx);
            }
            LexError::EmptyChar { .. } => {
                diag_ctx
                    .error("empty character literal `''`".to_string())
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(
                        span,
                        "expected one Unicode scalar inside `'...'`".to_string(),
                    )
                    .with_help(
                        "Put a single character inside the quotes, such as `'a'` or `'\\n'`, or write `\"\"` for an empty string."
                            .to_string(),
                    )
                    .emit(diag_ctx);
            }
            LexError::InvalidChar { .. } => {
                diag_ctx
                    .error("this `'...'` literal holds more than one character".to_string())
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(
                        span,
                        "character literals hold a single Unicode scalar".to_string(),
                    )
                    .with_help(
                        "Switch to double quotes `\"...\"` for a multi-character string literal."
                            .to_string(),
                    )
                    .emit(diag_ctx);
            }
            LexError::InvalidEscape {
                escape_char,
                message,
                ..
            } => {
                diag_ctx
                    .error(format!(
                        "unrecognized escape sequence `\\{escape_char}` ({message})"
                    ))
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(
                        span,
                        format!("unknown or malformed escape `\\{escape_char}`"),
                    )
                    .with_help(
                        "Supported escapes are `\\n`, `\\t`, `\\r`, `\\\\`, `\\'`, `\\\"`, `\\0`, `\\$`, and `\\x{...}` (1–6 hex digits)."
                            .to_string(),
                    )
                    .emit(diag_ctx);
            }
            LexError::InvalidNumber { message, .. } => {
                let help = if message.contains("Scientific notation") {
                    "Include a decimal part before the exponent, for example `1.0e6` instead of `1e6`."
                } else if message.contains("Radix prefixes") {
                    "Write base prefixes in lowercase: `0b` (binary), `0o` (octal), or `0x` (hex)."
                } else if message.contains("Hexadecimal digits") {
                    "Pick either all lowercase (`0x1a2f`) or all uppercase (`0x1A2F`) hex letters."
                } else if message.contains("separators") {
                    "Place `_` only between digits, such as `1_000_000` (not at the start or end)."
                } else {
                    "Check that every digit matches the number's base and that `.` is followed by digits."
                };
                diag_ctx
                    .error(format!("malformed number literal: {message}"))
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, "malformed number literal".to_string())
                    .with_help(help.to_string())
                    .emit(diag_ctx);
            }
            LexError::UnexpectedChar { char, .. } => {
                let help = match char {
                    '`' => {
                        "Flurry doesn't use backticks — use `\"...\"` for strings or `'a'` for characters."
                    }
                    '\\' => {
                        "A bare `\\` isn't used for line continuations in Flurry; expressions can span lines naturally."
                    }
                    _ => "Remove this character or replace it with valid Flurry syntax.",
                };
                diag_ctx
                    .error(format!(
                        "unexpected character `{char}` is not part of Flurry syntax"
                    ))
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, format!("unexpected `{char}` here"))
                    .with_help(help.to_string())
                    .emit(diag_ctx);
            }
            LexError::UnterminatedComment { .. } => {
                diag_ctx
                    .error(
                        "reached the end of the file inside an unclosed `{- ... -}` block comment"
                            .to_string(),
                    )
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, "this `{-` block comment was never closed".to_string())
                    .with_help(
                        "Add a matching `-}` to close the block comment (note that `{- ... -}` comments nest)."
                            .to_string(),
                    )
                    .emit(diag_ctx);
            }
            LexError::UnterminatedMacro { .. } => {
                diag_ctx
                    .error(
                        "reached the end of the file inside unclosed `'{ ... }` macro content"
                            .to_string(),
                    )
                    .with_code(error_code)
                    .with_primary_span(span)
                    .with_error_label(span, "this `'{` macro block was never closed".to_string())
                    .with_help("Add a matching `}` to close the `'{ ... }` block.".to_string())
                    .emit(diag_ctx);
            }
        }
    }
}

/// Result type for lexer operations
pub type LexResult<T> = Result<T, LexError>;
