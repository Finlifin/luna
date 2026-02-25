//! The Resolver – orchestrates early name resolution.
//!
//! ## Pipeline
//!
//! 1. **VFS scan** – walk the file system, parse `.fl` files, build the scope
//!    tree and collect unresolved `ImportDirective`s.
//! 2. **Import resolution** – fixpoint loop that resolves every `use` path
//!    against the scope tree, with cycle detection.
//! 3. The resulting [`ResolveOutput`] contains the fully populated scope tree,
//!    a DefId→name table, and the list of resolved imports.

use std::collections::HashMap;

use diagnostic::DiagnosticContext;
use rustc_span::SourceMap;

use crate::binding::Binding;
use crate::error::{ResolveError, ResolveResult};
use crate::ids::{DefId, DefIdGen, ScopeId, ScopeIdGen};
use crate::import::{ImportDirective, ImportKind, ResolvedImport};
use crate::namespace::Namespace;
use crate::scope::{Scope, ScopeKind, ScopeTree};
use crate::scanner::VfsScanner;

// ── ResolveOutput ────────────────────────────────────────────────────────────

/// The output of a complete name-resolution pass.
pub struct ResolveOutput {
    /// The fully constructed scope tree.
    pub scope_tree: ScopeTree,
    /// DefId → human-readable name.
    pub def_names: HashMap<DefId, String>,
    /// How many DefIds were allocated.
    pub def_count: u32,
    /// Errors collected (non-fatal) during resolution.
    pub errors: Vec<ResolveError>,
}

// ── Resolver ─────────────────────────────────────────────────────────────────

/// The main name-resolution orchestrator.
pub struct Resolver<'a> {
    source_map: &'a SourceMap,
    diag_ctx: &'a DiagnosticContext<'a>,
    scope_tree: ScopeTree,
    def_gen: DefIdGen,
    scope_gen: ScopeIdGen,
    /// Root scope id.
    root_scope: ScopeId,
    /// Unresolved import directives.
    unresolved_imports: Vec<ImportDirective>,
    /// Accumulated errors.
    errors: Vec<ResolveError>,
    /// DefId → name mapping.
    def_names: Vec<(DefId, String)>,
}

impl<'a> Resolver<'a> {
    /// Create a new resolver for a package.
    pub fn new(
        source_map: &'a SourceMap,
        diag_ctx: &'a DiagnosticContext<'a>,
    ) -> Self {
        let mut scope_gen = ScopeIdGen::new();
        let mut def_gen = DefIdGen::new();
        let mut scope_tree = ScopeTree::new();

        // Create the synthetic root scope
        let root_id = scope_gen.next();
        let root_def = def_gen.next();
        let root = Scope::new(root_id, ScopeKind::Root, None, Some("root".into()), root_def, false);
        scope_tree.add_scope(root);

        Self {
            source_map,
            diag_ctx,
            scope_tree,
            def_gen,
            scope_gen,
            root_scope: root_id,
            unresolved_imports: Vec::new(),
            errors: Vec::new(),
            def_names: vec![(root_def, "root".into())],
        }
    }

    /// Run the full name-resolution pass on a package VFS.
    pub fn resolve_package(mut self, vfs: &mut vfs::Vfs) -> ResolveOutput {
        // Phase 1: scan VFS and build scope tree
        if let Err(e) = self.scan_phase(vfs) {
            self.errors.push(e);
        }

        // Phase 2: resolve imports
        if let Err(e) = self.import_resolution_phase() {
            self.errors.push(e);
        }

        // Build output
        let def_count = self.def_gen.count();
        let def_names = self.def_names.into_iter().collect();
        ResolveOutput {
            scope_tree: self.scope_tree,
            def_names,
            def_count,
            errors: self.errors,
        }
    }

    /// Phase 1: VFS scanning.
    fn scan_phase(&mut self, vfs: &mut vfs::Vfs) -> ResolveResult<()> {
        let mut scanner = VfsScanner::new(
            self.source_map,
            self.diag_ctx,
            vfs,
            &mut self.scope_tree,
            &mut self.def_gen,
            &mut self.scope_gen,
        );

        scanner.scan_package(self.root_scope)?;

        let (imports, def_names) = scanner.into_results();
        self.unresolved_imports = imports;
        self.def_names.extend(def_names);

        Ok(())
    }

