//! Session-bound inputs for HIR lowering. ASTs and resolution data are owned;
//! the source map is borrowed for the database lifetime and diagnostics retain
//! shared ownership. Query reuse cannot outlive either dependency.

use std::sync::Arc;

use ast::Ast;
use diagnostic::DiagnosticContext;
use resolve::{ModuleTree, ScopeId};
use rustc_span::SourceMap;

/// All inputs required to lower one file's AST to HIR.
pub struct HirQueryInput<'session> {
    /// Parsed AST of the source file.
    pub ast: Arc<Ast>,
    pub additional_files: Vec<(vfs::FileId, Arc<Ast>, ScopeId)>,
    /// Module/scope tree produced by name resolution.
    pub module_tree: Arc<ModuleTree>,
    /// Top-level scope of the file being lowered.
    pub file_scope: ScopeId,
    /// File ID of the source file being lowered.
    pub file_id: vfs::FileId,
    source_map: &'session SourceMap,
    diagnostics: Arc<DiagnosticContext<'session>>,
}

impl<'session> HirQueryInput<'session> {
    pub fn new(
        ast: Arc<Ast>,
        file_id: vfs::FileId,
        module_tree: Arc<ModuleTree>,
        file_scope: ScopeId,
        source_map: &'session SourceMap,
        diagnostics: Arc<DiagnosticContext<'session>>,
    ) -> Self {
        Self {
            ast,
            additional_files: Vec::new(),
            file_id,
            module_tree,
            file_scope,
            source_map,
            diagnostics,
        }
    }

    /// Borrow the AST.
    pub fn ast(&self) -> &Ast {
        &self.ast
    }

    pub fn file_id(&self) -> vfs::FileId {
        self.file_id
    }

    /// Borrow the module tree.
    pub fn module_tree(&self) -> &ModuleTree {
        &self.module_tree
    }

    /// The file's top-level scope ID.
    pub fn file_scope(&self) -> ScopeId {
        self.file_scope
    }

    pub fn source_map(&self) -> &SourceMap {
        self.source_map
    }

    pub fn diag_ctx(&self) -> &DiagnosticContext<'session> {
        &self.diagnostics
    }
}
