use crate::diagnostic::{DiagnosticContext, FlurryError};

// AST lowering error codes
pub const LOWER_ERROR_BASE: u32 = 4000;

#[derive(Debug, Clone)]
pub enum LowerError {
    LiteralError {
        message: String,
        span: rustc_span::Span,
    },
    UnresolvedIdentifier {
        message: String,
        span: rustc_span::Span,
    },
    InternalError(String),
    ScopeError(String),
}

impl LowerError {
    pub fn message(&self) -> &str {
        match self {
            LowerError::LiteralError { message, .. } => message,
            LowerError::UnresolvedIdentifier { message, .. } => message,
            LowerError::InternalError(message) => message,
            LowerError::ScopeError(message) => message,
        }
    }

    pub fn to_span(&self) -> rustc_span::Span {
        match self {
            LowerError::LiteralError { span, .. } => *span,
            LowerError::UnresolvedIdentifier { span, .. } => *span,
            LowerError::InternalError(_) => rustc_span::DUMMY_SP,
            LowerError::ScopeError(_) => rustc_span::DUMMY_SP,
        }
    }
}

impl FlurryError for LowerError {
    fn error_code(&self) -> u32 {
        match self {
            LowerError::LiteralError { .. } => LOWER_ERROR_BASE + 1,
            LowerError::UnresolvedIdentifier { .. } => LOWER_ERROR_BASE + 2,
            LowerError::InternalError(_) => LOWER_ERROR_BASE + 3,
            LowerError::ScopeError(_) => LOWER_ERROR_BASE + 4,
        }
    }

    fn error_name(&self) -> &'static str {
        match self {
            LowerError::LiteralError { .. } => "literal_error",
            LowerError::UnresolvedIdentifier { .. } => "unresolved_identifier",
            LowerError::InternalError(_) => "internal_error",
            LowerError::ScopeError(_) => "scope_error",
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
}
