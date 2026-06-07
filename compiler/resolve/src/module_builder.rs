//! Module tree builder – constructs the scope tree and resolves imports.
//!
//! This module handles the **early build phase** of name resolution:
//!
//! 1. **VFS scan** – walk the file system, parse `.fl` files, build the scope
//!    tree and collect unresolved `ImportDirective`s.
//! 2. **Import resolution** – fixpoint loop that resolves every `use` path
//!    against the scope tree, with cycle detection.
//!
//! The resulting [`ModuleTree`] is then consumed by [`Resolver`](crate::resolver::Resolver)
//! during AST lowering for name resolution queries.

use std::collections::HashMap;

use diagnostic::DiagnosticContext;
use rustc_span::SourceMap;
use symbol::{PathAnchor, Symbol};

use crate::binding::Binding;
use crate::error::{ResolveError, ResolveResult};
use crate::ids::{DefId, DefIdGen, ScopeId, ScopeIdGen};
use crate::impl_directive::ImplDirective;
use crate::import::{ImportDirective, ImportKind, ResolvedImport};
use crate::scanner::VfsScanner;
use crate::scope::{Scope, ScopeKind, ScopeTree};

/// The product of the module-building phase: a fully constructed scope tree
/// with all imports resolved.
///
/// This is the immutable artifact that the [`Resolver`](crate::resolver::Resolver)
/// borrows during AST lowering.
pub struct ModuleTree {
    /// The fully constructed scope tree.
    pub scope_tree: ScopeTree,
    /// DefId → human-readable name.
    pub def_names: HashMap<DefId, Symbol>,
    /// How many DefIds were allocated.
    pub def_count: u32,
    /// All `impl` / `impl Trait for Type` blocks discovered during scanning.
    pub impls: Vec<ImplDirective>,
    /// Errors collected (non-fatal) during the build phase.
    pub errors: Vec<ResolveError>,
    /// VFS FileId → the scope that owns the top-level definitions of that file.
    ///
    /// For entry files (`main.fl` / `lib.fl`) this is the package scope;
    /// for named files it is the file-level module scope.
    pub file_scopes: HashMap<vfs::FileId, ScopeId>,
}

/// Build the module tree for a package.
///
/// This is the **early build phase** that:
/// 1. Scans the VFS, parses `.fl` files, and constructs the scope tree.
/// 2. Resolves all `use` import directives in a fixpoint loop.
///
/// The resulting [`ModuleTree`] is then consumed by
/// [`Resolver::new`](crate::resolver::Resolver::new) for name-resolution
/// queries during AST lowering.
pub fn build_module_tree(
    source_map: &SourceMap,
    diag_ctx: &DiagnosticContext<'_>,
    vfs: &mut vfs::Vfs,
) -> ModuleTree {
    let mut builder = ModuleBuilder::new(source_map, diag_ctx);
    builder.build(vfs)
}

/// Internal builder that owns mutable state while constructing a [`ModuleTree`].
struct ModuleBuilder<'a> {
    source_map: &'a SourceMap,
    diag_ctx: &'a DiagnosticContext<'a>,
    scope_tree: ScopeTree,
    def_gen: DefIdGen,
    scope_gen: ScopeIdGen,
    /// Root scope id.
    root_scope: ScopeId,
    /// Unresolved import directives.
    unresolved_imports: Vec<ImportDirective>,
    /// Collected impl directives.
    impls: Vec<ImplDirective>,
    /// Accumulated errors.
    errors: Vec<ResolveError>,
    /// DefId → name mapping.
    def_names: Vec<(DefId, Symbol)>,
    /// Index: DefId → ScopeId (built once after scan phase).
    def_to_scope: HashMap<DefId, ScopeId>,
    /// VFS FileId → the scope that owns the file's top-level definitions.
    file_scopes: HashMap<vfs::FileId, ScopeId>,
}

impl<'a> ModuleBuilder<'a> {
    fn new(source_map: &'a SourceMap, diag_ctx: &'a DiagnosticContext<'a>) -> Self {
        let mut scope_gen = ScopeIdGen::new();
        let mut def_gen = DefIdGen::new(0); // pkg=0 is the local package
        let mut scope_tree = ScopeTree::new();

        // Create the synthetic root scope.
        let root_id = scope_gen.next();
        let root_def = def_gen.next();
        let root = Scope::new(
            root_id,
            ScopeKind::Root,
            None,
            Some(Symbol::intern("<root>")),
            root_def,
            false,
        );
        scope_tree.add_scope(root);

        Self {
            source_map,
            diag_ctx,
            scope_tree,
            def_gen,
            scope_gen,
            root_scope: root_id,
            unresolved_imports: Vec::new(),
            impls: Vec::new(),
            errors: Vec::new(),
            def_names: vec![(root_def, Symbol::intern("<root>"))],
            def_to_scope: HashMap::new(),
            file_scopes: HashMap::new(),
        }
    }

