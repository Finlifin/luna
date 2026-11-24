//! Integration tests for `middle` / `interface` mid-end incremental queries
//! and hardened `open_functions` diagnostics/spans.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use interface::{CompilerConfig, CompilerInstance, Session};
use middle::{ElaboratedPackage, ElaborationQueryInput, LunaDatabase};

#[path = "support/execution.rs"]
mod execution_support;

fn lower_and_query_source<'sess>(
    instance: &mut CompilerInstance<'sess>,
    sess: &'sess Session,
    filename: &str,
    source: &str,
) {
    let source_file = sess
        .source_map
        .new_source_file(rustc_span::FileName::Custom(filename.into()), source.into());
    let file_id = instance
        .vfs_mut()
        .add_file(PathBuf::from(filename), source_file.clone());
    instance.vfs_mut().selected_root = Some(file_id);
    let mut budget = macro_expander::MacroBudget::default();
    let (expanded, lex_errors) =
        macro_expander::expand_source_file(&sess.source_map, &source_file, &mut budget)
            .expect("macro expansion");
    assert!(lex_errors.is_empty(), "{lex_errors:?}");
    let mut parser = parse::parser::Parser::new(
        &sess.source_map,
        expanded.tokens,
        expanded.symbols,
        expanded.start_pos,
    );
    parser.parse(&instance.diag_ctx);
    assert!(!instance.diag_ctx.has_errors());
    let ast = parser.finalize();
    instance.vfs_mut().set_ast(file_id, ast);
    let module_tree =
        resolve::build_module_tree(&sess.source_map, &instance.diag_ctx, &mut instance.vfs);
    assert!(module_tree.errors.is_empty(), "{:?}", module_tree.errors);
    let file_scope = module_tree.file_scopes[&file_id];
    let ast = instance.vfs.get_ast(file_id).unwrap().clone();
    let mut input = middle::HirQueryInput::new(
        Arc::new(ast),
        file_id,
        Arc::new(module_tree),
        file_scope,
        &sess.source_map,
        Arc::clone(&instance.diag_ctx),
    );
    for (id, _) in instance.vfs.files() {
        if id == file_id {
            continue;
        }
        let extra_ast = instance.vfs.get_ast(id).unwrap().clone();
        let scope = input.module_tree.file_scopes[&id];
        input
            .additional_files
            .push((id, Arc::new(extra_ast), scope));
    }
    instance.set_hir_input(Arc::new(input));
}

#[test]
fn interface_midend_queries_elaborate_and_invalidate_real_flurry_sources() {
    static ELAB_COUNT: AtomicUsize = AtomicUsize::new(0);
    fn elaborate_provider(db: &middle::Db<'_>) -> Arc<ElaboratedPackage> {
        ELAB_COUNT.fetch_add(1, Ordering::SeqCst);
        let hir = db.hir_package(());
        let _iface = db.package_interface(());
        let config = db.elaboration_input().unwrap_or_default();
        match elaboration::program::elaborate_packages(hir.package(), &config.package_identities) {
            Ok(nir) => Arc::new(ElaboratedPackage {
                succeeded: true,
                struct_count: nir.structs.len(),
                enum_count: nir.enums.len(),
                function_count: nir.functions.len(),
                trait_count: nir.traits.len(),
                impl_count: nir.implementations.len(),
                warnings: vec![],
                errors: vec![],
                nir_lisp: nir.to_string(),
            }),
            Err(err) => Arc::new(ElaboratedPackage {
                succeeded: false,
                struct_count: 0,
                enum_count: 0,
                function_count: 0,
                trait_count: 0,
                impl_count: 0,
                warnings: vec![],
                errors: vec![err.to_string()],
                nir_lisp: String::new(),
            }),
        }
    }

    let sess = Session::new(CompilerConfig::new("midend-demo", PathBuf::from(".")));
    let mut instance = CompilerInstance::new(&sess);
    ast_lowering::set_providers(&mut instance.db.providers);
    instance.db.providers.elaborated_package = elaborate_provider;
    instance.set_elaboration_input(Arc::new(ElaborationQueryInput::new(BTreeMap::from([(
        0, 1,
    )]))));

    lower_and_query_source(
        &mut instance,
        &sess,
        "first.fl",
        "typealias FlagBit = .bool'builtin;
        trait Flag { fn flag(self) -> FlagBit; }
        struct Boxed { value: FlagBit }
        impl Flag for Boxed { fn flag(self) -> FlagBit { self.value } }
        fn entry() -> FlagBit { true }",
    );

    let (iface1, elab1) = instance.enter(|compiler| {
        let iface = compiler.package_interface();
        let trait_impls = compiler.package_trait_impls();
        assert_eq!(trait_impls.traits.len(), 1);
        assert_eq!(trait_impls.impls.len(), 1);
        let trait_owner = trait_impls.traits[0];
        let sig = compiler
            .item_signature(trait_owner)
            .expect("trait item signature");
        assert_eq!(sig.name.as_str(), "Flag");
        assert!(matches!(
            sig.kind,
            middle::ItemKindSummary::Trait { item_count: 1, .. }
        ));
        let elab = compiler.elaborated_package();
        assert!(elab.succeeded, "{:?}", elab.errors);
        assert_eq!(elab.struct_count, 1);
        assert_eq!(elab.trait_count, 1);
        assert_eq!(elab.impl_count, 1);
        assert!(Arc::ptr_eq(&iface, &compiler.package_interface()));
        assert!(Arc::ptr_eq(&elab, &compiler.elaborated_package()));
        (iface, elab)
    });
    assert_eq!(ELAB_COUNT.load(Ordering::SeqCst), 1);

    let mut second_instance = CompilerInstance::new(&sess);
    ast_lowering::set_providers(&mut second_instance.db.providers);
    second_instance.db = instance.db.clone();
    lower_and_query_source(
        &mut second_instance,
        &sess,
        "second.fl",
        "typealias FlagBit = .bool'builtin;
        fn only() -> FlagBit { false }",
    );
    second_instance.enter(|compiler| {
        let iface2 = compiler.package_interface();
        assert!(!Arc::ptr_eq(&iface1, &iface2));
        let trait_impls2 = compiler.package_trait_impls();
        assert!(trait_impls2.traits.is_empty());
        assert!(trait_impls2.impls.is_empty());
        let elab2 = compiler.elaborated_package();
        assert!(!Arc::ptr_eq(&elab1, &elab2));
        assert!(elab2.succeeded, "{:?}", elab2.errors);
        assert_eq!(elab2.struct_count, 0);
        assert_eq!(elab2.trait_count, 0);
    });
    assert_eq!(ELAB_COUNT.load(Ordering::SeqCst), 2);
}

#[test]
fn open_functions_emit_structured_diagnostics_with_real_spans_on_invalid_calls() {
    let source = "typealias Type = .Type'builtin;
        comptime fn Pick(flag: bool) -> Type { if flag { bool } else { .u8'builtin } }
        fn choose(comptime flag: bool, value: defer Pick(flag)) -> defer Pick(flag) { value }
        fn bad() -> bool { choose(true, 1) }";
    let err = execution_support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        !err.contains("0..0:"),
        "expected non-dummy source span in diagnostic: {err}"
    );
}
