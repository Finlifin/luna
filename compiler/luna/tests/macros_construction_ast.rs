//! End-to-end tests for Phase 4 `@define_macro1` and Mode 1 AST construction macros.
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
fn official_alternatives_define_macro1_works_in_comptime_and_native() {
    let dir = unique_temp_dir("luna-macro1-alternatives");
    fs::create_dir_all(dir.join("scripts")).unwrap();

    // Official `alternatives` example from docs/macro/construction-macro.md
    fs::write(
        dir.join("scripts/macros.fl"),
        r#"
@use meta.macros.*;

const alternatives = @define_macro1 {
    fn(...patterns: Repetition<meta.ast.FlurryPattern>) -> TokenBuffer {
        let combined_pattern = patterns.fold({{ (not _) }}, |acc, pattern_ast| {
            {{ $acc or $pattern_ast }}
        });

        {{ $combined_pattern => }}
    }
}
"#,
    )
    .unwrap();

    fs::write(
        dir.join("main.fl"),
        r#"
@use scripts.macros.alternatives;
use std.prelude.*;

pure comptime fn classify_comptime(answer: str) -> Integer {
    if answer is do {
        @alternatives("yes", "Y") 1,
        @alternatives("no", "N",) 2,
        @alternatives() 99,
        _ => 3,
    }
}

comptime fn check_comptime() -> bool {
    classify_comptime("yes") == 1
        and classify_comptime("Y") == 1
        and classify_comptime("no") == 2
        and classify_comptime("N") == 2
        and classify_comptime("maybe") == 3
}

fn classify_runtime(answer: str) -> i32 {
    if answer is do {
        @alternatives("yes", "Y") 1,
        @alternatives("no", "N",) 2,
        @alternatives() 99,
        _ => 3,
    }
}

fn run() -> bool {
    let a = classify_runtime("Y");
    let b = classify_runtime("no");
    let c = classify_runtime("other");
    println("{},{},{}", a, b, c) match {
        _! => a == 1 and b == 2 and c == 3,
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
    assert_eq!(stdout, "1,2,3\n(bool true)\n");
}

#[test]
fn all_ten_flurry_ast_node_kinds_optional_and_multi_param_work() {
    let dir = unique_temp_dir("luna-macro1-all-kinds");
    fs::create_dir_all(dir.join("scripts")).unwrap();

    fs::write(
        dir.join("scripts/macros.fl"),
        r#"
@use meta.macros.*;
use meta.ComptimeTextBytes;

const emit_default_answer = @define_macro1 {
    fn() -> TokenBuffer {
        {{ 7 }}
    }
}

const wrap_default = @define_macro1 {
    fn() -> TokenBuffer {
        {{ @emit_default_answer() + 3 }}
    }
};

const add_optional = @define_macro1 {
    fn(base: FlurryExpr, extra: ?FlurryExpr) -> TokenBuffer {
        extra match {
            ext? => {{ ($base) + ($ext) }},
            null => {{ $base }},
        }
    }
};

const make_binding = @define_macro1 {
    fn(name: FlurryIdent, ty: FlurryType, val: FlurryLiteral) -> TokenBuffer {
        {{ let $name: $ty = $val; }}
    }
};

const match_pat_block = @define_macro1 {
    fn(target: FlurryExpr, pat: FlurryPattern, body: FlurryBlock) -> TokenBuffer {
        {{
            $target match {
                $pat => $body,
                _ => 0,
            }
        }}
    }
};

const inspect_item_stmt_ext_path = @define_macro1 {
    fn(
        item_ast: FlurryItem,
        stmt_ast: FlurryStatement,
        ext_ast: FlurryExtendedStatement,
        path_ast: FlurryPath,
    ) -> TokenBuffer {
        let item_ok = item_ast.syntax_node() match {
            n? => n.kind == .struct_def,
            null => false,
        };
        let child_ok = item_ast.syntax_tree().children_of(0).length() >= 1;
        let stmt_ok = stmt_ast.text().as(ComptimeTextBytes).contains("let");
        let ext_ok = ext_ast.text().as(ComptimeTextBytes).contains("fn");
        let path_ok = path_ast.text().as(ComptimeTextBytes).contains(".");
        let all_ok = item_ok and child_ok and stmt_ok and ext_ok and path_ok;
        {{
            $item_ast
            $ext_ast
            const INSPECT_ALL_OK: bool = $all_ok;
        }}
    }
};
"#,
    )
    .unwrap();

    fs::write(
        dir.join("main.fl"),
        r#"
@use scripts.macros.*;

@inspect_item_stmt_ext_path(
    struct Boxed { value: Integer },
    let dummy = 1;,
    comptime fn twice(x: Integer) -> Integer { x * 2 },
    std.io.Reader
)

comptime fn verify_all() -> bool {
    @make_binding(base_lit, Integer, 10)
    let sum_none = @add_optional(base_lit);
    let sum_some = @add_optional(base_lit, 5);
    let matched = @match_pat_block(
        sum_some,
        15 or 16,
        {
            7 + 3
        }
    );
    let wrapped = @wrap_default();
    let base_answer = @emit_default_answer();
    let boxed = Boxed { value: twice(matched) };
    INSPECT_ALL_OK
        and sum_none == 10
        and sum_some == 15
        and boxed.value == 20
        and wrapped == 10
        and base_answer == 7
}
"#,
    )
    .unwrap();

    let val = invoke_sysroot_project(&dir.join("main.fl"), "verify_all").unwrap();
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(val, Value::Bool(true));
}

#[test]
fn ast_macro_dependency_visibility_delimiters_and_parse_errors_are_diagnosed() {
    let dir = unique_temp_dir("luna-macro1-diag");
    fs::create_dir_all(dir.join("dep_pkg")).unwrap();
    fs::create_dir_all(dir.join("app_pkg/scripts")).unwrap();

    fs::write(
        dir.join("dep_pkg/lib.fl"),
        r#"
@use meta.macros.*;

pub const add_two = @define_macro1 {
    fn(a: FlurryExpr, b: FlurryExpr) -> TokenBuffer {
        {{ ($a) + ($b) }}
    }
};

pub const only_id = @define_macro1 {
    fn(id: FlurryIdent) -> TokenBuffer {
        {{ $id }}
    }
};

private const secret_ast = @define_macro1 {
    fn() -> TokenBuffer {
        {{ 1 }}
    }
};
"#,
    )
    .unwrap();

    fs::write(
        dir.join("app_pkg/main.fl"),
        r#"
@use dep_alias.add_two;

comptime fn check_dep() -> Integer {
    @add_two(19, 23)
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
                name: "dep_pkg".into(),
                entry: dir.join("dep_pkg/lib.fl"),
            },
        ],
        vec![DependencyEdge {
            requester: 0,
            alias: "dep_alias".into(),
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
    let val = luna::invocation::invoke(&nir, "check_dep", 10_000_000).unwrap();
    assert_eq!(val.to_string(), "(integer (sign positive) (hex \"2a\"))");

    // 1. Private Mode 1 dependency macro -> visibility error [4010]
    fs::write(
        dir.join("app_pkg/main.fl"),
        r#"
@use dep_alias.secret_ast;
const X = @secret_ast();
"#,
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

    // 2. Wrong delimiter (`{ ... }` instead of `( ... )`) -> [4011]
    fs::write(
        dir.join("app_pkg/main.fl"),
        r#"
@use dep_alias.add_two;
const X: Integer = @add_two { 1, 2 };
"#,
    )
    .unwrap();
    let delim_err = expect_err(packages::compile_with_source_protocols_for_target(
        &pkgs,
        &deps,
        0,
        Some(builtin_id),
        &base_protocols,
        sysroot_selectors(&ci, &hi),
        target,
    ));
    assert!(delim_err.contains("macro expansion"), "{delim_err}");

    // 3. Invalid AST arg, missing arg, extra arg -> [4012]
    for bad_call in [
        "@use dep_alias.only_id; comptime fn f() -> Integer { @only_id(1 + 2) }",
        "@use dep_alias.add_two; const X: Integer = @add_two(1);",
        "@use dep_alias.add_two; const X: Integer = @add_two(1, 2, 3);",
    ] {
        fs::write(dir.join("app_pkg/main.fl"), bad_call).unwrap();
        let arg_err = expect_err(packages::compile_with_source_protocols_for_target(
            &pkgs,
            &deps,
            0,
            Some(builtin_id),
            &base_protocols,
            sysroot_selectors(&ci, &hi),
            target,
        ));
        assert!(arg_err.contains("macro expansion"), "{bad_call}: {arg_err}");
    }

    // 4. Invalid `@define_macro1` definitions in scripts/
    let bad_defs = [
        // Non-trailing `...`
        r#"@use meta.macros.*; const m = @define_macro1 { fn(...a: Repetition<FlurryPattern>, b: FlurryExpr) -> TokenBuffer { {{ 1 }} } };"#,
        // `...` on non-`Repetition` type
        r#"@use meta.macros.*; const m = @define_macro1 { fn(...a: FlurryPattern) -> TokenBuffer { {{ 1 }} } };"#,
        // `Repetition` without `...`
        r#"@use meta.macros.*; const m = @define_macro1 { fn(a: Repetition<FlurryPattern>) -> TokenBuffer { {{ 1 }} } };"#,
        // Bare `@define_macro1` outside `const`
        r#"@use meta.macros.*; fn main() { @define_macro1 { fn() -> TokenBuffer { {{ 1 }} } } }"#,
    ];
    for (idx, bad_script) in bad_defs.iter().enumerate() {
        let dir_bad = unique_temp_dir("luna-macro1-baddef");
        fs::create_dir_all(dir_bad.join("scripts")).unwrap();
        fs::write(dir_bad.join("scripts/macros.fl"), bad_script).unwrap();
        fs::write(
            dir_bad.join("main.fl"),
            "@use scripts.macros.*; fn run() -> i32 { 0 }",
        )
        .unwrap();
        let def_err = expect_err(compile_sysroot_project(&dir_bad.join("main.fl")));
        assert!(!def_err.is_empty(), "bad def #{idx} unexpectedly succeeded");
        let _ = fs::remove_dir_all(&dir_bad);
    }

    let _ = fs::remove_dir_all(&dir);
}
