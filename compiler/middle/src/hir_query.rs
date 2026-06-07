//! Query inputs for the `hir_package` query.
//!
//! [`HirQueryInput`] bundles all data that `ast_lowering::lower_to_hir` needs.
//! Owned data (AST, module tree) are held via `Arc`; data that carries a
//! lifetime (`SourceMap`, `DiagnosticContext`) is stored as a raw pointer and
//! accessed through `unsafe` accessors.
//!
//! # Lifetime safety
//!
//! The caller (the compiler driver in `luna/src/main.rs`) must guarantee that
//! both pointer targets remain live for as long as the `HirQueryInput` is
//! accessible — in practice for the entire compilation.

use std::sync::Arc;

use ast::Ast;
use diagnostic::DiagnosticContext;
use resolve::{ModuleTree, ScopeId};
use rustc_span::SourceMap;

/// All inputs required to lower one file's AST to HIR.
pub struct HirQueryInput {
    /// Parsed AST of the source file.
    pub ast: Arc<Ast>,
    /// Module/scope tree produced by name resolution.
    pub module_tree: Arc<ModuleTree>,
    /// Top-level scope of the file being lowered.
    pub file_scope: ScopeId,
    /// Raw pointer to the session's `SourceMap`.
    source_map_ptr: *const SourceMap,
    /// Raw pointer to the compiler's `DiagnosticContext` (lifetime erased).
    diag_ctx_ptr: *const DiagnosticContext<'static>,
}

// SAFETY: Luna is single-threaded.  The raw pointers are only dereferenced on
// the main thread inside `ast_lowering::lower_to_hir`.
unsafe impl Send for HirQueryInput {}
unsafe impl Sync for HirQueryInput {}

impl HirQueryInput {
    /// Construct a new query input.
    ///
    /// # Safety
    ///
    /// `source_map_ptr` and `diag_ctx_ptr` must remain valid for the entire
    /// duration that this `HirQueryInput` is live.
    pub fn new(
        ast: Arc<Ast>,
        module_tree: Arc<ModuleTree>,
        file_scope: ScopeId,
        source_map_ptr: *const SourceMap,
        diag_ctx_ptr: *const DiagnosticContext<'static>,
    ) -> Self {
        HirQueryInput { ast, module_tree, file_scope, source_map_ptr, diag_ctx_ptr }
    }

    /// Borrow the AST.
    pub fn ast(&self) -> &Ast {
        &self.ast
    }

    /// Borrow the module tree.
    pub fn module_tree(&self) -> &ModuleTree {
        &self.module_tree
    }

    /// The file's top-level scope ID.
    pub fn file_scope(&self) -> ScopeId {
        self.file_scope
    }

    /// Obtain a reference to the source map.
    ///
    /// # Safety
    ///
    /// The pointer passed to [`new`](Self::new) must still be valid.
    pub unsafe fn source_map(&self) -> &SourceMap {
        &*self.source_map_ptr
    }

    /// Obtain a reference to the diagnostic context.
    ///
    /// # Safety
    ///
    /// The pointer passed to [`new`](Self::new) must still be valid and the
    /// lifetime `'a` must be no longer than the pointee's actual lifetime.
    pub unsafe fn diag_ctx<'a>(&self) -> &'a DiagnosticContext<'a> {
        &*(self.diag_ctx_ptr as *const DiagnosticContext<'a>)
    }
}
