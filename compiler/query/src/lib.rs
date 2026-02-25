//! # Luna Query System
//!
//! A demand-driven, memoized computation framework for the Luna compiler,
//! modeled after [rustc's query system][rustc-query].
//!
//! [rustc-query]: https://rustc-dev-guide.rust-lang.org/query.html
//!
//! ## Core Concepts
//!
//! | Concept            | Type                     | Role |
//! |--------------------|--------------------------|------|
//! | **Query cache**    | [`QueryCache<K, V>`]     | Per-query memoization store. One instance per query kind. |
//! | **Query engine**   | [`QueryEngine`]          | Global coordinator: tracks in-flight queries, detects cycles. |
//! | **Provider**       | `fn(Compiler, &K) -> V`  | The function that *computes* a query result. Lives in the luna crate. |
//! | **Cycle error**    | [`CycleError`]           | Returned when a dependency cycle is detected among queries. |
//!
//! ## Architecture Overview
//!
//! ```text
//!   CompilerInstance
//!       ├── query_engine: QueryEngine        ← global active-query stack
//!       └── queries: Queries                 ← all QueryCache<K,V> fields
//!
//!   Compiler<'c>  (thin handle, Deref → CompilerInstance)
//!       │
//!       ├── cx.my_query(key)                ← user-facing query method
//!       │       │
//!       │       └── query_engine.execute(
//!       │               &queries.my_query,  ← the cache
//!       │               "my_query",         ← name for diagnostics
//!       │               &key,               ← the input
//!       │               |k| describe(k),    ← human-readable description
//!       │               |k| provider(cx,k), ← calls the provider
//!       │           )
//!       │
//!       └── ... other queries ...
//! ```
//!
//! ## How to Add a New Query (Step by Step)
//!
//! ### 1. Choose key and value types
//!
//! The **key** (`K`) must implement `Eq + Hash + Clone + Debug`.
//! The **value** (`V`) must implement `Clone`.
//!
//! Good keys: `FileId`, `AstNodeId`, `(FileId, u32)`, interned symbols, …
//!
//! ```ignore
//! // Example: a query that resolves the type of a definition.
//! // Key   = AstNodeId   (which definition?)
//! // Value = TypeId       (what is its type?)
//! ```
//!
//! ### 2. Add a `QueryCache` field
//!
//! In the luna crate, find the `Queries` struct (or create one) inside
//! `compiler/mod.rs` and add a field:
//!
//! ```ignore
//! pub struct Queries {
//!     pub type_of: QueryCache<AstNodeId, TypeId>,
//!     // ... other queries ...
//! }
//! ```
//!
//! Initialize it in `Queries::new()`:
//!
//! ```ignore
//! impl Queries {
//!     pub fn new() -> Self {
//!         Queries {
//!             type_of: QueryCache::new(),
//!         }
//!     }
//! }
//! ```
//!
//! ### 3. Write the provider function
//!
//! The provider is a plain function that takes `(Compiler<'c>, &K) -> V`.
//! It may call other queries through `cx`.
//!
//! ```ignore
//! // In some module, e.g. `providers.rs`:
//! fn type_of_provider(cx: Compiler<'_>, key: &AstNodeId) -> TypeId {
//!     let (ast, node) = cx.vfs.resolve_node(*key).unwrap();
//!     // ... type-check the node, possibly calling other queries ...
//!     computed_type
//! }
//! ```
//!
//! ### 4. Expose a method on `Compiler`
//!
//! Add a convenience method so callers write `cx.type_of(node_id)`:
//!
//! ```ignore
//! impl<'c> Compiler<'c> {
//!     pub fn type_of(self, key: AstNodeId) -> TypeId {
//!         self.query_engine.execute(
//!             &self.queries.type_of,
//!             "type_of",
//!             &key,
//!             |k| format!("{:?}", k),
//!             |k| providers::type_of_provider(self, k),
//!         )
//!     }
//! }
//! ```
//!
//! That's it. The engine handles caching and cycle detection automatically.
//!
//! ## Cycle Detection
//!
//! Each [`QueryCache`] tracks which keys are currently being computed
//! (the "active set"). The [`QueryEngine`] maintains a global stack of
//! [`QueryInvocation`]s for diagnostic purposes.
//!
//! If a provider tries to compute a key that is already in the active set
//! of *any* cache, a [`CycleError`] is raised containing the full query
//! stack so the developer can diagnose the loop.
//!
//! ## Panic Safety
//!
//! Active-set bookkeeping uses RAII guards ([`ActiveKeyGuard`],
//! [`ActiveStackGuard`]) so that a panic inside a provider will still
//! correctly clean up the active state, preventing "phantom cycle"
//! errors on subsequent queries.
//!
//! ## Incremental Compilation (Future)
//!
//! The current design does **not** track fine-grained dependencies between
//! queries. To support incremental compilation later:
//!
//! 1. Add a `dependencies: Vec<QueryInvocation>` to each cache entry.
//! 2. Record which queries the provider called (the engine already has
//!    the active stack — tap into it).
//! 3. On file change, invalidate affected caches and transitively
//!    invalidate dependents.
//!
//! The [`QueryCache::invalidate`] and [`QueryCache::clear`] methods are
//! provided as building blocks for this future work.

