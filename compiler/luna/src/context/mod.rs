use std::primitive;

use rustc_span::SourceMap;

use crate::{
    context::scope::Item,
    diagnostic::DiagnosticContext,
    hir::{Definition, Expr, Hir, HirMapping, Module},
    hir_expr, hir_int, hir_placeholder, hir_put_expr, hir_str, hir_symbol, hir_update,
    intrinsic::setup_intrinsics,
};

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

    /// Set up the compiler context with intrinsics
    pub fn setup(&mut self, hir: &'hir Hir) {
        // Register builtin package `builtin`
        let scope_root = self.scope_manager.root;

        // Create placeholder for builtin package to handle circular reference
        let builtin_package_id = crate::hir_placeholder!(hir);

        let builtin_scope = self
            .scope_manager
            .add_scope(
                Some(hir.intern_str("builtin")),
                Some(scope_root),
                false,
                builtin_package_id,
            )
            .expect("Failed to create builtin scope");

        // Now create the actual package definition with the scope
        let builtin_package_def = crate::hir_package!(hir, "builtin", builtin_scope);
        crate::hir_update!(
            hir,
            builtin_package_id,
            Definition(hir.intern_definition(builtin_package_def), 0)
        );

        // Register modules beneath the builtin package using macro
        for builtin_module in ["std", "math", "meta", "build", "attrs"] {
            let module_id = hir_placeholder!(hir);
            let module_name = hir.intern_str(builtin_module);
            let scope_id = self
                .scope_manager
                .add_scope(Some(module_name), Some(builtin_scope), false, module_id)
                .expect("Failed to create builtin module scope");
            hir_update!(
                hir,
                module_id,
                Definition(
                    hir.intern_definition(Definition::Module(Module {
                        name: module_name,
                        clauses: hir.empty.clauses,
                        scope_id
                    },)),
                    module_id
                )
            );
        }

        setup_intrinsics(hir, self, builtin_scope, builtin_package_id);
    }

    pub fn diag_ctx<'ctx>(&'ctx self) -> &'ctx DiagnosticContext<'ctx> {
        &self.diag_ctx
    }
}
