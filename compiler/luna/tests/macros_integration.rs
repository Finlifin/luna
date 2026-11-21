//! Cross-layer integration tests covering all five macro layers together:
//! file-local template macros (`define` / `$name`), `@define_macro1` (Mode 1 AST),
//! `@define_macro2` (Mode 2 PEG), `^macro3` (Mode 3 TokenBuffer), `^macro4` / `@builtin.str.raw`
//! (Mode 4 raw source), `TokenBuffer` / `ToTokens`, `scripts/` + dependency macro preparation,
//! `CompilationInputs`, CLI `dump` / `build` / `test`, and cross-layer diagnostic provenance.
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

fn expect_err<T>(res: Result<T, String>) -> String {
    match res {
        Ok(_) => panic!("expected Err, got Ok"),
        Err(e) => e,
    }
}

#[test]
fn all_five_macro_layers_compose_in_comptime_native_and_cli_emits() {
    // Part A: Multi-package comptime + runtime composition combining:
    // - Mode 4 (^macro4) -> Mode 3 (^macro3) across dependency boundary
    // - Mode 2 (@define_macro2) -> Template (define / $)
    // - Template -> Mode 1 (@define_macro1)
    // - @builtin.str.raw
    let dir_multi = unique_temp_dir("luna-macro-integration-multi");
    fs::create_dir_all(dir_multi.join("macro_dep")).unwrap();
    fs::create_dir_all(dir_multi.join("app_pkg/scripts")).unwrap();

    fs::write(
        dir_multi.join("macro_dep/lib.fl"),
        r#"
@use meta.macros.*;
use meta.ast.*;
use meta.macros.macro3;

^macro3
pub comptime fn wrap_add(tokens: TokenBuffer) -> TokenBuffer {
    {{ ($tokens + 1) }}
}

pub const alternatives = @define_macro1 {
    fn(...patterns: Repetition<FlurryPattern>) -> TokenBuffer {
        patterns.fold({{ (not _) }}, |acc, item| {{ $acc or $item }})
    }
};
"#,
    )
    .unwrap();

    fs::write(
        dir_multi.join("app_pkg/scripts/macros.fl"),
        r#"
@use meta.macros.*;
use meta.ast.{PegNode, TokenBuffer};
use meta.macros.macro4;

^macro4
pub comptime fn raw_const_and_bump(src: str) -> TokenBuffer {
    {{
        const RAW_TAG: str = $src;
        const BUMPED_CT: Integer = @macro_dep.wrap_add { 40 };
        const BUMPED_RT: i32 = @macro_dep.wrap_add { 40 };
    }}
}

pub const pipeline = @define_macro2 {
    Entry -> Step+;
    Step -> ("add" ~ FlurryExpr ~ ";") | ("mul" ~ FlurryExpr ~ ";");

    fn(tree: PegNode) -> TokenBuffer {
        let steps = tree.children();
        var acc = {{}};
        var i: Integer = 0;
        while i < steps.length() {
            let parts = steps[i].children();
            let op_text = parts[0].text();
            let val_node = parts[1];
            let call_buf = if op_text == "add" {
                {{ $$step_add(acc, $val_node) }}
            } else {
                {{ $$step_mul(acc, $val_node) }}
            };
            acc = acc.concat(call_buf);
            i = i + 1;
        }
        acc
    }
};
"#,
    )
    .unwrap();

    fs::write(
        dir_multi.join("app_pkg/main.fl"),
        r#"
@use scripts.macros.{raw_const_and_bump, pipeline};
@use macro_dep.{wrap_add, alternatives};
@use builtin.str.raw as raw;

define step_add(target, delta) {
    $target = $target + $delta;
}

define step_mul(target, factor) {
    $target = $target * $factor;
}

define match_any(val, p1, p2) {
    $val match {
        @alternatives($p1, $p2) => true,
        _ => false,
    }
}

@raw_const_and_bump { tag: $ @ ^ " \ }

const VERBATIM: str = @raw { verbatim: $ @ };

comptime fn verify_comptime() -> bool {
    RAW_TAG == " tag: $ @ ^ \" \\ "
        and BUMPED_CT == 41
        and VERBATIM == " verbatim: $ @ "
        and @wrap_add { 9 } == 10
}

fn run() -> bool {
    var acc: i32 = 0;
    @pipeline {
        add 10;
        mul 3;
        add 2;
    }
    let two: i32 = 2;
    let nine: i32 = 9;
    BUMPED_RT == 41
        and acc == 32
        and $match_any(two, 1, 2)
        and not $match_any(nine, 1, 2)
}
"#,
    )
    .unwrap();

    let raw_packages = vec![
        SourcePackage {
            resolution_id: 0,
            identity: 100,
            name: "app_pkg".into(),
            entry: dir_multi.join("app_pkg/main.fl"),
        },
        SourcePackage {
            resolution_id: 1,
            identity: 101,
            name: "macro_dep".into(),
            entry: dir_multi.join("macro_dep/lib.fl"),
        },
    ];
    let raw_deps = vec![DependencyEdge {
        requester: 0,
        alias: "macro_dep".into(),
        provider: 1,
    }];
    let (all_packages, all_deps, builtin_id, protocols) = sysroot_packages(raw_packages, raw_deps);
    let (ci, hi) = load_intrinsic_selectors();
    let sel = sysroot_selectors(&ci, &hi);
    let target = Some(nir::target::TargetData::new(64).unwrap());

    let (artifact, _) = packages::compile_with_target_and_inputs(
        &all_packages,
        &all_deps,
        0,
        Some(builtin_id),
        &protocols,
        sel,
        target,
    )
    .unwrap();
    let frontend::Artifact::Nir(nir) = artifact else {
        panic!("expected NIR artifact");
    };
    assert_eq!(
        luna::invocation::invoke(&nir, "verify_comptime", 20_000_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&nir, "run", 20_000_000).unwrap(),
        Value::Bool(true)
    );
    let _ = fs::remove_dir_all(&dir_multi);

    // Part B: Single-package native LLVM build + CLI `dump --emit ast/hir/nir` + `luna test`
    let dir_cli = unique_temp_dir("luna-macro-integration-cli");
    fs::create_dir_all(dir_cli.join("scripts")).unwrap();
    fs::write(
        dir_cli.join("scripts/macros.fl"),
        r#"
@use meta.macros.*;
use meta.ast.*;
use meta.macros.{macro3, macro4};

pub const alternatives = @define_macro1 {
    fn(...patterns: Repetition<FlurryPattern>) -> TokenBuffer {
        patterns.fold({{ (not _) }}, |acc, item| {{ $acc or $item }})
    }
};

pub const statements = @define_macro2 {
    Entry -> FlurryExtendedStatement*;

    fn(body: Repetition<meta.ast.FlurryExtendedStatement>) -> TokenBuffer {
        {{ $body }}
    }
};

^macro3
pub comptime fn wrap_add(tokens: TokenBuffer) -> TokenBuffer {
    {{ ($tokens + 1) }}
}

^macro4
pub comptime fn raw_msg(src: str) -> TokenBuffer {
    {{ $src }}
}
"#,
    )
    .unwrap();

    fs::write(
        dir_cli.join("main.fl"),
        r#"
@use scripts.macros.{alternatives, statements, wrap_add, raw_msg};
use std.prelude.*;

define bump_var(v) {
    $v = @wrap_add { $v };
}

fn match_code(code: i32) -> bool {
    code match {
        @alternatives(1, 3, 5) => true,
        _ => false,
    }
}

fn run_io() -> !std.io.IoErr bool {
    var count: i32 = 10;
    $bump_var(count)
    @statements {
        let msg = @raw_msg {raw: $ @};
        println("{}", msg)!;
        println("{}", @builtin.str.raw {verbatim: #1})!;
    }
    count == 11 and match_code(3) and not match_code(4)
}

fn run() -> bool {
    run_io() match {
        ok! => ok,
        error _ => false,
    }
}

test {
    var x: i32 = 4;
    $bump_var(x)
    @statements {
        assumes x == 5;
        assumes match_code(5);
    }
}
"#,
    )
    .unwrap();

    let sysroot = library_dir();
    for emit_kind in ["ast", "hir", "nir"] {
        let mut cmd = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"));
        cmd.current_dir(&dir_cli)
            .args(["dump", "main.fl", "--emit", emit_kind, "--sysroot"])
            .arg(&sysroot);
        if emit_kind == "nir" {
            cmd.args(["--build-target", "x86_64", "linux", "64"]);
        }
        let dump = cmd.output().unwrap();
        assert!(
            dump.status.success(),
            "luna dump --emit {emit_kind} failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&dump.stdout),
            String::from_utf8_lossy(&dump.stderr)
        );
    }

    let build = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&dir_cli)
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
        .arg(&sysroot)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "luna build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let run = ProcessCommand::new(dir_cli.join("program"))
        .current_dir(&dir_cli)
        .output()
        .unwrap();
    assert!(run.status.success(), "{run:?}");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "raw: $ @\nverbatim: #1\n(bool true)\n"
    );

    let test_cmd = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&dir_cli)
        .args(["test", "main.fl", "--sysroot"])
        .arg(&sysroot)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&dir_cli);
    assert!(
        test_cmd.status.success(),
        "luna test failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&test_cmd.stdout),
        String::from_utf8_lossy(&test_cmd.stderr)
    );
}

