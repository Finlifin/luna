//! Query system for Luna compiler
//! 
//! This module implements a query-based compilation system similar to rustc,
//! where different compilation phases are modeled as queries that can be cached
//! and computed on-demand.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::rc::Rc;

use rustc_data_structures::fx::{FxHashMap, FxHasher};

/// A unique identifier for a query kind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueryKind(pub u16);

/// The key used to identify a specific query instance
pub trait QueryKey: Clone + Eq + Hash + fmt::Debug + 'static {
    type Value: Clone + fmt::Debug + 'static;
    const KIND: QueryKind;
    const NAME: &'static str;
}

/// The result of executing a query
pub trait QueryResult: Clone + fmt::Debug + 'static {}

impl<T: Clone + fmt::Debug + 'static> QueryResult for T {}

/// A query descriptor that combines a key with its associated value type
pub struct QueryDesc<K: QueryKey> {
    pub key: K,
    _phantom: PhantomData<K::Value>,
}

impl<K: QueryKey> QueryDesc<K> {
    pub fn new(key: K) -> Self {
        Self {
            key,
            _phantom: PhantomData,
        }
    }
}

impl<K: QueryKey> Clone for QueryDesc<K> {
    fn clone(&self) -> Self {
        Self::new(self.key.clone())
    }
}

impl<K: QueryKey> fmt::Debug for QueryDesc<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryDesc({:?})", self.key)
    }
}

impl<K: QueryKey> Hash for QueryDesc<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        K::KIND.hash(state);
        self.key.hash(state);
    }
}

impl<K: QueryKey> PartialEq for QueryDesc<K> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl<K: QueryKey> Eq for QueryDesc<K> {}

/// A type-erased query key for storage in the cache
#[derive(Clone, Debug)]
pub struct ErasedQueryKey {
    pub kind: QueryKind,
    pub key_hash: u64,
    pub key_debug: String,
}

impl Hash for ErasedQueryKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.key_hash.hash(state);
    }
}

impl PartialEq for ErasedQueryKey {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.key_hash == other.key_hash
    }
}

impl Eq for ErasedQueryKey {}

impl ErasedQueryKey {
    pub fn new<K: QueryKey>(desc: &QueryDesc<K>) -> Self {
        let mut hasher = FxHasher::default();
        desc.key.hash(&mut hasher);
        
        Self {
            kind: K::KIND,
            key_hash: hasher.finish(),
            key_debug: format!("{:?}", desc.key),
        }
    }
}

/// Dependency tracking for queries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryDependency {
    pub query: ErasedQueryKey,
    pub dependency_type: DependencyType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyType {
    /// Direct dependency - query A directly calls query B
    Direct,
    /// Conditional dependency - query A might call query B based on some condition
    Conditional,
    /// Invalidation dependency - query A should be invalidated when query B changes
    Invalidation,
}

/// Dependency graph that tracks relationships between queries
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// Maps each query to its direct dependencies
    dependencies: FxHashMap<ErasedQueryKey, Vec<QueryDependency>>,
    /// Maps each query to queries that depend on it (reverse dependencies)
    dependents: FxHashMap<ErasedQueryKey, Vec<ErasedQueryKey>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            dependencies: FxHashMap::default(),
            dependents: FxHashMap::default(),
        }
    }

    /// Add a dependency relationship
    pub fn add_dependency(&mut self, query: ErasedQueryKey, dependency: QueryDependency) {
        // Add to dependencies map
        self.dependencies
            .entry(query.clone())
            .or_insert_with(Vec::new)
            .push(dependency.clone());

        // Add to reverse dependencies map
        self.dependents
            .entry(dependency.query.clone())
            .or_insert_with(Vec::new)
            .push(query);
    }

    /// Get all queries that should be invalidated when the given query changes
    pub fn get_invalidation_targets(&self, query: &ErasedQueryKey) -> Vec<ErasedQueryKey> {
        let mut targets = Vec::new();
        let mut visited = std::collections::HashSet::new();
        
        self.collect_invalidation_targets(query, &mut targets, &mut visited);
        targets
    }

    fn collect_invalidation_targets(
        &self,
        query: &ErasedQueryKey,
        targets: &mut Vec<ErasedQueryKey>,
        visited: &mut std::collections::HashSet<ErasedQueryKey>,
    ) {
        if visited.contains(query) {
            return;
        }
        visited.insert(query.clone());

        if let Some(dependents) = self.dependents.get(query) {
            for dependent in dependents {
                targets.push(dependent.clone());
                // Recursively collect invalidation targets
                self.collect_invalidation_targets(dependent, targets, visited);
            }
        }
    }

    /// Check if there's a cycle in the dependency graph
    pub fn has_cycle(&self, start: &ErasedQueryKey) -> bool {
        let mut visited = std::collections::HashSet::new();
        let mut rec_stack = std::collections::HashSet::new();
        
        self.has_cycle_util(start, &mut visited, &mut rec_stack)
    }

    fn has_cycle_util(
        &self,
        query: &ErasedQueryKey,
        visited: &mut std::collections::HashSet<ErasedQueryKey>,
        rec_stack: &mut std::collections::HashSet<ErasedQueryKey>,
    ) -> bool {
        visited.insert(query.clone());
        rec_stack.insert(query.clone());

        if let Some(dependencies) = self.dependencies.get(query) {
            for dep in dependencies {
                if !visited.contains(&dep.query) {
                    if self.has_cycle_util(&dep.query, visited, rec_stack) {
                        return true;
                    }
                } else if rec_stack.contains(&dep.query) {
                    return true;
                }
            }
        }

        rec_stack.remove(query);
        false
    }

    /// Get direct dependencies of a query
    pub fn get_dependencies(&self, query: &ErasedQueryKey) -> Vec<QueryDependency> {
        self.dependencies.get(query).cloned().unwrap_or_default()
    }

    /// Clear all dependencies for a query
    pub fn clear_dependencies(&mut self, query: &ErasedQueryKey) {
        if let Some(deps) = self.dependencies.remove(query) {
            // Remove from reverse dependencies
            for dep in deps {
                if let Some(dependents) = self.dependents.get_mut(&dep.query) {
                    dependents.retain(|q| q != query);
                }
            }
        }
    }
}