    /// Run both build phases and produce a [`ModuleTree`].
    fn build(&mut self, vfs: &mut vfs::Vfs) -> ModuleTree {
        // Phase 1: VFS scan → scope tree + unresolved imports
        if let Err(e) = self.scan_phase(vfs) {
            self.errors.push(e);
        }

        // Build DefId→ScopeId index now that the scope tree is complete.
        self.def_to_scope = self
            .scope_tree
            .iter()
            .map(|s| (s.owner_def, s.id))
            .collect();

        // Phase 2: fixpoint import resolution
        if let Err(e) = self.import_resolution_phase() {
            self.errors.push(e);
        }

        // Produce the final artifact.
        let def_count = self.def_gen.count();
        let def_names = std::mem::take(&mut self.def_names).into_iter().collect();
        let impls = std::mem::take(&mut self.impls);
        let errors = std::mem::take(&mut self.errors);
        let scope_tree = std::mem::replace(&mut self.scope_tree, ScopeTree::new());
        let file_scopes = std::mem::take(&mut self.file_scopes);

        ModuleTree {
            scope_tree,
            def_names,
            def_count,
            impls,
            errors,
            file_scopes,
        }
    }

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

        let (imports, impls, def_names, file_scopes) = scanner.into_results();
        self.unresolved_imports = imports;
        self.impls = impls;
        self.def_names.extend(def_names);
        self.file_scopes = file_scopes;

