//! Luna query system — generic infrastructure layer.
//!
//! This crate is the **infrastructure** tier of Luna's query system,
//! analogous to `rustc_query_system`. It provides keyed memoisation,
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
//! # Shared access
//!
//! Cache and stack state use synchronization rather than unchecked `Sync`.
//! One engine serializes provider evaluation across threads, while allowing
//! recursive queries on its owning thread. Providers must not wait for work
//! that re-enters the same engine from another thread. This is a safety boundary,
//! not a parallel query scheduler; caches belong to one engine.
//!
//! # Incremental compilation
//!
//! The engine records invocation-local query dependencies and invalidates
//! observed consumers through [`QueryEngine::update_input`]. Cached reads are
//! dependencies too. Diagnostic [`DepNode`] strings are not cache identities.
//! Persistent fingerprints, unchanged-result reuse and cross-session storage
//! are not implemented; the driver still creates a database per compilation.
//!
//! # Adding a new query
//!
//! See the module-level docs on [`crate::howto`] and the live example
//! `hir_package` in `middle::queries`.

#![deny(rustdoc::broken_intra_doc_links)]

mod cache;
mod cycle;
mod dep;
mod dependencies;
mod howto;
mod stack;

pub use cache::{CacheState, QueryCache, QueryResult};
pub use cycle::CycleError;
pub use dep::{DepNode, QueryHooks};
pub use stack::{ActiveQuery, QueryDesc, QueryStack};

use std::hash::Hash;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::{Condvar, Mutex};

/// Structured error returned when query evaluation cannot produce a value
/// (cycle, poisoned prior panic, or internal dependency invariant violation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryFailure {
    Cycle(CycleError),
    Poisoned { query: &'static str },
    DependencyInvariant { detail: &'static str },
}

impl std::fmt::Display for QueryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cycle(err) => write!(f, "query cycle detected:\n{err}"),
            Self::Poisoned { query } => {
                write!(f, "query `{query}` was poisoned by a previous panic")
            }
            Self::DependencyInvariant { detail } => {
                write!(f, "query dependency invariant failed: {detail}")
            }
        }
    }
}

impl std::error::Error for QueryFailure {}

impl<V> QueryResult<V> {
    /// Convert a [`QueryResult`] into a standard [`Result`], mapping `Cycle` and
    /// `Poisoned` to [`QueryFailure`].
    pub fn into_result(self, query: &'static str) -> Result<V, QueryFailure> {
        match self {
            Self::Ok(v) => Ok(v),
            Self::Cycle(err) => Err(QueryFailure::Cycle(err)),
            Self::Poisoned => Err(QueryFailure::Poisoned { query }),
        }
    }
}

/// Shared query-engine state owned by `middle::Db`.
///
/// Holds the active query stack and optional dep-graph hooks.  Caches
/// themselves live next to each query on `Db` (one [`QueryCache`] per
/// query kind).
pub struct QueryEngine {
    stack: Mutex<QueryStack>,
    owner: Mutex<Option<(std::thread::ThreadId, usize)>>,
    available: Condvar,
    hooks: QueryHooks,
    dependencies: Mutex<dependencies::Dependencies>,
}

impl QueryEngine {
    pub fn new() -> Self {
        QueryEngine {
            stack: Mutex::new(QueryStack::new()),
            owner: Mutex::new(None),
            available: Condvar::new(),
            dependencies: Mutex::new(Default::default()),
            hooks: QueryHooks::noop(),
        }
    }

    pub fn with_hooks(hooks: QueryHooks) -> Self {
        QueryEngine {
            stack: Mutex::new(QueryStack::new()),
            owner: Mutex::new(None),
            available: Condvar::new(),
            dependencies: Mutex::new(Default::default()),
            hooks,
        }
    }

