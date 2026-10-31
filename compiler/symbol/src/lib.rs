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
    pub const INVALID: DefId = DefId {
        pkg: u32::MAX,
        index: u32::MAX,
    };

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

/// Monotonic definition identities shared by source resolution and generated
/// declarations. Keep one allocator for a scan/generation session. Reconstruct
/// only from the authoritative next-unused index, never from the owner count:
/// clause parameters also consume identities. Dropped drafts need not reuse IDs.
#[derive(Debug)]
pub struct DefIdGen {
    pkg: u32,
    next: u32,
}

impl DefIdGen {
    pub fn new(pkg: u32) -> Self {
        Self::from_next_index(pkg, 0)
    }

    /// Resume the resolver's allocation frontier. The caller must supply an
    /// index beyond every identity it previously allocated, including drafts.
    pub fn from_next_index(pkg: u32, next: u32) -> Self {
        Self { pkg, next }
    }

    /// The maximum index is reserved as the exhausted cursor. Exhaustion never
    /// wraps around or advances state, even in optimized builds.
    pub fn try_next(&mut self) -> Option<DefId> {
        let next = self.next.checked_add(1)?;
        let id = DefId::new(self.pkg, self.next);
        self.next = next;
        Some(id)
    }

    /// Compatibility entrypoint for source scanners. Fallible generation paths
    /// should use try_next and report exhaustion through their own diagnostics.
    pub fn next(&mut self) -> DefId {
        self.try_next()
            .expect("definition identity space exhausted")
    }

    /// Switch package ownership while retaining the global scan frontier.
    pub fn set_package(&mut self, package: u32) {
        self.pkg = package;
    }

    /// Next unused scan index, including identities of discarded declarations.
    pub fn count(&self) -> u32 {
        self.next
    }
}
impl Default for DefIdGen {
    fn default() -> Self {
        Self::new(0)
    }
}
impl fmt::Display for DefIdGen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(definition-id-generator (package {}) (next {}))",
            self.pkg, self.next
        )
    }
}

#[cfg(test)]
mod definition_id_tests {
    use super::*;

    #[test]
    fn source_and_generated_identities_share_a_resumable_frontier() {
        let mut source = DefIdGen::new(7);
        assert_eq!(source.next(), DefId::new(7, 0));
        assert_eq!(source.next(), DefId::new(7, 1));
        source.set_package(9);
        assert_eq!(source.next(), DefId::new(9, 2));
        let mut generated = DefIdGen::from_next_index(7, source.count());
        assert_eq!(generated.try_next(), Some(DefId::new(7, 3)));
        assert_eq!(generated.try_next(), Some(DefId::new(7, 4)));
        assert_eq!(generated.count(), 5);
        assert_eq!(
            generated.to_string(),
            "(definition-id-generator (package 7) (next 5))"
        );
    }

    #[test]
    fn identity_exhaustion_does_not_wrap_or_return_the_invalid_sentinel() {
        for package in [0, u32::MAX] {
            let mut generator = DefIdGen::from_next_index(package, u32::MAX - 1);
            assert_eq!(
                generator.try_next(),
                Some(DefId::new(package, u32::MAX - 1))
            );
            assert_eq!(generator.try_next(), None);
            assert_eq!(generator.try_next(), None);
            assert_eq!(generator.count(), u32::MAX);
            generator.set_package(3);
            assert_eq!(generator.try_next(), None);
        }
    }
}
