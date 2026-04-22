use crate::binding::{Binding, Resolution};
use crate::error::{ResolveError, ResolveResult};
use crate::ids::{DefId, ScopeId};
use crate::import::ResolvedImport;
use crate::module_builder::ModuleTree;
use crate::rib::RibStack;
use crate::scope::ScopeTree;

/// Early name resolver for AST lowering.
///
/// Constructed from a [`ModuleTree`] (the product of the build phase) and
/// provides lookup / resolution methods that AST lowering calls as it walks
/// the AST.
///
/// ## Error policy
///
/// Every lookup method that is expected to find a result returns
/// `ResolveResult<…>`.  Callers (AST lowering) should collect or emit
/// these errors through the diagnostic infrastructure.
pub struct Resolver<'a> {
    /// The pre-built module tree (scope tree + def names).
    tree: &'a ModuleTree,
    /// Rib stack for ordered (lexical) scopes.
    ribs: RibStack,
}

impl<'a> Resolver<'a> {
    /// Create a resolver that queries against `tree`.
    pub fn new(tree: &'a ModuleTree) -> Self {
        Self {
            tree,
            ribs: RibStack::new(),
        }
    }

    // ── High-level resolution API ────────────────────────────────────────

    /// Resolve a simple name starting from `scope_id`, walking up the scope
    /// chain.
    ///
    /// Returns `Err(ResolveError::UnresolvedName)` if the name cannot be
    /// found in any enclosing scope.
    pub fn resolve_name(
        &self,
        name: &str,
        scope_id: ScopeId,
        span: rustc_span::Span,
    ) -> ResolveResult<Resolution> {
        // 1. Check the rib stack first (lexical / ordered bindings).
        if let Some(binding) = self.ribs.lookup(name) {
            return Ok(Resolution::from_binding(binding));
        }

        // 2. Walk up the scope tree.
        for scope in self.scope_tree().ancestors(scope_id) {
            if let Some(binding) = self.lookup_in_scope(name, scope.id) {
                return Ok(Resolution::from_binding(&binding));
            }
        }

        Err(ResolveError::UnresolvedName {
            name: name.to_string(),
            span,
        })
    }

    /// Resolve a dotted path (e.g. `a.b.c`) starting from `scope_id`.
    ///
    /// Each segment is resolved to the scope it names; the **last** segment
    /// is resolved as a name within the final scope and returned as a
    /// `Resolution`.
    ///
    /// Returns an error if any segment cannot be resolved.
    pub fn resolve_path(
        &self,
        segments: &[String],
        scope_id: ScopeId,
        span: rustc_span::Span,
    ) -> ResolveResult<Resolution> {
        if segments.is_empty() {
            return Err(ResolveError::InternalError(
                "empty path in resolve_path".into(),
            ));
        }

        if segments.len() == 1 {
            return self.resolve_name(&segments[0], scope_id, span);
        }

        // Resolve the prefix segments to a scope.
        let (prefix, tail) = segments.split_at(segments.len() - 1);
        let target_scope = self.resolve_scope_path(prefix, scope_id, span)?;

        // Resolve the final name in that scope (direct only, no ancestor walk).
        let name = &tail[0];
        let binding = self.lookup_in_scope(name, target_scope).ok_or_else(|| {
            ResolveError::UnresolvedName {
                name: name.clone(),
                span,
            }
        })?;

        Ok(Resolution::from_binding(&binding).with_import_source(target_scope))
    }

