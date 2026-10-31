//! VFS scanner – walks the virtual file system to discover files, parse them,
//! and build the package scope tree.
//!
//! Migrated and refactored from `luna/src/scan/vfs_scanner.rs`.

use std::collections::HashMap;
use std::sync::Arc;

use ast::Ast;
use diagnostic::{DiagnosticContext, FlurryError};
use lex::lex;
use parse::parser::Parser;
use rustc_span::{SourceFile, SourceMap};
use symbol::Symbol;

use crate::binding::{BindingKind, Visibility};
use crate::error::{ResolveError, ResolveResult};
use crate::ids::{DefId, DefIdGen, ScopeId, ScopeIdGen};
use crate::impl_directive::ImplDirective;
use crate::import::ImportDirective;
use crate::scope::{Scope, ScopeKind, ScopeTree};

use super::ast_scanner::AstScanner;

/// Walks the VFS, parses source files, and builds the package scope tree.
pub struct VfsScanner<'a> {
    pub file_offset: u32,
    /// Source map for span resolution.
    pub source_map: &'a SourceMap,
    /// Diagnostic context for reporting errors.
    pub diag_ctx: &'a DiagnosticContext<'a>,
    /// The VFS being scanned.
    pub vfs: &'a mut vfs::Vfs,
    /// The scope tree being built.
    pub scope_tree: &'a mut ScopeTree,
    /// DefId generator.
    pub def_gen: &'a mut DefIdGen,
    /// ScopeId generator.
    pub scope_gen: &'a mut ScopeIdGen,
    /// Collected import directives (to be resolved later).
    pub imports: Vec<ImportDirective>,
    /// Collected impl directives.
    pub impls: Vec<ImplDirective>,
    /// DefId → name mapping for diagnostics.
    pub def_names: Vec<(DefId, Symbol)>,
    /// VFS FileId → the scope that owns that file's top-level definitions.
    pub file_scopes: HashMap<vfs::FileId, ScopeId>,
}

impl<'a> VfsScanner<'a> {
    pub fn new(
        source_map: &'a SourceMap,
        diag_ctx: &'a DiagnosticContext<'a>,
        vfs: &'a mut vfs::Vfs,
        scope_tree: &'a mut ScopeTree,
        def_gen: &'a mut DefIdGen,
        scope_gen: &'a mut ScopeIdGen,
    ) -> Self {
        Self {
            file_offset: 0,
            source_map,
            diag_ctx,
            vfs,
            scope_tree,
            def_gen,
            scope_gen,
            imports: Vec::new(),
            impls: Vec::new(),
            def_names: Vec::new(),
            file_scopes: HashMap::new(),
        }
    }

    /// Scan the package rooted in the VFS.
    ///
    /// Returns the collected import directives; the scope tree is mutated in place.
    pub fn scan_package(&mut self, root_scope: ScopeId) -> ResolveResult<()> {
        let package_name = Symbol::intern(&self.vfs.name);
        let package_def = self.def_gen.next();
        let package_scope_id = self.scope_gen.next();

        let scope = Scope::new(
            package_scope_id,
            ScopeKind::Package,
            Some(root_scope),
            Some(package_name),
            package_def,
            false,
        );
        self.scope_tree.add_scope(scope);
        self.scope_tree.add_child(root_scope, package_scope_id);

        // Register package name in root scope
        {
            let binding = crate::binding::Binding {
                entry: None,
                kind: BindingKind::Definition(package_def, root_scope),
                vis: Visibility::Public,
            };
            if let Some(root) = self.scope_tree.get_mut(root_scope) {
                let _ = root.items.define(package_name, binding);
            }
        }

        self.def_names.push((package_def, package_name));

        // Reserve directory module slots before attaching any source bodies.
        let mut paths = std::collections::BTreeSet::new();
        for directory in &self.vfs.source_directories {
            paths.insert(directory.clone());
        }
        for (_, entry) in self.vfs.files() {
            let mut parent = entry.rel_path.parent();
            while let Some(path) = parent.filter(|p| !p.as_os_str().is_empty() && !p.is_absolute())
            {
                paths.insert(path.to_path_buf());
                parent = path.parent();
            }
        }
        let mut directories = std::collections::BTreeMap::new();
        directories.insert(std::path::PathBuf::new(), package_scope_id);
        for path in paths {
            let parent = directories[path.parent().unwrap_or(std::path::Path::new(""))];
            let name = Symbol::intern(&path.file_name().unwrap().to_string_lossy());
            let scope = self.reserve_module(parent, name)?;
            directories.insert(path, scope);
        }
        let mut bodies = std::collections::HashSet::new();
        let file_count = self.vfs.file_count();
        for i in 0..file_count {
            let file_id = vfs::FileId::from_raw(i as u32);
            let path = &self.vfs.file(file_id).rel_path;
            let directory = path
                .parent()
                .filter(|p| !p.is_absolute())
                .unwrap_or(std::path::Path::new(""));
            let parent = *directories.get(directory).unwrap_or(&package_scope_id);
            self.scan_source_file(file_id, parent, package_def)?;
            let scope = self.file_scopes[&file_id];
            if !bodies.insert(scope) {
                return Err(ResolveError::ScopeCreationFailed {
                    message: "multiple source bodies for one module".into(),
                    span: rustc_span::DUMMY_SP,
                });
            }
        }

        Ok(())
    }

