use rustc_span::SourceMap;

use crate::diagnostic::DiagnosticContext;

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

    pub fn diag_ctx<'ctx>(&'ctx self) -> &'ctx DiagnosticContext<'ctx> {
        &self.diag_ctx
    }
}
