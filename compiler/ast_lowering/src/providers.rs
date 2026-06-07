//! Provider registration for the `ast_lowering` pass.
//!
//! Call [`set_providers`] once at startup to register this crate's
//! implementations into the compiler's [`Providers`] dispatch table:
//!
//! ```ignore
//! ast_lowering::set_providers(&mut instance.db.providers);
//! ```

use std::sync::Arc;

use hir::{HirArena, Package};
use middle::queries::{Db, Providers};
use middle::HirPackageBox;
use resolve::Resolver;

/// Provider for the `hir_package` query.
///
/// Reads all inputs from `db.hir_input()`, runs `lower_to_hir`, and returns
/// the resulting HIR package wrapped in `HirPackageBox`.
fn lower_package_ast(db: &Db) -> Arc<HirPackageBox> {
    let input = db
        .hir_input()
        .expect("hir_package: hir_input not set — call db.set_hir_input(...) first");

    // SAFETY: the driver guarantees both pointers remain valid for the
    // duration of this call (they point into locals that outlive the query).
    let source_map = unsafe { input.source_map() };
    let diag_ctx = unsafe { input.diag_ctx() };

    let resolver = Resolver::new(&input.module_tree);

    let arena = HirArena::new();
    let mut package = Package::new();
    crate::lower_to_hir(
        input.ast(),
        &arena,
        source_map,
        diag_ctx,
        &mut package,
        &resolver,
        input.file_scope(),
    );

    // SAFETY: `package` borrows from `arena`.  We erase the `'hir` lifetime so
    // the borrow checker permits moving `arena`.  Both are immediately bundled
    // together inside `HirPackageBox`, which upholds the invariant.
    let package: hir::Package<'static> = unsafe { std::mem::transmute(package) };
    Arc::new(HirPackageBox::new(arena, package))
}

/// Register `ast_lowering` providers into the compiler's dispatch table.
///
/// Must be called **before** issuing any HIR queries.
pub fn set_providers(providers: &mut Providers) {
    providers.hir_package = lower_package_ast;
}
