//! Interned string type used throughout the compiler.
//!
//! [`Symbol`] is allocated into a global pool via [`internment::Intern`], so:
//!
//! * **`Copy`** – zero-cost to duplicate.
//! * **O(1) `Eq` / `Hash`** – pointer comparison.
//! * **`Deref<Target = str>`** – use anywhere a `&str` is expected.

use std::{fmt, ops::Deref};

use internment::Intern;

/// An interned, pointer-identity string.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Symbol(Intern<String>);

impl Symbol {
    /// Intern a string slice, returning the canonical [`Symbol`].
    #[inline]
    pub fn intern(s: &str) -> Self {
        Symbol(Intern::new(s.to_owned()))
    }

    #[inline]
    pub fn invalid() -> Self {
        Symbol(Intern::new("<invalid>".to_owned()))
    }

    /// View the underlying string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Split the internal pointer into two `u32` halves for storage in the
    /// flat AST child array.
    ///
    /// Reconstruct with [`Symbol::from_raw_parts`].
    #[inline]
    pub fn to_raw_parts(self) -> (u32, u32) {
        // SAFETY: Intern<String> is a single thin-pointer field (&'static String),
        // which is 8 bytes on 64-bit platforms, identical in layout to u64.
        let bits: u64 = unsafe { std::mem::transmute(self.0) };
        ((bits >> 32) as u32, bits as u32)
    }

    /// Reconstruct a [`Symbol`] from the two `u32` halves produced by
    /// [`Symbol::to_raw_parts`].
    ///
    /// # Safety
    ///
    /// The `hi`/`lo` pair must have been produced by [`Symbol::to_raw_parts`]
    /// within the **same process run**.  Passing arbitrary values is undefined
    /// behaviour.
    #[inline]
    pub unsafe fn from_raw_parts(hi: u32, lo: u32) -> Self {
        let bits: u64 = ((hi as u64) << 32) | (lo as u64);
        // SAFETY: see to_raw_parts – we are inverting the same transmute.
        Symbol(unsafe { std::mem::transmute(bits) })
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Deref for Symbol {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Symbol {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<str> for Symbol {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Symbol {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<Symbol> for str {
    fn eq(&self, other: &Symbol) -> bool {
        self == other.as_str()
    }
}

impl From<&str> for Symbol {
    #[inline]
    fn from(s: &str) -> Self {
        Symbol::intern(s)
    }
}

impl From<String> for Symbol {
    #[inline]
    fn from(s: String) -> Self {
        Symbol(Intern::new(s))
    }
}

/// The root anchor of a Flurry path.
///
/// Flurry path syntax:
/// - `a.b.c`   → starts from the current scope (`Local`)
/// - `.a.b.c`  → starts one scope up; each leading `.` adds one level (`Super(n)`)
/// - `@a.b.c`  → starts from the package root (`Package`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathAnchor {
    /// `a.b.c` — resolves from the current scope (no prefix).
    Local,
    /// `.a.b.c` — resolves `n` scopes above the current module.
    ///
    /// Each leading `.` corresponds to one level: `.foo` = `Super(1)`,
    /// `..foo` = `Super(2)`, etc.
    Super(u32),
    /// `@a.b.c` — resolves from the package root.
    Package,
}

/// A definition ID — uniquely identifies a name-binding site within a package.
///
/// Every item (function, struct, enum, module, type alias, …), every type
/// parameter, and every local variable that introduces a name gets a `DefId`.
///
/// `DefId` lives here in the `symbol` crate so that both the `resolve` crate
/// (which produces it) and the `hir` crate (which stores it in `Path::res`)
/// can use it without a circular dependency.
///
/// Mirrors rustc's `DefId`: a `(pkg, index)` pair where `pkg` identifies the
/// package (crate) and `index` is the per-package definition counter.
/// Both fields are `u32::MAX` for the sentinel `INVALID` value.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DefId {
    /// The package (crate) this definition belongs to.
    pub pkg: u32,
    /// Per-package index of the definition.
    pub index: u32,
}

impl DefId {
    pub const INVALID: DefId = DefId { pkg: u32::MAX, index: u32::MAX };

    #[inline]
    pub fn new(pkg: u32, index: u32) -> Self {
        DefId { pkg, index }
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
            write!(f, "DefId({}:{})", self.pkg, self.index)
        }
    }
}

impl fmt::Display for DefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "d{}:{}", self.pkg, self.index)
    }
}
