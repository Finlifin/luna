//! Type interning arena.
//!
//! Provides the low-level arena + hash-set that ensures each structurally
//! unique [`TyKind`] is allocated exactly once.

use std::cell::RefCell;
use std::mem;

use rustc_arena_modified::typed_arena::TypedArena;
use rustc_data_structures::fx::FxHashSet;

use crate::types::{Ty, TyKind};

/// The interning arena for semantic types.
///
/// All [`Ty`] pointers produced by this interner are valid for the `'tcx`
/// lifetime (i.e. the borrow lifetime of `&'tcx self`).
pub struct TyInterner {
    /// The backing arena – stores every unique `TyKind`.
    arena: TypedArena<TyKind<'static>>,
    /// Auxiliary arena for interned `Ty` slices (tuples, fn params, etc.).
    slice_arena: TypedArena<Ty<'static>>,
    /// The deduplication set – maps to pointers inside `arena`.
    set: RefCell<FxHashSet<&'static TyKind<'static>>>,
}

impl TyInterner {
    /// Create a new, empty interner.
    pub fn new() -> Self {
        TyInterner {
            arena: TypedArena::new(),
            slice_arena: TypedArena::new(),
            set: RefCell::new(FxHashSet::default()),
        }
    }

    /// Intern a [`TyKind`], returning a thin `Ty` pointer.
    ///
    /// If a structurally identical `TyKind` has already been interned,
    /// the existing allocation is reused. Otherwise a new allocation is
    /// made in the arena.
    ///
    /// # Safety contract (lifetime transmute)
    ///
    /// Same pattern as [`HirArena`](hir::HirArena): the arena stores
    /// `TyKind<'static>` and we transmute to `TyKind<'tcx>` on the way
    /// in/out. This is sound because the arena owns the memory and `'tcx`
    /// is the borrow lifetime of `&'tcx self`.
    pub fn intern<'tcx>(&'tcx self, kind: TyKind<'tcx>) -> Ty<'tcx> {
        // SAFETY: transmute TyKind<'tcx> → TyKind<'static> for the set lookup.
        let kind_static: TyKind<'static> = unsafe { mem::transmute(kind) };

        let mut set = self.set.borrow_mut();

        // Fast path: already interned.
        if let Some(&existing) = set.get(&kind_static) {
            return Ty(unsafe { mem::transmute::<&'static TyKind<'static>, &'tcx TyKind<'tcx>>(existing) });
        }

        // Slow path: allocate in the arena and insert into the set.
        let allocated: &'static TyKind<'static> = unsafe {
            let r = self.arena.alloc(kind_static);
            // The arena returns &TyKind<'static> with the arena's lifetime,
            // but we need 'static for the set. Since the arena lives at
            // least as long as `self`, and `set` is inside `self`, this is fine.
            &*(r as *const TyKind<'static>)
        };
        set.insert(allocated);

        Ty(unsafe { mem::transmute::<&'static TyKind<'static>, &'tcx TyKind<'tcx>>(allocated) })
    }

    /// Allocate a slice of `Ty` values in the arena.
    ///
    /// Used by the [`TyCtxt`](crate::TyCtxt) to intern sub-type slices
    /// (e.g. tuple elements, function parameters, ADT generic args).
    pub(crate) fn alloc_ty_slice(&self, tys: Vec<Ty<'static>>) -> &[Ty<'static>] {
        self.slice_arena.alloc_from_iter_reg(tys)
    }
}

impl Default for TyInterner {
    fn default() -> Self {
        Self::new()
    }
}
