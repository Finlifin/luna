//! Core identifiers used throughout name resolution.
//!
//! These are **resolve-local** IDs – cheap `Copy` handles into the
//! resolver's own tables. They do *not* carry lifetime parameters and are
//! independent of the old `luna::hir::HirId`.

use std::fmt;

// ── DefId ────────────────────────────────────────────────────────────────────

/// A definition ID – uniquely identifies a name-binding site within a package.
///
/// Every item (function, struct, enum, module, type alias, …), every type
/// parameter, and every local variable that introduces a name gets a `DefId`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DefId(u32);

impl DefId {
    pub const INVALID: DefId = DefId(u32::MAX);

    #[inline]
    pub fn new(raw: u32) -> Self {
        DefId(raw)
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

impl fmt::Debug for DefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Self::INVALID {
            write!(f, "DefId(INVALID)")
        } else {
            write!(f, "DefId({})", self.0)
        }
    }
}

impl fmt::Display for DefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "d{}", self.0)
    }
}

// ── ScopeId ──────────────────────────────────────────────────────────────────

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

// ── ModuleId ─────────────────────────────────────────────────────────────────

/// Identifies a module (file-scope, directory-module, or inline `mod`).
/// A module always has a corresponding `ScopeId` but the reverse is not true
/// (e.g. function bodies create scopes but not modules).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(u32);

impl ModuleId {
    pub const INVALID: ModuleId = ModuleId(u32::MAX);

    #[inline]
    pub fn new(raw: u32) -> Self {
        ModuleId(raw)
    }

    #[inline]
    pub fn raw(self) -> u32 {
        self.0
    }

    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Self::INVALID {
            write!(f, "ModuleId(INVALID)")
        } else {
            write!(f, "ModuleId({})", self.0)
        }
    }
}

// ── AstNodeRef ───────────────────────────────────────────────────────────────

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

// ── DefIdGen ─────────────────────────────────────────────────────────────────

/// Monotonic allocator for [`DefId`]s.
pub struct DefIdGen {
    next: u32,
}

impl DefIdGen {
    pub fn new() -> Self {
        Self { next: 0 }
    }

    pub fn next(&mut self) -> DefId {
        let id = DefId::new(self.next);
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
        Self::new()
    }
}

// ── ScopeIdGen ───────────────────────────────────────────────────────────────

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