#[test]
fn scripts_consume_dependency_macros_and_track_compilation_inputs() {
    let dir = unique_temp_dir("luna-macro-integration-scripts-deps");
    fs::create_dir_all(dir.join("dep_macros")).unwrap();
    fs::create_dir_all(dir.join("app_pkg/scripts")).unwrap();

    fs::write(
        dir.join("dep_macros/lib.fl"),
        r#"
@use meta.macros.*;
use meta.ast.{FlurryExpr, TokenBuffer};
use meta.macros.macro3;

pub const double_expr = @define_macro1 {
    fn(e: FlurryExpr) -> TokenBuffer {
        {{ ($e + $e) }}
    }
};

^macro3
pub comptime fn emit_double(tokens: TokenBuffer) -> TokenBuffer {
    {{ @dep_macros.double_expr($tokens) }}
}
"#,
    )
    .unwrap();

    // Multi-file `scripts/` without `scripts/lib.fl`: scripts/a.fl uses `dep_macros` both inside its
    // own body (`@dep_macros.double_expr`) and via emitted Mode 3 -> Mode 1 expansion (`@dep_macros.emit_double`).
    fs::write(
        dir.join("app_pkg/scripts/a.fl"),
        r#"
@use dep_macros.double_expr;
use meta.ast.TokenBuffer;
use meta.macros.macro3;

^macro3
pub comptime fn twice_plus_local(tokens: TokenBuffer) -> TokenBuffer {
    let local = @double_expr(5);
    {{ ($local + @dep_macros.emit_double { $tokens }) }}
}
"#,
    )
    .unwrap();

    fs::write(
        dir.join("app_pkg/scripts/b.fl"),
        r#"
use meta.ast.TokenBuffer;
use meta.macros.macro3;

^macro3
pub comptime fn add_ten(tokens: TokenBuffer) -> TokenBuffer {
    {{ ($tokens + 10) }}
}
"#,
    )
    .unwrap();

    fs::write(
        dir.join("app_pkg/main.fl"),
        r#"
@use scripts.a.{twice_plus_local as expand_num};
@use scripts.b.*;

comptime fn compute() -> Integer {
    @add_ten { @expand_num { 6 } }
}
"#,
    )
    .unwrap();

    let raw_packages = vec![
        SourcePackage {
            resolution_id: 0,
            identity: 100,
            name: "app_pkg".into(),
            entry: dir.join("app_pkg/main.fl"),
        },
        SourcePackage {
            resolution_id: 1,
            identity: 101,
            name: "dep_macros".into(),
            entry: dir.join("dep_macros/lib.fl"),
        },
    ];
    let raw_deps = vec![DependencyEdge {
        requester: 0,
        alias: "dep_macros".into(),
        provider: 1,
    }];
    let (all_packages, all_deps, builtin_id, protocols) = sysroot_packages(raw_packages, raw_deps);
    let (ci, hi) = load_intrinsic_selectors();
    let sel = sysroot_selectors(&ci, &hi);
    let target = Some(nir::target::TargetData::new(64).unwrap());

    let (artifact, mut inputs) = packages::compile_with_target_and_inputs(
        &all_packages,
        &all_deps,
        0,
        Some(builtin_id),
        &protocols,
        sel,
        target,
    )
    .unwrap();
    let frontend::Artifact::Nir(nir) = artifact else {
        panic!("expected NIR artifact");
    };
    // (5 + 5) + ((6 + 6)) + 10 = 32
    assert_eq!(
        luna::invocation::invoke(&nir, "compute", 20_000_000).unwrap(),
        Value::Integer(comptime_runtime::Integer::from_i128(32))
    );

    assert!(
        inputs
            .sources
            .iter()
            .any(|s| s.path.ends_with("app_pkg/scripts/a.fl")),
        "missing scripts/a.fl in CompilationInputs: {:?}",
        inputs.sources.iter().map(|s| &s.path).collect::<Vec<_>>()
    );
    assert!(
        inputs
            .sources
            .iter()
            .any(|s| s.path.ends_with("app_pkg/scripts/b.fl")),
        "missing scripts/b.fl in CompilationInputs: {:?}",
        inputs.sources.iter().map(|s| &s.path).collect::<Vec<_>>()
    );
    assert!(
        inputs
            .sources
            .iter()
            .any(|s| s.path.ends_with("dep_macros/lib.fl")),
        "missing dep_macros/lib.fl in CompilationInputs: {:?}",
        inputs.sources.iter().map(|s| &s.path).collect::<Vec<_>>()
    );

    assert!(inputs.changed_sources().unwrap().is_empty());
    fs::write(dir.join("app_pkg/scripts/b.fl"), "// edited\n").unwrap();
    let changed = inputs.changed_sources().unwrap();
    assert_eq!(changed.len(), 1);
    assert!(changed[0].path.ends_with("app_pkg/scripts/b.fl"));
    inputs.sources.clear();

    let _ = fs::remove_dir_all(&dir);

    // Pure on-demand preparation: neither `app_ondemand/main.fl` nor `scripts/macros.fl`
    // mentions `@dep_only` outside `{{ ... }}`; expanding `@emit_triple` emits `@dep_only.triple`
    // and triggers on-demand preparation of direct dependency `dep_only`.
    let dir_od = unique_temp_dir("luna-macro-integration-ondemand");
    fs::create_dir_all(dir_od.join("dep_only")).unwrap();
    fs::create_dir_all(dir_od.join("app_od/scripts")).unwrap();

    fs::write(
        dir_od.join("dep_only/lib.fl"),
        r#"
use meta.ast.TokenBuffer;
use meta.macros.macro3;

^macro3
pub comptime fn triple(tokens: TokenBuffer) -> TokenBuffer {
    {{ ($tokens * 3) }}
}
"#,
    )
    .unwrap();

    fs::write(
        dir_od.join("app_od/scripts/macros.fl"),
        r#"
use meta.ast.TokenBuffer;
use meta.macros.macro3;

^macro3
pub comptime fn emit_triple(tokens: TokenBuffer) -> TokenBuffer {
    {{ @dep_only.triple { $tokens } }}
}
"#,
    )
    .unwrap();

    fs::write(
        dir_od.join("app_od/main.fl"),
        r#"
@use scripts.macros.emit_triple;

comptime fn compute() -> Integer {
    @emit_triple { 7 }
}
"#,
    )
    .unwrap();

    let od_packages = vec![
        SourcePackage {
            resolution_id: 0,
            identity: 200,
            name: "app_od".into(),
            entry: dir_od.join("app_od/main.fl"),
        },
        SourcePackage {
            resolution_id: 1,
            identity: 201,
            name: "dep_only".into(),
            entry: dir_od.join("dep_only/lib.fl"),
        },
    ];
    let od_deps = vec![DependencyEdge {
        requester: 0,
        alias: "dep_only".into(),
        provider: 1,
    }];
    let (all_od_pkgs, all_od_deps, od_builtin, od_protocols) =
        sysroot_packages(od_packages, od_deps);
    let (od_artifact, _) = packages::compile_with_target_and_inputs(
        &all_od_pkgs,
        &all_od_deps,
        0,
        Some(od_builtin),
        &od_protocols,
        sysroot_selectors(&ci, &hi),
        Some(nir::target::TargetData::new(64).unwrap()),
    )
    .unwrap();
    let frontend::Artifact::Nir(od_nir) = od_artifact else {
        panic!("expected NIR artifact");
    };
    assert_eq!(
        luna::invocation::invoke(&od_nir, "compute", 20_000_000).unwrap(),
        Value::Integer(comptime_runtime::Integer::from_i128(21))
    );
    let _ = fs::remove_dir_all(&dir_od);
}

