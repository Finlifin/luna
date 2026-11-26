//! End-to-end tests for Phase 5 `@define_macro2` and Mode 2 PEG construction macros.
use comptime_interpretor::Value;
use luna::cli::{self, Command};
use luna::frontend;
use luna::packages::{self, SourcePackage};
use resolve::DependencyEdge;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{seq}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn library_dir() -> PathBuf {
    if let Ok(path) = std::env::var("LUNA_TEST_SYSROOT") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

fn sysroot_packages(
    mut user_packages: Vec<SourcePackage>,
    mut user_deps: Vec<DependencyEdge>,
) -> (
    Vec<SourcePackage>,
    Vec<DependencyEdge>,
    u32,
    elaboration::program::Protocols,
) {
    let lib = library_dir();
    let mut builtin_id = None;
    let mut next_id = user_packages
        .iter()
        .map(|p| p.resolution_id)
        .max()
        .unwrap_or(0)
        + 1;
    let mut next_identity = user_packages.iter().map(|p| p.identity).max().unwrap_or(0) + 1;

    let mut sysroot_ids = Vec::new();
    for name in ["builtin", "meta", "attrs"] {
        let entry = lib.join(name).join("lib.fl");
        if !entry.is_file() {
            continue;
        }
        let res_id = next_id;
        let ident = next_identity;
        next_id += 1;
        next_identity += 1;
        if name == "builtin" {
            builtin_id = Some(res_id);
        }
        sysroot_ids.push((name.to_string(), res_id, ident));
        user_packages.push(SourcePackage {
            resolution_id: res_id,
            identity: ident,
            name: name.to_string(),
            entry,
        });
    }

    for i in 0..user_packages.len() {
        let req_id = user_packages[i].resolution_id;
        for (name, prov_id, _) in &sysroot_ids {
            if req_id != *prov_id {
                user_deps.push(DependencyEdge {
                    requester: req_id,
                    alias: name.clone(),
                    provider: *prov_id,
                });
            }
        }
    }

    let mut protocols = elaboration::program::Protocols::default();
    protocols
        .builtin_implementation_packages
        .extend(sysroot_ids.iter().map(|(_, _, ident)| *ident));
    (
        user_packages,
        user_deps,
        builtin_id.expect("builtin sysroot package must exist"),
        protocols,
    )
}

type IntrinsicBindingVec = Vec<(nir::intrinsics::Operation, String)>;

fn load_intrinsic_selectors() -> (IntrinsicBindingVec, IntrinsicBindingVec) {
    let conf = fs::read_to_string(library_dir().join("language-items.conf"))
        .expect("read language-items.conf");
    let mut comptime = Vec::new();
    let mut host = Vec::new();
    for line in conf.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().to_string();
        if let Some(name) = key.strip_prefix("comptime-intrinsic.") {
            if !value.starts_with("std.") {
                if let Some(op) = nir::intrinsics::Operation::from_name(name) {
                    comptime.push((op, value));
                }
            }
        } else if let Some(name) = key.strip_prefix("host-intrinsic.") {
            if !value.starts_with("std.") {
                if let Some(op) = nir::intrinsics::Operation::from_name(name) {
                    host.push((op, value));
                }
            }
        }
    }
    (comptime, host)
}

fn sysroot_selectors<'a>(
    comptime_intrinsics: &'a [(nir::intrinsics::Operation, String)],
    host_intrinsics: &'a [(nir::intrinsics::Operation, String)],
) -> luna::protocols::type_constructors::Selectors<'a> {
    luna::protocols::type_constructors::Selectors {
        comptime_intrinsics,
        host_intrinsics,
        copy: Some("builtin.protocols.resources.Copy"),
        drop: Some("builtin.protocols.resources.Drop"),
        splice: Some("builtin.protocols.staging.Splice"),
        meta_list: Some("meta.List"),
        tuple: Some("builtin.types.tuple.Tuple"),
        index: Some("builtin.protocols.ops.Index"),
        ordering: Some("builtin.numeric.comparison.Ord"),
        equality: Some("builtin.numeric.comparison.Eq"),
        iterator: Some("builtin.protocols.iterator.Iterator"),
        into_iterator: Some("builtin.protocols.iterator.IntoIterator"),
        list_pattern: Some("builtin.protocols.patterns.ListPatternView"),
        comptime_list_pattern: Some("meta.data.patterns.ComptimeListPatternView"),
        arithmetic: [
            Some("builtin.protocols.ops.Add"),
            Some("builtin.protocols.ops.Sub"),
            Some("builtin.protocols.ops.Mul"),
            Some("builtin.protocols.ops.Div"),
            Some("builtin.protocols.ops.Rem"),
            Some("builtin.protocols.ops.Neg"),
            Some("builtin.protocols.ops.Not"),
        ],
        ..Default::default()
    }
}

fn expect_err<T>(res: Result<T, String>) -> String {
    match res {
        Ok(_) => panic!("expected Err, got Ok"),
        Err(e) => e,
    }
}

