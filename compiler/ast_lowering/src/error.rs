//! Lowering errors.
//!
//! Every lowering error implements [`FlurryError`] and is emitted through the
//! [`DiagnosticContext`], matching the project-wide diagnostic convention.

use diagnostic::{DiagnosticBuilder, DiagnosticContext, FlurryError};
use rustc_span::{BytePos, Span};

// ── Error codes ──────────────────────────────────────────────────────────────
//
// Lowering errors use the E2xxx range:
//   E2001  – unsupported AST node during lowering
//   E2002  – invalid AST structure (missing expected children)
//   E2003  – unsupported clause kind
//   E2004  – invalid function parameter
//   E2005  – invalid pattern form
//   E2006  – invalid item in context
//   E2007  – invalid enum variant
//   E2008  – invalid struct field
//   E2009  – missing identifier
//   E2010  – invalid type expression

/// A lowering error carrying enough information to produce a full diagnostic.
#[derive(Debug)]
pub struct LoweringError {
    pub kind: LoweringErrorKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum LoweringErrorKind {
    /// Encountered an AST `NodeKind` that has no HIR equivalent (yet).
    UnsupportedNode(String),
    /// The AST node's child layout is broken / unexpected.
    MalformedAst(String),
    /// A clause kind that we cannot lower.
    UnsupportedClause(String),
    /// A function parameter kind that we cannot lower.
    InvalidParameter(String),
    /// A pattern form that we cannot lower.
    InvalidPattern(String),
    /// An item appeared where it is not allowed.
    InvalidItemInContext(String),
    /// An enum variant form that we cannot lower.
    InvalidEnumVariant(String),
    /// A struct field is malformed.
    InvalidStructField(String),
    /// Expected an identifier node but got something else.
    MissingIdentifier,
    /// A type expression that we cannot lower.
    InvalidTypeExpr(String),
}

impl LoweringError {
    pub fn new(kind: LoweringErrorKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn unsupported_node(name: impl Into<String>, span: Span) -> Self {
        Self::new(LoweringErrorKind::UnsupportedNode(name.into()), span)
    }

    pub fn malformed_ast(msg: impl Into<String>, span: Span) -> Self {
        Self::new(LoweringErrorKind::MalformedAst(msg.into()), span)
    }

    pub fn unsupported_clause(msg: impl Into<String>, span: Span) -> Self {
        Self::new(LoweringErrorKind::UnsupportedClause(msg.into()), span)
    }

    pub fn invalid_parameter(msg: impl Into<String>, span: Span) -> Self {
        Self::new(LoweringErrorKind::InvalidParameter(msg.into()), span)
    }

    pub fn invalid_pattern(msg: impl Into<String>, span: Span) -> Self {
        Self::new(LoweringErrorKind::InvalidPattern(msg.into()), span)
    }

    pub fn invalid_item(msg: impl Into<String>, span: Span) -> Self {
        Self::new(LoweringErrorKind::InvalidItemInContext(msg.into()), span)
    }

    pub fn invalid_enum_variant(msg: impl Into<String>, span: Span) -> Self {
        Self::new(LoweringErrorKind::InvalidEnumVariant(msg.into()), span)
    }

    pub fn invalid_struct_field(msg: impl Into<String>, span: Span) -> Self {
        Self::new(LoweringErrorKind::InvalidStructField(msg.into()), span)
    }

    pub fn missing_identifier(span: Span) -> Self {
        Self::new(LoweringErrorKind::MissingIdentifier, span)
    }

    pub fn invalid_type_expr(msg: impl Into<String>, span: Span) -> Self {
        Self::new(LoweringErrorKind::InvalidTypeExpr(msg.into()), span)
    }
}

impl FlurryError for LoweringError {
    fn error_code(&self) -> u32 {
        match &self.kind {
            LoweringErrorKind::UnsupportedNode(_) => 2001,
            LoweringErrorKind::MalformedAst(_) => 2002,
            LoweringErrorKind::UnsupportedClause(_) => 2003,
            LoweringErrorKind::InvalidParameter(_) => 2004,
            LoweringErrorKind::InvalidPattern(_) => 2005,
            LoweringErrorKind::InvalidItemInContext(_) => 2006,
            LoweringErrorKind::InvalidEnumVariant(_) => 2007,
            LoweringErrorKind::InvalidStructField(_) => 2008,
            LoweringErrorKind::MissingIdentifier => 2009,
            LoweringErrorKind::InvalidTypeExpr(_) => 2010,
        }
    }

    fn error_name(&self) -> &'static str {
        match &self.kind {
            LoweringErrorKind::UnsupportedNode(_) => "unsupported AST node",
            LoweringErrorKind::MalformedAst(_) => "malformed AST structure",
            LoweringErrorKind::UnsupportedClause(_) => "unsupported clause",
            LoweringErrorKind::InvalidParameter(_) => "invalid parameter",
            LoweringErrorKind::InvalidPattern(_) => "invalid pattern",
            LoweringErrorKind::InvalidItemInContext(_) => "invalid item in context",
            LoweringErrorKind::InvalidEnumVariant(_) => "invalid enum variant",
            LoweringErrorKind::InvalidStructField(_) => "invalid struct field",
            LoweringErrorKind::MissingIdentifier => "missing identifier",
            LoweringErrorKind::InvalidTypeExpr(_) => "invalid type expression",
        }
    }

    fn emit(&self, diag_ctx: &DiagnosticContext, _base_pos: BytePos) {
        let message = match &self.kind {
            LoweringErrorKind::UnsupportedNode(name) => {
                format!("unsupported AST node `{}` during lowering", name)
            }
            LoweringErrorKind::MalformedAst(msg) => {
                format!("malformed AST: {}", msg)
            }
            LoweringErrorKind::UnsupportedClause(msg) => {
                format!("unsupported clause form: {}", msg)
            }
            LoweringErrorKind::InvalidParameter(msg) => {
                format!("invalid function parameter: {}", msg)
            }
            LoweringErrorKind::InvalidPattern(msg) => {
                format!("invalid pattern: {}", msg)
            }
            LoweringErrorKind::InvalidItemInContext(msg) => {
                format!("invalid item in this context: {}", msg)
            }
            LoweringErrorKind::InvalidEnumVariant(msg) => {
                format!("invalid enum variant: {}", msg)
            }
            LoweringErrorKind::InvalidStructField(msg) => {
                format!("invalid struct field: {}", msg)
            }
            LoweringErrorKind::MissingIdentifier => "expected an identifier".to_string(),
            LoweringErrorKind::InvalidTypeExpr(msg) => {
                format!("invalid type expression: {}", msg)
            }
        };

        DiagnosticBuilder::error(message)
            .with_code(self.error_code())
            .with_primary_span(self.span)
            .with_error_label(self.span, self.error_name().to_string())
            .emit(diag_ctx);
    }
}
