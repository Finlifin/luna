//! End-to-end tests for file-local template macros (`docs/macro/template-macro.md`).
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use luna::cli::{self, Command, Emit};
use luna::frontend::{self, Artifact};
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicU64, Ordering};

use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_path(prefix: &str, suffix: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{nanos}-{seq}{suffix}",
        std::process::id()
    ))
}

fn library_dir() -> PathBuf {
    if let Ok(path) = std::env::var("LUNA_TEST_SYSROOT") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

fn run_sysroot_interpret(source: &str, entry: &str) -> Result<Value, String> {
    let path = unique_temp_path("luna-macro-template", ".fl");
    std::fs::write(&path, source).unwrap();
    let library = library_dir();
    let Command::Check(options) = cli::parse([
        "check".into(),
        path.clone().into_os_string(),
        "--sysroot".into(),
        library.into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("expected check options")
    };
    let artifact = frontend::compile(&options);
    let _ = std::fs::remove_file(&path);
    let Artifact::Nir(package) = artifact? else {
        panic!("expected NIR artifact")
    };
    luna::execution::interpret(&package, entry, 10_000_000)
}

fn invoke_sysroot_comptime(source: &str, entry: &str) -> Result<Value, String> {
    let path = unique_temp_path("luna-macro-template-ct", ".fl");
    std::fs::write(&path, source).unwrap();
    let library = library_dir();
    let Command::Check(options) = cli::parse([
        "check".into(),
        path.clone().into_os_string(),
        "--sysroot".into(),
        library.into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("expected check options")
    };
    let result = frontend::invoke_comptime(&options, entry, 10_000_000, None, None);
    let _ = std::fs::remove_file(&path);
    result
}

#[test]
fn doc_example_fixed_constant_functions_compile_and_run() {
    let source = r#"
        define constant_function 2 {
            fn $0() -> i32 {
                $1
            }
        }

        $constant_function one 1
        $constant_function two 2

        fn run() -> bool {
            one() == 1 and two() == 2
        }
    "#;
    assert_eq!(
        run_sysroot_interpret(source, "run").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn doc_example_repeated_constants_compile_and_run() {
    let source = r#"
        define constants 2 ... {
            const $0 = $1;
        }

        $constants FIRST 1 SECOND 2 THIRD 3 $

        comptime fn run() -> Integer {
            FIRST + SECOND + THIRD
        }
    "#;
    assert_eq!(
        invoke_sysroot_comptime(source, "run").unwrap(),
        Value::Integer(comptime_runtime::Integer::from_i128(6))
    );
}

#[test]
fn doc_example_named_parameter_print_value_builds_and_runs_native() {
    let dir = unique_temp_path("luna-macro-template-native", "");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    let main_fl = dir.join("main.fl");
    let source = r#"
        use std.prelude.*;

        define print_value(value) {
            println("{}", $value)!;
        }

        fn print_it() -> !std.io.IoErr void {
            let lhs: i32 = 1;
            let rhs: i32 = 2;
            $print_value(lhs + rhs)
        }

        fn run() -> bool {
            print_it() match {
                _! => true,
                error _ => false,
            }
        }
    "#;
    std::fs::write(&main_fl, source).unwrap();

    let library = library_dir();
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
        .arg(&library)
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
    let _ = std::fs::remove_dir_all(&dir);
    assert!(run.status.success(), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3\n(bool true)\n");
}

#[test]
fn duplicated_template_argument_re_evaluates_on_every_insertion() {
    let source = r#"
        typealias i32 = .i32'builtin;

        define add_twice(expr) {
            $expr + $expr
        }

        fn bump(p: *i32) -> result: i32
        where ensures result >= 0 and result <= 100
        {
            if p.* >= 0 and p.* < 100 {
                let next: i32 = p.* + 1;
                p.* = next;
                return next;
            }
            0
        }

        fn run() -> i32 {
            var counter: i32 = 0;
            $add_twice(bump(counter.ref))
        }
    "#;
    // First call evaluates to 1, second evaluates to 2 -> 1 + 2 = 3.
    assert_eq!(
        support::run_iir(source, "run", vec![]),
        Value::SignedInteger(3)
    );
}

#[test]
fn template_macros_are_file_local_in_multi_file_packages() {
    let dir = unique_temp_path("luna-macro-file-local", "");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("lib.fl"),
        r#"
            use helper.from_helper;
            define local_val() { 10 }
            pub fn from_root() -> bool {
                let a: i32 = $local_val();
                let b: i32 = from_helper();
                a == 10 and b == 32
            }
        "#,
    )
    .unwrap();
    std::fs::write(
        dir.join("helper.fl"),
        r#"
            pub fn from_helper() -> i32 { 32 }
        "#,
    )
    .unwrap();

    let library = library_dir();
    let check_ok = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
        .arg(dir.join("lib.fl"))
        .args(["--sysroot"])
        .arg(&library)
        .args([
            "--build-target",
            "x86_64",
            "linux",
            "64",
            "--interpret",
            "from_root",
        ])
        .output()
        .unwrap();
    assert!(
        check_ok.status.success(),
        "{}",
        String::from_utf8_lossy(&check_ok.stderr)
    );
    assert_eq!(check_ok.stdout, b"(bool true)\n");

    // Now try using `local_val` inside `helper.fl` without defining it there.
    std::fs::write(
        dir.join("helper.fl"),
        r#"
            pub fn from_helper() -> i32 { $local_val() }
        "#,
    )
    .unwrap();
    let check_err = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
        .arg("check")
        .arg(dir.join("lib.fl"))
        .args(["--sysroot"])
        .arg(&library)
        .args(["--build-target", "x86_64", "linux", "64"])
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!check_err.status.success());
    let stderr = String::from_utf8_lossy(&check_err.stderr);
    assert!(stderr.contains("undefined template macro"), "{stderr}");
}

#[test]
fn template_macros_emit_ast_hir_and_reject_invalid_calls_and_post_expansion_type_errors() {
    let path = unique_temp_path("luna-macro-emit", ".fl");
    std::fs::write(
        &path,
        r#"
            typealias i32 = .i32'builtin;
            define id(x) { $x }
            fn run() -> i32 { $id(7) }
        "#,
    )
    .unwrap();

    for emit in [Emit::Ast, Emit::Hir, Emit::Nir] {
        let emit_flag = match emit {
            Emit::Ast => "ast",
            Emit::Hir => "hir",
            Emit::Nir => "nir",
            _ => unreachable!(),
        };
        let Command::Compile(options) = cli::parse([
            path.as_os_str().to_owned(),
            "--emit".into(),
            emit_flag.into(),
        ])
        .unwrap() else {
            panic!("expected compile")
        };
        assert!(frontend::compile(&options).is_ok(), "emit {emit_flag}");
    }
    let _ = std::fs::remove_file(&path);

    let err_undef = run_sysroot_interpret(
        r#"
            fn run() -> i32 { $not_defined(1) }
        "#,
        "run",
    )
    .unwrap_err();
    assert!(
        err_undef.contains("undefined template macro"),
        "{err_undef}"
    );

    let err_type = support::compile_with_protocols(
        r#"
            typealias i32 = .i32'builtin;
            define bad_val() { true }
            fn run() -> i32 { $bad_val() }
        "#,
        &Default::default(),
    )
    .unwrap_err();
    assert!(!err_type.is_empty());
}
