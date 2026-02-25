//! VFS scanner – walks the virtual file system to discover files, parse them,
//! and build the package scope tree.
//!
//! Migrated and refactored from `luna/src/scan/vfs_scanner.rs`.

use std::sync::Arc;

use ast::Ast;
use diagnostic::{DiagnosticContext, FlurryError};
use lex::lex;
use parse::parser::Parser;
use rustc_span::{SourceFile, SourceMap};

use crate::binding::{BindingKind, Visibility};
use crate::error::{ResolveError, ResolveResult};
use crate::ids::{DefId, DefIdGen, ScopeId, ScopeIdGen};
use crate::import::ImportDirective;
use crate::namespace::Namespace;
use crate::scope::{Scope, ScopeKind, ScopeTree};

use super::ast_scanner::AstScanner;

/// Walks the VFS, parses source files, and builds the package scope tree.
pub struct VfsScanner<'a> {
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
    /// DefId → name mapping for diagnostics.
    pub def_names: Vec<(DefId, String)>,
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
            source_map,
            diag_ctx,
            vfs,
            scope_tree,
            def_gen,
            scope_gen,
            imports: Vec::new(),
            def_names: Vec::new(),
        }
    }

    /// Scan the package rooted in the VFS.
    ///
    /// Returns the collected import directives; the scope tree is mutated in place.
    pub fn scan_package(
        &mut self,
        root_scope: ScopeId,
    ) -> ResolveResult<()> {
        let package_name = self.vfs.name.clone();
        let package_def = self.def_gen.next();
        let package_scope_id = self.scope_gen.next();

        let scope = Scope::new(
            package_scope_id,
            ScopeKind::Package,
            Some(root_scope),
            Some(package_name.clone()),
            package_def,
            false,
        );
        self.scope_tree.add_scope(scope);
        self.scope_tree.add_child(root_scope, package_scope_id);

        // Register package name in root scope
        {
            let binding = crate::binding::Binding {
                kind: BindingKind::Module,
                def_id: package_def,
                defined_in: root_scope,
                ast_ref: None,
                ns: Namespace::Type,
                vis: Visibility::Public,
            };
            if let Some(root) = self.scope_tree.get_mut(root_scope) {
                let _ = root.items.define(package_name.clone(), Namespace::Type, binding);
            }
        }

        self.def_names.push((package_def, package_name));

        // Scan all source files
        let file_count = self.vfs.file_count();
        for i in 0..file_count {
            let file_id = vfs::FileId::from_raw(i as u32);
            self.scan_source_file(file_id, package_scope_id, package_def)?;
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
        let file_name = rel_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let is_entry = file_name == "main.fl" || file_name == "lib.fl";
        let module_name = if is_entry {
            None // Entry files merge into parent scope
        } else if file_name.ends_with(".fl") {
            Some(file_name[..file_name.len() - 3].to_string())
        } else {
            Some(file_name.to_string())
        };

        // Parse the file if not already parsed
        if self.vfs.get_ast(file_id).is_none() {
            self.parse_file(file_id, &source_file)?;
        }

        // Determine the scope to scan into
        let scan_scope = if let Some(ref mod_name) = module_name {
            let mod_def = self.def_gen.next();
            let mod_scope_id = self.scope_gen.next();

            let scope = Scope::new(
                mod_scope_id,
                ScopeKind::Module,
                Some(parent_scope),
                Some(mod_name.clone()),
                mod_def,
                false,
            );
            self.scope_tree.add_scope(scope);
            self.scope_tree.add_child(parent_scope, mod_scope_id);

            // Register the file module in the parent scope
            let binding = crate::binding::Binding {
                kind: BindingKind::Module,
                def_id: mod_def,
                defined_in: parent_scope,
                ast_ref: None,
                ns: Namespace::Type,
                vis: Visibility::Public,
            };
            if let Some(ps) = self.scope_tree.get_mut(parent_scope) {
                let _ = ps.items.define(mod_name.clone(), Namespace::Type, binding);
            }
            self.def_names.push((mod_def, mod_name.clone()));

            mod_scope_id
        } else {
            parent_scope
        };

        // Scan the AST
        let ast = self.vfs.get_ast(file_id)
            .ok_or_else(|| ResolveError::InternalError("AST not found after parsing".into()))?;

        // We need a temporary borrow workaround: collect data, then create scanner
        let ast_ptr: *const Ast = ast;
        let mut scanner = AstScanner {
            ast: unsafe { &*ast_ptr },
            source_map: self.source_map,
            file_id,
            scope_tree: self.scope_tree,
            def_gen: self.def_gen,
            scope_gen: self.scope_gen,
            imports: &mut self.imports,
            def_names: &mut self.def_names,
        };

        scanner.scan_file_items(scan_scope)?;

        Ok(())
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

        let (tokens, lex_errors) = lex(content, source_file.start_pos);

        for err in lex_errors {
            err.emit(self.diag_ctx, source_file.start_pos);
        }

        let mut parser = Parser::new(self.source_map, tokens, source_file.start_pos);
        parser.parse(self.diag_ctx);
        let ast = parser.finalize();
        self.vfs.set_ast(file_id, ast);

        Ok(())
    }

    /// Consume the scanner and return the collected imports and def names.
    pub fn into_results(self) -> (Vec<ImportDirective>, Vec<(DefId, String)>) {
        (self.imports, self.def_names)
    }
}