#[test]
fn cross_layer_diagnostics_preserve_provenance_and_reject_invalid_boundaries() {
    // 1. Post-expansion type error through Mode 2 + Template chain
    let dir_type = unique_temp_dir("luna-macro-integration-post-type-err");
    fs::create_dir_all(dir_type.join("scripts")).unwrap();
    fs::write(
        dir_type.join("scripts/macros.fl"),
        r#"
@use meta.macros.*;
use meta.ast.{PegNode, TokenBuffer};

pub const wrap_stmt = @define_macro2 {
    Entry -> FlurryExpr;

    fn(n: PegNode) -> TokenBuffer {
        {{ let generated: i32 = $n; }}
    }
};
"#,
    )
    .unwrap();
    fs::write(
        dir_type.join("main.fl"),
        r#"
@use scripts.macros.wrap_stmt;
define bool_lit() { true }
fn bad() -> i32 {
    @wrap_stmt { $bool_lit() }
    0
}
"#,
    )
    .unwrap();
    let err_type = expect_err(compile_sysroot_project(&dir_type.join("main.fl")));
    assert!(!err_type.is_empty());
    let _ = fs::remove_dir_all(&dir_type);

    // 2. Mode 4 invoked from generated token stream (`^macro3` returning `@builtin.str.raw { ... }`)
    let dir_mode4_gen = unique_temp_dir("luna-macro-integration-mode4-generated");
    fs::create_dir_all(dir_mode4_gen.join("scripts")).unwrap();
    fs::write(
        dir_mode4_gen.join("scripts/macros.fl"),
        r#"
use meta.ast.TokenBuffer;
use meta.macros.macro3;

^macro3
pub comptime fn emit_raw(tokens: TokenBuffer) -> TokenBuffer {
    {{ @builtin.str.raw { nope } }}
}
"#,
    )
    .unwrap();
    fs::write(
        dir_mode4_gen.join("main.fl"),
        r#"
@use scripts.macros.emit_raw;
const X: str = @emit_raw { x };
"#,
    )
    .unwrap();
    let err_mode4_gen = expect_err(compile_sysroot_project(&dir_mode4_gen.join("main.fl")));
    assert!(
        err_mode4_gen.contains("main.fl"),
        "expected consumer call site span in main.fl: {err_mode4_gen}"
    );
    assert!(
        err_mode4_gen.contains("cannot be invoked from generated token streams"),
        "{err_mode4_gen}"
    );
    assert!(
        err_mode4_gen.contains("scripts.macros.emit_raw"),
        "{err_mode4_gen}"
    );
    let _ = fs::remove_dir_all(&dir_mode4_gen);

    // 3. Infinite recursive macro depth limit with provenance chain
    let dir_depth = unique_temp_dir("luna-macro-integration-depth");
    fs::create_dir_all(dir_depth.join("scripts")).unwrap();
    fs::write(
        dir_depth.join("scripts/macros.fl"),
        r#"
use meta.ast.TokenBuffer;
use meta.macros.macro3;

^macro3
pub comptime fn self_loop(tokens: TokenBuffer) -> TokenBuffer {
    {{ @self_loop { $tokens } }}
}
"#,
    )
    .unwrap();
    fs::write(
        dir_depth.join("main.fl"),
        r#"
@use scripts.macros.self_loop;
const X: Integer = @self_loop { 1 };
"#,
    )
    .unwrap();
    let err_depth = expect_err(compile_sysroot_project(&dir_depth.join("main.fl")));
    assert!(
        err_depth.contains("depth limit") || err_depth.contains("4005"),
        "{err_depth}"
    );
    assert!(err_depth.contains("self_loop"), "{err_depth}");
    let _ = fs::remove_dir_all(&dir_depth);

    // 4. Invalid handler return type (`Integer` instead of `TokenBuffer`)
    let dir_ret = unique_temp_dir("luna-macro-integration-bad-ret");
    fs::create_dir_all(dir_ret.join("scripts")).unwrap();
    fs::write(
        dir_ret.join("scripts/macros.fl"),
        r#"
use meta.ast.TokenBuffer;
use meta.macros.macro3;

^macro3
pub comptime fn bad_ret(tokens: TokenBuffer) -> Integer {
    1
}
"#,
    )
    .unwrap();
    fs::write(
        dir_ret.join("main.fl"),
        r#"
@use scripts.macros.bad_ret;
const X: Integer = @bad_ret { 1 };
"#,
    )
    .unwrap();
    let err_ret = expect_err(compile_sysroot_project(&dir_ret.join("main.fl")));
    assert!(
        err_ret.contains("return type must be") && err_ret.contains("TokenBuffer"),
        "{err_ret}"
    );
    let _ = fs::remove_dir_all(&dir_ret);
}
