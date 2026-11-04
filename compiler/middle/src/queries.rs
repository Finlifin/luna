//! Luna compiler database — the root of the query system.
//!
//! # Architecture
//!
//! ```text
//!   ┌──────────────────────────────────────┐
//!   │  Db                                  │
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

use query::{QueryCache, QueryDesc, QueryEngine, force_get_with};

use crate::hir_package::HirPackageBox;
use crate::hir_query::HirQueryInput;

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

/// Luna compiler queries over session-bound input dependencies.
pub trait LunaDatabase<'session> {
    /// Downcast to the concrete [`Db`].
    fn as_db(&self) -> &Db<'session>;

    // ── HIR queries ───────────────────────────────────────────────────────

    /// Lower the package's AST to HIR, returning an owned [`HirPackageBox`].
    ///
    /// Memoised via [`query::QueryCache`] until input replacement invalidates
    /// the key (`()`) and its observed consumers. Cycles panic as an ICE.
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
/// Input replacement is synchronized by the query engine, but that does not
/// make the arenas or diagnostic sinks thread-safe. Auto traits retain those
/// restrictions instead of imposing a manual `Send`/`Sync` promise.
///
/// ```compile_fail
/// fn transfer(db: middle::Db<'static>) {
///     std::thread::spawn(move || drop(db));
/// }
/// ```
///
/// Session dependencies cannot be retained by a longer-lived database.
///
/// ```compile_fail
/// fn remember<'session>(db: &middle::Db<'static>,
///     input: std::sync::Arc<middle::HirQueryInput<'session>>) {
///     db.set_hir_input(input);
/// }
/// ```
#[derive(Clone)]
pub struct Db<'session> {
    /// Registered query providers (function pointers, trivially `Sync`).
    pub providers: Providers,

    /// Active query stack + optional dep-graph hooks.
    pub query_engine: Arc<QueryEngine>,

    /// Inputs for the `hir_package` query (set before calling the query).
    hir_input: Arc<std::sync::Mutex<Option<Arc<HirQueryInput<'session>>>>>,

    /// Per-query memo tables.
    pub caches: QueryCaches,
}

impl<'session> LunaDatabase<'session> for Db<'session> {
    #[inline]
    fn as_db(&self) -> &Db<'session> {
        self
    }
}

impl<'session> Db<'session> {
    /// Construct an empty compiler database with default (panicking) providers.
    pub fn new() -> Self {
        Db {
            providers: Providers::default(),
            query_engine: Arc::new(QueryEngine::new()),
            hir_input: Arc::new(std::sync::Mutex::new(None)),
            caches: QueryCaches::new(),
        }
    }

    /// Store the query inputs for the `hir_package` query.
    ///
    /// Replacing input invalidates HIR and its observed query consumers.
    /// Input cannot be replaced from inside an active query provider.
    pub fn set_hir_input(&self, input: Arc<HirQueryInput<'session>>) {
        self.query_engine
            .update_input(&self.caches.hir_package, &(), || {
                *self.hir_input.lock().unwrap_or_else(|e| e.into_inner()) = Some(input);
            })
            .expect("HIR input replacement during query evaluation");
    }

    /// Retrieve the current query input, if any.
    pub fn hir_input(&self) -> Option<Arc<HirQueryInput<'session>>> {
        self.hir_input
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
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

impl Default for Db<'_> {
    fn default() -> Self {
        Self::new()
    }
}