use std::cell::RefCell;
use std::fmt;
use std::hash::Hash;

use rustc_data_structures::fx::{FxHashMap, FxHashSet};

// ── QueryInvocation ──────────────────────────────────────────────────────────

/// Describes a single in-flight query computation.
///
/// Used for cycle-error diagnostics and debug logging.
#[derive(Clone)]
pub struct QueryInvocation {
    /// Static name of the query kind (e.g. `"type_of"`).
    pub name: &'static str,
    /// Human-readable description of this particular invocation
    /// (e.g. `"type_of(FileId(3), node 42)"`).
    pub description: String,
}

impl fmt::Display for QueryInvocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.description)
    }
}

impl fmt::Debug for QueryInvocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

// ── CycleError ───────────────────────────────────────────────────────────────

/// Error produced when a dependency cycle is detected among queries.
///
/// Contains the full query stack at the point of detection so the
/// developer can see which queries form the loop.
#[derive(Debug)]
pub struct CycleError {
    /// The query invocations forming the cycle, from outermost to the
    /// repeated invocation that closed the loop.
    pub cycle: Vec<QueryInvocation>,
}

impl fmt::Display for CycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "query cycle detected:")?;
        for (i, q) in self.cycle.iter().enumerate() {
            writeln!(f, "  #{}: {}", i, q)?;
        }
        Ok(())
    }
}

impl std::error::Error for CycleError {}

// ── QueryEngine ──────────────────────────────────────────────────────────────

/// Global query coordinator.
///
/// Maintains the stack of currently executing queries for cycle-error
/// diagnostics. One instance lives in [`CompilerInstance`].
///
/// All actual memoization is in per-query [`QueryCache`]s; the engine
/// merely orchestrates execution.
pub struct QueryEngine {
    /// Stack of currently executing query invocations (outermost first).
    active: RefCell<Vec<QueryInvocation>>,
}

impl QueryEngine {
    /// Create a fresh query engine with an empty active stack.
    pub fn new() -> Self {
        QueryEngine {
            active: RefCell::new(Vec::new()),
        }
    }

    /// Execute a query, returning a cached result or computing it.
    ///
    /// This is the main entry point called by query methods on `Compiler`.
    ///
    /// # Flow
    ///
    /// 1. **Cache hit** → return the memoized value immediately.
    /// 2. **Cycle check** → if `key` is already in `cache`'s active set,
    ///    return [`CycleError`].
    /// 3. **Push** → mark `key` active in both the cache and the global
    ///    stack.
    /// 4. **Compute** → call `compute(key)`. The provider may call other
    ///    queries, growing the stack recursively.
    /// 5. **Pop & cache** → remove from active sets, store the result.
    ///
    /// # Panics
    ///
    /// Does **not** panic. Returns `Err(CycleError)` on cycles. If the
    /// caller wants to ICE on cycles, use [`execute`](Self::execute)
    /// instead.
    pub fn try_execute<K, V>(
        &self,
        cache: &QueryCache<K, V>,
        name: &'static str,
        key: &K,
        describe: impl FnOnce(&K) -> String,
        compute: impl FnOnce(&K) -> V,
    ) -> Result<V, CycleError>
    where
        K: Eq + Hash + Clone,
        V: Clone,
    {
        // 1. Cache hit.
        if let Some(value) = cache.get(key) {
            return Ok(value);
        }

        // 2. Cycle detection (check the cache's per-key active set).
        if cache.is_active(key) {
            let desc = describe(key);
            let mut cycle: Vec<_> = self.active.borrow().clone();
            cycle.push(QueryInvocation {
                name,
                description: desc,
            });
            return Err(CycleError { cycle });
        }

        // 3. Mark active (RAII guards ensure cleanup on panic).
        let desc = describe(key);
        let _cache_guard = cache.mark_active(key.clone());
        let _stack_guard = self.push_active(QueryInvocation {
            name,
            description: desc,
        });

        // 4. Compute.
        let value = compute(key);

        // 5. Cache (guards drop automatically, removing from active sets).
        cache.insert(key.clone(), value.clone());

        Ok(value)
    }

    /// Like [`try_execute`](Self::try_execute), but panics with a
    /// descriptive message on cycle errors (internal compiler error).
    pub fn execute<K, V>(
        &self,
        cache: &QueryCache<K, V>,
        name: &'static str,
        key: &K,
        describe: impl FnOnce(&K) -> String,
        compute: impl FnOnce(&K) -> V,
    ) -> V
    where
        K: Eq + Hash + Clone,
        V: Clone,
    {
        self.try_execute(cache, name, key, describe, compute)
            .unwrap_or_else(|e| {
                panic!("internal compiler error: {}", e);
            })
    }

    /// Current depth of the active query stack (useful for debug logging).
    pub fn depth(&self) -> usize {
        self.active.borrow().len()
    }

    // ── Internal ─────────────────────────────────────────────────────────

