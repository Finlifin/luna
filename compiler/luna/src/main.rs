//! Luna compiler driver.
//!
//! Orchestrates the compilation pipeline:
//!
//! ```text
//!   source file
//!     → lex  → parse  → resolve          (standalone phases)
//!     → hir_package query                 (dispatches ast_lowering provider)
//!     → [typeck / mir / codegen — TODO]
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use interface::{CompilerConfig, CompilerInstance, Session};
use parse::parser::Parser;

fn main() {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = CompilerConfig::new("test", cwd);
    let sess = Session::new(config);

    // ── Sysroot ──────────────────────────────────────────────────────────────
    if let Some(ref sr) = sess.sysroot {
        println!("sysroot: {}", sr.root.display());
    } else {
        println!("warning: sysroot not found – builtin/std unavailable");
    }

    let mut instance = CompilerInstance::new(&sess);

    for (i, vfs) in instance.sysroot_vfs.iter().enumerate() {
        println!(
            "  sysroot[{}] \"{}\" – {} source file(s)",
            i,
            vfs.name,
            vfs.file_count()
        );
    }

    // ── Load & lex ──────────────────────────────────────────────────────────
    let file_path = Path::new("test.fl");
    let source_file = sess
        .source_map
        .load_file(file_path)
        .expect("failed to load test.fl");
    let file_id = instance
        .vfs_mut()
        .add_file(file_path.to_path_buf(), source_file.clone());

    let src = source_file.src.as_ref().expect("source text not available");
    let (tokens, symbols, _lex_errors) = lex::lex(src, source_file.start_pos);

    // ── Parse ────────────────────────────────────────────────────────────────
    let mut parser = Parser::new(&sess.source_map, tokens, symbols, source_file.start_pos);
    parser.parse(&instance.diag_ctx);
    let ast = parser.finalize();
    instance.vfs_mut().set_ast(file_id, ast);

    {
        let ast = instance.vfs.get_ast(file_id).expect("AST not found");
        let lisp = ast.dump_to_s_expression(ast.root, &sess.source_map);
        std::fs::write("ast.lisp", &lisp).expect("failed to write ast.lisp");
        println!("ast dumped to ast.lisp ({} nodes)", ast.nodes.len());
    }

    // ── Name resolution ──────────────────────────────────────────────────────
    let module_tree =
        resolve::build_module_tree(&sess.source_map, &instance.diag_ctx, &mut instance.vfs);
    if !module_tree.errors.is_empty() {
        println!("resolve: {} error(s)", module_tree.errors.len());
        for err in &module_tree.errors {
            println!("  {}", err.message());
        }
    }
    println!(
        "name resolution complete: {} def(s), {} scope(s)",
        module_tree.def_count,
        module_tree.scope_tree.len(),
    );

    let file_scope = module_tree
        .file_scopes
        .get(&file_id)
        .copied()
        .unwrap_or(resolve::ScopeId::ROOT);

    // ── Register providers ────────────────────────────────────────────────────
    //
    // Each compiler-pass crate registers its function-pointer implementations
    // into the Providers dispatch table.  Must happen before any HIR queries.
    ast_lowering::set_providers(&mut instance.db.providers);

    // ── Set query inputs ──────────────────────────────────────────────────────
    //
    // Bundle all data needed by ast_lowering::lower_to_hir into HirQueryInput.
    // SourceMap and DiagnosticContext are borrowed by raw pointer; they live at
    // least as long as the `hir_package()` call below.
    {
        let hir_input = Arc::new(middle::HirQueryInput::new(
            Arc::new(
                instance
                    .vfs
                    .get_ast(file_id)
                    .expect("AST not found")
                    .clone(),
            ),
            Arc::new(module_tree),
            file_scope,
            &sess.source_map as *const rustc_span::SourceMap,
            &instance.diag_ctx as *const diagnostic::DiagnosticContext<'_>
                as *const diagnostic::DiagnosticContext<'static>,
        ));
        instance.set_hir_input(hir_input);
    }

    // ── Issue hir_package query ───────────────────────────────────────────────
    let pkg_box = instance.enter(|compiler| compiler.hir_package());

    let pkg = pkg_box.package();
    println!(
        "hir_package query complete: {} definition(s), {} body(ies)",
        pkg.num_defs(),
        pkg.num_bodies(),
    );

    // ── HIR serialization ─────────────────────────────────────────────────────
    let hir_lisp = pkg.dump_to_lisp();
    std::fs::write("hir.lisp", &hir_lisp).expect("failed to write hir.lisp");
    println!("HIR dumped to hir.lisp");
    println!("{}", hir_lisp);
}