    /// Phase 2: fixpoint import resolution.
    fn import_resolution_phase(&mut self) -> ResolveResult<()> {
        // Group imports by owning scope for potential batching
        let mut remaining_count = self.unresolved_imports.len();
        let mut iteration = 0;
        let max_iterations = remaining_count + 1; // safety bound

        while remaining_count > 0 && iteration < max_iterations {
            let mut progress = false;

            for i in 0..self.unresolved_imports.len() {
                if self.unresolved_imports[i].resolved {
                    continue;
                }

                match self.try_resolve_import(i) {
                    Ok(resolved) => {
                        self.unresolved_imports[i].resolved = true;
                        let owner_scope = self.unresolved_imports[i].owner_scope;

                        // Apply the resolved import to the scope
                        if let Some(scope) = self.scope_tree.get_mut(owner_scope) {
                            scope.items.add_import(resolved);
                        }

                        progress = true;
                        remaining_count -= 1;
                    }
                    Err(ResolveError::UnresolvedImportSegment { .. }) => {
                        // Might succeed in a later iteration once more scopes are populated
                    }
                    Err(e) => {
                        self.errors.push(e);
                        self.unresolved_imports[i].resolved = true;
                        remaining_count -= 1;
                    }
                }
            }

            if !progress {
                // No progress – remaining imports are unresolvable
                for imp in &self.unresolved_imports {
                    if !imp.resolved {
                        self.errors.push(ResolveError::UnresolvedImportSegment {
                            segment: imp.path_segments.join("."),
                            span: imp.span,
                        });
                    }
                }
                break;
            }

            iteration += 1;
        }

        Ok(())
    }

    /// Try to resolve a single import directive. Returns `Err` if a path
    /// segment cannot be found (might succeed later).
    fn try_resolve_import(&self, import_idx: usize) -> ResolveResult<ResolvedImport> {
        let directive = &self.unresolved_imports[import_idx];
        let starting_scope = directive.owner_scope;

        // Walk the path segments to find the target scope
        let target_scope = self.resolve_path_segments(
            &directive.path_segments,
            starting_scope,
            directive.span,
        )?;

        // Now build the resolved import from the kind
        match &directive.kind {
            ImportKind::Single { name, .. } => {
                // Verify the name exists in the target scope
                self.verify_name_in_scope(name, target_scope, directive.span)?;
                Ok(ResolvedImport::Single(target_scope, name.clone()))
            }
            ImportKind::Glob { .. } => {
                Ok(ResolvedImport::All(target_scope))
            }
            ImportKind::Multi { names, .. } => {
                for name in names {
                    self.verify_name_in_scope(name, target_scope, directive.span)?;
                }
                Ok(ResolvedImport::Multi(target_scope, names.clone()))
            }
            ImportKind::Alias { original, alias, .. } => {
                self.verify_name_in_scope(original, target_scope, directive.span)?;
                Ok(ResolvedImport::Alias {
                    source_scope: target_scope,
                    original: original.clone(),
                    alias: alias.clone(),
                })
            }
        }
    }

    /// Walk a sequence of path segments starting from `scope_id`, resolving
    /// each segment to its scope.
    fn resolve_path_segments(
        &self,
        segments: &[String],
        mut scope_id: ScopeId,
        span: rustc_span::Span,
    ) -> ResolveResult<ScopeId> {
        for segment in segments {
            if segment == "super" {
                let parent = self.scope_tree.get(scope_id)
                    .and_then(|s| s.parent)
                    .ok_or_else(|| ResolveError::UnresolvedImportSegment {
                        segment: "super".into(),
                        span,
                    })?;
                scope_id = parent;
                continue;
            }

            // Look up the segment name in the current scope
            let binding = self.lookup_in_scope(segment, scope_id, Namespace::Type)
                .ok_or_else(|| ResolveError::UnresolvedImportSegment {
                    segment: segment.clone(),
                    span,
                })?;

            // The segment must refer to a module/package scope
            let child_scope = self.find_scope_for_def(binding.def_id)
                .ok_or_else(|| ResolveError::UnresolvedImportSegment {
                    segment: segment.clone(),
                    span,
                })?;

            scope_id = child_scope;
        }

        Ok(scope_id)
    }

    /// Look up a name in a scope, including direct declarations and imports.
    fn lookup_in_scope(&self, name: &str, scope_id: ScopeId, ns: Namespace) -> Option<Binding> {
        let scope = self.scope_tree.get(scope_id)?;

        // Direct declarations
        if let Some(b) = scope.items.get_local(name, ns) {
            return Some(b.clone());
        }

        // Imports
        for import in scope.items.imports() {
            match import {
                ResolvedImport::All(source_scope) => {
                    if let Some(b) = self.lookup_direct_in_scope(name, *source_scope, ns) {
                        return Some(b);
                    }
                }
                ResolvedImport::Multi(source_scope, names) => {
                    if names.iter().any(|n| n == name) {
                        if let Some(b) = self.lookup_direct_in_scope(name, *source_scope, ns) {
                            return Some(b);
                        }
                    }
                }
                ResolvedImport::Single(source_scope, imported_name) => {
                    if imported_name == name {
                        if let Some(b) = self.lookup_direct_in_scope(name, *source_scope, ns) {
                            return Some(b);
                        }
                    }
                }
                ResolvedImport::Alias { source_scope, original, alias } => {
                    if alias == name {
                        if let Some(b) = self.lookup_direct_in_scope(original, *source_scope, ns) {
                            return Some(b);
                        }
                    }
                }
            }
        }

        None
    }

