//! Keyed memoisation for a single query kind.

use std::collections::HashMap;
use std::hash::Hash;

/// Per-key state inside a [`QueryCache`].
#[derive(Clone, Debug)]
pub enum CacheState<V> {
    /// Slot reserved but not yet claimed (rarely observed).
    NotComputed,
    /// Currently being evaluated — re-entry means a cycle.
    InProgress,
    /// Successfully computed value.
    Complete(V),
    /// A previous evaluation panicked; do not reuse.
    Poisoned,
}

/// Outcome of [`crate::try_get_with`].
#[derive(Clone, Debug)]
pub enum QueryResult<V> {
    Ok(V),
    Cycle(crate::CycleError),
    Poisoned,
}

/// Memo table for one query kind (`K` → `V`).
///
/// State transitions are synchronized; query execution is owned by its engine.
pub struct QueryCache<K, V> {
    map: std::sync::Mutex<HashMap<K, CacheState<V>>>,
    nodes: std::sync::Mutex<HashMap<K, crate::dependencies::NodeId>>,
}

impl<K, V> QueryCache<K, V>
where
    K: Eq + Hash,
{
    pub fn new() -> Self {
        QueryCache {
            map: std::sync::Mutex::new(HashMap::new()),
            nodes: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn node(&self, key: &K) -> Result<crate::dependencies::NodeId, crate::QueryFailure>
    where
        K: Clone,
    {
        let mut nodes = self.nodes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&id) = nodes.get(key) {
            return Ok(id);
        }
        let id = crate::dependencies::NodeId::fresh()?;
        nodes.insert(key.clone(), id);
        Ok(id)
    }

    pub(crate) fn invalidate(&self, key: &K) {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }

    pub fn get(&self, key: &K) -> Option<CacheState<V>>
    where
        V: Clone,
    {
        self.map
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
            .cloned()
    }

    /// Transition `NotComputed` / absent → `InProgress`.
    ///
    /// Returns `false` if the key was already `InProgress`, `Complete`, or
    /// `Poisoned` (caller should re-read).
    pub fn start(&self, key: &K) -> bool
    where
        K: Clone,
    {
        let mut map = self.map.lock().unwrap_or_else(|error| error.into_inner());
        match map.get(key) {
            None | Some(CacheState::NotComputed) => {
                map.insert(key.clone(), CacheState::InProgress);
                true
            }
            Some(CacheState::InProgress)
            | Some(CacheState::Complete(_))
            | Some(CacheState::Poisoned) => false,
        }
    }

    pub fn complete(&self, key: &K, value: V)
    where
        K: Clone,
    {
        self.map
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(key.clone(), CacheState::Complete(value));
    }

    pub fn poison(&self, key: &K)
    where
        K: Clone,
    {
        self.map
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(key.clone(), CacheState::Poisoned);
    }

    /// Number of entries (any state). Useful for tests.
    pub fn len(&self) -> usize {
        self.map
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.map
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_empty()
    }

    /// Clear all memoised results. Not used by the driver today.
    pub fn clear(&self) {
        self.map
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }
}

impl<K, V> Default for QueryCache<K, V>
where
    K: Eq + Hash,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Cloning a [`QueryCache`] preserves all completed and poisoned memoized
/// entries as well as key-to-dependency-node identities (`nodes`), omitting any
/// transient `InProgress`/`NotComputed` slots from another evaluation stack.
impl<K: Eq + Hash + Clone, V: Clone> Clone for QueryCache<K, V> {
    fn clone(&self) -> Self {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let nodes = self.nodes.lock().unwrap_or_else(|e| e.into_inner());
        let cloned_map: HashMap<K, CacheState<V>> = map
            .iter()
            .filter_map(|(k, state)| match state {
                CacheState::InProgress | CacheState::NotComputed => None,
                CacheState::Complete(v) => Some((k.clone(), CacheState::Complete(v.clone()))),
                CacheState::Poisoned => Some((k.clone(), CacheState::Poisoned)),
            })
            .collect();
        let cloned_nodes = nodes.clone();
        QueryCache {
            map: std::sync::Mutex::new(cloned_map),
            nodes: std::sync::Mutex::new(cloned_nodes),
        }
    }
}