    /// Resolve a sequence of path segments to a scope (for qualified access).
    ///
    /// Unlike `resolve_path`, this returns the *scope* rather than a
    /// `Resolution`, and is useful when you need to enter a module scope.
    pub fn resolve_scope_path(
        &self,
        segments: &[String],
        mut scope_id: ScopeId,
        span: rustc_span::Span,
    ) -> ResolveResult<ScopeId> {
        for (i, segment) in segments.iter().enumerate() {
            if segment == "super" {
                let parent = self
                    .scope_tree()
                    .get(scope_id)
                    .and_then(|s| s.parent)
                    .ok_or_else(|| ResolveError::UnresolvedImportSegment {
                        segment: "super".into(),
                        span,
                    })?;
                scope_id = parent;
                continue;
            }

            // First segment: walk up the scope chain. Subsequent segments:
            // look only in the current scope.
            let binding = if i == 0 {
                self.resolve_name_to_binding(segment, scope_id)
            } else {
                self.lookup_in_scope(segment, scope_id)
            };

            let binding = binding.ok_or_else(|| ResolveError::UnresolvedImportSegment {
                segment: segment.clone(),
                span,
            })?;

            let child_scope = self.find_scope_for_def(binding.def_id).ok_or_else(|| {
                ResolveError::UnresolvedImportSegment {
                    segment: segment.clone(),
                    span,
                }
            })?;

            scope_id = child_scope;
        }

        Ok(scope_id)
    }

    // ── Rib management (for AST lowering) ────────────────────────────────

    /// Access the rib stack (e.g. to push / pop ribs during lowering).
    pub fn ribs(&self) -> &RibStack {
        &self.ribs
    }

    /// Mutable access to the rib stack.
    pub fn ribs_mut(&mut self) -> &mut RibStack {
        &mut self.ribs
    }

    // ── Low-level lookup helpers ─────────────────────────────────────────

    /// Look up a name by walking up the scope tree (no rib stack).
    /// Returns `None` if not found.
    fn resolve_name_to_binding(&self, name: &str, scope_id: ScopeId) -> Option<Binding> {
        for scope in self.scope_tree().ancestors(scope_id) {
            if let Some(b) = self.lookup_in_scope(name, scope.id) {
                return Some(b);
            }
        }
        None
    }

    /// Look up a name in a single scope (direct declarations + resolved
    /// imports).
    fn lookup_in_scope(&self, name: &str, scope_id: ScopeId) -> Option<Binding> {
        let scope = self.scope_tree().get(scope_id)?;

        // Direct declarations (including clauses).
        if let Some(b) = scope.items.get_local(name) {
            return Some(b.clone());
        }

        // Resolved imports.
        for import in scope.items.imports() {
            match import {
                ResolvedImport::Glob(source_scope) => {
                    if let Some(b) = self.lookup_direct(name, *source_scope) {
                        return Some(b);
                    }
                }
                ResolvedImport::Multi(source_scope, names) => {
                    if names.iter().any(|n| n == name) {
                        if let Some(b) = self.lookup_direct(name, *source_scope) {
                            return Some(b);
                        }
                    }
                }
                ResolvedImport::Single(source_scope, imported_name) => {
                    if imported_name == name {
                        if let Some(b) = self.lookup_direct(name, *source_scope) {
                            return Some(b);
                        }
                    }
                }
                ResolvedImport::Alias {
                    source_scope,
                    original,
                    alias,
                } => {
                    if alias == name {
                        if let Some(b) = self.lookup_direct(original, *source_scope) {
                            return Some(b);
                        }
                    }
                }
            }
        }

        None
    }

    /// Look up a name in direct declarations only (no imports, no ancestor walk).
    fn lookup_direct(&self, name: &str, scope_id: ScopeId) -> Option<Binding> {
        let scope = self.scope_tree().get(scope_id)?;
        scope.items.get_direct(name).cloned()
    }

    /// Find the scope that a DefId owns (module, ADT, etc.).
    fn find_scope_for_def(&self, def_id: DefId) -> Option<ScopeId> {
        // Linear scan – fine for now; could be indexed on ModuleTree later.
        for scope in self.scope_tree().iter() {
            if scope.owner_def == def_id {
                return Some(scope.id);
            }
        }
        None
    }

    // ── Accessors ────────────────────────────────────────────────────────

    /// The underlying scope tree.
    pub fn scope_tree(&self) -> &ScopeTree {
        &self.tree.scope_tree
    }

    /// The underlying module tree.
    pub fn module_tree(&self) -> &ModuleTree {
        self.tree
    }

    /// Look up the human-readable name for a DefId.
    pub fn def_name(&self, def_id: DefId) -> Option<&str> {
        self.tree.def_names.get(&def_id).map(|s| s.as_str())
    }
}
