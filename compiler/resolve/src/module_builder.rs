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

use crate::BindingKind;
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

        self.finish()
    }

    fn finish(&mut self) -> ModuleTree {
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
            self.rebuild_globs();
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

                        if let Err(error) = self.install_import(
                            owner_scope,
                            &resolved,
                            self.unresolved_imports[i].visibility,
                            self.unresolved_imports[i].span,
                        ) {
                            self.errors.push(error);
                        }

                        if let ResolvedImport::Glob(source) = &resolved {
                            self.scope_tree
                                .get_mut(owner_scope)
                                .unwrap()
                                .items
                                .glob_entries
                                .push((*source, self.unresolved_imports[i].visibility));
                        }
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

        self.rebuild_globs();
        // Public ambiguity belongs to the exporting module, even when unused.
        for scope in self.scope_tree.iter() {
            for import in scope.items.reexports() {
                if let ResolvedImport::Glob(source) = import {
                    for (name, candidates) in self
                        .scope_tree
                        .get(*source)
                        .unwrap()
                        .items
                        .selected_candidates()
                    {
                        if scope.items.get_local(name).is_none()
                            && candidates
                                .iter()
                                .any(|b| b.entry_visibility() == crate::binding::Visibility::Public)
                            && scope.items.ambiguous_glob(name)
                        {
                            self.errors.push(ResolveError::InvalidNodeType {
                                message: format!("ambiguous public glob name `{name}`"),
                                span: rustc_span::DUMMY_SP,
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn pending_explicit(&self, owner: ScopeId, name: Symbol) -> bool {
        self.unresolved_imports.iter().any(|directive| {
            !directive.resolved
                && directive.owner_scope == owner
                && match &directive.kind {
                    ImportKind::Single { name: imported, .. } => *imported == name,
                    ImportKind::Alias { alias, .. } => *alias == name,
                    ImportKind::Multi { names, .. } => names.contains(&name),
                    ImportKind::Glob { .. } => false,
                }
        })
    }

    fn rebuild_globs(&mut self) {
        let ids: Vec<_> = self.scope_tree.iter().map(|s| s.id).collect();
        for id in &ids {
            self.scope_tree
                .get_mut(*id)
                .unwrap()
                .items
                .glob_candidates
                .clear();
        }
        loop {
            let mut additions = Vec::new();
            for scope in self.scope_tree.iter() {
                for (source, visibility) in &scope.items.glob_entries {
                    let public = *visibility == crate::binding::Visibility::Public;
                    for (name, candidates) in self
                        .scope_tree
                        .get(*source)
                        .unwrap()
                        .items
                        .selected_candidates()
                    {
                        if self.pending_explicit(*source, name)
                            || scope.items.get_local(name).is_some()
                        {
                            continue;
                        }
                        for binding in candidates {
                            if public
                                && binding.entry_visibility() != crate::binding::Visibility::Public
                            {
                                continue;
                            }
                            if !binding.accessible(&self.scope_tree, scope.id) {
                                continue;
                            }
                            let binding = binding.imported(scope.id, *visibility);
                            additions.push((scope.id, name, binding));
                        }
                    }
                }
            }
            let mut changed = false;
            for (owner, name, binding) in additions {
                let candidates = self
                    .scope_tree
                    .get_mut(owner)
                    .unwrap()
                    .items
                    .glob_candidates
                    .entry(name)
                    .or_default();
                if let Some(existing) = candidates
                    .iter_mut()
                    .find(|existing| existing.kind == binding.kind)
                {
                    changed |= existing.merge_entry(&binding);
                } else {
                    candidates.push(binding);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn install_import(
        &mut self,
        owner: ScopeId,
        import: &ResolvedImport,
        visibility: crate::binding::Visibility,
        span: rustc_span::Span,
    ) -> ResolveResult<()> {
        if matches!(import, ResolvedImport::Glob(_)) {
            return Ok(());
        }
        let (source, mut names, _glob) = match import {
            ResolvedImport::Single(source, name) => (*source, vec![(*name, *name)], false),
            ResolvedImport::Alias {
                source_scope,
                original,
                alias,
            } => (*source_scope, vec![(*original, *alias)], false),
            ResolvedImport::Multi(source, names) => {
                (*source, names.iter().map(|&n| (n, n)).collect(), false)
            }
            ResolvedImport::Glob(source) => (
                *source,
                self.scope_tree
                    .get(*source)
                    .unwrap()
                    .items
                    .declarations()
                    .keys()
                    .map(|&n| (n, n))
                    .collect(),
                true,
            ),
        };
        names.sort_by(|a, b| a.1.as_str().cmp(b.1.as_str()));
        for (name, alias) in names {
            let binding = self.lookup_in_scope(name, source).ok_or_else(|| {
                ResolveError::UnresolvedImportSegment {
                    segment: name.to_string(),
                    span,
                }
            })?;
            if !binding.accessible(&self.scope_tree, owner) {
                return Err(ResolveError::InvalidNodeType {
                    message:
                        "import entry or original declaration is not accessible from this module"
                            .into(),
                    span,
                });
            }
            if visibility == crate::binding::Visibility::Public
                && binding.vis != crate::binding::Visibility::Public
            {
                return Err(ResolveError::InvalidNodeType {
                    message: "pub use cannot expose a non-public declaration".into(),
                    span,
                });
            }
            let binding = binding.imported(owner, visibility);
            let scope = self.scope_tree.get_mut(owner).unwrap();
            if let Some(existing) = scope.items.get_direct(alias) {
                if existing.kind == binding.kind {
                    let mut merged = existing.clone();
                    merged.merge_entry(&binding);
                    scope.items.define_or_overwrites(alias, merged);
                    continue;
                }
                return Err(ResolveError::DuplicateDefinition {
                    name: alias.to_string(),
                    first_span: span,
                    second_span: span,
                });
            }
            scope
                .items
                .define(alias, binding)
                .map_err(|_| ResolveError::InternalError("import insertion conflict".into()))?;
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
            directive.origin_scope.unwrap_or(starting_scope),
            starting_scope,
            directive.span,
        )?;

        match &directive.kind {
            ImportKind::Single { name, .. } => {
                self.verify_name_in_scope(*name, target_scope, directive.span)?;
                Ok(ResolvedImport::Single(target_scope, *name))
            }
            ImportKind::Glob { .. } => Ok(ResolvedImport::Glob(target_scope)),
            ImportKind::Multi { names, .. } => {
                for name in names {
                    self.verify_name_in_scope(*name, target_scope, directive.span)?;
                }
                Ok(ResolvedImport::Multi(target_scope, names.clone()))
            }
            ImportKind::Alias {
                original, alias, ..
            } => {
                self.verify_name_in_scope(*original, target_scope, directive.span)?;
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
        starting_requester: ScopeId,
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
            PathAnchor::Package => self
                .scope_tree
                .ancestors(scope_id)
                .find(|scope| scope.kind == ScopeKind::Package)
                .map(|scope| scope.id)
                .ok_or_else(|| ResolveError::ModuleNotFound {
                    message: "current package root is unavailable".into(),
                    span,
                })?,
        };

        // Walk name segments.
        for (index, name) in segments.iter().enumerate() {
            let binding = (if index == 0 && anchor == PathAnchor::Local {
                self.scope_tree
                    .ancestors(scope_id)
                    .find_map(|s| self.lookup_in_scope(*name, s.id))
            } else {
                self.lookup_in_scope(*name, scope_id)
            })
            .ok_or_else(|| ResolveError::UnresolvedImportSegment {
                segment: name.to_string(),
                span,
            })?;

            if !binding.accessible(&self.scope_tree, starting_requester) {
                return Err(ResolveError::InvalidNodeType {
                    message: "module path entry is not accessible from this scope".into(),
                    span,
                });
            }
            match binding.kind {
                BindingKind::Definition(def_id, _) => {
                    // Follow the DefId to its owning scope.
                    scope_id = self.find_scope_for_def(def_id).ok_or_else(|| {
                        ResolveError::UnresolvedImportSegment {
                            segment: name.as_str().to_owned(),
                            span,
                        }
                    })?;
                }
                _ => {}
            }
        }

        Ok(scope_id)
    }

    /// Look up a name in a scope (direct declarations + imports).
    fn lookup_in_scope(&self, name: Symbol, scope_id: ScopeId) -> Option<Binding> {
        let scope = self.scope_tree.get(scope_id)?;

        if let Some(b) = scope.items.get_local(name) {
            return Some(b.clone());
        }

        scope.items.unique_glob(name).cloned()
    }

    /// Look up a name in direct declarations only (no import traversal).
    fn lookup_direct_in_scope(&self, name: Symbol, scope_id: ScopeId) -> Option<Binding> {
        let scope = self.scope_tree.get(scope_id)?;
        scope.items.get_direct(name).cloned()
    }

    /// Verify that a name exists in a scope.
    fn verify_name_in_scope(
        &self,
        name: Symbol,
        scope_id: ScopeId,
        span: rustc_span::Span,
    ) -> ResolveResult<()> {
        if !self.pending_explicit(scope_id, name) && self.lookup_in_scope(name, scope_id).is_some()
        {
            return Ok(());
        }
        Err(ResolveError::UnresolvedImportSegment {
            segment: name.to_string(),
            span,
        })
    }

    /// Find the ScopeId that a DefId owns.
    fn find_scope_for_def(&self, def_id: DefId) -> Option<ScopeId> {
        self.def_to_scope.get(&def_id).copied().filter(|id| {
            self.scope_tree
                .get(*id)
                .is_some_and(|scope| scope.is_module() || scope.kind == ScopeKind::AdtBody)
        })
    }
}

impl ModuleTree {
    /// Dump the scope tree as an S-expression string (for debugging).
    pub fn dump_scope_tree(&self) -> String {
        let roots: Vec<_> = self
            .scope_tree
            .iter()
            .filter(|s| s.parent.is_none())
            .map(|s| s.id)
            .collect();
        let mut out = String::new();
        if roots.len() != 1 {
            out.push_str("(scope-forest\n");
        }
        for root in &roots {
            self.dump_scope_recursive(&mut out, *root, usize::from(roots.len() != 1));
        }
        if roots.len() != 1 {
            out.push_str(")\n");
        }
        out
    }

    fn dump_scope_recursive(&self, out: &mut String, scope_id: ScopeId, indent: usize) {
        use std::fmt::Write;
        let Some(scope) = self.scope_tree.get(scope_id) else {
            return;
        };
        let pad = " ".repeat(indent);
        writeln!(
            out,
            "{pad}(scope (kind {:?}) (name {}) (id {})",
            scope.kind,
            dump_string(scope.name.as_deref().unwrap_or("<anon>")),
            scope.id.raw()
        )
        .unwrap();
        if let Some(site) = scope.block_site {
            writeln!(
                out,
                "{pad} (source-block (file {}) (node {}))",
                site.file.raw(),
                site.node
            )
            .unwrap();
        }
        let mut declarations: Vec<_> = scope.items.declarations().iter().collect();
        declarations.sort_by_key(|(name, _)| name.to_string());
        for (name, binding) in declarations {
            writeln!(
                out,
                "{pad} (binding (name {}) (visibility {:?}) {})",
                dump_string(name.as_str()),
                binding.vis,
                dump_binding(binding.kind)
            )
            .unwrap();
        }
        for clause in scope.items.clauses() {
            writeln!(
                out,
                "{pad} (clause (name {}) {})",
                dump_string(clause.name.as_str()),
                dump_binding(clause.binding.kind)
            )
            .unwrap();
        }
        for (tag, imports) in [
            ("import", scope.items.imports()),
            ("reexport", scope.items.reexports()),
        ] {
            for import in imports {
                let value = match import {
                    ResolvedImport::Glob(scope) => format!("(glob (scope {}))", scope.raw()),
                    ResolvedImport::Single(scope, name) => format!(
                        "(single (scope {}) (name {}))",
                        scope.raw(),
                        dump_string(name.as_str())
                    ),
                    ResolvedImport::Multi(scope, names) => format!(
                        "(multi (scope {}) (names {}))",
                        scope.raw(),
                        names
                            .iter()
                            .map(|n| dump_string(n.as_str()))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                    ResolvedImport::Alias {
                        source_scope,
                        original,
                        alias,
                    } => format!(
                        "(alias (scope {}) (original {}) (name {}))",
                        source_scope.raw(),
                        dump_string(original.as_str()),
                        dump_string(alias.as_str())
                    ),
                };
                writeln!(out, "{pad} ({tag} {value})").unwrap();
            }
        }
        for child in &scope.children {
            self.dump_scope_recursive(out, *child, indent + 2);
        }
        writeln!(out, "{pad})").unwrap();
    }
}

/// A package's direct dependency name, independent of the provider's own name.
#[derive(Clone, Debug)]
pub struct DependencyEdge {
    pub requester: u32,
    pub alias: String,
    pub provider: u32,
}
/// Resolver output for explicitly supplied packages. File offsets identify AST
/// references across independent VFS arrays; source paths remain package-local.
pub struct PackageSetTree {
    pub tree: ModuleTree,
    pub file_offsets: HashMap<u32, u32>,
    pub package_scopes: HashMap<u32, ScopeId>,
}

/// Validate explicit graph inputs without reading source files or creating scopes.
/// Package ids here are resolution ids, not stable semantic identities.
pub fn validate_package_dependencies(
    packages: impl IntoIterator<Item = u32>,
    dependencies: &[DependencyEdge],
    builtin: Option<u32>,
) -> ResolveResult<()> {
    let invalid = ResolveError::InvalidPackageGraph;
    let mut ids = std::collections::HashSet::new();
    for id in packages {
        if !ids.insert(id) {
            return Err(invalid(format!(
                "duplicate source package resolution id {id}"
            )));
        }
    }
    if let Some(builtin) = builtin {
        if !ids.contains(&builtin) {
            return Err(invalid(format!("builtin package {builtin} is absent")));
        }
    }
    let mut aliases = std::collections::HashMap::new();
    for edge in dependencies {
        let context = || {
            format!(
                "dependency from package {} via alias {:?} to package {}",
                edge.requester, edge.alias, edge.provider
            )
        };
        if !ids.contains(&edge.requester) {
            return Err(invalid(format!(
                "{}: requester package {} is absent",
                context(),
                edge.requester
            )));
        }
        if !ids.contains(&edge.provider) {
            return Err(invalid(format!(
                "{}: provider package {} is absent",
                context(),
                edge.provider
            )));
        }
        if edge.alias.is_empty() {
            return Err(invalid(format!("{}: dependency alias is empty", context())));
        }
        if let Some(previous) = aliases.insert((edge.requester, &edge.alias), edge.provider) {
            return Err(invalid(format!(
                "{}: duplicate dependency alias, already bound to package {previous}",
                context()
            )));
        }
        if edge.requester == edge.provider {
            return Err(invalid(format!(
                "{}: a package cannot declare itself as a dependency",
                context()
            )));
        }
        if let Some(builtin) = builtin {
            if edge.requester != builtin && edge.alias == "builtin" && edge.provider != builtin {
                return Err(invalid(format!(
                    "{}: builtin dependency alias must refer to package {builtin}",
                    context()
                )));
            }
        }
    }
    Ok(())
}

pub fn build_package_set(
    source_map: &SourceMap,
    diag_ctx: &DiagnosticContext<'_>,
    packages: &mut [(u32, &mut vfs::Vfs)],
    dependencies: &[DependencyEdge],
) -> ResolveResult<PackageSetTree> {
    build_package_set_with_builtin(source_map, diag_ctx, packages, dependencies, None)
}

/// Select the builtin package by identity. Its prelude is an ordinary package-
/// visible glob entry in every other package, never an implicit public export.
pub fn build_package_set_with_builtin(
    source_map: &SourceMap,
    diag_ctx: &DiagnosticContext<'_>,
    packages: &mut [(u32, &mut vfs::Vfs)],
    dependencies: &[DependencyEdge],
    builtin: Option<u32>,
) -> ResolveResult<PackageSetTree> {
    use crate::binding::Visibility;
    let invalid = |message: &str| ResolveError::InternalError(message.into());
    validate_package_dependencies(packages.iter().map(|(id, _)| *id), dependencies, builtin)?;
    let mut effective_dependencies = dependencies.to_vec();
    if let Some(builtin) = builtin {
        for (requester, _) in packages.iter() {
            if *requester != builtin
                && !dependencies
                    .iter()
                    .any(|edge| edge.requester == *requester && edge.alias == "builtin")
            {
                effective_dependencies.push(DependencyEdge {
                    requester: *requester,
                    alias: "builtin".into(),
                    provider: builtin,
                });
            }
        }
    }
    let dependencies = &effective_dependencies;
    let mut builder = ModuleBuilder::new(source_map, diag_ctx);
    let mut roots = HashMap::new();
    let mut package_scopes = HashMap::new();
    let mut file_offsets = HashMap::new();
    let mut file_offset = 0u32;
    for (identity, vfs) in packages.iter_mut() {
        builder.def_gen.set_package(*identity);
        let root = builder.scope_gen.next();
        let root_def = builder.def_gen.next();
        builder.scope_tree.add_scope(Scope::new(
            root,
            ScopeKind::Root,
            None,
            None,
            root_def,
            false,
        ));
        roots.insert(*identity, root);
        file_offsets.insert(*identity, file_offset);
        let count =
            u32::try_from(vfs.files().count()).map_err(|_| invalid("too many source files"))?;
        let next_offset = file_offset
            .checked_add(count)
            .filter(|n| *n < u32::MAX)
            .ok_or_else(|| invalid("combined file identity space exhausted"))?;
        let mut scanner = VfsScanner::new(
            source_map,
            diag_ctx,
            vfs,
            &mut builder.scope_tree,
            &mut builder.def_gen,
            &mut builder.scope_gen,
        );
        scanner.file_offset = file_offset;
        if let Err(error) = scanner.scan_package(root) {
            builder.errors.push(error);
        }
        let (imports, impls, names, files) = scanner.into_results();
        builder.unresolved_imports.extend(imports);
        builder.impls.extend(impls);
        builder.def_names.extend(names);
        builder.file_scopes.extend(
            files
                .into_iter()
                .map(|(file, scope)| (vfs::FileId::from_raw(file.raw() + file_offset), scope)),
        );
        let scope = builder
            .scope_tree
            .iter()
            .find(|s| s.parent == Some(root) && s.kind == ScopeKind::Package)
            .ok_or_else(|| invalid("package scan did not create a scope"))?
            .id;
        package_scopes.insert(*identity, scope);
        file_offset = next_offset;
    }
    for dependency in dependencies {
        let root = roots[&dependency.requester];
        let provider_scope = package_scopes[&dependency.provider];
        let provider = builder.scope_tree.get(provider_scope).unwrap();
        let binding = Binding {
            kind: BindingKind::Definition(provider.owner_def, provider.parent.unwrap()),
            vis: Visibility::Public,
            entry: None,
        };
        builder
            .scope_tree
            .get_mut(root)
            .unwrap()
            .items
            .define(Symbol::intern(&dependency.alias), binding)
            .map_err(|_| ResolveError::InvalidPackageGraph(format!("dependency from package {} via alias {:?} to package {} conflicts with a package name", dependency.requester, dependency.alias, dependency.provider)))?;
    }
    if let Some(builtin) = builtin {
        for (identity, _) in packages.iter() {
            if *identity == builtin {
                continue;
            }
            let mut import = ImportDirective::new(
                package_scopes[identity],
                PathAnchor::Local,
                ImportKind::Glob { source_scope: None },
                vec![Symbol::intern("prelude")],
                rustc_span::DUMMY_SP,
                0,
                vfs::FileId::INVALID,
                Visibility::Package,
            );
            import.origin_scope = Some(package_scopes[&builtin]);
            builder.unresolved_imports.push(import);
        }
    }
    Ok(PackageSetTree {
        tree: builder.finish(),
        file_offsets,
        package_scopes,
    })
}

fn dump_binding(kind: BindingKind) -> String {
    let node =
        |n: crate::ids::AstNodeRef| format!("(ast (file {}) (node {}))", n.file.raw(), n.node);
    let definition = |id: DefId| format!("(definition (package {}) (local {}))", id.pkg, id.index);
    match kind {
        BindingKind::Definition(id, scope) => {
            format!("(decl {} (scope {}))", definition(id), scope.raw())
        }
        BindingKind::ClauseParam(id, reference) => {
            format!("(parameter {} {})", definition(id), node(reference))
        }
        BindingKind::FnParam(reference) => format!("(fn-parameter {})", node(reference)),
        BindingKind::Local(reference) => format!("(local {})", node(reference)),
        BindingKind::ExpandedLocal(reference, output) => {
            format!("(expanded-local {} (output {output}))", node(reference))
        }
        BindingKind::LoopLabel(reference) => format!("(loop-label {})", node(reference)),
    }
}
fn dump_string(value: &str) -> String {
    use std::fmt::Write;
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => write!(out, "\\u{:04X}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod package_graph_tests {
    use super::*;
    use diagnostic::FlurryError;

    fn edge(requester: u32, alias: &str, provider: u32) -> DependencyEdge {
        DependencyEdge {
            requester,
            alias: alias.into(),
            provider,
        }
    }

    #[test]
    fn invalid_graphs_have_input_diagnostics_with_edge_context() {
        for (edges, builtin, fragments) in [
            (
                vec![edge(9, "helper", 2)],
                None,
                vec![
                    "requester package 9 is absent",
                    "alias \"helper\"",
                    "to package 2",
                ],
            ),
            (
                vec![edge(1, "helper", 9)],
                None,
                vec![
                    "provider package 9 is absent",
                    "from package 1",
                    "alias \"helper\"",
                ],
            ),
            (
                vec![edge(1, "", 2)],
                None,
                vec!["alias is empty", "from package 1"],
            ),
            (
                vec![edge(1, "helper", 2), edge(1, "helper", 3)],
                None,
                vec![
                    "duplicate dependency alias",
                    "already bound to package 2",
                    "to package 3",
                ],
            ),
            (
                vec![edge(1, "self", 1)],
                None,
                vec!["cannot declare itself", "alias \"self\""],
            ),
            (
                vec![edge(1, "builtin", 3)],
                Some(2),
                vec![
                    "builtin dependency alias must refer to package 2",
                    "to package 3",
                ],
            ),
            (vec![], Some(9), vec!["builtin package 9 is absent"]),
        ] {
            let error = validate_package_dependencies([1, 2, 3], &edges, builtin).unwrap_err();
            assert!(matches!(error, ResolveError::InvalidPackageGraph(_)));
            assert_eq!(error.error_code(), 4010);
            assert_eq!(error.error_name(), "invalid_package_graph");
            for fragment in fragments {
                assert!(error.message().contains(fragment), "{}", error.message());
            }
        }
        assert!(
            validate_package_dependencies([1, 1], &[], None)
                .unwrap_err()
                .message()
                .contains("resolution id 1")
        );
    }

    #[test]
    fn aliases_are_requester_local_and_validation_recovers_after_failure() {
        assert!(validate_package_dependencies([1, 2, 3], &[edge(1, "shared", 9)], None).is_err());
        validate_package_dependencies(
            [1, 2, 3],
            &[
                edge(1, "shared", 3),
                edge(2, "shared", 3),
                edge(1, "builtin", 3),
            ],
            Some(3),
        )
        .unwrap();
        // Invalid names in diagnostics are escaped; graph validation does not
        // manufacture terminal control sequences from a user-provided alias.
        let error = validate_package_dependencies([1], &[edge(1, "bad\n\x1b", 9)], None)
            .unwrap_err()
            .message();
        assert!(!error.contains('\n') && !error.contains('\x1b'));
        assert!(error.contains("bad\\n\\u{1b}"));
    }
}
