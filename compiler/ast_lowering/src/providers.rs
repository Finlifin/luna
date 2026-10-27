//! Provider registration for the `ast_lowering` pass.
//!
//! Call [`set_providers`] once at startup to register this crate's
//! implementations into the compiler's [`Providers`] dispatch table:
//!
//! ```ignore
//! ast_lowering::set_providers(&mut instance.db.providers);
//! ```
//!
//! This is step 4 of adding a new query (see `query::howto`). The
//! `hir_package` provider below is the live reference implementation.

use std::sync::Arc;

use hir::{HirArena, Package};
use middle::HirPackageBox;
use middle::queries::{Db, Providers};
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

    let mut resolver = Resolver::new(&input.module_tree);

    let arena = HirArena::new();
    let mut package = Package::new();
    crate::lower_to_hir(
        input.ast(),
        input.file_id(),
        &arena,
        source_map,
        diag_ctx,
        &mut package,
        &mut resolver,
        input.file_scope(),
    );

    let root = package.root_mod;
    for (file_id, ast, scope) in &input.additional_files {
        let mut resolver = Resolver::new(&input.module_tree);
        crate::lower_to_hir(
            ast,
            *file_id,
            &arena,
            source_map,
            diag_ctx,
            &mut package,
            &mut resolver,
            *scope,
        );
    }
    package.root_mod = root;
    // Directory-only modules have no file to lower, but remain real HIR
    // containers. Reconstruct their parent edges from resolved scope identity.
    let scopes: Vec<_> = input
        .module_tree
        .scope_tree
        .iter()
        .filter(|scope| {
            matches!(
                scope.kind,
                resolve::scope::ScopeKind::Package | resolve::scope::ScopeKind::Module
            )
        })
        .collect();
    let mut pending_identities = std::collections::BTreeSet::new();
    let missing: Vec<_> = scopes
        .iter()
        .filter(|scope| {
            package.definition_owner(scope.owner_def).is_none()
                && pending_identities.insert(scope.owner_def)
        })
        .collect();
    // Publication also checks that generated module identities do not collide
    // with the resolver identities of already-lowered comptime parameters.
    let mut publication_fuel = missing
        .len()
        .saturating_mul(4)
        .saturating_add(package.staged_parameter_definitions.len())
        .saturating_add(1);
    let mut generated = hir::definitions::DefinitionBatch::new(&mut package);
    for scope in missing {
        let owner = generated
            .reserve()
            .expect("generated module owner identity exhausted");
        let item = arena.alloc_item(hir::Item {
            owner_id: owner,
            ident: hir::common::Ident::new(scope.name.unwrap(), rustc_span::DUMMY_SP),
            kind: hir::ItemKind::Mod(hir::item::ModDef { items: Vec::new() }),
            span: rustc_span::DUMMY_SP,
        });
        generated
            .define(
                hir::owner::OwnerInfo {
                    node: hir::owner::OwnerNode::Item(item),
                    nodes: hir::owner::OwnerNodes::new(),
                },
                Some(scope.owner_def),
                None,
            )
            .expect("generated module owner must belong to its publication batch");
    }
    generated
        .publish(&mut publication_fuel)
        .expect("resolved generated modules must have complete unique identities");
    for scope in &scopes {
        let owner = package.definition_owner(scope.owner_def).unwrap();
        let mut item = package.owner(owner).unwrap().node.expect_item().clone();
        if let hir::ItemKind::Mod(module) = &mut item.kind {
            for child in &scope.children {
                let child = input.module_tree.scope_tree.get(*child).unwrap();
                if matches!(child.kind, resolve::scope::ScopeKind::Module) {
                    let child_owner = package.definition_owner(child.owner_def).unwrap();
                    if !module.items.contains(&child_owner) {
                        module.items.push(child_owner);
                    }
                }
            }
        }
        package.insert_owner(
            owner,
            hir::owner::OwnerInfo {
                node: hir::owner::OwnerNode::Item(arena.alloc_item(item)),
                nodes: hir::owner::OwnerNodes::new(),
            },
        );
    }

    crate::test_templates::link(&mut package, &arena, diag_ctx);
    crate::function_templates::link(&mut package, &arena, diag_ctx);

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