fn compile_sysroot_project(entry: &Path) -> Result<luna::frontend::Artifact, String> {
    let Command::Check(options) = cli::parse([
        "check".into(),
        entry.as_os_str().to_os_string(),
        "--sysroot".into(),
        library_dir().into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("expected check options")
    };
    frontend::compile(&options)
}

fn invoke_sysroot_project(entry: &Path, fn_name: &str) -> Result<Value, String> {
    let Command::Check(options) = cli::parse([
        "check".into(),
        entry.as_os_str().to_os_string(),
        "--sysroot".into(),
        library_dir().into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("expected check options")
    };
    frontend::invoke_comptime(&options, fn_name, 30_000_000, None, None)
}

#[test]
fn official_statements_define_macro2_works_in_comptime_and_native() {
    let dir = unique_temp_dir("luna-macro2-statements");
    fs::create_dir_all(dir.join("scripts")).unwrap();

    // Official `statements` example from docs/macro/construction-macro.md
    fs::write(
        dir.join("scripts/macros.fl"),
        r#"
@use meta.macros.*;
use meta.ast.*;

const statements = @define_macro2 {
    Entry -> FlurryExtendedStatement*;

    fn(body: Repetition<meta.ast.FlurryExtendedStatement>) -> TokenBuffer {
        {{ $body }}
    }
}
"#,
    )
    .unwrap();

    fs::write(
        dir.join("main.fl"),
        r#"
@use scripts.macros.statements;
use std.prelude.*;

comptime fn check_comptime() -> bool {
    @statements {}
    @statements {
        let base: Integer = 20;
        pure comptime fn add_one(x: Integer) -> Integer { x + 1 }
        let out = add_one(base);
    }
    out == 21
}

fn print_lines() -> !std.io.IoErr void {
    @statements {}
    @statements {
        println("first")!;
        println("second")!;
    }
}

fn run() -> bool {
    print_lines() match {
        _! => true,
        error _ => false,
    }
}
"#,
    )
    .unwrap();

    let comptime_val = invoke_sysroot_project(&dir.join("main.fl"), "check_comptime").unwrap();
    assert_eq!(comptime_val, Value::Bool(true));

    let build = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&dir)
        .args([
            "build",
            "main.fl",
            "--entry",
            "run",
            "--output",
            "program",
            "--opt-level",
            "0",
            "--sysroot",
        ])
        .arg(library_dir())
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let run = ProcessCommand::new(dir.join("program"))
        .current_dir(&dir)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&dir);
    assert!(run.status.success(), "{run:?}");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(stdout, "first\nsecond\n(bool true)\n");
}

