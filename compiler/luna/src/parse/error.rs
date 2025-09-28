use crate::{
    diagnostic::{DiagnosticContext, FlurryError},
    lex::TokenKind,
};

pub const PARSE_ERROR_BASE: u32 = 2000;

#[derive(Debug, Clone)]
pub enum ParseError {
    UnexpectedToken {
        message: String,
        expected: TokenKind,
        found: TokenKind,
        span: rustc_span::Span,
    },
    InvalidSyntax {
        message: String,
        found: TokenKind,
        span: rustc_span::Span,
    },

    // 这两个仅用于控制流, 非错误
    MeetPostObjectStart,
    MeetPostId,
}

impl ParseError {
    pub fn message(&self) -> &str {
        match self {
            ParseError::UnexpectedToken { message, .. } => message,
            ParseError::InvalidSyntax { message, .. } => message,
            ParseError::MeetPostObjectStart => {
                "Received unexpected MeetPostObjectStart, this is a bug"
            }
            ParseError::MeetPostId => "Received unexpected MeetPostId, this is a bug",
        }
    }

    pub fn unexpected_token(expected: TokenKind, found: TokenKind, span: rustc_span::Span) -> Self {
        ParseError::UnexpectedToken {
            message: format!("Expected `{}`, found `{}`", expected.lexme(), found.lexme()),
            expected,
            found,
            span,
        }
    }

    pub fn invalid_syntax(message: String, found: TokenKind, span: rustc_span::Span) -> Self {
        ParseError::InvalidSyntax {
            message,
            found,
            span,
        }
    }

    pub fn to_span(&self) -> rustc_span::Span {
        match self {
            ParseError::UnexpectedToken { span, .. } => span.clone(),
            ParseError::InvalidSyntax { span, .. } => span.clone(),
            ParseError::MeetPostObjectStart => rustc_span::DUMMY_SP,
            ParseError::MeetPostId => rustc_span::DUMMY_SP,
        }
    }
}

impl FlurryError for ParseError {
    fn error_code(&self) -> u32 {
        match self {
            ParseError::UnexpectedToken { .. } => PARSE_ERROR_BASE + 1,
            ParseError::InvalidSyntax { .. } => PARSE_ERROR_BASE + 2,
            ParseError::MeetPostObjectStart => PARSE_ERROR_BASE + 3,
            ParseError::MeetPostId => PARSE_ERROR_BASE + 4,
        }
    }

    fn emit(&self, diag_ctx: &DiagnosticContext, _base_pos: rustc_span::BytePos) {
        let span = self.to_span();

        diag_ctx
            .error(self.message().to_string())
            .with_code(self.error_code())
            .with_error_label(span, self.message().to_string())
            .with_primary_span(span)
            .emit(diag_ctx);
    }

    fn error_name(&self) -> &'static str {
        match self {
            ParseError::UnexpectedToken { .. } => "unexpected_token",
            ParseError::InvalidSyntax { .. } => "invalid_syntax",
            ParseError::MeetPostObjectStart => "meet_post_object_start",
            ParseError::MeetPostId => "meet_post_id",
        }
    }
}
