//! Query engine implementation for Luna compiler
//! 
//! This module provides the actual implementation of queries,
//! connecting the query system to the compiler's data structures.

use super::query::{QueryCache, QueryCtx, QueryDesc, HirQuery, TypeQuery, ScopeQuery, DependencyGraph, QueryDependency, DependencyType};

/// The main query engine that implements query execution
pub struct QueryEngine {
    cache: QueryCache,
    dependency_graph: DependencyGraph,
}

impl QueryEngine {
    pub fn new() -> Self {
        Self {
            cache: QueryCache::new(),
            dependency_graph: DependencyGraph::new(),
        }
    }

    /// Create a new query context
    pub fn create_context(&mut self) -> QueryCtx {
        QueryCtx::new(&self.cache, &mut self.dependency_graph)
    }

    /// Execute a HIR query with dependency tracking
    pub fn hir(&mut self, file_id: u32) -> Vec<String> {
        let query = QueryDesc::new(HirQuery { file_id });
        
        // Check cache first
        let key = super::query::ErasedQueryKey::new(&query);
        if let Some(cached) = self.cache.get::<Vec<String>>(&key) {
            return (*cached).clone();
        }

        // Execute query - in a real implementation, this might depend on other queries
        let result = vec![
            format!("fn main() for file {}", file_id),
            format!("return statement for file {}", file_id),
        ];

        // Cache result
        self.cache.insert(key, result.clone());
        result
    }

    /// Execute a type query with dependency tracking
    pub fn type_of(&mut self, node_id: u32) -> String {
        let query = QueryDesc::new(TypeQuery { node_id });
        
        // Check cache first
        let key = super::query::ErasedQueryKey::new(&query);
        if let Some(cached) = self.cache.get::<String>(&key) {
            return (*cached).clone();
        }

        // This query might depend on HIR - let's simulate that
        let _hir = self.hir(1); // This creates a dependency
        
        // Execute query
        let result = format!("Type of node {} is i32", node_id);

        // Cache result and track dependency
        let hir_key = super::query::ErasedQueryKey::new(&QueryDesc::new(HirQuery { file_id: 1 }));
        let dependency = QueryDependency {
            query: hir_key,
            dependency_type: DependencyType::Direct,
        };
        self.dependency_graph.add_dependency(key.clone(), dependency);
        
        self.cache.insert(key, result.clone());
        result
    }

    /// Execute a scope query with dependency tracking
    pub fn scope(&mut self, scope_id: u32) -> Vec<String> {
        let query = QueryDesc::new(ScopeQuery { scope_id });
        
        // Check cache first
        let key = super::query::ErasedQueryKey::new(&query);
        if let Some(cached) = self.cache.get::<Vec<String>>(&key) {
            return (*cached).clone();
        }

        // Execute query
        let result = vec![
            format!("variable_a in scope {}", scope_id),
            format!("variable_b in scope {}", scope_id),
        ];

        // Cache result
        self.cache.insert(key, result.clone());
        result
    }

    /// Invalidate a query and all its dependents
    pub fn invalidate_query(&mut self, query_key: &super::query::ErasedQueryKey) {
        let mut ctx = self.create_context();
        ctx.invalidate(query_key);
    }

    /// Get dependency information for debugging
    pub fn get_dependency_info(&self, query_key: &super::query::ErasedQueryKey) -> Vec<QueryDependency> {
        self.dependency_graph.get_dependencies(query_key)
    }

    /// Clear all cached results
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Get cache statistics
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }
}

impl Default for QueryEngine {
    fn default() -> Self {
        Self::new()
    }
}