    /// Look up a name directly (no import traversal) in a scope.
    fn lookup_direct_in_scope(&self, name: &str, scope_id: ScopeId, ns: Namespace) -> Option<Binding> {
        let scope = self.scope_tree.get(scope_id)?;
        scope.items.get_direct(name, ns).cloned()
    }

    /// Verify a name exists in a scope.
    fn verify_name_in_scope(
        &self,
        name: &str,
        scope_id: ScopeId,
        span: rustc_span::Span,
    ) -> ResolveResult<()> {
        // Check across all namespaces
        for ns in Namespace::ALL {
            if self.lookup_direct_in_scope(name, scope_id, ns).is_some() {
                return Ok(());
            }
        }
        Err(ResolveError::UnresolvedName {
            name: name.to_string(),
            span,
        })
    }

    /// Find the ScopeId that a DefId owns (if it's a module/package/ADT).
    fn find_scope_for_def(&self, def_id: DefId) -> Option<ScopeId> {
        // Linear scan – fine for now; could be indexed later.
        for scope in self.scope_tree.iter() {
            if scope.owner_def == def_id {
                return Some(scope.id);
            }
        }
        None
    }

    /// Resolve a name by walking up the scope tree (for non-import use).
    pub fn resolve_name(
        &self,
        name: &str,
        ns: Namespace,
        start_scope: ScopeId,
    ) -> Option<Binding> {
        for scope in self.scope_tree.ancestors(start_scope) {
            if let Some(b) = self.lookup_in_scope(name, scope.id, ns) {
                return Some(b);
            }
        }
        None
    }

    /// Access the scope tree (for inspection / testing).
    pub fn scope_tree(&self) -> &ScopeTree {
        &self.scope_tree
    }

    /// Access the root scope id.
    pub fn root_scope(&self) -> ScopeId {
        self.root_scope
    }
}

// ── Dump ─────────────────────────────────────────────────────────────────────

impl ResolveOutput {
    /// Dump the scope tree as an S-expression string (for debugging).
    pub fn dump_scope_tree(&self) -> String {
        let mut out = String::new();
        // Find the root
        for scope in self.scope_tree.iter() {
            if scope.parent.is_none() {
                self.dump_scope_recursive(&mut out, scope.id, 0);
                break;
            }
        }
        out
    }

    fn dump_scope_recursive(&self, out: &mut String, scope_id: ScopeId, indent: usize) {
        let Some(scope) = self.scope_tree.get(scope_id) else { return };

        let pad = " ".repeat(indent);
        let name = scope.name.as_deref().unwrap_or("<anon>");
        out.push_str(&format!("{}({:?} \"{}\" {:?}\n", pad, scope.kind, name, scope.id));

        // Declarations
        for (decl_name, per_ns) in scope.items.declarations() {
            for (ns, binding) in per_ns.iter() {
                out.push_str(&format!(
                    "{}  (def {} {:?} {:?} {:?})\n",
                    pad, decl_name, ns, binding.kind, binding.def_id
                ));
            }
        }

        // Clauses
        for clause in scope.items.clauses() {
            out.push_str(&format!(
                "{}  (clause {} {:?} {:?})\n",
                pad, clause.name, clause.binding.ns, clause.binding.def_id
            ));
        }

        // Imports
        for import in scope.items.imports() {
            match import {
                ResolvedImport::All(s) => {
                    out.push_str(&format!("{}  (import-all {:?})\n", pad, s));
                }
                ResolvedImport::Multi(s, names) => {
                    out.push_str(&format!("{}  (import-multi {:?} {:?})\n", pad, s, names));
                }
                ResolvedImport::Single(s, n) => {
                    out.push_str(&format!("{}  (import-single {:?} \"{}\")\n", pad, s, n));
                }
                ResolvedImport::Alias { source_scope, original, alias } => {
                    out.push_str(&format!(
                        "{}  (import-alias {:?} \"{}\" as \"{}\")\n",
                        pad, source_scope, original, alias
                    ));
                }
            }
        }

        // Recurse into children
        let children: Vec<_> = scope.children.clone();
        for child_id in children {
            self.dump_scope_recursive(out, child_id, indent + 2);
        }

        out.push_str(&format!("{})\n", pad));
    }
}
