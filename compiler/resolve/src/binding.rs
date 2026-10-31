//! Bindings and Resolutions – what a name resolves to.

use crate::ids::{AstNodeRef, DefId, ScopeId};

/// A single name-to-definition binding discovered during name resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub kind: BindingKind,
    /// The original declaration's permission; importing never changes it.
    pub vis: Visibility,
    /// Permission on the current name entry; original declaration permission
    /// and defining scope remain in `vis` and `kind`.
    pub entry: Option<(Visibility, ScopeId)>,
}

/// What syntactic construct introduced a name.
/// 这里的AstNodeRef在lowering阶段可以通过AstNodeRef->HirId映射来找到这个名字在HIR中的定义位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingKind {
    Definition(DefId, ScopeId),
    FnParam(AstNodeRef),
    ClauseParam(DefId, AstNodeRef),
    Local(AstNodeRef),
    /// A local introduced by a contextual construct rather than an identifier
    /// AST node. The output ordinal distinguishes bindings at the same site.
    ExpandedLocal(AstNodeRef, u32),
    LoopLabel(AstNodeRef),
}

/// Visibility of a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Visibility {
    /// Visible everywhere (explicit `pub`).
    Public,
    /// Visible within the current package only (no keyword; the default).
    #[default]
    Package,
    /// Only visible within the defining module and its children.
    Private,
}

/// The result of resolving a single name.
///
/// This carries enough information for later phases (HIR lowering, type
/// checking) to know exactly *what* a name refers to and *where* it was
/// defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub kind: BindingKind,
}

impl Resolution {
    pub fn from_binding(binding: &Binding) -> Self {
        Resolution { kind: binding.kind }
    }
}

impl Binding {
    pub fn entry_visibility(&self) -> Visibility {
        self.entry.map_or(self.vis, |(vis, _)| vis)
    }
    pub fn accessible(&self, tree: &crate::scope::ScopeTree, requester: ScopeId) -> bool {
        fn permits(
            tree: &crate::scope::ScopeTree,
            vis: Visibility,
            owner: ScopeId,
            requester: ScopeId,
        ) -> bool {
            match vis {
                Visibility::Public => true,
                Visibility::Private => {
                    // Definitions and import entries may live in an ADT or
                    // function scope. Privacy belongs to their lexical module,
                    // not to those implementation scopes.
                    let module = tree.ancestors(owner).find(|scope| scope.is_module());
                    module
                        .is_some_and(|module| tree.ancestors(requester).any(|s| s.id == module.id))
                }
                Visibility::Package => {
                    let package = |scope| {
                        tree.ancestors(scope)
                            .find(|s| s.kind == crate::scope::ScopeKind::Package)
                            .map(|s| s.id)
                    };
                    package(owner).is_some() && package(owner) == package(requester)
                }
            }
        }
        let origin = match self.kind {
            BindingKind::Definition(_, owner) => permits(tree, self.vis, owner, requester),
            _ => true,
        };
        origin
            && self
                .entry
                .is_none_or(|(vis, owner)| permits(tree, vis, owner, requester))
    }
    pub fn imported(&self, owner: ScopeId, visibility: Visibility) -> Self {
        Self {
            kind: self.kind,
            vis: self.vis,
            entry: Some((visibility, owner)),
        }
    }
    /// Entries for the same identity in one scope form a union of permissions.
    pub fn merge_entry(&mut self, other: &Self) -> bool {
        let rank = |vis| match vis {
            Visibility::Private => 0,
            Visibility::Package => 1,
            Visibility::Public => 2,
        };
        if self.entry.is_some() && rank(other.entry_visibility()) > rank(self.entry_visibility()) {
            self.entry = other.entry;
            true
        } else {
            false
        }
    }
}
