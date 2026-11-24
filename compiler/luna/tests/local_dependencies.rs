//! Integration tests for GAP-02-26 (`docs/compilation/local-dependencies.md`):
//! - Sibling argument inference (`apply_first(|item| item + 1, ...)`) native -O0 / -O3
//! - Producer failure diagnostic without deadlock (`missing_input`)
//! - Avoiding unnecessary waits on unrelated layout (`Type.size_of(*Link)`)
//! - Out-of-order / dependent input scheduling independence
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-local-deps-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self, opt_level: &str) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "build",
                "main.fl",
                "--entry",
                "run",
                "--output",
                "program",
                "--opt-level",
                opt_level,
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library"))
            .output()
            .unwrap()
    }

    fn run(&self) -> Output {
        ProcessCommand::new(self.0.join("program"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for NativeCase {
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
fn canonical_apply_first_suspends_lambda_for_sibling_and_runs_native() {
    let source = r#"
        use std.prelude.*;

        fn apply_first(function: fn(T) -> U, value: T) -> U
        where T, U
        {
            function(value)
        }

        fn apply_second(value: T, function: fn(T) -> U) -> U
        where T, U
        {
            function(value)
        }

        fn run() -> i32 {
            let forty_one: i32 = 41;
            let twenty: i32 = 20;
            let direct = apply_first(
                |item| if item >= 0 and item < 1000 { item + 1 } else { item },
                forty_one,
            );
            let _ = println("{}", direct);
            let nested = apply_first(
                |item| if item >= 0 and item < 1000 { item + 1 } else { item },
                apply_second(
                    twenty,
                    |x| if x >= 0 and x < 1000 { x * 2 } else { x },
                ),
            );
            if direct == 42 and nested == 41 {
                direct
            } else {
                0
            }
        }
    "#;

    let case = NativeCase::new(source);
    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let output = case.run();
        assert_success(&output);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "42\n(signed-integer 42)\n",
            "opt-level -O{opt}"
        );
    }
}

#[test]
fn canonical_missing_input_reports_producer_failure_without_deadlock() {
    let unbound_source = r#"
        fn apply_first(function: fn(T) -> U, value: T) -> U
        where T, U
        {
            function(value)
        }

        fn missing_input() -> i32 {
            apply_first(|item| item + 1, unknown_value)
        }
    "#;
    let err = support::compile(unbound_source).unwrap_err();
    assert!(
        err.contains("unknown_value"),
        "expected diagnostic mentioning unknown_value, got: {err}"
    );

    // Also test an elaboration-time failure in the sibling producer:
    // the lambda must not remain stuck waiting or fail solely with an unresolved lambda error.
    let elaboration_producer_failure = r#"
        fn apply_first(function: fn(T) -> U, value: T) -> U
        where T, U
        {
            function(value)
        }

        fn broken_producer() -> i32 {
            apply_first(|item| item + 1, do { let x: i32 = 1; x + true })
        }
    "#;
    let elab_err = support::compile(elaboration_producer_failure).unwrap_err();
    assert!(
        !elab_err.contains("generic argument context is unresolved"),
        "lambda wait masked producer failure: {elab_err}"
    );
}

#[test]
fn canonical_pointer_size_of_does_not_wait_for_recursive_struct_layout() {
    let source = r#"
        struct Link {
            value: u32,
            next: ?*Link,
        }

        struct GenericLink where T {
            value: T,
            next: ?*GenericLink<T>,
        }

        const POINTER_BYTES = Type.size_of(*Link);
        const GENERIC_POINTER_BYTES = Type.size_of(*GenericLink<u32>);

        comptime fn verify_pointer_sizes() -> bool {
            assumes POINTER_BYTES == Type.size_of(usize);
            assumes GENERIC_POINTER_BYTES == Type.size_of(usize);
            true
        }

        const CHECKED = verify_pointer_sizes();

        fn run() -> bool {
            CHECKED
                and POINTER_BYTES == Type.size_of(usize)
                and GENERIC_POINTER_BYTES == Type.size_of(usize)
        }
    "#;

    assert_eq!(support::run(source), Value::Bool(true));

    let case = NativeCase::new(source);
    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n", "opt-level -O{opt}");
    }
}

#[test]
fn out_of_order_dependent_inputs_and_speculative_lambda_probes_agree() {
    let source = r#"
        typealias Boxed<T> = T;

        fn make_pair(
            implicit first: Boxed<T>,
            implicit second: fn(T) -> U,
            .tag: bool = true,
        ) -> Tuple<U, bool>
        where T, U
        {
            (second(first), tag)
        }

        fn choose(flag: bool) -> i32 {
            let thirty_two: i32 = 32;
            let ten: i32 = 10;
            let pair = if flag {
                make_pair(
                    .second = |x| if x >= 0 and x < 1000 { x + 10 } else { x },
                    .first = thirty_two,
                )
            } else {
                make_pair(
                    .first = ten,
                    .second = |x| if x >= 0 and x < 1000 { x * 2 } else { x },
                    .tag = false,
                )
            };
            let (n, b) = pair;
            if b {
                n
            } else if n >= 0 and n < 1000 {
                n + 1
            } else {
                n
            }
        }

        fn run() -> i32 {
            if choose(true) == 42 and choose(false) == 21 {
                42
            } else {
                0
            }
        }
    "#;

    let case = NativeCase::new(source);
    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(signed-integer 42)\n", "opt-level -O{opt}");
    }
}
