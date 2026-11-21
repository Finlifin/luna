//! End-to-end tests for Phase 3 construction macros: `^macro3` (TokenBuffer), `^macro4` (raw source),
//! `scripts/` preparation, dependency macro packages, `CompilationInputs`, and diagnostics.
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
    let base_count = user_packages.len() as u32;
    let mut builtin_id = None;
    let mut prelude_ids = Vec::new();
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
        if name != "std" {
            prelude_ids.push(res_id);
        }
        sysroot_ids.push((name.to_string(), res_id, ident));
        user_packages.push(SourcePackage {
            resolution_id: res_id,
            identity: ident,
            name: name.to_string(),
            entry,
        });
    }

    // Wire sysroot edges for every package (including sysroot inter-dependencies).
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
    let _ = base_count;
    let mut protocols = elaboration::program::Protocols::default();
    protocols
        .builtin_implementation_packages
        .extend(sysroot_ids.iter().map(|(_, _, ident)| *ident));
    let _ = prelude_ids;
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
fn scripts_macro3_passthrough_and_token_transformer_work_in_comptime_and_native() {
    let dir = unique_temp_dir("luna-macro3-scripts");
    fs::create_dir_all(dir.join("scripts")).unwrap();

    // Official `^macro3 passthrough` without `pub` (package-visible inside scripts/) +
    // token-transforming `^macro3 make_adder` generating a native/comptime function using `{{ ... }}`.
    fs::write(
        dir.join("scripts/macros.fl"),
        r#"
        use meta.ast.{Token, TokenBuffer};
        use meta.macros.macro3;

        ^macro3
        comptime fn passthrough(tokens: TokenBuffer) -> TokenBuffer {
            tokens
        }

        ^macro3
        comptime fn make_adder(tokens: TokenBuffer) -> TokenBuffer {
            let fn_name = tokens.get(0) match {
                t? => t,
                null => Token.ident("missing"),
            };
            let delta = tokens.get(1) match {
                t? => t,
                null => Token.int_literal(0),
            };
            {{
                fn $fn_name(x: i32) -> i32 {
                    if x >= 0 and x < 1000 {
                        x + $delta
                    } else {
                        0
                    }
                }
            }}
        }
        "#,
    )
    .unwrap();

    fs::write(
        dir.join("main.fl"),
        r#"
        @use scripts.macros.{passthrough, make_adder};
        use std.prelude.*;

        @passthrough {
            const ANSWER: i32 = 42;
            comptime fn double_it(x: i32) -> i32 {
                if x >= 0 and x < 1000 {
                    x + x
                } else {
                    0
                }
            }
        }

        @make_adder { add_ten 10 }

        comptime fn check_comptime() -> bool {
            ANSWER == 42 and double_it(21) == 42
        }

        fn run() -> bool {
            let base = ANSWER;
            let result = if base >= 10 and base < 1000 {
                add_ten(base - 10)
            } else {
                0
            };
            println("result={}", result) match {
                _! => result == 42,
                error _ => false,
            }
        }
        "#,
    )
    .unwrap();

    let comptime_val = invoke_sysroot_project(&dir.join("main.fl"), "check_comptime").unwrap();
    assert_eq!(comptime_val, Value::Bool(true));

    // Native executable build and execution
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
    assert_eq!(stdout, "result=42\n(bool true)\n");
}

