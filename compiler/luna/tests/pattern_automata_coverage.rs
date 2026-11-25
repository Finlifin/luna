//! Integration tests for `pattern_automata` coverage, list `Or`+`Bind` specialization,
//! and unreachable match arm warnings (`W_UNREACHABLE_MATCH_ARM`).
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct CliCase(PathBuf);

impl CliCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-pa-cov-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn library_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
    }

    fn run_cli(&self, extra: &[&str]) -> Output {
        let bin = std::env::var_os("CARGO_BIN_EXE_luna")
            .unwrap_or_else(|| std::ffi::OsString::from(env!("CARGO_BIN_EXE_luna")));
        ProcessCommand::new(bin)
            .current_dir(&self.0)
            .args(extra)
            .arg("--sysroot")
            .arg(Self::library_dir())
            .output()
            .unwrap()
    }

    fn dump_nir(&self) -> Output {
        self.run_cli(&[
            "dump",
            "main.fl",
            "--build-target",
            "x86_64",
            "linux",
            "64",
            "--emit",
            "nir",
        ])
    }

    fn build_and_run(&self) -> Output {
        let build = self.run_cli(&[
            "build",
            "main.fl",
            "--entry",
            "run",
            "--output",
            "program",
            "--opt-level",
            "0",
            "--fuel",
            "10000000",
        ]);
        assert!(
            build.status.success(),
            "build failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        ProcessCommand::new(self.0.join("program"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for CliCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn list_or_pattern_with_bind_compiles_match_plan_and_preserves_bindings() {
    let source = r#"
        use std.prelude.*;
        use std.mem.AllocErr;

        comptime fn pick_comptime(xs: meta.List<Integer>) -> Integer {
            xs match {
                ([1] as ys) or ([2] as ys) => ys.length() + ys[0],
                ([3] as ys) or (_ as ys) => ys.length() * 10,
            }
        }

        fn pick_slice(s: Slice<i64>) -> usize {
            s match {
                ([1] as ys) or ([2] as ys) => ys.ref.len(),
                _ => 99,
            }
        }

        const CT_OK: bool =
            pick_comptime([1]) == 2
                and pick_comptime([2]) == 3
                and pick_comptime([3]) == 10
                and pick_comptime([4, 5]) == 20;

        comptime fn check_ct() -> bool {
            CT_OK
        }

        fn exercise() -> !AllocErr bool {
            var v1 = Vec<i64>.new();
            v1.ref.push(2)!;
            var v2 = Vec<i64>.new();
            v2.ref.push(7)!;
            v2.ref.push(8)!;
            CT_OK
                and pick_slice(v1.ref.as_slice()) == 1
                and pick_slice(v2.ref.as_slice()) == 99
        }

        fn run() -> bool {
            exercise() match {
                ok! => ok,
                error _ => false,
            }
        }
    "#;

    let pkg = support::compile(source).unwrap();
    for name in ["pick_comptime", "pick_slice"] {
        let fn_id = support::source_function(&pkg, name);
        let plans: Vec<_> = pkg
            .match_plans
            .plans
            .iter()
            .filter(|(k, _)| k.function == Some(fn_id))
            .map(|(_, p)| p)
            .collect();
        assert_eq!(plans.len(), 1, "expected 1 match plan for {name}");
        assert!(plans[0].is_exhaustive, "{name} plan must be exhaustive");
        assert!(
            plans[0].exact,
            "{name} plan must remain exact for Or+Bind list patterns"
        );
        assert!(
            plans[0].unreachable_arms.is_empty(),
            "{name} must have no unreachable arms: {:?}",
            plans[0].unreachable_arms
        );
    }

    assert_eq!(
        support::invoke(source, "check_ct").unwrap(),
        Value::Bool(true)
    );

    let cli = CliCase::new(source);
    let dump = cli.dump_nir();
    assert_success(&dump);
    let stderr = String::from_utf8_lossy(&dump.stderr);
    assert!(
        !stderr.contains("W_UNREACHABLE_MATCH_ARM"),
        "unexpected unreachable warning:\n{stderr}"
    );
    let run_out = cli.build_and_run();
    assert_success(&run_out);
    assert_eq!(run_out.stdout, b"(bool true)\n");
}

#[test]
fn unreachable_match_arm_emits_warning_without_breaking_guards_or_floats() {
    let shadowed_source = r#"
        fn classify(x: i64) -> i64 {
            x match {
                0..=10 => 1,
                5 => 2,
                _ => 3,
            }
        }

        fn shadowed_after_wild(flag: bool) -> i64 {
            flag match {
                _ => 0,
                true => 1,
            }
        }

        fn run() -> bool {
            classify(5) == 1 and shadowed_after_wild(true) == 0
        }
    "#;

    let shadowed_case = CliCase::new(shadowed_source);
    let shadowed_dump = shadowed_case.dump_nir();
    assert_success(&shadowed_dump);
    let stderr = String::from_utf8_lossy(&shadowed_dump.stderr);
    assert_eq!(
        stderr.matches("W_UNREACHABLE_MATCH_ARM").count(),
        2,
        "expected two W_UNREACHABLE_MATCH_ARM warnings:\n{stderr}"
    );
    assert!(stderr.contains("arm 2"), "{stderr}");
    assert_eq!(support::run(shadowed_source), Value::Bool(true));

    // Guarded arms, floats, negations, and uninhabited Never matches must NOT emit
    // spurious W_UNREACHABLE_MATCH_ARM warnings.
    let clean_source = r#"
        fn guarded(x: i64) -> i64 {
            x match {
                n if n > 10 => n,
                11 => 11,
                _ => 0,
            }
        }

        fn float_match(f: f64) -> bool {
            f match {
                0.0 => true,
                1.0 => true,
                _ => false,
            }
        }

        fn negated_match(x: i64) -> bool {
            x match {
                not 1 => false,
                1 => true,
            }
        }

        enum Empty {}

        fn absurd_empty(x: Empty) -> i64 {
            x match {
                _ => 0,
            }
        }

        fn run() -> bool {
            guarded(11) == 11 and float_match(0.0) and negated_match(1)
        }
    "#;

    let clean_case = CliCase::new(clean_source);
    let clean_dump = clean_case.dump_nir();
    assert_success(&clean_dump);
    let clean_stderr = String::from_utf8_lossy(&clean_dump.stderr);
    assert!(
        !clean_stderr.contains("W_UNREACHABLE_MATCH_ARM"),
        "unexpected W_UNREACHABLE_MATCH_ARM warning:\n{clean_stderr}"
    );
    assert_eq!(support::run(clean_source), Value::Bool(true));
}
