//! Luna compiler database — the root of the query system.
//!
//! # Architecture
//!
//! ```text
//!   ┌──────────────────────────────────────────┐
//!   │  Db                                      │
//!   │  ├ providers         – fn-ptr dispatch   │
//!   │  ├ query_engine      – stack + hooks     │
//!   │  ├ hir_input         – HIR query inputs  │
//!   │  ├ elaboration_input – mid-end inputs    │
//!   │  └ caches            – per-query memo    │
//!   └──────────────────────────────────────────┘
//! ```
//!
//! # Adding a new query
//!
//! See [`query::howto`] for the full checklist.  Summary:
//!
//! 1. Add a `fn` field to [`Providers`] (+ default or panicking `Default`).
//! 2. Add a [`query::QueryCache`] field to [`QueryCaches`] / [`Db`].
//! 3. Add a [`LunaDatabase`] method that calls [`query::force_get_with`]
//!    (or [`query::try_get_with`]) and dispatches to `self.providers.*`.
//! 4. Register the provider from the pass crate via `set_providers`.
//! 5. Optionally forward from `interface::Compiler`.
//!
//! The live examples include [`LunaDatabase::hir_package`],
//! [`LunaDatabase::package_interface`], [`LunaDatabase::item_signature`],
//! [`LunaDatabase::package_trait_impls`], and [`LunaDatabase::elaborated_package`].
//!
//! Providers are registered by calling `ast_lowering::set_providers(&mut db.providers)`
//! before issuing HIR queries.  Query inputs are set via `db.set_hir_input(…)`
//! and `db.set_elaboration_input(…)`.

use std::sync::Arc;

use hir::OwnerId;
use query::{QueryCache, QueryDesc, QueryEngine, QueryResult, force_get_with, try_get_with};

use crate::hir_package::HirPackageBox;
use crate::hir_query::HirQueryInput;
use crate::package_interface::{
    ElaboratedPackage, ElaborationQueryInput, ItemInterfaceSummary, PackageInterface,
    PackageTraitImpls,
};

// ── Query descriptors ─────────────────────────────────────────────────────────

const HIR_PACKAGE: QueryDesc = QueryDesc::new("hir_package");

const PACKAGE_INTERFACE: QueryDesc = QueryDesc::new("package_interface");

const ITEM_SIGNATURE: QueryDesc = QueryDesc::new("item_signature");

const PACKAGE_TRAIT_IMPLS: QueryDesc = QueryDesc::new("package_trait_impls");

const ELABORATED_PACKAGE: QueryDesc = QueryDesc::new("elaborated_package");

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
/// and a default or panicking stub in [`Default`].  See [`query::howto`].
#[derive(Clone, Copy)]
pub struct Providers {
    /// Provider for the `hir_package` query.
    pub hir_package: fn(&Db) -> Arc<HirPackageBox>,
    /// Provider for the `package_interface` query (defaults to extracting from `hir_package`).
    pub package_interface: fn(&Db) -> Arc<PackageInterface>,
    /// Provider for the per-owner `item_signature` query (defaults to reading `package_interface`).
    pub item_signature: fn(&Db, OwnerId) -> Option<Arc<ItemInterfaceSummary>>,
    /// Provider for the `package_trait_impls` query (defaults to reading `package_interface` and `hir_input`).
    pub package_trait_impls: fn(&Db) -> Arc<PackageTraitImpls>,
    /// Provider for the `elaborated_package` query.
    pub elaborated_package: fn(&Db) -> Arc<ElaboratedPackage>,
}

fn default_package_interface(db: &Db) -> Arc<PackageInterface> {
    let hir = db.hir_package(());
    Arc::new(PackageInterface::from_hir(hir.package()))
}

fn default_item_signature(db: &Db, owner: OwnerId) -> Option<Arc<ItemInterfaceSummary>> {
    let interface = db.package_interface(());
    interface.get(owner).cloned()
}

fn default_package_trait_impls(db: &Db) -> Arc<PackageTraitImpls> {
    let interface = db.package_interface(());
    let resolved_impls = db
        .hir_input()
        .map_or(0, |input| input.module_tree.impls.len());
    Arc::new(PackageTraitImpls::from_interface(
        &interface,
        resolved_impls,
    ))
}

