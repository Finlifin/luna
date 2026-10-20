//! Luna compiler database — the root of the query system.
//!
//! # Architecture
//!
//! ```text
//!   ┌──────────────────────────────────────┐
//!   │  Db                                  │
//!   │  ├ storage      – salsa Database     │
//!   │  ├ ty_ctxt      – type arena         │
//!   │  ├ providers    – fn-ptr dispatch    │
//!   │  ├ query_engine – stack + hooks      │
//!   │  ├ hir_input    – query inputs       │
//!   │  └ caches       – per-query memo     │
//!   └──────────────────────────────────────┘
//! ```
//!
//! # Adding a new query
//!
//! See [`query::howto`] for the full checklist.  Summary:
//!
//! 1. Add a `fn` field to [`Providers`] (+ panicking `Default`).
//! 2. Add a [`query::QueryCache`] field to [`QueryCaches`] / [`Db`].
//! 3. Add a [`LunaDatabase`] method that calls [`query::force_get_with`]
//!    (or [`query::try_get_with`]) and dispatches to `self.providers.*`.
//! 4. Register the provider from the pass crate via `set_providers`.
//! 5. Optionally forward from `interface::Compiler`.
//!
//! The live example is [`LunaDatabase::hir_package`].
//!
//! Providers are registered by calling `ast_lowering::set_providers(&mut db.providers)`
//! before issuing any HIR query.  Query inputs are set via `db.set_hir_input(…)`.

use std::sync::Arc;

use hir::hir_id::LocalDefId;
use query::{QueryCache, QueryDesc, QueryEngine, force_get_with};

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

// ── Query descriptors ─────────────────────────────────────────────────────────

const HIR_PACKAGE: QueryDesc = QueryDesc::new("hir_package");

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
///
/// # Extending
///
/// To add a query `foo(key) -> Val`:
/// ```ignore
/// pub struct Providers {
///     pub hir_package: fn(&Db) -> Arc<HirPackageBox>,
///     pub foo: fn(&Db, Key) -> Val,   // ← new slot
/// }
/// ```
/// and a panicking default in [`Default`].  See [`query::howto`].
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

// ── QueryCaches ───────────────────────────────────────────────────────────────

/// Per-query memo tables owned by [`Db`].
///
/// When adding a query, add a corresponding [`QueryCache`] here and share it
/// via `Arc` so `Db: Clone` keeps a single memo table.
#[derive(Clone, Default)]
pub struct QueryCaches {
    pub hir_package: Arc<QueryCache<(), Arc<HirPackageBox>>>,
}

impl QueryCaches {
    pub fn new() -> Self {
        QueryCaches {
            hir_package: Arc::new(QueryCache::new()),
        }
    }
}

// ── LunaDatabase trait ────────────────────────────────────────────────────────

/// Extension of [`salsa::Database`] with Luna compiler query methods.
pub trait LunaDatabase: salsa::Database {
    /// Downcast to the concrete [`Db`].
    fn as_db(&self) -> &Db;

    // ── ADT lookups (TyCtxt side tables, not demand queries) ──────────────

    fn adt_def(&self, id: NFId) -> Option<Arc<AdtDef>> {
        self.as_db().ty_ctxt.adt_def(id).map(Arc::new)
    }

    // ── Type lookups ──────────────────────────────────────────────────────

    fn def_ty_exists(&self, id: LocalDefId) -> bool {
        self.as_db().ty_ctxt.def_ty(id).is_some()
    }

    // ── HIR queries ───────────────────────────────────────────────────────

    /// Lower the package's AST to HIR, returning an owned [`HirPackageBox`].
    ///
    /// Memoised via [`query::QueryCache`]: the provider runs at most once per
    /// key (`()`) per database instance.  Cycles panic as an ICE.
    ///
    /// # Panics
    ///
    /// Panics if no provider has been registered, no input has been set,
    /// the query is poisoned, or a cycle is detected.
    fn hir_package(&self, key: ()) -> Arc<HirPackageBox> {
        self.as_db().hir_package_query(key)
    }
}

// ── Db ────────────────────────────────────────────────────────────────────────

/// The Luna compiler database.
///
/// Created once per compilation by `CompilerInstance::new()`.
#[derive(Clone)]
#[salsa::db]
pub struct Db {
    /// Salsa storage — host for [`salsa::Database`].
    storage: salsa::Storage<Db>,

    /// Type-interning arena (wrapped in `Arc` for cheap `Clone`).
    pub ty_ctxt: Arc<TyCtxt>,

    /// Registered query providers (function pointers, trivially `Sync`).
    pub providers: Providers,

    /// Active query stack + optional dep-graph hooks.
    pub query_engine: Arc<QueryEngine>,

    /// Inputs for the `hir_package` query (set before calling the query).
    hir_input: Arc<SyncCell<Option<Arc<HirQueryInput>>>>,

    /// Per-query memo tables.
    pub caches: QueryCaches,
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
            query_engine: Arc::new(QueryEngine::new()),
            hir_input: Arc::new(SyncCell::new(None)),
            caches: QueryCaches::new(),
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

    /// `hir_package` query body — goes through the generic query engine.
    fn hir_package_query(&self, key: ()) -> Arc<HirPackageBox> {
        force_get_with(
            &self.query_engine,
            &self.caches.hir_package,
            HIR_PACKAGE,
            key,
            |_| "()".to_string(),
            || (self.providers.hir_package)(self),
        )
    }
}

impl Default for Db {
    fn default() -> Self {
        Self::new()
    }
}

impl query::LunaDbBase for Db {}
