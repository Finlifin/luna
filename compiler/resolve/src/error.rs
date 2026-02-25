//! Resolve errors and diagnostics.

use diagnostic::{DiagnosticContext, FlurryError};
use rustc_span::Span;

/// Convenience alias.
pub type ResolveResult<T> = Result<T, ResolveError>;

/// Base error code for name-resolution errors.
pub const RESOLVE_ERROR_BASE: u32 = 4000;

/// Errors that can occur during early name resolution.
#[derive(Debug, Clone)]
pub enum ResolveError {
    /// A scope could not be created (e.g. duplicate module name).
    ScopeCreationFailed {
        message: String,
        span: Span,
    },
    /// A module referenced in a path could not be found.
    ModuleNotFound {
        message: String,
        span: Span,
    },
    /// An unexpected AST node kind was encountered.
    InvalidNodeType {
        message: String,
        span: Span,
    },
    /// File reading or lexing failed.
    FileParsingFailed {
        message: String,
        span: Span,
    },
    /// An identifier could not be resolved in the current scope chain.
    UnresolvedName {
        name: String,
        span: Span,
    },
    /// A cyclic import was detected.
    CyclicImport {
        message: String,
        span: Span,
    },
    /// Duplicate definition of the same name in the same namespace.
    DuplicateDefinition {
        name: String,
        first_span: Span,
        second_span: Span,
    },
    /// An `use` path segment could not be resolved.
    UnresolvedImportSegment {
        segment: String,
        span: Span,
    },
    /// Generic internal error.
    InternalError(String),
}

impl ResolveError {
    pub fn message(&self) -> String {
        match self {
            Self::ScopeCreationFailed { message, .. } => message.clone(),
            Self::ModuleNotFound { message, .. } => message.clone(),
            Self::InvalidNodeType { message, .. } => message.clone(),
            Self::FileParsingFailed { message, .. } => message.clone(),
            Self::UnresolvedName { name, .. } => {
                format!("unresolved name `{}`", name)
            }
            Self::CyclicImport { message, .. } => message.clone(),
            Self::DuplicateDefinition { name, .. } => {
                format!("duplicate definition of `{}`", name)
            }
            Self::UnresolvedImportSegment { segment, .. } => {
                format!("unresolved import path segment `{}`", segment)
            }
            Self::InternalError(msg) => msg.clone(),
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Self::ScopeCreationFailed { span, .. }
            | Self::ModuleNotFound { span, .. }
            | Self::InvalidNodeType { span, .. }
            | Self::FileParsingFailed { span, .. }
            | Self::UnresolvedName { span, .. }
            | Self::CyclicImport { span, .. }
            | Self::UnresolvedImportSegment { span, .. } => *span,
            Self::DuplicateDefinition { second_span, .. } => *second_span,
            Self::InternalError(_) => rustc_span::DUMMY_SP,
        }
    }
}

impl FlurryError for ResolveError {
    fn error_code(&self) -> u32 {
        match self {
            Self::ScopeCreationFailed { .. } => RESOLVE_ERROR_BASE + 1,
            Self::ModuleNotFound { .. } => RESOLVE_ERROR_BASE + 2,
            Self::InvalidNodeType { .. } => RESOLVE_ERROR_BASE + 3,
            Self::FileParsingFailed { .. } => RESOLVE_ERROR_BASE + 4,
            Self::UnresolvedName { .. } => RESOLVE_ERROR_BASE + 5,
            Self::CyclicImport { .. } => RESOLVE_ERROR_BASE + 6,
            Self::DuplicateDefinition { .. } => RESOLVE_ERROR_BASE + 7,
            Self::UnresolvedImportSegment { .. } => RESOLVE_ERROR_BASE + 8,
            Self::InternalError(_) => RESOLVE_ERROR_BASE + 9,
        }
    }

    fn error_name(&self) -> &'static str {
        match self {
            Self::ScopeCreationFailed { .. } => "scope_creation_failed",
            Self::ModuleNotFound { .. } => "module_not_found",
            Self::InvalidNodeType { .. } => "invalid_node_type",
            Self::FileParsingFailed { .. } => "file_parsing_failed",
            Self::UnresolvedName { .. } => "unresolved_name",
            Self::CyclicImport { .. } => "cyclic_import",
            Self::DuplicateDefinition { .. } => "duplicate_definition",
            Self::UnresolvedImportSegment { .. } => "unresolved_import_segment",
            Self::InternalError(_) => "internal_error",
        }
    }

    fn emit(&self, diag_ctx: &DiagnosticContext, _offset: rustc_span::BytePos) {
        let span = self.span();
        let message = self.message();

        diag_ctx
            .error(message.clone())
            .with_code(self.error_code())
            .with_error_label(span, message)
            .with_primary_span(span)
            .emit(diag_ctx);
    }
}
