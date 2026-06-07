//! Luna compiler database — the root of the query system.
//!
//! # Architecture
//!
//! ```text
//!   ┌─────────────────────────────────┐
//!   │  Db                             │
//!   │  ├ storage   – salsa engine     │
//!   │  ├ ty_ctxt   – type arena       │
//!   │  ├ providers – fn-ptr dispatch  │
//!   │  ├ hir_input – query inputs     │
//!   │  └ hir_cache – memoised result  │
//!   └─────────────────────────────────┘
//! ```
//!
//! Providers are registered by calling `ast_lowering::set_providers(&mut db.providers)`
//! before issuing any HIR query.  Query inputs are set via `db.set_hir_input(…)`.

use std::sync::Arc;

use hir::hir_id::LocalDefId;

use crate::hir_package::HirPackageBox;
use crate::hir_query::HirQueryInput;
use crate::ty::{AdtDef, NFId, TyCtxt};

// ── SyncCell ──────────────────────────────────────────────────────────────────

/// A `RefCell<T>` wrapper that unsafely implements `Sync`.
///
/// Required because salsa 0.22 bounds `Db: Sync` through `HasStorage`.
/// Luna is single-threaded; all `Db` accesses occur on the main thread.
struct SyncCell<T>(std::cell::RefCell<T>);

unsafe impl<T> Sync for SyncCell<T> {}

impl<T> SyncCell<T> {
    fn new(val: T) -> Self {
        SyncCell(std::cell::RefCell::new(val))
    }

    fn borrow(&self) -> std::cell::Ref<'_, T> {
        self.0.borrow()
    }

    fn borrow_mut(&self) -> std::cell::RefMut<'_, T> {
        self.0.borrow_mut()
    }
}

impl<T: Clone> Clone for SyncCell<T> {
    fn clone(&self) -> Self {
        SyncCell(std::cell::RefCell::new(self.0.borrow().clone()))
    }
}

// ── Providers ─────────────────────────────────────────────────────────────────

/// Function-pointer dispatch table for compiler queries.
///
/// Each field is a plain `fn` pointer (not a closure), so `Providers` is
/// trivially `Clone`, `Copy`, and `Sync`.
///
/// Register implementations by calling the appropriate `set_providers`
/// function from each compiler-pass crate **before** issuing queries:
///
/// ```ignore
/// ast_lowering::set_providers(&mut instance.db.providers);
/// ```
#[derive(Clone, Copy)]
pub struct Providers {
    /// Provider for the `hir_package` query.
    pub hir_package: fn(&Db) -> Arc<HirPackageBox>,
}

impl Default for Providers {
    fn default() -> Self {
        fn missing_hir_package(_: &Db) -> Arc<HirPackageBox> {
            panic!(
                "hir_package: no provider registered — \
                 call ast_lowering::set_providers(&mut db.providers) first"
            )
        }
        Providers {
            hir_package: missing_hir_package,
        }
    }
}

// ── LunaDatabase trait ────────────────────────────────────────────────────────

/// Extension of [`salsa::Database`] with Luna compiler query methods.
pub trait LunaDatabase: salsa::Database {
    /// Downcast to the concrete [`Db`].
    fn as_db(&self) -> &Db;

    // ── ADT queries ───────────────────────────────────────────────────────

    fn adt_def(&self, id: NFId) -> Option<Arc<AdtDef>> {
        self.as_db().ty_ctxt.adt_def(id).map(Arc::new)
    }

    // ── Type queries ──────────────────────────────────────────────────────

    fn def_ty_exists(&self, id: LocalDefId) -> bool {
        self.as_db().ty_ctxt.def_ty(id).is_some()
    }

    // ── HIR queries ───────────────────────────────────────────────────────

    /// Lower the package's AST to HIR, returning an owned [`HirPackageBox`].
    ///
    /// The result is **memoised**: the provider is called at most once per
    /// database instance.
    ///
    /// # Panics
    ///
    /// Panics if no provider has been registered or no input has been set.
    fn hir_package(&self, _: ()) -> Arc<HirPackageBox> {
        self.as_db().hir_package_impl()
    }
}

// ── Db ────────────────────────────────────────────────────────────────────────

/// The Luna compiler database.
///
/// Created once per compilation by `CompilerInstance::new()`.
#[derive(Clone)]
#[salsa::db]
pub struct Db {
    /// Salsa storage — tracks query dependencies and memoised results.
    storage: salsa::Storage<Db>,

    /// Type-interning arena (wrapped in `Arc` for cheap `Clone`).
    pub ty_ctxt: Arc<TyCtxt>,

    /// Registered query providers (function pointers, trivially `Sync`).
    pub providers: Providers,

    /// Inputs for the `hir_package` query (set before calling the query).
    hir_input: Arc<SyncCell<Option<Arc<HirQueryInput>>>>,

    /// Memoised result of the `hir_package` query.
    hir_cache: Arc<SyncCell<Option<Arc<HirPackageBox>>>>,
}

#[salsa::db]
impl salsa::Database for Db {}

// SAFETY: Luna is single-threaded.  `TyCtxt` and `HirArena` use `RefCell`
// internally, but we never access a `Db` from multiple threads concurrently.
// Salsa 0.22 requires `Db: Sync` via `HasStorage`; this impl satisfies it.
unsafe impl Sync for Db {}

impl LunaDatabase for Db {
    #[inline]
    fn as_db(&self) -> &Db {
        self
    }
}

impl Db {
    /// Construct an empty compiler database with default (panicking) providers.
    pub fn new() -> Self {
        Db {
            storage: salsa::Storage::default(),
            ty_ctxt: Arc::new(TyCtxt::new()),
            providers: Providers::default(),
            hir_input: Arc::new(SyncCell::new(None)),
            hir_cache: Arc::new(SyncCell::new(None)),
        }
    }

    /// Store the query inputs for the `hir_package` query.
    ///
    /// Must be called before the first invocation of `hir_package(())`.
    pub fn set_hir_input(&self, input: Arc<HirQueryInput>) {
        *self.hir_input.borrow_mut() = Some(input);
    }

    /// Retrieve the current query input, if any.
    pub fn hir_input(&self) -> Option<Arc<HirQueryInput>> {
        self.hir_input.borrow().clone()
    }

    /// Internal implementation of the `hir_package` query.
    fn hir_package_impl(&self) -> Arc<HirPackageBox> {
        // Fast path: already computed.
        {
            let cache = self.hir_cache.borrow();
            if let Some(result) = cache.as_ref() {
                return result.clone();
            }
        }

        // Slow path: call the registered provider function.
        let result = (self.providers.hir_package)(self);
        *self.hir_cache.borrow_mut() = Some(result.clone());
        result
    }
}

impl Default for Db {
    fn default() -> Self {
        Self::new()
    }
}

impl query::LunaDbBase for Db {}