#[test]
fn custom_dsl_peg_node_ordered_choice_and_flurry_projections_work() {
    let dir = unique_temp_dir("luna-macro2-dsl");
    fs::create_dir_all(dir.join("peg_dep")).unwrap();
    fs::create_dir_all(dir.join("app_pkg")).unwrap();

    fs::write(
        dir.join("peg_dep/lib.fl"),
        r#"
@use meta.macros.*;
use meta.ast.{FlurryExpr, PegNode, TokenBuffer};
use meta.ComptimeTextBytes;

pub const script_dsl = @define_macro2 {
    Entry -> Step*;
    Step -> ("inc" ~ FlurryIdent ~ ";") | ("set" ~ FlurryIdent ~ "=" ~ FlurryExpr ~ ";");

    fn(tree: PegNode) -> TokenBuffer {
        let root_ok = tree.rule() == .Entry and tree.text().as(ComptimeTextBytes).contains(";");
        var acc = {{ let dsl_meta_ok: bool = $root_ok; }};
        let steps = tree.children();
        var i: Integer = 0;
        while i < steps.length() {
            let step = steps[i];
            let step_ok = step.rule() == .Step;
            let parts = step.children();
            let stmt_buf = if step_ok and parts.length() == 3 {
                let target = parts[1];
                {{ $target = $target + 1; }}
            } else if step_ok and parts.length() == 5 {
                let target = parts[1];
                let expr_val = parts[3];
                {{ $target = $expr_val; }}
            } else {
                {{}}
            };
            acc = acc.concat(stmt_buf);
            i = i + 1;
        }
        acc
    }
};

pub const opt_expr = @define_macro2 {
    Entry -> FlurryExpr?;

    fn(expr: ?FlurryExpr) -> TokenBuffer {
        expr match {
            e? => {{ ($e) + 1 }},
            null => {{ 0 }},
        }
    }
};

pub const single_expr = @define_macro2 {
    Entry -> FlurryExpr;

    fn(expr: FlurryExpr) -> TokenBuffer {
        {{ ($expr) * 2 }}
    }
};

private const secret_peg = @define_macro2 {
    Entry -> FlurryExpr;

    fn(e: FlurryExpr) -> TokenBuffer {
        {{ $e }}
    }
};
"#,
    )
    .unwrap();

    fs::write(
        dir.join("app_pkg/main.fl"),
        r#"
@use dep_peg.{script_dsl, opt_expr, single_expr};

comptime fn verify_dsl() -> bool {
    var count: Integer = 10;
    @script_dsl {
        set count = count + 5;
        inc count;
        inc count;
    }
    let o0 = @opt_expr {};
    let o1 = @opt_expr { 41 };
    let dbl = @single_expr { 21 };
    dsl_meta_ok and count == 17 and o0 == 0 and o1 == 42 and dbl == 42
}
"#,
    )
    .unwrap();

    let (pkgs, deps, builtin_id, base_protocols) = sysroot_packages(
        vec![
            SourcePackage {
                resolution_id: 0,
                identity: 100,
                name: "app_pkg".into(),
                entry: dir.join("app_pkg/main.fl"),
            },
            SourcePackage {
                resolution_id: 1,
                identity: 101,
                name: "peg_dep".into(),
                entry: dir.join("peg_dep/lib.fl"),
            },
        ],
        vec![DependencyEdge {
            requester: 0,
            alias: "dep_peg".into(),
            provider: 1,
        }],
    );
    let (ci, hi) = load_intrinsic_selectors();
    let sel = sysroot_selectors(&ci, &hi);
    let target = Some(nir::target::TargetData::new(64).unwrap());

    let artifact = packages::compile_with_source_protocols_for_target(
        &pkgs,
        &deps,
        0,
        Some(builtin_id),
        &base_protocols,
        sel,
        target,
    )
    .unwrap();
    let luna::frontend::Artifact::Nir(nir) = artifact else {
        panic!("expected NIR artifact");
    };
    let val = luna::invocation::invoke(&nir, "verify_dsl", 10_000_000).unwrap();
    assert_eq!(val, Value::Bool(true));

    // Verify private Mode 2 dependency macro triggers `[4010]` visibility error.
    fs::write(
        dir.join("app_pkg/main.fl"),
        "@use dep_peg.secret_peg;\nconst X: Integer = @secret_peg { 1 };",
    )
    .unwrap();
    let vis_err = expect_err(packages::compile_with_source_protocols_for_target(
        &pkgs,
        &deps,
        0,
        Some(builtin_id),
        &base_protocols,
        sysroot_selectors(&ci, &hi),
        target,
    ));
    assert!(vis_err.contains("macro expansion"), "{vis_err}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn peg_macro_grammar_signature_delimiter_and_input_errors_are_diagnosed() {
    // 1. Missing Entry rule -> [4013]
    // 2. Unguarded left recursion -> [4013]
    // 3. Parameter mismatch with Entry rule (`Entry -> FlurryExpr*;` vs `Repetition<FlurryPattern>`)
    for (idx, bad_script) in [
        r#"@use meta.macros.*; const m = @define_macro2 { Step -> FlurryIdent; fn(n: meta.ast.PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> Entry | "x"; fn(n: meta.ast.PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> FlurryExpr*; fn(ps: Repetition<meta.ast.FlurryPattern>) -> TokenBuffer { {{ 1 }} } };"#,
    ]
    .iter()
    .enumerate()
    {
        let dir = unique_temp_dir("luna-macro2-badgrammar");
        fs::create_dir_all(dir.join("scripts")).unwrap();
        fs::write(dir.join("scripts/macros.fl"), bad_script).unwrap();
        fs::write(
            dir.join("main.fl"),
            "@use scripts.macros.*; fn run() -> i32 { 0 }",
        )
        .unwrap();
        let err = expect_err(compile_sysroot_project(&dir.join("main.fl")));
        assert!(!err.is_empty(), "bad PEG def #{idx} unexpectedly succeeded");
        let _ = fs::remove_dir_all(&dir);
    }

    // 4. Runtime invocation diagnostics: wrong delimiter [4011], unmatched input [4014], trailing tokens [4014]
    let dir = unique_temp_dir("luna-macro2-callerrs");
    fs::create_dir_all(dir.join("scripts")).unwrap();
    fs::write(
        dir.join("scripts/macros.fl"),
        r#"
@use meta.macros.*;
use meta.ast.PegNode;

const cmd = @define_macro2 {
    Entry -> "inc" ~ FlurryIdent ~ ";";
    fn(n: PegNode) -> TokenBuffer { {{ 0 }} }
};
"#,
    )
    .unwrap();

    for bad_call in [
        "@use scripts.macros.cmd;\nconst X = @cmd(inc x;);",
        "@use scripts.macros.cmd;\nconst X = @cmd { set x; };",
        "@use scripts.macros.cmd;\nconst X = @cmd { inc x; trailing };",
    ] {
        fs::write(dir.join("main.fl"), bad_call).unwrap();
        let err = expect_err(compile_sysroot_project(&dir.join("main.fl")));
        assert!(err.contains("macro expansion"), "{bad_call}: {err}");
    }

    let _ = fs::remove_dir_all(&dir);
}
