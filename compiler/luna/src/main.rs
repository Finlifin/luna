use std::path::{Path, PathBuf};

use interface::{CompilerConfig, CompilerInstance, Session};
use parse::parser::Parser;

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
    {
        let ast = instance.vfs.get_ast(file_id).expect("AST not found");
        let lisp = ast.dump_to_s_expression(ast.root, &sess.source_map);
        std::fs::write("ast.lisp", &lisp).expect("failed to write ast.lisp");
        println!("ast dumped to ast.lisp ({} nodes)", ast.nodes.len());
    }

    // ── Name Resolution ──────────────────────────────────────────────────
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
    let _resolver = resolve::Resolver::new(&module_tree);

    // ── AST Lowering ─────────────────────────────────────────────────────
    let ast = instance.vfs.get_ast(file_id).expect("AST not found");
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

    // ── Type Checking ────────────────────────────────────────────────────
    typeck::typeck_package(&package, &instance.ty_ctxt);

    // Report types.
    for (owner_id, _info) in package.owners() {
        if let Some(ty) = instance.ty_ctxt.def_ty(owner_id.def_id) {
            let item = package.item(owner_id).unwrap();
            println!("  type {:?} : {}", item.ident, ty);
        }
    }
    println!("type checking complete");

    // ── MIR Lowering ─────────────────────────────────────────────────────
    let mir_bodies = mir_build::build_mir(&package, &instance.ty_ctxt);
    println!("MIR lowering complete: {} body(ies)", mir_bodies.len(),);

    // Dump MIR for inspection.
    let mut mir_dump = String::new();
    for body in &mir_bodies {
        mir_dump.push_str(&body.dump());
        mir_dump.push('\n');
    }
    std::fs::write("mir.dump", &mir_dump).expect("failed to write mir.dump");
    println!("MIR dumped to mir.dump");

    // ── LLVM Codegen ───────────────────────────────────────────────────
    let codegen_result = codegen::codegen_llvm(&mir_bodies, &instance.ty_ctxt);

    // Dump LLVM IR for inspection.
    codegen_result
        .write_ir("output.ll")
        .expect("failed to write output.ll");
    println!("LLVM IR written to output.ll");

    // Emit object file and link with libc.
    match codegen_result.write_object("output.o") {
        Ok(()) => {
            println!("object file written to output.o");

            // Link with cc (uses C ABI / libc).
            let link_status = std::process::Command::new("cc")
                .args(["-o", "output", "output.o", "-lm"])
                .status();
            match link_status {
                Ok(status) if status.success() => {
                    println!("linked output.o -> output");
                    let run_result = std::process::Command::new("./output").output();
                    match run_result {
                        Ok(output) => {
                            let stdout = String::from_utf8_lossy(&output.stdout);
                            if !stdout.is_empty() {
                                print!("{}", stdout);
                            }
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            if !stderr.is_empty() {
                                eprint!("{}", stderr);
                            }
                        }
                        Err(e) => println!("failed to run ./output: {}", e),
                    }
                }
                Ok(status) => println!("linker failed with: {}", status),
                Err(e) => println!("cc not available: {} (skipping link)", e),
            }
        }
        Err(e) => println!("failed to emit object file: {}", e),
    }
}