/// Query execution context with dependency tracking
pub struct QueryCtx<'ctx> {
    pub cache: &'ctx QueryCache,
    pub dependency_graph: &'ctx mut DependencyGraph,
    pub stack: Vec<ErasedQueryKey>,
}

impl<'ctx> QueryCtx<'ctx> {
    pub fn new(cache: &'ctx QueryCache, dependency_graph: &'ctx mut DependencyGraph) -> Self {
        Self {
            cache,
            dependency_graph,
            stack: Vec::new(),
        }
    }

    /// Execute a query and return its result with proper dependency tracking
    pub fn query<K: QueryKey>(&mut self, desc: QueryDesc<K>) -> K::Value
    where
        K::Value: Clone + 'static,
    {
        let erased_key = ErasedQueryKey::new(&desc);
        
        // Check for cycles using the dependency graph
        if self.dependency_graph.has_cycle(&erased_key) {
            panic!("Query cycle detected: {:?}", erased_key);
        }

        // Check cache
        if let Some(cached) = self.cache.get::<K::Value>(&erased_key) {
            // Track dependency if we're executing within another query
            if let Some(parent) = self.stack.last() {
                let dependency = QueryDependency {
                    query: erased_key.clone(),
                    dependency_type: DependencyType::Direct,
                };
                self.dependency_graph.add_dependency(parent.clone(), dependency);
            }
            
            return (*cached).clone();
        }

        // Execute query
        self.stack.push(erased_key.clone());
        let result = self.execute_query(desc);
        self.stack.pop();

        // Cache result
        self.cache.insert(erased_key, result.clone());

        result
    }

    fn execute_query<K: QueryKey>(&mut self, desc: QueryDesc<K>) -> K::Value
    where
        K::Value: Clone + 'static,
    {
        // This will be implemented by the query engine
        // For now, we'll panic to indicate unimplemented queries
        panic!("Query {:?} not implemented", desc);
    }

    /// Invalidate a query and all its dependents
    pub fn invalidate(&mut self, query: &ErasedQueryKey) {
        let targets = self.dependency_graph.get_invalidation_targets(query);
        
        // Remove from cache
        self.cache.remove(query);
        
        // Remove all dependent queries from cache
        for target in targets {
            self.cache.remove(&target);
        }
        
        // Clear dependencies for invalidated queries
        self.dependency_graph.clear_dependencies(query);
        for target in self.dependency_graph.get_invalidation_targets(query) {
            self.dependency_graph.clear_dependencies(&target);
        }
    }

    /// Get all dependencies of a query
    pub fn get_query_dependencies(&self, query: &ErasedQueryKey) -> Vec<QueryDependency> {
        self.dependency_graph.get_dependencies(query)
    }
}

/// Query cache that stores results for single-threaded use
pub struct QueryCache {
    cache: RefCell<FxHashMap<ErasedQueryKey, Rc<dyn Any>>>,
}

impl QueryCache {
    pub fn new() -> Self {
        Self {
            cache: RefCell::new(FxHashMap::default()),
        }
    }

    pub fn get<T: 'static>(&self, key: &ErasedQueryKey) -> Option<Rc<T>> {
        self.cache
            .borrow()
            .get(key)
            .and_then(|value| value.clone().downcast::<T>().ok())
    }

    pub fn insert<T: 'static>(&self, key: ErasedQueryKey, value: T) {
        let rc_value: Rc<dyn Any> = Rc::new(value);
        self.cache.borrow_mut().insert(key, rc_value);
    }

    pub fn remove(&self, key: &ErasedQueryKey) -> bool {
        self.cache.borrow_mut().remove(key).is_some()
    }

    pub fn clear(&self) {
        self.cache.borrow_mut().clear();
    }

    pub fn len(&self) -> usize {
        self.cache.borrow().len()
    }
}

impl Default for QueryCache {
    fn default() -> Self {
        Self::new()
    }
}

// Example query types for the Luna compiler

/// Query for getting the HIR of a file
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HirQuery {
    pub file_id: u32,
}

impl QueryKey for HirQuery {
    type Value = Vec<String>; // Simplified HIR representation
    const KIND: QueryKind = QueryKind(1);
    const NAME: &'static str = "hir";
}

/// Query for getting type information
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeQuery {
    pub node_id: u32,
}

impl QueryKey for TypeQuery {
    type Value = String; // Simplified type representation
    const KIND: QueryKind = QueryKind(2);
    const NAME: &'static str = "type_of";
}

/// Query for scope information
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScopeQuery {
    pub scope_id: u32,
}

impl QueryKey for ScopeQuery {
    type Value = Vec<String>; // List of symbols in scope
    const KIND: QueryKind = QueryKind(3);
    const NAME: &'static str = "scope";
}