impl Default for Providers {
    fn default() -> Self {
        fn missing_hir_package(_: &Db) -> Arc<HirPackageBox> {
            panic!(
                "hir_package: no provider registered — \
                 call ast_lowering::set_providers(&mut db.providers) first"
            )
        }
        fn missing_elaborated_package(_: &Db) -> Arc<ElaboratedPackage> {
            panic!(
                "elaborated_package: no provider registered — \
                 register db.providers.elaborated_package before querying elaboration"
            )
        }
        Providers {
            hir_package: missing_hir_package,
            package_interface: default_package_interface,
            item_signature: default_item_signature,
            package_trait_impls: default_package_trait_impls,
            elaborated_package: missing_elaborated_package,
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
    pub package_interface: Arc<QueryCache<(), Arc<PackageInterface>>>,
    pub item_signature: Arc<QueryCache<OwnerId, Option<Arc<ItemInterfaceSummary>>>>,
    pub package_trait_impls: Arc<QueryCache<(), Arc<PackageTraitImpls>>>,
    pub elaborated_package: Arc<QueryCache<(), Arc<ElaboratedPackage>>>,
}

impl QueryCaches {
    pub fn new() -> Self {
        QueryCaches {
            hir_package: Arc::new(QueryCache::new()),
            package_interface: Arc::new(QueryCache::new()),
            item_signature: Arc::new(QueryCache::new()),
            package_trait_impls: Arc::new(QueryCache::new()),
            elaborated_package: Arc::new(QueryCache::new()),
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

    // ── Mid-end interface, signature & elaboration queries ─────────────────

    /// Compute the package's owned [`PackageInterface`] summary.
    ///
    /// By default reads `hir_package(())`, so replacing HIR inputs
    /// transitively invalidates cached package interfaces.
    fn package_interface(&self, key: ()) -> Arc<PackageInterface> {
        self.as_db().package_interface_query(key)
    }

    /// Look up the [`ItemInterfaceSummary`] for a single [`OwnerId`].
    ///
    /// Memoised per owner and invalidated transitively when upstream package
    /// HIR/interface inputs change.
    fn item_signature(&self, owner: OwnerId) -> Option<Arc<ItemInterfaceSummary>> {
        self.as_db().item_signature_query(owner)
    }

    /// Cycle-safe variant of [`LunaDatabase::item_signature`].
    fn try_item_signature(&self, owner: OwnerId) -> QueryResult<Option<Arc<ItemInterfaceSummary>>> {
        self.as_db().try_item_signature_query(owner)
    }

    /// Compute the package's [`PackageTraitImpls`] inventory.
    fn package_trait_impls(&self, key: ()) -> Arc<PackageTraitImpls> {
        self.as_db().package_trait_impls_query(key)
    }

    /// Run mid-end package elaboration via the registered provider.
    ///
    /// # Panics
    ///
    /// Panics if no provider has been registered, the query is poisoned, or a
    /// cycle is detected.
    fn elaborated_package(&self, key: ()) -> Arc<ElaboratedPackage> {
        self.as_db().elaborated_package_query(key)
    }

    /// Cycle-safe variant of [`LunaDatabase::elaborated_package`].
    fn try_elaborated_package(&self, key: ()) -> QueryResult<Arc<ElaboratedPackage>> {
        self.as_db().try_elaborated_package_query(key)
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
///
/// Registering an `elaborated_package` provider connects mid-end queries to
/// upstream `hir_package` / `package_interface` results with automatic
/// memoisation and dependency invalidation:
///
/// ```
/// use std::collections::BTreeMap;
/// use std::sync::Arc;
/// use middle::{Db, ElaboratedPackage, ElaborationQueryInput, LunaDatabase};
///
/// fn elaborate_stub(db: &Db) -> Arc<ElaboratedPackage> {
///     let config = db.elaboration_input().expect("elaboration input set");
///     Arc::new(ElaboratedPackage {
///         succeeded: !config.package_identities.is_empty(),
///         struct_count: 0,
///         enum_count: 0,
///         function_count: 0,
///         trait_count: 0,
///         impl_count: 0,
///         warnings: Vec::new(),
///         errors: Vec::new(),
///         nir_lisp: "(package)".into(),
///     })
/// }
///
/// let mut db = Db::new();
/// db.providers.elaborated_package = elaborate_stub;
/// db.set_elaboration_input(Arc::new(ElaborationQueryInput::new(BTreeMap::from([(0, 1)]))));
/// let first = db.elaborated_package(());
/// assert!(first.succeeded);
/// assert!(Arc::ptr_eq(&first, &db.elaborated_package(())));
/// ```
#[derive(Clone)]
pub struct Db<'session> {
    /// Registered query providers (function pointers, trivially `Sync`).
    pub providers: Providers,

    /// Active query stack + optional dep-graph hooks.
    pub query_engine: Arc<QueryEngine>,

    /// Inputs for the `hir_package` query (set before calling the query).
    hir_input: Arc<std::sync::Mutex<Option<Arc<HirQueryInput<'session>>>>>,

    /// Inputs for the `elaborated_package` query.
    elaboration_input: Arc<std::sync::Mutex<Option<Arc<ElaborationQueryInput>>>>,

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
    /// Construct an empty compiler database with default providers.
    pub fn new() -> Self {
        Db {
            providers: Providers::default(),
            query_engine: Arc::new(QueryEngine::new()),
            hir_input: Arc::new(std::sync::Mutex::new(None)),
            elaboration_input: Arc::new(std::sync::Mutex::new(None)),
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

    /// Retrieve the current HIR query input, if any.
    pub fn hir_input(&self) -> Option<Arc<HirQueryInput<'session>>> {
        self.hir_input
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Store the configuration input for the `elaborated_package` query.
    ///
    /// Replacing input invalidates `elaborated_package` and its observed
    /// consumers without invalidating `hir_package`.
    pub fn set_elaboration_input(&self, input: Arc<ElaborationQueryInput>) {
        self.query_engine
            .update_input(&self.caches.elaborated_package, &(), || {
                *self
                    .elaboration_input
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(input);
            })
            .expect("elaboration input replacement during query evaluation");
    }

    /// Retrieve the current elaboration query input, if any.
    pub fn elaboration_input(&self) -> Option<Arc<ElaborationQueryInput>> {
        self.elaboration_input
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

    fn package_interface_query(&self, key: ()) -> Arc<PackageInterface> {
        force_get_with(
            &self.query_engine,
            &self.caches.package_interface,
            PACKAGE_INTERFACE,
            key,
            |_| "()".to_string(),
            || (self.providers.package_interface)(self),
        )
    }

    fn item_signature_query(&self, owner: OwnerId) -> Option<Arc<ItemInterfaceSummary>> {
        force_get_with(
            &self.query_engine,
            &self.caches.item_signature,
            ITEM_SIGNATURE,
            owner,
            |id| format!("{id}"),
            || (self.providers.item_signature)(self, owner),
        )
    }

    fn try_item_signature_query(
        &self,
        owner: OwnerId,
    ) -> QueryResult<Option<Arc<ItemInterfaceSummary>>> {
        try_get_with(
            &self.query_engine,
            &self.caches.item_signature,
            ITEM_SIGNATURE,
            owner,
            |id| format!("{id}"),
            || (self.providers.item_signature)(self, owner),
        )
    }

    fn package_trait_impls_query(&self, key: ()) -> Arc<PackageTraitImpls> {
        force_get_with(
            &self.query_engine,
            &self.caches.package_trait_impls,
            PACKAGE_TRAIT_IMPLS,
            key,
            |_| "()".to_string(),
            || (self.providers.package_trait_impls)(self),
        )
    }

    fn elaborated_package_query(&self, key: ()) -> Arc<ElaboratedPackage> {
        force_get_with(
            &self.query_engine,
            &self.caches.elaborated_package,
            ELABORATED_PACKAGE,
            key,
            |_| "()".to_string(),
            || (self.providers.elaborated_package)(self),
        )
    }

    fn try_elaborated_package_query(&self, key: ()) -> QueryResult<Arc<ElaboratedPackage>> {
        try_get_with(
            &self.query_engine,
            &self.caches.elaborated_package,
            ELABORATED_PACKAGE,
            key,
            |_| "()".to_string(),
            || (self.providers.elaborated_package)(self),
        )
    }
}

impl Default for Db<'_> {
    fn default() -> Self {
        Self::new()
    }
}
