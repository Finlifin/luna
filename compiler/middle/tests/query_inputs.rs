use middle::{Db, HirPackageBox, HirQueryInput, LunaDatabase};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn owned_package(index: u32) -> HirPackageBox {
    HirPackageBox::new(|arena| {
        let mut package = hir::Package::new();
        let owner = package.alloc_owner_id();
        package.root_mod = owner;
        let item = arena.alloc_item(hir::Item {
            owner_id: owner,
            ident: hir::common::Ident::new(
                hir::common::Symbol::intern(&format!("input_{index}")),
                rustc_span::DUMMY_SP,
            ),
            span: rustc_span::DUMMY_SP,
            kind: hir::ItemKind::Mod(hir::item::ModDef { items: vec![] }),
        });
        package.insert_owner(
            owner,
            hir::owner::OwnerInfo {
                node: hir::owner::OwnerNode::Item(item),
                nodes: hir::owner::OwnerNodes::new(),
            },
        );
        package
    })
}

fn root_name(package: &HirPackageBox) -> String {
    let view = package.package();
    view.item(view.root_mod)
        .unwrap()
        .ident
        .name
        .as_str()
        .to_string()
}

#[test]
fn replacing_hir_input_recomputes_hir_and_observed_consumers_across_db_clones() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    fn provider(db: &Db) -> Arc<HirPackageBox> {
        CALLS.fetch_add(1, Ordering::SeqCst);
        Arc::new(owned_package(db.hir_input().unwrap().file_id().raw()))
    }
    let source_map = rustc_span::SourceMap::new(rustc_span::source_map::FilePathMapping::empty());
    let diagnostics = Arc::new(diagnostic::DiagnosticContext::new(&source_map));
    let input = |id| {
        Arc::new(HirQueryInput::new(
            Arc::new(ast::Ast::new(rustc_span::BytePos(0))),
            vfs::FileId::from_raw(id),
            Arc::new(resolve::ModuleTree {
                scope_tree: resolve::scope::ScopeTree::new(),
                def_names: Default::default(),
                def_count: 0,
                impls: vec![],
                errors: vec![],
                file_scopes: Default::default(),
            }),
            resolve::ScopeId::ROOT,
            &source_map,
            Arc::clone(&diagnostics),
        ))
    };
    let mut db = Db::new();
    db.providers.hir_package = provider;
    db.set_hir_input(input(1));
    let first = db.hir_package(());
    let clone = db.clone();
    assert!(Arc::ptr_eq(&first, &clone.hir_package(())));
    let consumers = query::QueryCache::new();
    let observed = || {
        query::force_get_with(
            &db.query_engine,
            &consumers,
            query::QueryDesc::new("hir_consumer"),
            (),
            |_| "()".into(),
            || {
                let _ = db.hir_package(());
                db.hir_input().unwrap().file_id().raw()
            },
        )
    };
    assert_eq!(observed(), 1);
    clone.set_hir_input(input(2));
    assert_eq!(observed(), 2);
    let second = db.hir_package(());
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(Arc::ptr_eq(&second, &clone.hir_package(())));
    assert_eq!(CALLS.load(Ordering::SeqCst), 2);
    // The earlier result remains owned after replacement.
    assert_eq!(root_name(&first), "input_1");
    assert_eq!(root_name(&second), "input_2");
}

#[test]
fn owned_hir_survives_the_database_and_all_query_inputs() {
    fn provider(db: &Db) -> Arc<HirPackageBox> {
        let input = db.hir_input().unwrap();
        assert!(!input.diag_ctx().has_errors());
        let _ = input.source_map();
        Arc::new(owned_package(input.file_id().raw()))
    }
    let package = {
        let source_map =
            rustc_span::SourceMap::new(rustc_span::source_map::FilePathMapping::empty());
        let diagnostics = Arc::new(diagnostic::DiagnosticContext::new(&source_map));
        let input = Arc::new(HirQueryInput::new(
            Arc::new(ast::Ast::new(rustc_span::BytePos(0))),
            vfs::FileId::from_raw(17),
            Arc::new(resolve::ModuleTree {
                scope_tree: resolve::scope::ScopeTree::new(),
                def_names: Default::default(),
                def_count: 0,
                impls: vec![],
                errors: vec![],
                file_scopes: Default::default(),
            }),
            resolve::ScopeId::ROOT,
            &source_map,
            diagnostics.clone(),
        ));
        let mut db = Db::new();
        db.providers.hir_package = provider;
        db.set_hir_input(input.clone());
        drop(input);
        drop(diagnostics);
        db.hir_package(())
    };
    assert_eq!(root_name(&package), "input_17");
    // Moving the owner does not move the arena allocation or its nodes.
    let moved = vec![package];
    assert_eq!(root_name(&moved[0]), "input_17");
    assert!(moved[0].package().to_string().contains("input_17"));
}
