use std::mem;

use rustc_span::SourceMap;

use crate::diagnostic::DiagnosticContext;

pub mod scan;
pub mod scope;

pub struct CompilerContext<'hir> {
    pub scope_manager: scope::ScopeManager<'hir>,
    diag_ctx: DiagnosticContext<'hir>,
}

impl<'hir> CompilerContext<'hir> {
    pub fn new(source_map: &'hir SourceMap) -> CompilerContext<'hir> {
        let scope_manager = scope::ScopeManager::new();

        Self {
            scope_manager,
            diag_ctx: DiagnosticContext::new(source_map),
        }
    }

    pub fn diag_ctx<'ctx>(&'ctx mut self) -> &'ctx mut DiagnosticContext<'ctx> {
        unsafe { mem::transmute(&mut self.diag_ctx) }
    }
}
