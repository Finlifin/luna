//! Snapshot-local closure identity. Retain the allocation while using it as a key.
use super::Closure;
use std::sync::Arc;

/// This is an execution-graph identity, never a type identity or serialized key.
/// Holding the Arc prevents allocation reuse while a snapshot table retains it.
#[derive(Clone)]
pub struct ClosureIdentity(Arc<Closure>);
impl ClosureIdentity {
    pub fn new(closure: &Arc<Closure>) -> Self {
        Self(closure.clone())
    }
}
impl PartialEq for ClosureIdentity {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ClosureIdentity {}
impl std::hash::Hash for ClosureIdentity {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::ptr::hash(Arc::as_ptr(&self.0), state);
    }
}
