//! # How to add a new query
//!
//! Luna uses a rustc-style **provider table** plus the generic
//! [`crate::try_get_with`] / [`crate::force_get_with`] helpers in this crate.
//! Salsa is only the `Database` host; memoisation and cycles are handled here.
//!
//! ## Checklist
//!
//! 1. **`Providers`** (in `middle::queries`):
//!    add a `fn` field and a panicking default in `Default`.
//! 2. **`QueryCaches` / `Db`**: add a [`QueryCache`](crate::QueryCache)`<K, V>`
//!    field (wrapped in `Arc` so `Db: Clone` shares memo).
//! 3. **`LunaDatabase`**: add a method that calls
//!    [`force_get_with`](crate::force_get_with) / [`try_get_with`](crate::try_get_with)
//!    with a [`QueryDesc`](crate::QueryDesc), the cache, and a closure that
//!    invokes `self.providers.<name>(…)`.
//! 4. **Pass crate**: implement the provider and register it in
//!    `set_providers(&mut Providers)`.
//! 5. **(Optional)** forward from `interface::Compiler`.
//!
//! ## Live example
//!
//! `hir_package` is the only production query today:
//!
//! - Provider slot: `Providers::hir_package`
//! - Cache: `QueryCaches::hir_package`
//! - Method: `LunaDatabase::hir_package`
//! - Registration: `ast_lowering::set_providers`
//!
//! ## Keys and values
//!
//! - `K` must be `Eq + Hash + Clone` (e.g. `()`, `LocalDefId`).
//! - `V` must be `Clone` (prefer `Arc<T>` for large results).
//!
//! ## Cycles
//!
//! Mutual recursion among queries is detected when a key is re-entered while
//! `InProgress`.  Prefer [`try_get_with`](crate::try_get_with) and emit a
//! diagnostic from [`CycleError`](crate::CycleError); use
//! [`force_get_with`](crate::force_get_with) only when a cycle is an ICE.
//!
//! ## Invalidation
//!
//! There is no fine-grained invalidation.  Start a new `Db` for a new
//! compilation.  [`DepNode`](crate::DepNode) / [`QueryHooks`](crate::QueryHooks)
//! exist for future incremental work and tracing.

// This module exists solely for rustdoc.