    /// Push an invocation onto the global active stack.
    /// Returns an RAII guard that pops on drop.
    fn push_active(&self, invocation: QueryInvocation) -> ActiveStackGuard<'_> {
        self.active.borrow_mut().push(invocation);
        ActiveStackGuard {
            stack: &self.active,
        }
    }
}

impl Default for QueryEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ── QueryCache ───────────────────────────────────────────────────────────────

/// Per-query memoization cache with cycle-detection support.
///
/// Each query kind in the compiler gets its own `QueryCache<K, V>`.
/// The cache stores computed results and tracks which keys are currently
/// being computed (for cycle detection).
///
/// # Type Parameters
///
/// - `K` – the query key (must be `Eq + Hash + Clone`).
/// - `V` – the query value (must be `Clone`).
pub struct QueryCache<K: Eq + Hash, V> {
    /// Memoized results.
    results: RefCell<FxHashMap<K, V>>,
    /// Keys currently being computed (for cycle detection).
    active: RefCell<FxHashSet<K>>,
}

impl<K: Eq + Hash + Clone, V: Clone> QueryCache<K, V> {
    /// Create an empty cache.
    pub fn new() -> Self {
        QueryCache {
            results: RefCell::new(FxHashMap::default()),
            active: RefCell::new(FxHashSet::default()),
        }
    }

    // ── Lookup ───────────────────────────────────────────────────────────

    /// Look up a cached result. Returns `None` on cache miss.
    pub fn get(&self, key: &K) -> Option<V> {
        self.results.borrow().get(key).cloned()
    }

    /// Check whether a result is cached for `key`.
    pub fn contains(&self, key: &K) -> bool {
        self.results.borrow().contains_key(key)
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.results.borrow().len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.results.borrow().is_empty()
    }

    // ── Mutation ─────────────────────────────────────────────────────────

    /// Insert a computed result into the cache.
    pub fn insert(&self, key: K, value: V) {
        self.results.borrow_mut().insert(key, value);
    }

    /// Remove a single cached entry (for invalidation).
    pub fn invalidate(&self, key: &K) {
        self.results.borrow_mut().remove(key);
    }

    /// Remove all cached entries.
    pub fn clear(&self) {
        self.results.borrow_mut().clear();
    }

    // ── Active-set (cycle detection) ─────────────────────────────────────

    /// Is `key` currently being computed?
    fn is_active(&self, key: &K) -> bool {
        self.active.borrow().contains(key)
    }

    /// Mark `key` as "being computed". Returns an RAII guard that removes
    /// the key from the active set on drop (even if the provider panics).
    fn mark_active(&self, key: K) -> ActiveKeyGuard<'_, K> {
        self.active.borrow_mut().insert(key.clone());
        ActiveKeyGuard {
            active: &self.active,
            key,
        }
    }
}

impl<K: Eq + Hash + Clone, V: Clone> Default for QueryCache<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

// ── RAII Guards ──────────────────────────────────────────────────────────────

/// RAII guard: removes a key from a [`QueryCache`]'s active set on drop.
struct ActiveKeyGuard<'a, K: Eq + Hash> {
    active: &'a RefCell<FxHashSet<K>>,
    key: K,
}

impl<K: Eq + Hash> Drop for ActiveKeyGuard<'_, K> {
    fn drop(&mut self) {
        self.active.borrow_mut().remove(&self.key);
    }
}

/// RAII guard: pops the top entry from the [`QueryEngine`]'s active stack
/// on drop.
struct ActiveStackGuard<'a> {
    stack: &'a RefCell<Vec<QueryInvocation>>,
}

impl Drop for ActiveStackGuard<'_> {
    fn drop(&mut self) {
        self.stack.borrow_mut().pop();
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit() {
        let engine = QueryEngine::new();
        let cache: QueryCache<u32, String> = QueryCache::new();
        let mut called = 0u32;

        let v1 = engine.execute(
            &cache,
            "test",
            &42,
            |k| format!("{k}"),
            |_k| {
                called += 1;
                "hello".to_string()
            },
        );
        let v2 = engine.execute(
            &cache,
            "test",
            &42,
            |k| format!("{k}"),
            |_k| {
                called += 1;
                "world".to_string()
            },
        );

        assert_eq!(v1, "hello");
        assert_eq!(v2, "hello"); // cached, not recomputed
        assert_eq!(called, 1);
    }

    #[test]
    fn cycle_detection() {
        let engine = QueryEngine::new();
        let cache: QueryCache<u32, u32> = QueryCache::new();

        // Manually mark key 1 as active, then try to execute it.
        let _guard = cache.mark_active(1);
        let result = engine.try_execute(&cache, "test", &1, |k| format!("{k}"), |_k| 0);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("query cycle detected"));
    }

    #[test]
    fn invalidation() {
        let cache: QueryCache<u32, String> = QueryCache::new();
        cache.insert(1, "one".into());
        assert!(cache.contains(&1));

        cache.invalidate(&1);
        assert!(!cache.contains(&1));
    }

    #[test]
    fn clear() {
        let cache: QueryCache<u32, String> = QueryCache::new();
        cache.insert(1, "one".into());
        cache.insert(2, "two".into());
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert!(cache.is_empty());
    }
}