    /// Snapshot the active stack without exposing a lock across query calls.
    pub fn stack(&self) -> QueryStack {
        self.stack.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn enter(&self) -> EvaluationGuard<'_> {
        let current = std::thread::current().id();
        let mut owner = self.owner.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            match owner.as_mut() {
                None => {
                    *owner = Some((current, 1));
                    break;
                }
                Some((thread, depth)) if *thread == current => {
                    *depth += 1;
                    break;
                }
                Some(_) => {
                    owner = self
                        .available
                        .wait(owner)
                        .unwrap_or_else(|e| e.into_inner());
                }
            }
        }
        EvaluationGuard { engine: self }
    }

    /// Change an input and invalidate its query plus all observed consumers.
    /// Invalidates before running update, so a panic cannot leave stale results valid.
    /// Reentrant input changes are rejected before either mutation takes place.
    pub fn update_input<K: Eq + Hash + Clone, V>(
        &self,
        cache: &QueryCache<K, V>,
        key: &K,
        update: impl FnOnce(),
    ) -> Result<(), InputUpdateError> {
        let _evaluation = self.enter();
        if !self.stack().is_empty() {
            return Err(InputUpdateError::DuringQuery);
        }
        let node = cache.node(key).map_err(InputUpdateError::Failure)?;
        self.dependencies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .invalidate(node);
        struct UpdateGuard<'a> {
            engine: &'a QueryEngine,
            node: dependencies::NodeId,
        }
        impl Drop for UpdateGuard<'_> {
            fn drop(&mut self) {
                self.engine
                    .dependencies
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .invalidate(self.node);
            }
        }
        // A callback may read queries before finishing its mutation. Retire
        // those intermediate results on both normal return and unwinding.
        let _update = UpdateGuard { engine: self, node };
        update();
        Ok(())
    }

    /// Look up `key` in `cache`, computing it with `compute` on a miss and
    /// returning structured [`QueryFailure`] on cycle, poison, or dependency error.
    pub fn get<K, V, D, F>(
        &self,
        cache: &QueryCache<K, V>,
        desc: QueryDesc,
        key: K,
        key_display: D,
        compute: F,
    ) -> Result<V, QueryFailure>
    where
        K: Eq + Hash + Clone,
        V: Clone,
        D: FnOnce(&K) -> String,
        F: FnOnce() -> V,
    {
        try_get_with(self, cache, desc, key, key_display, compute)
    }

    /// Diagnostic session-local identities; not persistent cache keys.
    pub fn dependency_dump(&self) -> String {
        self.dependencies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .to_string()
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
        // Fresh evaluation stack across clones, while preserving committed
        // dependency edges and dirty flags so cloned caches stay consistent.
        let dependencies = self
            .dependencies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone_settled();
        QueryEngine {
            stack: Mutex::new(QueryStack::new()),
            owner: Mutex::new(None),
            available: Condvar::new(),
            hooks: self.hooks.clone(),
            dependencies: Mutex::new(dependencies),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputUpdateError {
    DuringQuery,
    Failure(QueryFailure),
}

impl std::fmt::Display for InputUpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuringQuery => {
                f.write_str("cannot update query input during an active evaluation")
            }
            Self::Failure(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for InputUpdateError {}

pub type InputUpdateDuringQuery = InputUpdateError;

// Providers may recursively request queries on this thread. Other threads wait
// until the complete evaluation finishes, including its hooks and unwind path.
struct EvaluationGuard<'a> {
    engine: &'a QueryEngine,
}

impl Drop for EvaluationGuard<'_> {
    fn drop(&mut self) {
        let mut owner = self.engine.owner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, depth)) = owner.as_mut() {
            *depth -= 1;
            if *depth == 0 {
                *owner = None;
                self.engine.available.notify_all();
            }
        }
    }
}

