//! Import directives – representation of `use` statements before resolution.

use crate::ids::ScopeId;

// ── ImportKind ───────────────────────────────────────────────────────────────

/// The shape of a `use` import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportKind {
    /// `use a.b.c`  – import a single name from the target scope.
    Single {
        /// The scope the name was found in (filled during resolution).
        source_scope: Option<ScopeId>,
        /// The original name being imported.
        name: String,
    },
    /// `use a.b.*`  – glob import of all public names.
    Glob {
        source_scope: Option<ScopeId>,
    },
    /// `use a.b.{x, y, z}` – import a selected set of names.
    Multi {
        source_scope: Option<ScopeId>,
        names: Vec<String>,
    },
    /// `use a.b.c as d` – import with a local alias.
    Alias {
        source_scope: Option<ScopeId>,
        original: String,
        alias: String,
    },
}

// ── ImportDirective ──────────────────────────────────────────────────────────

/// A not-yet-resolved `use` statement.
///
/// The scanner collects these; the resolver processes them in a fixpoint loop.
#[derive(Debug, Clone)]
pub struct ImportDirective {
    /// The scope that contains this `use` statement.
    pub owner_scope: ScopeId,
    /// The kind + payload of the import.
    pub kind: ImportKind,
    /// The full path segments leading to the imported item, *excluding* the
    /// final name / glob / multi selector.
    /// e.g. for `use a.b.c`, this would be `["a", "b"]` and the name `"c"` is
    /// in `kind`.
    pub path_segments: Vec<String>,
    /// Source span for diagnostics.
    pub span: rustc_span::Span,
    /// The AST node index of the original `use` statement.
    pub ast_node: ast::NodeIndex,
    /// The VFS file that contains this import.
    pub file_id: vfs::FileId,
    /// Whether this import has been resolved.
    pub resolved: bool,
}

impl ImportDirective {
    pub fn new(
        owner_scope: ScopeId,
        kind: ImportKind,
        path_segments: Vec<String>,
        span: rustc_span::Span,
        ast_node: ast::NodeIndex,
        file_id: vfs::FileId,
    ) -> Self {
        Self {
            owner_scope,
            kind,
            path_segments,
            span,
            ast_node,
            file_id,
            resolved: false,
        }
    }
}

// ── ResolvedImport ───────────────────────────────────────────────────────────

/// A fully resolved import, ready to be "linked" into the target scope's
/// [`ItemScope`].
#[derive(Debug, Clone)]
pub enum ResolvedImport {
    /// Import all public names from a scope.
    All(ScopeId),
    /// Import selected names from a scope.
    Multi(ScopeId, Vec<String>),
    /// Import a single name from a scope.
    Single(ScopeId, String),
    /// Import with alias.
    Alias {
        source_scope: ScopeId,
        original: String,
        alias: String,
    },
}
