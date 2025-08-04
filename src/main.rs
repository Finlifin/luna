mod ast_lower;
mod basic;
mod comptime;
mod context;
mod diagnostic;
mod hir;
mod lex;
mod parse;
mod query;
mod typing;
mod vfs;

use std::path::Path;

use crate::{
    context::{scope::{ScopeManager, ScopeSExpressionVisitor}, CompilerContext},
    hir::Hir,
    parse::ast_visitor::dump_ast_to_s_expression,
    query::QueryEngine,
    vfs::{dump_vfs_to_s_expression, VfsScopeScanner},
};

fn main() {
    let hir = Hir::new();
    let mut context = CompilerContext::new(&hir.source_map);

    let vfs = vfs::Vfs::build_from_path(
        Path::new("test_project").to_path_buf(),
        &[
            ".git",
            ".cache",
            "target",
            ".vscode",
            ".idea",
            ".metals",
            ".scala-build",
        ],
        &hir.source_map,
    );

    let mut vfs_scanner = VfsScopeScanner::new();
    let _scan_result = vfs_scanner.scan_vfs(&vfs, &mut context, &hir);

    let lowering_context = ast_lower::LoweringContext::new(&mut context, &hir, &vfs);
    lowering_context.lower().expect("Failed to lower AST");
    let mut scope_visitor = ScopeSExpressionVisitor::new();
    let scope_s_expr = context.scope_manager.accept(&mut scope_visitor);
    std::fs::write("scope.lisp", &scope_s_expr).unwrap();
    let vfs_s_expr = dump_vfs_to_s_expression(&vfs, vfs.root);
    std::fs::write("vfs.lisp", &vfs_s_expr).unwrap();
    dbg!(hir.get(context.scope_manager.scope_hir_id(context.scope_manager.root).unwrap()).unwrap());


    // // Demonstrate the query system with dependency tracking
    // println!("\n=== Query System Demo with Dependency Tracking ===");
    // let mut query_engine = QueryEngine::new();

    // // Test HIR queries
    // println!("HIR for file 1: {:?}", query_engine.hir(1));
    // println!("HIR for file 2: {:?}", query_engine.hir(2));

    // // Test type queries (these depend on HIR)
    // println!("Type of node 42: {}", query_engine.type_of(42));
    // println!("Type of node 100: {}", query_engine.type_of(100));

    // // Test scope queries
    // println!("Scope 1 contents: {:?}", query_engine.scope(1));
    // println!("Scope 5 contents: {:?}", query_engine.scope(5));

    // // Demonstrate dependency tracking
    // println!("\n=== Dependency Tracking Demo ===");
    // println!("Cache size: {}", query_engine.cache_len());

    // // Show dependencies for type query
    // use crate::query::{ErasedQueryKey, QueryDesc, TypeQuery};
    // let type_query_key = ErasedQueryKey::new(&QueryDesc::new(TypeQuery { node_id: 42 }));
    // let deps = query_engine.get_dependency_info(&type_query_key);
    // println!("Dependencies for type query of node 42: {:?}", deps);

    // // Test invalidation
    // println!("\n=== Cache Invalidation Demo ===");
    // println!(
    //     "Cache size before invalidation: {}",
    //     query_engine.cache_len()
    // );

    // use crate::query::HirQuery;
    // let hir_query_key = ErasedQueryKey::new(&QueryDesc::new(HirQuery { file_id: 1 }));
    // query_engine.invalidate_query(&hir_query_key);

    // println!(
    //     "Cache size after HIR invalidation: {}",
    //     query_engine.cache_len()
    // );

    // // Re-run type query to see it gets recomputed
    // println!(
    //     "Re-running type query after HIR invalidation: {}",
    //     query_engine.type_of(42)
    // );
    // println!(
    //     "Cache size after recomputation: {}",
    //     query_engine.cache_len()
    // );

    // println!("Enhanced query system demo completed!");
}
