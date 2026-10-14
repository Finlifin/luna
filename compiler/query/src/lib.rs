//! Luna query system — generic infrastructure layer.
//!
//! This crate is the **infrastructure** tier of Luna's query system,
//! analogous to `rustc_query_system`.  It re-exports
//! [salsa](https://crates.io/crates/salsa) and provides keyed memoisation,
//! cycle detection, and panic-safe evaluation for rustc-style provider
//! tables owned by `middle::Db`.
//!
//! # Crate responsibility split
//!
//! | Crate       | Responsibility                                          |
//! |-------------|----------------------------------------------------------|
//! | **query**   | Generic infra: cache, stack, cycles, `try_get_with`     |
//! | `middle`    | Concrete queries: `LunaDatabase` trait + `Db` + `Providers` |
//! | `interface` | Coordinator: owns `Db`, wires up passes                 |
//!
//! # Incremental compilation
//!
//! This layer does **not** implement a dependency graph or incremental
//! invalidation.  Cache entries live for the lifetime of a [`Db`]; the
//! driver rebuilds the database for a fresh compilation.  [`DepNode`] is
//! a placeholder for a future dep-graph hook.
//!
//! # Adding a new query
//!
//! See the module-level docs on [`crate::howto`] and the live example
//! `hir_package` in `middle::queries`.

#![deny(rustdoc::broken_intra_doc_links)]

mod cache;
mod cycle;
mod dep;
mod howto;
mod stack;

pub use cache::{CacheState, QueryCache, QueryResult};
pub use cycle::CycleError;
pub use dep::{DepNode, QueryHooks};
pub use stack::{ActiveQuery, QueryDesc, QueryStack};

use std::cell::RefCell;
use std::hash::Hash;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// Re-export salsa so downstream crates can write `query::salsa::…`
/// or use salsa proc-macro attributes via `use query::salsa`.
pub use salsa;

/// Marker super-trait for all Luna query databases.
///
/// Every concrete database struct (currently only `middle::Db`) must
/// implement this trait in addition to [`salsa::Database`].  It acts as
/// a single extension point for Luna-wide database invariants that are
/// too generic for `middle` but too domain-specific for plain salsa.
pub trait LunaDbBase: salsa::Database + Send {}

/// Shared query-engine state owned by `middle::Db`.
///
/// Holds the active query stack and optional dep-graph hooks.  Caches
/// themselves live next to each query on `Db` (one [`QueryCache`] per
/// query kind).
pub struct QueryEngine {
    stack: RefCell<QueryStack>,
    hooks: QueryHooks,
}

impl QueryEngine {
    pub fn new() -> Self {
        QueryEngine {
            stack: RefCell::new(QueryStack::new()),
            hooks: QueryHooks::noop(),
        }
    }

    pub fn with_hooks(hooks: QueryHooks) -> Self {
        QueryEngine {
            stack: RefCell::new(QueryStack::new()),
            hooks,
        }
    }

    pub fn stack(&self) -> std::cell::Ref<'_, QueryStack> {
        self.stack.borrow()
    }

    pub fn hooks(&self) -> &QueryHooks {
        &self.hooks
    }
}

impl Default for QueryEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for QueryEngine {
    fn clone(&self) -> Self {
        // Fresh engine: do not share in-flight stack across clones.
        QueryEngine::with_hooks(self.hooks.clone())
    }
}

// SAFETY: Luna is single-threaded; `QueryEngine` is only touched on the
// main thread.  Required because salsa 0.22 bounds `Db: Sync`.
unsafe impl Sync for QueryEngine {}

