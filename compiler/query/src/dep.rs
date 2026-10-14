//! Placeholder dependency-graph hooks for a future incremental compiler.
//!
//! Today Luna rebuilds the whole [`crate::QueryEngine`] / `Db` per
//! compilation, so these hooks are no-ops unless the host installs
//! custom callbacks for tracing.

use std::sync::Arc;

/// Identifies a query invocation for a future dep-graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DepNode {
    pub query: &'static str,
    pub key: String,
}

/// Optional callbacks invoked around query evaluation.
///
/// All fields default to no-ops.  Cloning shares the same function
/// pointers / arcs.
#[derive(Clone)]
pub struct QueryHooks {
    pub on_query_start: Arc<dyn Fn(DepNode) + Send + Sync>,
    pub on_query_end: Arc<dyn Fn(DepNode) + Send + Sync>,
    /// Fired when a **cached** result is returned (read edge).
    pub on_query_read: Arc<dyn Fn(DepNode) + Send + Sync>,
}

impl QueryHooks {
    pub fn noop() -> Self {
        fn noop(_: DepNode) {}
        QueryHooks {
            on_query_start: Arc::new(noop),
            on_query_end: Arc::new(noop),
            on_query_read: Arc::new(noop),
        }
    }
}

impl Default for QueryHooks {
    fn default() -> Self {
        Self::noop()
    }
}

impl std::fmt::Debug for QueryHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryHooks").finish_non_exhaustive()
    }
}
