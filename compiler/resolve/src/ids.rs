//! Core identifiers used throughout name resolution.
//!
//! These are **resolve-local** IDs – cheap `Copy` handles into the
//! resolver's own tables. They do *not* carry lifetime parameters and are
//! independent of the old `luna::hir::HirId`.

use std::fmt;

// `DefId` lives in the `symbol` crate so that `hir` can also store it in
// `Path::res` without creating a `hir → resolve` dependency cycle.
pub use symbol::DefId;

/// Identifies a scope (lexical block) in the scope tree.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeId(u32);

impl ScopeId {
    pub const ROOT: ScopeId = ScopeId(0);
    pub const INVALID: ScopeId = ScopeId(u32::MAX);

    #[inline]
    pub fn new(raw: u32) -> Self {
        ScopeId(raw)
    }

    #[inline]
    pub fn raw(self) -> u32 {
        self.0
    }

    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }
}

impl fmt::Debug for ScopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Self::INVALID {
            write!(f, "ScopeId(INVALID)")
        } else {
            write!(f, "ScopeId({})", self.0)
        }
    }
}

impl fmt::Display for ScopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "s{}", self.0)
    }
}

/// A lightweight reference back to an AST node, so we can connect resolve-time
/// definitions to their source locations without depending on heavy HIR types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AstNodeRef {
    /// VFS file id (as raw u32).
    pub file: vfs::FileId,
    /// Node index inside that file's AST.
    pub node: ast::NodeIndex,
}

impl AstNodeRef {
    pub fn new(file: vfs::FileId, node: ast::NodeIndex) -> Self {
        Self { file, node }
    }
}

/// Monotonic allocator for [`DefId`]s.
pub struct DefIdGen {
    pkg: u32,
    next: u32,
}

impl DefIdGen {
    pub fn new(pkg: u32) -> Self {
        Self { pkg, next: 0 }
    }

    pub fn next(&mut self) -> DefId {
        let id = DefId::new(self.pkg, self.next);
        self.next += 1;
        id
    }

    /// How many DefIds have been allocated so far.
    pub fn count(&self) -> u32 {
        self.next
    }
}

impl Default for DefIdGen {
    fn default() -> Self {
        Self::new(0)
    }
}

/// Monotonic allocator for [`ScopeId`]s.
pub struct ScopeIdGen {
    next: u32,
}

impl ScopeIdGen {
    pub fn new() -> Self {
        // 0 is reserved for ROOT
        Self { next: 0 }
    }

    pub fn next(&mut self) -> ScopeId {
        let id = ScopeId::new(self.next);
        self.next += 1;
        id
    }
}

impl Default for ScopeIdGen {
    fn default() -> Self {
        Self::new()
    }
}
