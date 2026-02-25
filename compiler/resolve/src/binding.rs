//! Bindings and Resolutions – what a name resolves to.

use crate::ids::{AstNodeRef, DefId, ScopeId};
use crate::namespace::Namespace;

/// A single name-to-definition binding discovered during name resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// What kind of definition this binding refers to.
    pub kind: BindingKind,
    /// The definition ID.
    pub def_id: DefId,
    /// The scope in which this definition lives.
    pub defined_in: ScopeId,
    /// An optional back-reference to the AST node that introduced the name.
    pub ast_ref: Option<AstNodeRef>,
    /// The namespace this binding inhabits.
    pub ns: Namespace,
    /// The visibility of this binding (public vs. private).
    pub vis: Visibility,
}

/// What syntactic construct introduced a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingKind {
    /// A module (inline `mod`, file-scope, or directory-module).
    Module,
    /// A function definition.
    Function,
    /// A struct definition.
    Struct,
    /// An enum definition.
    Enum,
    /// A union definition.
    Union,
    /// A type alias.
    TypeAlias,
    /// A trait definition.
    Trait,
    /// An `impl` block (anonymous – usually not named).
    Impl,
    /// An enum variant.
    Variant,
    /// A struct field.
    Field,
    /// A type parameter (`[T]` or `[T : Trait]`).
    TypeParam,
    /// A value parameter in a function / clause.
    Param,
    /// A local `let` / `const` binding.
    Local,
    /// An imported name (re-export from another scope).
    Import,
    /// A built-in / intrinsic definition.
    Builtin,
}

/// Visibility of a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Visibility {
    /// Visible everywhere.
    #[default]
    Public,
    /// Only visible within the defining module and its children.
    Private,
}

// ── Resolution ───────────────────────────────────────────────────────────────

/// The result of resolving a single name.
///
/// This carries enough information for later phases (HIR lowering, type
/// checking) to know exactly *what* a name refers to and *where* it was
/// defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The definition this name resolved to.
    pub def_id: DefId,
    /// The kind of definition.
    pub kind: BindingKind,
    /// The scope where the definition lives.
    pub defined_in: ScopeId,
    /// If the resolution crossed module boundaries via `use`, this records the
    /// intermediate scope from which the name was imported.
    pub imported_from: Option<ScopeId>,
}

impl Resolution {
    pub fn from_binding(binding: &Binding) -> Self {
        Resolution {
            def_id: binding.def_id,
            kind: binding.kind,
            defined_in: binding.defined_in,
            imported_from: None,
        }
    }

    pub fn with_import_source(mut self, scope: ScopeId) -> Self {
        self.imported_from = Some(scope);
        self
    }
}
