use crate::{diagnostic::FlurryError, lex::TokenKind};

pub const PARSE_ERROR_BASE: u32 = 2000;

#[derive(Debug, Clone)]
pub enum ParseError {
    UnexpectedToken {
        message: String,
        expected: TokenKind,
        found: TokenKind,
        span: rustc_span::Span,
    },
}

impl ParseError {
    pub fn message(&self) -> &str {
        match self {
            ParseError::UnexpectedToken { message, .. } => message,
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

    pub fn to_span(&self) -> rustc_span::Span {
        match self {
            ParseError::UnexpectedToken { span, .. } => span.clone(),
        }
    }
}

impl FlurryError for ParseError {
    fn error_code(&self) -> u32 {
        match self {
            ParseError::UnexpectedToken { .. } => PARSE_ERROR_BASE + 1,
        }
    }
    
    fn emit(
        &self,
        diag_ctx: &mut crate::diagnostic::DiagnosticContext,
        _base_pos: rustc_span::BytePos,
    ) {
        let span = self.to_span();
        
        diag_ctx.error(self.message().to_string())
            .with_code(self.error_code())
            .with_primary_span(span)
            .with_error_label(span, "Unexpected token".to_string())
            .emit(diag_ctx);
    }

    fn error_name(&self) -> &'static str {
        match self {
            ParseError::UnexpectedToken { .. } => "unexpected_token",
        }
    }
}