        Ok(())
    }

    fn import_resolution_phase(&mut self) -> ResolveResult<()> {
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
                        let is_reexport = self.unresolved_imports[i].is_reexport;

                        // Apply the resolved import to the scope.
                        if let Some(scope) = self.scope_tree.get_mut(owner_scope) {
                            if is_reexport {
                                scope.items.add_reexport(resolved);
                            } else {
                                scope.items.add_import(resolved);
                            }
                        }

                        progress = true;
                        remaining_count -= 1;
                    }
                    Err(ResolveError::UnresolvedImportSegment { .. }) => {
                        // Might succeed in a later iteration once more scopes are populated.
                    }
                    Err(e) => {
                        self.errors.push(e);
                        self.unresolved_imports[i].resolved = true;
                        remaining_count -= 1;
                    }
                }
            }

            if !progress {
                // No progress – remaining imports are unresolvable.
                for imp in &self.unresolved_imports {
                    if !imp.resolved {
                        let anchor_prefix = match imp.anchor {
                            PathAnchor::Local => String::new(),
                            PathAnchor::Super(n) => ".".repeat(n as usize),
                            PathAnchor::Package => "@".to_string(),
                        };
                        let segs = imp
                            .path_segments
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        self.errors.push(ResolveError::UnresolvedImportSegment {
                            segment: format!("{}{}", anchor_prefix, segs),
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

    /// Try to resolve a single import directive.
    fn try_resolve_import(&self, import_idx: usize) -> ResolveResult<ResolvedImport> {
        let directive = &self.unresolved_imports[import_idx];
        let starting_scope = directive.owner_scope;

        let target_scope = self.resolve_path_segments(
            directive.anchor,
            &directive.path_segments,
            starting_scope,
            directive.span,
        )?;

        match &directive.kind {
            ImportKind::Single { name, .. } => {
                self.verify_name_in_scope(name.as_str(), target_scope, directive.span)?;
                Ok(ResolvedImport::Single(target_scope, *name))
            }
            ImportKind::Glob { .. } => Ok(ResolvedImport::Glob(target_scope)),
            ImportKind::Multi { names, .. } => {
                for name in names {
                    self.verify_name_in_scope(name.as_str(), target_scope, directive.span)?;
                }
                Ok(ResolvedImport::Multi(target_scope, names.clone()))
            }
            ImportKind::Alias {
                original, alias, ..
            } => {
                self.verify_name_in_scope(original.as_str(), target_scope, directive.span)?;
                Ok(ResolvedImport::Alias {
                    source_scope: target_scope,
                    original: *original,
                    alias: *alias,
                })
            }
        }
    }

    /// Walk a path anchor + name segments to find the target scope.
    fn resolve_path_segments(
        &self,
        anchor: PathAnchor,
        segments: &[Symbol],
        scope_id: ScopeId,
        span: rustc_span::Span,
    ) -> ResolveResult<ScopeId> {
        // Apply anchor first.
        let mut scope_id = match anchor {
            PathAnchor::Local => scope_id,
            PathAnchor::Super(n) => {
                let mut sid = scope_id;
                for _ in 0..n {
                    sid = self
                        .scope_tree
                        .get(sid)
                        .and_then(|s| s.parent)
                        .ok_or_else(|| ResolveError::UnresolvedImportSegment {
                            segment: ".".to_string(),
                            span,
                        })?;
                }
                sid
            }
            PathAnchor::Package => self.root_scope,
        };

        // Walk name segments.
        for name in segments {
            let binding = self
                .lookup_in_scope(name.as_str(), scope_id)
                .ok_or_else(|| ResolveError::UnresolvedImportSegment {
                    segment: name.as_str().to_owned(),
                    span,
                })?;

            scope_id = self.find_scope_for_def(binding.def_id).ok_or_else(|| {
                ResolveError::UnresolvedImportSegment {
                    segment: name.as_str().to_owned(),
                    span,
                }
            })?;
        }

        Ok(scope_id)
    }

    /// Look up a name in a scope (direct declarations + imports).
    fn lookup_in_scope(&self, name: &str, scope_id: ScopeId) -> Option<Binding> {
        let scope = self.scope_tree.get(scope_id)?;

        if let Some(b) = scope.items.get_local(name) {
            return Some(b.clone());
        }

        for import in scope.items.all_imports() {
            match import {
                ResolvedImport::Glob(source_scope) => {
                    if let Some(b) = self.lookup_direct_in_scope(name, *source_scope) {
                        return Some(b);
                    }
                }
                ResolvedImport::Multi(source_scope, names) => {
                    if names.iter().any(|n| n == name) {
                        if let Some(b) = self.lookup_direct_in_scope(name, *source_scope) {
                            return Some(b);
                        }
                    }
                }
                ResolvedImport::Single(source_scope, imported_name) => {
                    if imported_name == name {
                        if let Some(b) = self.lookup_direct_in_scope(name, *source_scope) {
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
                        if let Some(b) =
                            self.lookup_direct_in_scope(original.as_str(), *source_scope)
                        {
                            return Some(b);
                        }
                    }
                }
            }
        }

        None
    }

    /// Look up a name in direct declarations only (no import traversal).
    fn lookup_direct_in_scope(&self, name: &str, scope_id: ScopeId) -> Option<Binding> {
        let scope = self.scope_tree.get(scope_id)?;
        scope.items.get_direct(name).cloned()
    }

    /// Verify that a name exists in a scope.
    fn verify_name_in_scope(
        &self,
        name: &str,
        scope_id: ScopeId,
        span: rustc_span::Span,
    ) -> ResolveResult<()> {
        if self.lookup_direct_in_scope(name, scope_id).is_some() {
            return Ok(());
        }
        Err(ResolveError::UnresolvedName {
            name: name.to_string(),
            span,
        })
    }

    /// Find the ScopeId that a DefId owns.
    fn find_scope_for_def(&self, def_id: DefId) -> Option<ScopeId> {
        self.def_to_scope.get(&def_id).copied()
    }
}

impl ModuleTree {
    /// Dump the scope tree as an S-expression string (for debugging).
    pub fn dump_scope_tree(&self) -> String {
        let mut out = String::new();
        for scope in self.scope_tree.iter() {
            if scope.parent.is_none() {
                self.dump_scope_recursive(&mut out, scope.id, 0);
                break;
            }
        }
        out
    }

    fn dump_scope_recursive(&self, out: &mut String, scope_id: ScopeId, indent: usize) {
        let Some(scope) = self.scope_tree.get(scope_id) else {
            return;
        };

        let pad = " ".repeat(indent);
        let name = scope.name.as_deref().unwrap_or("<anon>");
        out.push_str(&format!(
            "{}({:?} \"{}\" {:?}\n",
            pad, scope.kind, name, scope.id
        ));

        // Declarations
        for (decl_name, binding) in scope.items.declarations() {
            out.push_str(&format!(
                "{}  (def {} {:?} {:?})\n",
                pad, decl_name, binding.kind, binding.def_id
            ));
        }

        // Clauses
        for clause in scope.items.clauses() {
            out.push_str(&format!(
                "{}  (clause {} {:?})\n",
                pad, clause.name, clause.binding.def_id
            ));
        }

        // Private imports
        for import in scope.items.imports() {
            match import {
                ResolvedImport::Glob(s) => {
                    out.push_str(&format!("{}  (import-all {:?})\n", pad, s));
                }
                ResolvedImport::Multi(s, names) => {
                    out.push_str(&format!("{}  (import-multi {:?} {:?})\n", pad, s, names));
                }
                ResolvedImport::Single(s, n) => {
                    out.push_str(&format!("{}  (import-single {:?} \"{}\")\n", pad, s, n));
                }
                ResolvedImport::Alias {
                    source_scope,
                    original,
                    alias,
                } => {
                    out.push_str(&format!(
                        "{}  (import-alias {:?} \"{}\" as \"{}\")\n",
                        pad, source_scope, original, alias
                    ));
                }
            }
        }

        // Re-exported imports
        for import in scope.items.reexports() {
            match import {
                ResolvedImport::Glob(s) => {
                    out.push_str(&format!("{}  (reexport-all {:?})\n", pad, s));
                }
                ResolvedImport::Multi(s, names) => {
                    out.push_str(&format!("{}  (reexport-multi {:?} {:?})\n", pad, s, names));
                }
                ResolvedImport::Single(s, n) => {
                    out.push_str(&format!("{}  (reexport-single {:?} \"{}\")\n", pad, s, n));
                }
                ResolvedImport::Alias {
                    source_scope,
                    original,
                    alias,
                } => {
                    out.push_str(&format!(
                        "{}  (reexport-alias {:?} \"{}\" as \"{}\")\n",
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
