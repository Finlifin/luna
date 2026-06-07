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
