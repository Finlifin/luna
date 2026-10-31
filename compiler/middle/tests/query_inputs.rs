use middle::{Db, HirPackageBox, HirQueryInput, LunaDatabase};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn replacing_hir_input_recomputes_hir_and_observed_consumers_across_db_clones() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    fn provider(db: &Db) -> Arc<HirPackageBox> {
        assert!(db.hir_input().is_some());
        CALLS.fetch_add(1, Ordering::SeqCst);
        Arc::new(HirPackageBox::new(
            hir::HirArena::new(),
            hir::Package::new(),
        ))
    }
    let source_map = rustc_span::SourceMap::new(rustc_span::source_map::FilePathMapping::empty());
    let diagnostics = diagnostic::DiagnosticContext::new(&source_map);
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
            &diagnostics as *const diagnostic::DiagnosticContext<'_>
                as *const diagnostic::DiagnosticContext<'static>,
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
    let _ = first.package();
}