/// Look up `key` in `cache`, computing it with `compute` on a miss.
///
/// # Behaviour
///
/// 1. If the cache holds [`CacheState::Complete`], return a clone.
/// 2. If [`CacheState::InProgress`], build a [`CycleError`] from the
///    active stack (including the would-be re-entrant frame) and return
///    [`QueryResult::Cycle`].
/// 3. If [`CacheState::Poisoned`], return [`QueryResult::Poisoned`].
/// 4. Otherwise mark `InProgress`, push onto the stack, run `compute`
///    under `catch_unwind`.  On success store `Complete`; on panic store
///    `Poisoned` and resume unwind.
///
/// `key_display` is only used for cycle / stack diagnostics.
pub fn try_get_with<K, V, D, F>(
    engine: &QueryEngine,
    cache: &QueryCache<K, V>,
    desc: QueryDesc,
    key: K,
    key_display: D,
    compute: F,
) -> QueryResult<V>
where
    K: Eq + Hash + Clone,
    V: Clone,
    D: FnOnce(&K) -> String,
    F: FnOnce() -> V,
{
    // Format the key once for diagnostics / hooks.
    let key_str = key_display(&key);

    // Fast path: already complete / poisoned / in-progress.
    match cache.get(&key) {
        Some(CacheState::Complete(v)) => {
            (engine.hooks.on_query_read)(DepNode {
                query: desc.name,
                key: key_str,
            });
            return QueryResult::Ok(v);
        }
        Some(CacheState::Poisoned) => return QueryResult::Poisoned,
        Some(CacheState::InProgress) => {
            let mut frames = engine.stack.borrow().frames().to_vec();
            frames.push(ActiveQuery { desc, key: key_str });
            return QueryResult::Cycle(CycleError { frames });
        }
        Some(CacheState::NotComputed) | None => {}
    }

    let dep = DepNode {
        query: desc.name,
        key: key_str.clone(),
    };
    (engine.hooks.on_query_start)(dep.clone());

    // Claim the slot before entering compute so re-entrant calls see InProgress.
    if !cache.start(&key) {
        match cache.get(&key) {
            Some(CacheState::Complete(v)) => return QueryResult::Ok(v),
            Some(CacheState::Poisoned) => return QueryResult::Poisoned,
            Some(CacheState::InProgress) => {
                let mut frames = engine.stack.borrow().frames().to_vec();
                frames.push(ActiveQuery { desc, key: key_str });
                return QueryResult::Cycle(CycleError { frames });
            }
            _ => {
                // Absent after failed start — claim again.
                let _ = cache.start(&key);
            }
        }
    }

    engine
        .stack
        .borrow_mut()
        .push(ActiveQuery { desc, key: key_str });

    struct StackGuard<'a, K: Eq + Hash + Clone, V> {
        engine: &'a QueryEngine,
        cache: &'a QueryCache<K, V>,
        key: K,
        completed: bool,
    }

    impl<'a, K: Eq + Hash + Clone, V> Drop for StackGuard<'a, K, V> {
        fn drop(&mut self) {
            self.engine.stack.borrow_mut().pop();
            if !self.completed {
                // Panic or early exit: poison so we never return a partial value.
                self.cache.poison(&self.key);
            }
        }
    }

    let mut guard = StackGuard {
        engine,
        cache,
        key: key.clone(),
        completed: false,
    };

    let outcome = catch_unwind(AssertUnwindSafe(compute));

    match outcome {
        Ok(value) => {
            cache.complete(&key, value.clone());
            guard.completed = true;
            drop(guard);
            (engine.hooks.on_query_end)(dep);
            QueryResult::Ok(value)
        }
        Err(payload) => {
            // Drop guard poisons + pops; then resume.
            drop(guard);
            (engine.hooks.on_query_end)(dep);
            std::panic::resume_unwind(payload);
        }
    }
}

/// Convenience: like [`try_get_with`], but panics on cycle / poison.
///
/// Prefer [`try_get_with`] when the caller can emit diagnostics from
/// [`CycleError`].  Provider-driven queries that treat cycles as ICE
/// can use this.
pub fn force_get_with<K, V, F>(
    engine: &QueryEngine,
    cache: &QueryCache<K, V>,
    desc: QueryDesc,
    key: K,
    key_display: impl FnOnce(&K) -> String,
    compute: F,
) -> V
where
    K: Eq + Hash + Clone,
    V: Clone,
    F: FnOnce() -> V,
{
    match try_get_with(engine, cache, desc, key, key_display, compute) {
        QueryResult::Ok(v) => v,
        QueryResult::Cycle(err) => panic!("query cycle detected:\n{err}"),
        QueryResult::Poisoned => panic!("query `{}` was poisoned by a previous panic", desc.name),
    }
}

/// Shared [`QueryEngine`] wrapped for cheap `Clone` of `Db`.
pub type SharedQueryEngine = Arc<QueryEngine>;