#[test]
fn scripts_macro4_raw_source_transforms_unlexable_text_into_token_buffer() {
    let dir = unique_temp_dir("luna-macro4-scripts");
    fs::create_dir_all(dir.join("scripts")).unwrap();

    fs::write(
        dir.join("scripts/lib.fl"),
        r#"
        use meta.ast.{Token, TokenBuffer};
        use meta.macros.macro4;
        use meta.ComptimeTextBytes;

        pure comptime fn trim_ascii_spaces(s: str) -> str {
            let len = s.as(ComptimeTextBytes).byte_length();
            var start: u64 = 0;
            var scanning_front = true;
            while start < len and scanning_front {
                s.as(ComptimeTextBytes).byte(start) match {
                    b? => {
                        if b == 32 {
                            start = start + 1;
                        } else {
                            scanning_front = false;
                        }
                    },
                    null => {
                        scanning_front = false;
                    },
                }
            }
            var end: u64 = len;
            var scanning_back = true;
            while end > start and scanning_back {
                s.as(ComptimeTextBytes).byte(end - 1) match {
                    b? => {
                        if b == 32 {
                            end = end - 1;
                        } else {
                            scanning_back = false;
                        }
                    },
                    null => {
                        scanning_back = false;
                    },
                }
            }
            s.as(ComptimeTextBytes).slice(start, end) match {
                v? => v,
                null => "",
            }
        }

        ^macro4
        comptime fn kv_text(raw: str) -> TokenBuffer {
            let trimmed = trim_ascii_spaces(raw);
            let prefix = if trimmed.as(ComptimeTextBytes).contains("mode=fast") {
                "FAST:"
            } else {
                "SLOW:"
            };
            let full = prefix.as(ComptimeTextBytes).concat(trimmed);
            {{ $full }}
        }
        "#,
    )
    .unwrap();

    fs::write(
        dir.join("main.fl"),
        r#"
        @use scripts.kv_text;

        comptime fn run_check() -> bool {
            use meta.ComptimeTextBytes;
            let cfg = @kv_text {  mode=fast symbols=$ @ \unlexable " };
            cfg == "FAST:mode=fast symbols=$ @ \\unlexable \""
        }
        "#,
    )
    .unwrap();

    let val = invoke_sysroot_project(&dir.join("main.fl"), "run_check").unwrap();
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(val, Value::Bool(true));
}

#[test]
fn multi_hop_macro_dependency_packages_and_compilation_inputs_work() {
    let dir = unique_temp_dir("luna-macro-deps");
    fs::create_dir_all(dir.join("pkg_c")).unwrap();
    fs::create_dir_all(dir.join("pkg_b")).unwrap();
    fs::create_dir_all(dir.join("pkg_a/scripts")).unwrap();

    fs::write(
        dir.join("pkg_c/lib.fl"),
        r#"
        use meta.ast.TokenBuffer;
        use meta.macros.macro3;

        ^macro3
        pub comptime fn emit_offset(tokens: TokenBuffer) -> TokenBuffer {
            {{ const OFFSET: Integer = ($tokens) + 1; }}
        }
        "#,
    )
    .unwrap();

    fs::write(
        dir.join("pkg_b/lib.fl"),
        r#"
        @use dep_c.emit_offset;
        use meta.ast.TokenBuffer;
        use meta.macros.macro3;

        @emit_offset { 0 }

        ^macro3
        pub comptime fn times_two_plus_one(tokens: TokenBuffer) -> TokenBuffer {
            let offset = OFFSET;
            {{ (($tokens) * 2) + $offset }}
        }
        "#,
    )
    .unwrap();

    fs::write(
        dir.join("pkg_a/scripts/local.fl"),
        r#"
        use meta.ast.TokenBuffer;
        use meta.macros.macro3;

        ^macro3
        comptime fn keep(tokens: TokenBuffer) -> TokenBuffer {
            tokens
        }
        "#,
    )
    .unwrap();

    fs::write(
        dir.join("pkg_a/main.fl"),
        r#"
        @use dep_b.times_two_plus_one;
        @use scripts.local.keep;

        @keep {
            comptime fn compute() -> Integer {
                @times_two_plus_one { 20 }
            }
        }
        "#,
    )
    .unwrap();

    let raw_packages = vec![
        SourcePackage {
            resolution_id: 0,
            identity: 100,
            name: "pkg_a".into(),
            entry: dir.join("pkg_a/main.fl"),
        },
        SourcePackage {
            resolution_id: 1,
            identity: 101,
            name: "pkg_b".into(),
            entry: dir.join("pkg_b/lib.fl"),
        },
        SourcePackage {
            resolution_id: 2,
            identity: 102,
            name: "pkg_c".into(),
            entry: dir.join("pkg_c/lib.fl"),
        },
    ];
    let raw_deps = vec![
        DependencyEdge {
            requester: 0,
            alias: "dep_b".into(),
            provider: 1,
        },
        DependencyEdge {
            requester: 1,
            alias: "dep_c".into(),
            provider: 2,
        },
    ];
    let (all_packages, all_deps, builtin_id, base_protocols) =
        sysroot_packages(raw_packages, raw_deps);
    let (ci, hi) = load_intrinsic_selectors();
    let sel = sysroot_selectors(&ci, &hi);

    let target = Some(nir::target::TargetData::new(64).unwrap());
    let (artifact, mut inputs) = packages::compile_with_target_and_inputs(
        &all_packages,
        &all_deps,
        0,
        Some(builtin_id),
        &base_protocols,
        sel,
        target,
    )
    .unwrap();
    let luna::frontend::Artifact::Nir(nir) = artifact else {
        panic!("expected NIR artifact")
    };
    let val = luna::invocation::invoke(&nir, "compute", 10_000_000).unwrap();
    assert_eq!(val.to_string(), "(integer (sign positive) (hex \"29\"))");

    // Verify `scripts/local.fl` is tracked in `CompilationInputs` and excluded from `pkg_a` runtime discovery.
    assert!(
        inputs
            .sources
            .iter()
            .any(|s| s.path.ends_with("pkg_a/scripts/local.fl")),
        "scripts file should be recorded in CompilationInputs: {:?}",
        inputs.sources.iter().map(|s| &s.path).collect::<Vec<_>>()
    );
    let pkg_a_discovery = inputs
        .discovery
        .iter()
        .find(|d| d.resolution_id == 0)
        .unwrap();
    assert!(
        !pkg_a_discovery
            .files
            .iter()
            .any(|p| p.to_string_lossy().contains("scripts")),
        "pkg_a normal discovery must exclude scripts/: {:?}",
        pkg_a_discovery.files
    );

    // Modifying `scripts/local.fl` is detected by `CompilationInputs::changed_sources`.
    assert!(inputs.changed_sources().unwrap().is_empty());
    fs::write(dir.join("pkg_a/scripts/local.fl"), "// edited\n").unwrap();
    let changed = inputs.changed_sources().unwrap();
    assert_eq!(changed.len(), 1);
    assert!(changed[0].path.ends_with("pkg_a/scripts/local.fl"));
    inputs.sources.clear();

    // Runtime import of `scripts` from `pkg_a` must NOT resolve (scripts are separate from runtime modules).
    let dir_runtime = unique_temp_dir("luna-macro-no-runtime-scripts");
    fs::create_dir_all(dir_runtime.join("scripts")).unwrap();
    fs::write(
        dir_runtime.join("scripts/macros.fl"),
        r#"
        pub fn helper() -> i32 { 1 }
        "#,
    )
    .unwrap();
    fs::write(
        dir_runtime.join("main.fl"),
        r#"
        use scripts.macros.helper;
        fn run() -> i32 { helper() }
        "#,
    )
    .unwrap();
    assert!(compile_sysroot_project(&dir_runtime.join("main.fl")).is_err());

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&dir_runtime);
}

