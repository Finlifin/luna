//! Impl directives – `impl` and `impl Trait for Type` blocks collected during
//! the scan phase.
//!
//! These are analogous to [`ImportDirective`](crate::import::ImportDirective):
//! gathered by the scanner and stored in [`ModuleTree`](crate::module_builder::ModuleTree)
//! for consumption by later phases (trait solving, method resolution).

use crate::ids::{DefId, ScopeId};

/// The kind of an impl block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplKind {
    /// `impl Type { … }` — inherent implementation.
    Inherent,
    /// `impl Trait for Type { … }` — trait implementation.
    TraitImpl,
}

/// An `impl` block discovered during the scan phase.
///
/// The scanner allocates a [`DefId`] and a body [`ScopeId`] for each impl,
/// scans the body items (methods, associated functions, …) into that scope,
/// and records the directive here.  Later phases use the AST node reference
/// and scope pointer to perform type-directed method / trait-impl resolution.
#[derive(Debug, Clone)]
pub struct ImplDirective {
    /// The DefId allocated for this impl block.
    pub def_id: DefId,
    /// The scope that contains this `impl` statement.
    pub owner_scope: ScopeId,
    /// The body scope created for this impl block.
    pub impl_scope: ScopeId,
    /// Whether this is an inherent impl or a trait impl.
    pub kind: ImplKind,
    /// The AST node index of the `impl` statement.
    pub ast_node: ast::NodeIndex,
    /// The VFS file that contains this impl.
    pub file_id: vfs::FileId,
    /// Source span for diagnostics.
    pub span: rustc_span::Span,
}