/// Look up `key` in `cache`, computing it with `compute` on a miss.
///
/// # Behaviour
///
/// 1. If the cache holds [`CacheState::Complete`], return a clone.
/// 2. If [`CacheState::InProgress`], build a [`CycleError`] from the
///    active stack (including the would-be re-entrant frame) and return
///    [`Err(QueryFailure::Cycle(..))`].
/// 3. If [`CacheState::Poisoned`], return [`Err(QueryFailure::Poisoned { .. })`].
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
) -> Result<V, QueryFailure>
where
    K: Eq + Hash + Clone,
    V: Clone,
    D: FnOnce(&K) -> String,
    F: FnOnce() -> V,
{
    let _evaluation = engine.enter();
    // Format the key once for diagnostics / hooks.
    let key_str = key_display(&key);

    let node = cache.node(&key)?;
    if engine
        .dependencies
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .read(
            node,
            DepNode {
                query: desc.name,
                key: key_str.clone(),
            },
        )
    {
        cache.invalidate(&key);
    }

    // Fast path: already complete / poisoned / in-progress.
    match cache.get(&key) {
        Some(CacheState::Complete(v)) => {
            (engine.hooks.on_query_read)(DepNode {
                query: desc.name,
                key: key_str,
            });
            return Ok(v);
        }
        Some(CacheState::Poisoned) => return Err(QueryFailure::Poisoned { query: desc.name }),
        Some(CacheState::InProgress) => {
            let mut frames = engine.stack().frames().to_vec();
            frames.push(ActiveQuery { desc, key: key_str });
            return Err(QueryFailure::Cycle(CycleError { frames }));
        }
        Some(CacheState::NotComputed) | None => {}
    }

    let dep = DepNode {
        query: desc.name,
        key: key_str.clone(),
    };

    // Claim the slot before entering compute so re-entrant calls see InProgress.
    if !cache.start(&key) {
        match cache.get(&key) {
            Some(CacheState::Complete(v)) => return Ok(v),
            Some(CacheState::Poisoned) => {
                return Err(QueryFailure::Poisoned { query: desc.name });
            }
            Some(CacheState::InProgress) => {
                let mut frames = engine.stack().frames().to_vec();
                frames.push(ActiveQuery { desc, key: key_str });
                return Err(QueryFailure::Cycle(CycleError { frames }));
            }
            _ => {
                // Absent after failed start — claim again.
                let _ = cache.start(&key);
            }
        }
    }

    engine
        .stack
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(ActiveQuery { desc, key: key_str });

    struct StackGuard<'a, K: Eq + Hash + Clone, V> {
        engine: &'a QueryEngine,
        cache: &'a QueryCache<K, V>,
        key: K,
        completed: bool,
    }

    impl<'a, K: Eq + Hash + Clone, V> StackGuard<'a, K, V> {
        fn finish_ok(mut self) -> Result<(), QueryFailure> {
            let finish_res = self
                .engine
                .dependencies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .finish();
            self.engine
                .stack
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop();
            self.completed = finish_res.is_ok();
            if finish_res.is_err() {
                self.cache.poison(&self.key);
            }
            std::mem::forget(self);
            finish_res
        }
    }

    impl<'a, K: Eq + Hash + Clone, V> Drop for StackGuard<'a, K, V> {
        fn drop(&mut self) {
            let _ = self
                .engine
                .dependencies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .finish();
            self.engine
                .stack
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop();
            if !self.completed {
                // Panic or early exit: poison so we never return a partial value.
                self.cache.poison(&self.key);
            }
        }
    }

    engine
        .dependencies
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .begin(node);
    let guard = StackGuard {
        engine,
        cache,
        key: key.clone(),
        completed: false,
    };

    let outcome = catch_unwind(AssertUnwindSafe(|| {
        (engine.hooks.on_query_start)(dep.clone());
        compute()
    }));

    match outcome {
        Ok(value) => {
            cache.complete(&key, value.clone());
            guard.finish_ok()?;
            (engine.hooks.on_query_end)(dep);
            Ok(value)
        }
        Err(payload) => {
            // Drop guard poisons + pops; then resume.
            drop(guard);
            (engine.hooks.on_query_end)(dep);
            std::panic::resume_unwind(payload);
        }
    }
}

/// Convenience alias for [`try_get_with`] that returns `Result<V, QueryFailure>`
/// without panicking on query cycles or poisoned slots.
pub fn force_get_with<K, V, F>(
    engine: &QueryEngine,
    cache: &QueryCache<K, V>,
    desc: QueryDesc,
    key: K,
    key_display: impl FnOnce(&K) -> String,
    compute: F,
) -> Result<V, QueryFailure>
where
    K: Eq + Hash + Clone,
    V: Clone,
    F: FnOnce() -> V,
{
    try_get_with(engine, cache, desc, key, key_display, compute)
}

/// Shared [`QueryEngine`] wrapped for cheap `Clone` of `Db`.
pub type SharedQueryEngine = Arc<QueryEngine>;