    /// Scan a single source file.
    fn scan_source_file(
        &mut self,
        file_id: vfs::FileId,
        parent_scope: ScopeId,
        _parent_def: DefId,
    ) -> ResolveResult<()> {
        let entry = self.vfs.file(file_id);
        let rel_path = entry.rel_path.clone();
        let source_file = entry.source_file.clone();

        // Determine module name from file path
        let file_name = rel_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let is_entry = self.vfs.selected_root.map_or(
            file_name == "main.fl" || file_name == "lib.fl",
            |selected| selected == file_id,
        ) || file_name == "mod.fl";
        let module_name: Option<Symbol> = if is_entry {
            None // Entry files merge into parent scope
        } else if file_name.ends_with(".fl") {
            Some(Symbol::intern(&file_name[..file_name.len() - 3]))
        } else {
            Some(Symbol::intern(file_name))
        };

        // Parse the file if not already parsed
        if self.vfs.get_ast(file_id).is_none() {
            self.parse_file(file_id, &source_file)?;
        }

        // Determine the scope to scan into
        let scan_scope = if let Some(mod_name) = module_name {
            self.reserve_module(parent_scope, mod_name)?
        } else {
            parent_scope
        };

        // Scan the AST
        let ast = self
            .vfs
            .get_ast(file_id)
            .ok_or_else(|| ResolveError::InternalError("AST not found after parsing".into()))?;

        // We need a temporary borrow workaround: collect data, then create scanner
        let ast_ptr: *const Ast = ast;
        let mut scanner = AstScanner {
            ast: unsafe { &*ast_ptr },
            source_map: self.source_map,
            file_id: vfs::FileId::from_raw(
                file_id.raw().checked_add(self.file_offset).ok_or_else(|| {
                    ResolveError::InternalError("combined file identity space exhausted".into())
                })?,
            ),
            scope_tree: self.scope_tree,
            def_gen: self.def_gen,
            scope_gen: self.scope_gen,
            imports: &mut self.imports,
            impls: &mut self.impls,
            def_names: &mut self.def_names,
        };

        scanner.scan_file_items(scan_scope)?;

        // Record the mapping from FileId to the scope that holds its definitions.
        self.file_scopes.insert(file_id, scan_scope);

        Ok(())
    }

    fn reserve_module(&mut self, parent: ScopeId, name: Symbol) -> ResolveResult<ScopeId> {
        let definition = self.def_gen.next();
        let id = self.scope_gen.next();
        let binding = crate::binding::Binding {
            entry: None,
            kind: BindingKind::Definition(definition, parent),
            vis: Visibility::Package,
        };
        self.scope_tree
            .get_mut(parent)
            .unwrap()
            .items
            .define(name, binding)
            .map_err(|_| ResolveError::DuplicateDefinition {
                name: name.to_string(),
                first_span: rustc_span::DUMMY_SP,
                second_span: rustc_span::DUMMY_SP,
            })?;
        self.scope_tree.add_scope(Scope::new(
            id,
            ScopeKind::Module,
            Some(parent),
            Some(name),
            definition,
            false,
        ));
        self.scope_tree.add_child(parent, id);
        self.def_names.push((definition, name));
        Ok(id)
    }

    /// Parse a source file and store its AST in the VFS.
    fn parse_file(
        &mut self,
        file_id: vfs::FileId,
        source_file: &Arc<SourceFile>,
    ) -> ResolveResult<()> {
        let content = source_file
            .src
            .as_ref()
            .ok_or_else(|| ResolveError::FileParsingFailed {
                message: "Source file content is None".into(),
                span: rustc_span::DUMMY_SP,
            })?;

        let (tokens, symbols, lex_errors) = lex(content, source_file.start_pos);

        let lex_failed = !lex_errors.is_empty();
        for err in lex_errors {
            err.emit(self.diag_ctx, source_file.start_pos);
        }

        let mut parser = Parser::new(self.source_map, tokens, symbols, source_file.start_pos);
        let parsed = parser.parse(self.diag_ctx);
        if lex_failed || !parsed {
            return Err(ResolveError::FileParsingFailed {
                message: "source file contains lexical or syntax errors".into(),
                span: rustc_span::Span::new(source_file.start_pos, source_file.start_pos),
            });
        }
        let ast = parser.finalize();
        ast.validate_structure()
            .map_err(|error| ResolveError::InvalidNodeType {
                span: ast.get_span(error.node()).unwrap_or(rustc_span::Span::new(
                    source_file.start_pos,
                    source_file.start_pos,
                )),
                message: error.to_string(),
            })?;
        self.vfs.set_ast(file_id, ast);

        Ok(())
    }

    /// Consume the scanner and return collected imports, impls, def names, and file scopes.
    pub fn into_results(
        self,
    ) -> (
        Vec<ImportDirective>,
        Vec<ImplDirective>,
        Vec<(DefId, Symbol)>,
        HashMap<vfs::FileId, ScopeId>,
    ) {
        (self.imports, self.impls, self.def_names, self.file_scopes)
    }
}
