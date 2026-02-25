//! Scope tree – the hierarchical structure of lexical scopes.
//!
//! Every module, function body, block, and inline `mod { }` creates a scope.
//! Scopes form a tree rooted at the package scope. Each scope owns an
//! [`ItemScope`] that stores the names visible in that scope.

use std::fmt;

use crate::ids::{DefId, ModuleId, ScopeId};
use crate::item_scope::ItemScope;

/// What syntactic construct created this scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// Package root scope.
    Package,
    /// A module scope (file, directory, or inline `mod`).
    Module,
    /// A function body.
    FnBody,
    /// A block expression (`{ ... }`).
    Block,
    /// A struct / enum / union body (for associated items).
    AdtBody,
    /// An `impl` block.
    ImplBlock,
    /// A trait definition body.
    TraitBody,
    /// The synthetic "root" scope that parents all packages.
    Root,
}

/// A node in the scope tree.
pub struct Scope {
    /// Unique id of this scope.
    pub id: ScopeId,
    /// What kind of scope this is.
    pub kind: ScopeKind,
    /// Parent scope (`None` only for the root).
    pub parent: Option<ScopeId>,
    /// Human-readable name (e.g. module name). `None` for anonymous scopes.
    pub name: Option<String>,
    /// The DefId of the item that opened this scope (e.g. the function, module, struct…).
    pub owner_def: DefId,
    /// If this scope corresponds to a module, its ModuleId.
    pub module_id: Option<ModuleId>,
    /// Whether this scope is *ordered* (i.e. names must be declared before use,
    /// like in a function body) or *unordered* (like in a module, where items
    /// can refer to each other regardless of textual order).
    pub ordered: bool,
    /// The names visible in this scope.
    pub items: ItemScope,
    /// Direct child scope IDs.
    pub children: Vec<ScopeId>,
}

impl Scope {
    /// Create a new scope.
    pub fn new(
        id: ScopeId,
        kind: ScopeKind,
        parent: Option<ScopeId>,
        name: Option<String>,
        owner_def: DefId,
        ordered: bool,
    ) -> Self {
        Self {
            id,
            kind,
            parent,
            name,
            owner_def,
            module_id: None,
            ordered,
            items: ItemScope::new(),
            children: Vec::new(),
        }
    }

    pub fn is_module(&self) -> bool {
        matches!(
            self.kind,
            ScopeKind::Module | ScopeKind::Package | ScopeKind::Root
        )
    }
}

impl fmt::Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scope")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("parent", &self.parent)
            .field("name", &self.name)
            .field("owner_def", &self.owner_def)
            .field("ordered", &self.ordered)
            .field("children", &self.children)
            .finish()
    }
}

// ── ScopeTree ────────────────────────────────────────────────────────────────

/// The complete scope tree for a compilation unit.
///
/// Scopes are stored in a flat `Vec` and addressed by [`ScopeId`] (which is
/// just an index).
pub struct ScopeTree {
    scopes: Vec<Scope>,
}

impl ScopeTree {
    pub fn new() -> Self {
        Self { scopes: Vec::new() }
    }

    /// Allocate a new scope and return its id.
    pub fn add_scope(&mut self, scope: Scope) -> ScopeId {
        let id = scope.id;
        let idx = id.index();
        if idx >= self.scopes.len() {
            self.scopes.resize_with(idx + 1, || {
                Scope::new(
                    ScopeId::INVALID,
                    ScopeKind::Root,
                    None,
                    None,
                    DefId::INVALID,
                    false,
                )
            });
        }
        self.scopes[idx] = scope;
        id
    }

    /// Get a scope by id.
    pub fn get(&self, id: ScopeId) -> Option<&Scope> {
        self.scopes.get(id.index()).filter(|s| s.id.is_valid())
    }

    /// Get a mutable scope by id.
    pub fn get_mut(&mut self, id: ScopeId) -> Option<&mut Scope> {
        self.scopes.get_mut(id.index()).filter(|s| s.id.is_valid())
    }

    /// Register a child scope under a parent.
    pub fn add_child(&mut self, parent: ScopeId, child: ScopeId) {
        if let Some(p) = self.get_mut(parent) {
            p.children.push(child);
        }
    }

    /// Walk up the parent chain from `scope_id`.
    pub fn ancestors(&self, scope_id: ScopeId) -> AncestorIter<'_> {
        AncestorIter {
            tree: self,
            current: Some(scope_id),
        }
    }

    /// Number of allocated scopes.
    pub fn len(&self) -> usize {
        self.scopes.iter().filter(|s| s.id.is_valid()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate over all valid scopes.
    pub fn iter(&self) -> impl Iterator<Item = &Scope> {
        self.scopes.iter().filter(|s| s.id.is_valid())
    }
}

impl Default for ScopeTree {
    fn default() -> Self {
        Self::new()
    }
}

// ── AncestorIter ─────────────────────────────────────────────────────────────

pub struct AncestorIter<'a> {
    tree: &'a ScopeTree,
    current: Option<ScopeId>,
}

impl<'a> Iterator for AncestorIter<'a> {
    type Item = &'a Scope;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.current?;
        let scope = self.tree.get(id)?;
        self.current = scope.parent;
        Some(scope)
    }
}
