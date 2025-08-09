use crate::diagnostic::{DiagnosticContext, FlurryError};

// Scope scanning error codes
/// Base error code for scan pass errors.
pub const SCAN_ERROR_BASE: u32 = 3000;

/// Result type returned by scan operations, wrapping either a value or a `ScanError`.
pub type ScanResult<T> = Result<T, ScanError>;

/// Errors that can occur during the scan pass, including scope creation, AST and file parsing, and import resolution errors.
#[derive(Debug, Clone)]
pub enum ScanError {
    ScopeCreationFailed {
        message: String,
        span: rustc_span::Span,
    },
    ModuleNotFound {
        message: String,
        span: rustc_span::Span,
    },
    InvalidNodeType {
        message: String,
        span: rustc_span::Span,
    },
    FileParsingFailed {
        message: String,
        span: rustc_span::Span,
    },
    UnresolvedIdentifier {
        message: String,
        span: rustc_span::Span,
    },
    CyclicImport {
        message: String,
        span: rustc_span::Span,
    },
    InternalError(String),
}

impl ScanError {
    pub fn message(&self) -> &str {
        match self {
            ScanError::ScopeCreationFailed { message, .. } => message,
            ScanError::ModuleNotFound { message, .. } => message,
            ScanError::InvalidNodeType { message, .. } => message,
            ScanError::FileParsingFailed { message, .. } => message,
            ScanError::UnresolvedIdentifier { message, .. } => message,
            ScanError::CyclicImport { message, .. } => message,
            ScanError::InternalError(message) => message,
        }
    }

    pub fn to_span(&self) -> rustc_span::Span {
        match self {
            ScanError::ScopeCreationFailed { span, .. } => *span,
            ScanError::ModuleNotFound { span, .. } => *span,
            ScanError::InvalidNodeType { span, .. } => *span,
            ScanError::FileParsingFailed { span, .. } => *span,
            ScanError::UnresolvedIdentifier { span, .. } => *span,
            ScanError::CyclicImport { span, .. } => *span,
            ScanError::InternalError(_) => rustc_span::DUMMY_SP,
        }
    }
}

impl FlurryError for ScanError {
    fn error_code(&self) -> u32 {
        match self {
            ScanError::ScopeCreationFailed { .. } => SCAN_ERROR_BASE + 1,
            ScanError::ModuleNotFound { .. } => SCAN_ERROR_BASE + 2,
            ScanError::InvalidNodeType { .. } => SCAN_ERROR_BASE + 3,
            ScanError::FileParsingFailed { .. } => SCAN_ERROR_BASE + 4,
            ScanError::UnresolvedIdentifier { .. } => SCAN_ERROR_BASE + 5,
            ScanError::CyclicImport { .. } => SCAN_ERROR_BASE + 6,
            ScanError::InternalError(_) => SCAN_ERROR_BASE + 7,
        }
    }

    fn error_name(&self) -> &'static str {
        match self {
            ScanError::ScopeCreationFailed { .. } => "scope_creation_failed",
            ScanError::ModuleNotFound { .. } => "module_not_found",
            ScanError::InvalidNodeType { .. } => "invalid_node_type",
            ScanError::FileParsingFailed { .. } => "file_parsing_failed",
            ScanError::UnresolvedIdentifier { .. } => "unresolved_identifier",
            ScanError::CyclicImport { .. } => "cyclic_import",
            ScanError::InternalError(_) => "internal_error",
        }
    }

    fn emit(&self, diag_ctx: &DiagnosticContext, offset: rustc_span::BytePos) {
        let span = self.to_span();

        diag_ctx
            .error(self.message().to_string())
            .with_code(self.error_code())
            .with_error_label(span, self.message().to_string())
            .with_primary_span(span)
            .emit(diag_ctx);
    }
}