#[test]
fn macro_visibility_delimiters_ambiguity_cycles_recursion_and_failures_are_diagnosed() {
    // 1. Private script macro & wrong delimiter & cycle
    let dir = unique_temp_dir("luna-macro-errors");
    fs::create_dir_all(dir.join("scripts")).unwrap();
    fs::write(
        dir.join("scripts/macros.fl"),
        r#"
        use meta.ast.TokenBuffer;
        use meta.macros.macro3;

        ^macro3
        private comptime fn hidden(tokens: TokenBuffer) -> TokenBuffer {
            tokens
        }

        ^macro3
        comptime fn ok(tokens: TokenBuffer) -> TokenBuffer {
            tokens
        }

        ^macro3
        comptime fn fail_now(tokens: TokenBuffer) -> TokenBuffer {
            let _ = tokens.tokens[999];
            tokens
        }
        "#,
    )
    .unwrap();

    fs::write(
        dir.join("main.fl"),
        r#"
        @use scripts.macros.hidden;
        fn run() -> i32 { 0 }
        "#,
    )
    .unwrap();
    let priv_err = expect_err(compile_sysroot_project(&dir.join("main.fl")));
    assert!(
        priv_err.contains("macro expansion") || priv_err.contains("4010"),
        "{priv_err}"
    );

    fs::write(
        dir.join("main.fl"),
        r#"
        @use scripts.macros.ok;
        @ok(123)
        "#,
    )
    .unwrap();
    let delim_err = expect_err(compile_sysroot_project(&dir.join("main.fl")));
    assert!(delim_err.contains("macro expansion"), "{delim_err}");

    fs::write(
        dir.join("main.fl"),
        r#"
        @use scripts.macros.fail_now;
        const X = @fail_now { 1 };
        "#,
    )
    .unwrap();
    let exec_err = expect_err(compile_sysroot_project(&dir.join("main.fl")));
    assert!(exec_err.contains("macro expansion"), "{exec_err}");

    // 2. Self-referential cycle in `scripts/`
    let dir_cycle = unique_temp_dir("luna-macro-cycle");
    fs::create_dir_all(dir_cycle.join("scripts")).unwrap();
    fs::write(
        dir_cycle.join("scripts/lib.fl"),
        r#"
        use meta.ast.TokenBuffer;
        use meta.macros.macro3;

        @scripts.self_macro { const A = 1; }

        ^macro3
        comptime fn self_macro(tokens: TokenBuffer) -> TokenBuffer {
            tokens
        }
        "#,
    )
    .unwrap();
    fs::write(
        dir_cycle.join("main.fl"),
        r#"
        @use scripts.self_macro;
        @self_macro { const B = 2; }
        "#,
    )
    .unwrap();
    let cycle_err = expect_err(compile_sysroot_project(&dir_cycle.join("main.fl")));
    assert!(cycle_err.contains("macro expansion"), "{cycle_err}");

    // 3. Invalid signature on `^macro3`
    let dir_sig = unique_temp_dir("luna-macro-bad-sig");
    fs::create_dir_all(dir_sig.join("scripts")).unwrap();
    fs::write(
        dir_sig.join("scripts/lib.fl"),
        r#"
        use meta.macros.macro3;

        ^macro3
        comptime fn bad_sig(x: Integer) -> Integer { x }
        "#,
    )
    .unwrap();
    fs::write(
        dir_sig.join("main.fl"),
        r#"
        @use scripts.bad_sig;
        const X = @bad_sig { 1 };
        "#,
    )
    .unwrap();
    let sig_err = expect_err(compile_sysroot_project(&dir_sig.join("main.fl")));
    assert!(
        sig_err.contains("TokenBuffer") || sig_err.contains("parameter"),
        "{sig_err}"
    );

    // 4. Dependency non-pub macro visibility + ambiguity + infinite recursion
    let dir_dep = unique_temp_dir("luna-macro-dep-err");
    fs::create_dir_all(dir_dep.join("m1")).unwrap();
    fs::create_dir_all(dir_dep.join("m2")).unwrap();
    fs::create_dir_all(dir_dep.join("app")).unwrap();
    fs::write(
        dir_dep.join("m1/lib.fl"),
        r#"
        use meta.ast.TokenBuffer;
        use meta.macros.macro3;

        ^macro3
        pub comptime fn dup(t: TokenBuffer) -> TokenBuffer { t }

        ^macro3
        comptime fn pkg_only(t: TokenBuffer) -> TokenBuffer { t }

        ^macro3
        pub comptime fn recurse(_t: TokenBuffer) -> TokenBuffer {
            {{ @m1_alias.recurse {} }}
        }
        "#,
    )
    .unwrap();
    fs::write(
        dir_dep.join("m2/lib.fl"),
        r#"
        use meta.ast.TokenBuffer;
        use meta.macros.macro3;

        ^macro3
        pub comptime fn dup(t: TokenBuffer) -> TokenBuffer { t }
        "#,
    )
    .unwrap();
    fs::write(
        dir_dep.join("app/main.fl"),
        r#"
        @use m1_alias.pkg_only;
        const X = @pkg_only { 1 };
        "#,
    )
    .unwrap();

    let (pkgs, deps, builtin_id, base_protocols) = sysroot_packages(
        vec![
            SourcePackage {
                resolution_id: 0,
                identity: 100,
                name: "app".into(),
                entry: dir_dep.join("app/main.fl"),
            },
            SourcePackage {
                resolution_id: 1,
                identity: 101,
                name: "m1".into(),
                entry: dir_dep.join("m1/lib.fl"),
            },
            SourcePackage {
                resolution_id: 2,
                identity: 102,
                name: "m2".into(),
                entry: dir_dep.join("m2/lib.fl"),
            },
        ],
        vec![
            DependencyEdge {
                requester: 0,
                alias: "m1_alias".into(),
                provider: 1,
            },
            DependencyEdge {
                requester: 0,
                alias: "m2_alias".into(),
                provider: 2,
            },
        ],
    );
    let (ci, hi) = load_intrinsic_selectors();
    let sel = sysroot_selectors(&ci, &hi);
    let target = Some(nir::target::TargetData::new(64).unwrap());

    let pkg_vis_err = expect_err(packages::compile_with_source_protocols_for_target(
        &pkgs,
        &deps,
        0,
        Some(builtin_id),
        &base_protocols,
        sel,
        target,
    ));
    assert!(pkg_vis_err.contains("macro expansion"), "{pkg_vis_err}");

    fs::write(
        dir_dep.join("app/main.fl"),
        r#"
        @use m1_alias.dup;
        @use m2_alias.dup;
        "#,
    )
    .unwrap();
    let ambig_err = expect_err(packages::compile_with_source_protocols_for_target(
        &pkgs,
        &deps,
        0,
        Some(builtin_id),
        &base_protocols,
        sel,
        target,
    ));
    assert!(ambig_err.contains("macro expansion"), "{ambig_err}");

    fs::write(
        dir_dep.join("app/main.fl"),
        r#"
        const R = @m1_alias.recurse {};
        "#,
    )
    .unwrap();
    let rec_err = expect_err(packages::compile_with_source_protocols_for_target(
        &pkgs,
        &deps,
        0,
        Some(builtin_id),
        &base_protocols,
        sel,
        target,
    ));
    assert!(rec_err.contains("macro expansion"), "{rec_err}");

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&dir_cycle);
    let _ = fs::remove_dir_all(&dir_sig);
    let _ = fs::remove_dir_all(&dir_dep);
}
