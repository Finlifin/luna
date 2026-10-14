//! Import directives – representation of `use` statements before resolution.

use symbol::{PathAnchor, Symbol};

use crate::ids::ScopeId;

/// The shape of a `use` import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportKind {
    /// `use a.b.c`  – import a single name from the target scope.
    Single {
        /// The scope the name was found in (filled during resolution).
        source_scope: Option<ScopeId>,
        /// The original name being imported.
        name: Symbol,
    },
    /// `use a.b.*` — gather accessible entries; `pub use` filters public entries.
    Glob { source_scope: Option<ScopeId> },
    /// `use a.b.{x, y, z}` – import a selected set of names.
    Multi {
        source_scope: Option<ScopeId>,
        names: Vec<Symbol>,
    },
    /// `use a.b.c as d` – import with a local alias.
    Alias {
        source_scope: Option<ScopeId>,
        original: Symbol,
        alias: Symbol,
    },
}

/// A not-yet-resolved `use` statement.
///
/// The scanner collects these; the resolver processes them in a fixpoint loop.
#[derive(Debug, Clone)]
pub struct ImportDirective {
    /// The scope that contains this `use` statement.
    pub owner_scope: ScopeId,
    /// Compiler-selected source package for an implicit prelude import. The
    /// consumer remains owner_scope for permission checks and entry visibility.
    pub origin_scope: Option<ScopeId>,
    /// The root anchor for the path (local, super, or package).
    pub anchor: PathAnchor,
    /// The kind + payload of the import.
    pub kind: ImportKind,
    /// The name segments of the path prefix leading to the imported item,
    /// *excluding* the final name / glob / multi selector.
    /// e.g. for `use a.b.c`, this would be `["a", "b"]` and the
    /// name `"c"` is in `kind`.
    pub path_segments: Vec<Symbol>,
    /// Source span for diagnostics.
    pub span: rustc_span::Span,
    /// The AST node index of the original `use` statement.
    pub ast_node: ast::NodeIndex,
    /// The VFS file that contains this import.
    pub file_id: vfs::FileId,
    /// Whether this import is a re-export (`pub use …`); the name becomes part
    /// of the owning scope's public API.
    pub is_reexport: bool,
    pub visibility: crate::binding::Visibility,
    /// Whether this import has been resolved.
    pub resolved: bool,
}

impl ImportDirective {
    pub fn new(
        owner_scope: ScopeId,
        anchor: PathAnchor,
        kind: ImportKind,
        path_segments: Vec<Symbol>,
        span: rustc_span::Span,
        ast_node: ast::NodeIndex,
        file_id: vfs::FileId,
        visibility: crate::binding::Visibility,
    ) -> Self {
        Self {
            owner_scope,
            origin_scope: None,
            anchor,
            kind,
            path_segments,
            span,
            ast_node,
            file_id,
            is_reexport: visibility == crate::binding::Visibility::Public,
            visibility,
            resolved: false,
        }
    }
}

/// A fully resolved import, ready to be "linked" into the target scope's
/// [`ItemScope`].
#[derive(Debug, Clone)]
pub enum ResolvedImport {
    /// Import all public names from a scope.
    Glob(ScopeId),
    /// Import selected names from a scope.
    Multi(ScopeId, Vec<Symbol>),
    /// Import a single name from a scope.
    Single(ScopeId, Symbol),
    /// Import with alias.
    Alias {
        source_scope: ScopeId,
        original: Symbol,
        alias: Symbol,
    },
}
