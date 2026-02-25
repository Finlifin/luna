use std::path::{Path, PathBuf};

use interface::{CompilerConfig, CompilerInstance, Session};
use parse::parser::Parser;
use std::mem;

fn main() {
    // ── Session ──────────────────────────────────────────────────────────
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = CompilerConfig::new("test", cwd);
    let sess = Session::new(config);

    // Report sysroot status.
    if let Some(ref sr) = sess.sysroot {
        println!("sysroot: {}", sr.root.display());
    } else {
        println!("warning: sysroot not found – builtin/std unavailable");
    }

    // ── Compiler instance ────────────────────────────────────────────────
    let mut instance = CompilerInstance::new(&sess);

    // Report sysroot packages loaded into VFS.
    for (i, vfs) in instance.sysroot_vfs.iter().enumerate() {
        println!(
            "  sysroot[{}] \"{}\" – {} source file(s)",
            i,
            vfs.name,
            vfs.file_count()
        );
    }

    // ── Load source file ─────────────────────────────────────────────────
    let file_path = Path::new("test.fl");
    let source_file = sess
        .source_map
        .load_file(file_path)
        .expect("failed to load test.fl");
    let file_id = instance
        .vfs_mut()
        .add_file(file_path.to_path_buf(), source_file.clone());

    // ── Lex & Parse ──────────────────────────────────────────────────────
    let src = source_file.src.as_ref().expect("source text not available");
    let (tokens, _lex_errors) = lex::lex(src, source_file.start_pos);

    let mut parser = Parser::new(&sess.source_map, tokens, source_file.start_pos);
    parser.parse(&instance.diag_ctx);
    let ast = parser.finalize();

    instance.vfs_mut().set_ast(file_id, ast);

    // ── AST dump ─────────────────────────────────────────────────────────
    let ast = instance.vfs.get_ast(file_id).expect("AST not found");
    let lisp = ast.dump_to_s_expression(ast.root, &sess.source_map);
    std::fs::write("ast.lisp", &lisp).expect("failed to write ast.lisp");
    println!("ast dumped to ast.lisp ({} nodes)", ast.nodes.len());

    // ── AST Lowering ─────────────────────────────────────────────────────
    let mut package = interface::hir::Package::new();
    ast_lowering::lower_to_hir(
        ast,
        &instance.hir_arena,
        &sess.source_map,
        &instance.diag_ctx,
        &mut package,
    );

    println!(
        "HIR lowering complete: {} definition(s), {} body(ies)",
        package.num_defs(),
        package.num_bodies(),
    );
    for (owner_id, info) in package.owners() {
        println!("  owner {:?}: {:?}", owner_id, info.node);
    {
        let owner_str = format!("{:?}", owner_id);
        let node_str = format!("{:#?}", info.node);
        println!(
            "  owner {:<6} size={:<3} align={:<2} node:\n{}",
            owner_str,
            mem::size_of_val(&info.node),
            mem::align_of_val(&info.node),
            node_str
        );
    }
    }
}
