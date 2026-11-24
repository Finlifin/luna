use middle::{
    Db, ElaboratedPackage, ElaborationQueryInput, HirPackageBox, HirQueryInput, LunaDatabase,
};
use std::collections::BTreeMap;
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

#[test]
fn midend_interface_signature_and_elaboration_queries_track_dependencies_and_cycles() {
    static INTERFACE_CALLS: AtomicUsize = AtomicUsize::new(0);
    fn hir_provider(db: &Db) -> Arc<HirPackageBox> {
        let index = db.hir_input().unwrap().file_id().raw();
        Arc::new(HirPackageBox::new(|arena| {
            let mut package = hir::Package::new();
            let root = package.alloc_owner_id();
            package.root_mod = root;
            let trait_owner = package.alloc_owner_id();
            let trait_item = arena.alloc_item(hir::Item {
                owner_id: trait_owner,
                ident: hir::common::Ident::new(
                    hir::common::Symbol::intern(&format!("Trait_{index}")),
                    rustc_span::DUMMY_SP,
                ),
                span: rustc_span::DUMMY_SP,
                kind: hir::ItemKind::Trait(hir::item::TraitDef {
                    comptime_only: false,
                    super_traits: None,
                    clause_params: &[],
                    clause_constraints: &[],
                    items: vec![],
                }),
            });
            package.insert_owner(
                trait_owner,
                hir::owner::OwnerInfo {
                    node: hir::owner::OwnerNode::Item(trait_item),
                    nodes: hir::owner::OwnerNodes::new(),
                },
            );
            let root_item = arena.alloc_item(hir::Item {
                owner_id: root,
                ident: hir::common::Ident::new(
                    hir::common::Symbol::intern(&format!("pkg_{index}")),
                    rustc_span::DUMMY_SP,
                ),
                span: rustc_span::DUMMY_SP,
                kind: hir::ItemKind::Mod(hir::item::ModDef {
                    items: vec![trait_owner],
                }),
            });
            package.insert_owner(
                root,
                hir::owner::OwnerInfo {
                    node: hir::owner::OwnerNode::Item(root_item),
                    nodes: hir::owner::OwnerNodes::new(),
                },
            );
            package
        }))
    }
    fn elab_provider(db: &Db) -> Arc<ElaboratedPackage> {
        INTERFACE_CALLS.fetch_add(1, Ordering::SeqCst);
        let trait_impls = db.package_trait_impls(());
        let config = db.elaboration_input().unwrap_or_default();
        Arc::new(ElaboratedPackage {
            succeeded: !config.package_identities.is_empty(),
            struct_count: 0,
            enum_count: 0,
            function_count: 0,
            trait_count: trait_impls.traits.len(),
            impl_count: trait_impls.impls.len(),
            warnings: vec![],
            errors: vec![],
            nir_lisp: trait_impls.to_string(),
        })
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
    db.providers.hir_package = hir_provider;
    db.providers.elaborated_package = elab_provider;
    db.set_hir_input(input(3));
    db.set_elaboration_input(Arc::new(ElaborationQueryInput::new(BTreeMap::from([(
        0, 99,
    )]))));

    let trait_owner = hir::OwnerId::new(hir::LocalDefId::new(1));
    let iface = db.package_interface(());
    assert_eq!(iface.len(), 2);
    assert!(iface.to_string().contains("(package-interface"));
    let sig = db.item_signature(trait_owner).expect("trait signature");
    assert_eq!(sig.name.as_str(), "Trait_3");
    assert!(sig.to_string().contains("(trait "));
    let elab = db.elaborated_package(());
    assert!(elab.succeeded);
    assert_eq!(elab.trait_count, 1);
    assert!(elab.to_string().contains("(elaborated-package"));

    // Replacing HIR invalidates interface, item_signature, trait_impls, and elaborated_package.
    db.set_hir_input(input(4));
    let sig2 = db
        .item_signature(trait_owner)
        .expect("updated trait signature");
    assert_eq!(sig2.name.as_str(), "Trait_4");
    let elab2 = db.elaborated_package(());
    assert!(!Arc::ptr_eq(&elab, &elab2));
    assert_eq!(INTERFACE_CALLS.load(Ordering::SeqCst), 2);

    // Cycle-safe `try_item_signature` reports QueryResult::Cycle without panicking.
    static SAW_CYCLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    fn cyclic_sig(db: &Db, owner: hir::OwnerId) -> Option<Arc<middle::ItemInterfaceSummary>> {
        match db.try_item_signature(owner) {
            query::QueryResult::Ok(v) => v,
            query::QueryResult::Cycle(_) => {
                SAW_CYCLE.store(true, Ordering::SeqCst);
                None
            }
            query::QueryResult::Poisoned => None,
        }
    }
    let mut cyclic_db = Db::new();
    cyclic_db.providers.item_signature = cyclic_sig;
    assert!(matches!(
        cyclic_db.try_item_signature(trait_owner),
        query::QueryResult::Ok(None)
    ));
    assert!(SAW_CYCLE.load(Ordering::SeqCst));
    // Actually force a cycle on `try_elaborated_package`:
    fn cyclic_elab(db: &Db) -> Arc<ElaboratedPackage> {
        match db.try_elaborated_package(()) {
            query::QueryResult::Ok(v) => v,
            query::QueryResult::Cycle(_) | query::QueryResult::Poisoned => {
                Arc::new(ElaboratedPackage {
                    succeeded: false,
                    struct_count: 0,
                    enum_count: 0,
                    function_count: 0,
                    trait_count: 0,
                    impl_count: 0,
                    warnings: vec![],
                    errors: vec!["cycle detected".into()],
                    nir_lisp: String::new(),
                })
            }
        }
    }
    cyclic_db.providers.elaborated_package = cyclic_elab;
    let cyc = cyclic_db.elaborated_package(());
    assert!(!cyc.succeeded);
    assert_eq!(cyc.errors, vec!["cycle detected"]);
}
