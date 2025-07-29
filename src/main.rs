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

use std::path::{Path, PathBuf};

use diagnostic::{DiagnosticContext, FlurryError};
use rustc_span::source_map;

use crate::{
    context::{CompilerContext, scan::scan, scope::ScopeSExpressionVisitor},
    hir::Hir,
    lex::lex,
    parse::{ast_visitor::dump_ast_to_s_expression, parser::Parser},
    vfs::{Vfs, VfsScopeScanner},
};

fn main() {
    let hir = Hir::new();
    let mut context = CompilerContext::new(&hir.source_map);

    // // 从命令行参数获取文件名，如果没有则使用默认文件
    // let args: Vec<String> = std::env::args().collect();
    // let filename = if args.len() > 1 { &args[1] } else { "test.fl" };
    // // 加载测试文件
    // let source_file = match hir.source_map.load_file(&Path::new(filename)) {
    //     Ok(file) => file,
    //     Err(_) => panic!("MAN!"),
    // };

    // let (tokens, errors) = lex(&source_file.src.as_ref().unwrap(), source_file.start_pos);
    // for err in errors {
    //     err.emit(context.diag_ctx(), source_file.start_pos);
    // }

    // let mut parser = Parser::new(&hir.source_map, tokens);
    // parser.parse(&mut context.diag_ctx());
    // let ast = parser.ast;

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

    let mut scope_visitor = ScopeSExpressionVisitor::new();
    let scope_s_expr = context.scope_manager.accept(&mut scope_visitor);
    std::fs::write("scope.lisp", &scope_s_expr).unwrap();

    // let visitor_s_expr = dump_ast_to_s_expression(&ast, ast.root, &hir.source_map);
    // std::fs::write("ast.lisp", visitor_s_expr).unwrap();
}

// fn main() {
//     let source_map = create_source_map();
//     // Test the new build_from_path functionality
//     println!("\n=== Testing build_from_path ===");
//     let vfs_from_path = vfs::Vfs::build_from_path(
//         Path::new(".").to_path_buf(),
//         &[
//             ".git",
//             ".cache",
//             "target",
//             "node_modules",
//             ".vscode",
//             ".idea",
//             ".metals",
//             ".scala-build",
//         ],
//         &source_map,
//     );
//     println!(
//         "VFS built from current directory with {} nodes",
//         vfs_from_path.nodes.len()
//     );

//     // Test VFS visitor - dump to S-expression
//     println!("\n=== Testing VFS S-expression dump ===");
//     let s_expr = vfs::dump_vfs_to_s_expression(&vfs_from_path, vfs_from_path.root);
//     println!("VFS S-expression:\n{}", s_expr);

//     // Test VFS visitor - count nodes
//     println!("\n=== Testing VFS node counting ===");
//     let (file_count, dir_count, total_count) =
//         vfs::count_vfs_nodes(&vfs_from_path, vfs_from_path.root);
//     println!(
//         "Files: {}, Directories: {}, Total: {}",
//         file_count, dir_count, total_count
//     );
// }
