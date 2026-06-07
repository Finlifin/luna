//! HIR package ownership wrapper.
//!
//! [`HirPackageBox`] co-locates a [`HirArena`] and the [`Package`] allocated
//! from it, erasing the `'hir` lifetime so the value can be stored in the
//! database and shared behind `Arc`.
//!
//! # Lifetime safety
//!
//! `HirPackageBox` transmutes `Package<'hir>` to `Package<'static>` and stores
//! both the arena and the package in the same allocation.  Accessing the
//! package via [`HirPackageBox::package`] rebinds the lifetime to the borrow
//! of the `HirPackageBox`, ensuring the arena is always live.

use hir::{HirArena, Package};

// ── HirPackageBox ─────────────────────────────────────────────────────────────

/// An owned HIR package with its backing arena.
///
/// `Package<'hir>` borrows from `HirArena`.  By moving both into this struct
/// together we can safely erase `'hir` to `'static`: the `package()` accessor
/// rebinds it to the borrow lifetime of `self`.
pub struct HirPackageBox {
    /// The arena that owns all `&'hir` allocations in `package`.
    arena: HirArena,
    /// The HIR package.  Lifetime erased to `'static`; use `package()`.
    ///
    /// # Invariant
    /// All `&'hir` pointers inside this value point into `arena` above.
    package: Package<'static>,
}

impl HirPackageBox {
    /// Move an `arena`-allocated `Package` into an owned box.
    ///
    /// After this call the arena is co-located with the package; callers
    /// must not access `package` after `arena` has been dropped.
    /// This invariant is upheld automatically because both live in `Self`.
    pub fn new(arena: HirArena, package: Package<'_>) -> Self {
        // SAFETY: `package` borrows from `arena`.  Both are moved into `Self`
        // together, so the transmuted `Package<'static>` is only accessible via
        // `package()` which re-constrains the lifetime to `'a ≤ lifetime-of-self`.
        let package =
            unsafe { std::mem::transmute::<Package<'_>, Package<'static>>(package) };
        HirPackageBox { arena, package }
    }

    /// Borrow the HIR package with a safe lifetime `'a` tied to `self`.
    pub fn package<'a>(&'a self) -> &'a Package<'a> {
        // SAFETY: rebind lifetime from 'static (internal repr) to 'a (borrow of self).
        unsafe {
            std::mem::transmute::<&Package<'static>, &'a Package<'a>>(&self.package)
        }
    }

    /// Borrow the HIR arena.
    pub fn arena(&self) -> &HirArena {
        &self.arena
    }
}

// SAFETY: `HirArena` uses `Cell`/`RefCell` internally (single-threaded).
// Luna is single-threaded; we never access `HirPackageBox` from multiple
// threads concurrently.  The unsafe impls satisfy salsa's `Db: Sync` bound.
unsafe impl Sync for HirPackageBox {}
unsafe impl Send for HirPackageBox {}
