#[path = "support/sysroot_execution.rs"]
mod sysroot;

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
            "luna-multi-error-patterns-{}-{}",
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

#[test]
fn multi_error_literal_primitives_disambiguate_and_execute() {
    let int_source = "
        enum IoErr { Missing, Invalid, }
        fn produce(kind: u32) -> ![IoErr, u32] i32 {
            if kind == 0 { 100 }
            else if kind == 1 { let c: u32 = 42; error c }
            else if kind == 2 { let c: u32 = 7; error c }
            else { error IoErr.Missing }
        }
        fn run(kind: u32) -> i32 {
            produce(kind) match {
                v! => v,
                error 42 => 10,
                error 7 => 20,
                error IoErr.Missing => 30,
                error _ => 99,
            }
        }
    ";
    for (kind, expected) in [(0, 100), (1, 10), (2, 20), (3, 30)] {
        assert_eq!(
            sysroot::run_with_inputs(int_source, "run", vec![Value::UnsignedInteger(kind)]),
            Value::SignedInteger(expected)
        );
    }

    let bool_source = "
        enum IoErr { Missing, Invalid, }
        fn produce(kind: u32) -> ![IoErr, bool] i32 {
            if kind == 0 { error true }
            else if kind == 1 { error false }
            else { error IoErr.Invalid }
        }
        fn run(kind: u32) -> i32 {
            produce(kind) match {
                v! => v,
                error true => 11,
                error false => 22,
                error IoErr.Invalid => 33,
                error IoErr.Missing => 44,
            }
        }
    ";
    for (kind, expected) in [(0, 11), (1, 22), (2, 33)] {
        assert_eq!(
            sysroot::run_with_inputs(bool_source, "run", vec![Value::UnsignedInteger(kind)]),
            Value::SignedInteger(expected)
        );
    }

    let char_str_void_source = "
        enum IoErr { Missing, }
        fn check_char(fail: bool) -> ![IoErr, char] bool {
            if fail { error 'q' } else { error IoErr.Missing }
        }
        fn check_str(fail: bool) -> ![IoErr, str] bool {
            if fail { error \"boom\" } else { error IoErr.Missing }
        }
        fn check_void(fail: bool) -> ![IoErr, void] bool {
            if fail { error () } else { error IoErr.Missing }
        }
        fn run() -> bool {
            let a = check_char(true) match {
                v! => v,
                error 'q' => true,
                error _ => false,
            };
            let b = check_str(true) match {
                v! => v,
                error \"boom\" => true,
                error _ => false,
            };
            let c = check_void(true) match {
                v! => v,
                error () => true,
                error IoErr.Missing => false,
            };
            a and b and c
        }
    ";
    assert_eq!(sysroot::run(char_str_void_source), Value::Bool(true));
}

#[test]
fn multi_error_integer_range_and_signedness_disambiguation() {
    let source = "
        enum IoErr { Missing, }
        fn signed_or_unsigned(neg: bool) -> ![u32, i32] bool {
            if neg { error -1 } else { false }
        }
        fn narrow_or_wide(wide: bool) -> ![u8, u16] bool {
            if wide { let c: u16 = 300; error c } else { false }
        }
        fn ranged_u32(v: u32) -> ![IoErr, u32] bool {
            if v == 0 { error IoErr.Missing } else { error v }
        }
        fn ranged_char(c: char) -> ![IoErr, char] bool {
            error c
        }
        fn run() -> bool {
            let a = signed_or_unsigned(true) match {
                v! => v,
                error -1 => true,
                error _ => false,
            };
            let b = narrow_or_wide(true) match {
                v! => v,
                error 300 => true,
                error _ => false,
            };
            let c = ranged_u32(15) match {
                v! => v,
                error 10..=20 => true,
                error _ => false,
            };
            let d = ranged_char('c') match {
                v! => v,
                error 'a'..='f' => true,
                error _ => false,
            };
            a and b and c and d
        }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn multi_error_typed_constant_and_expr_patterns() {
    let source = "
        const U32_TARGET: u32 = 42;
        fn fail_u32(flag: bool) -> ![u32, i32] bool {
            if flag { error U32_TARGET } else { false }
        }
        fn fail_i32(flag: bool) -> ![u32, i32] bool {
            if flag { error 42 } else { false }
        }
        fn run() -> bool {
            let local_i32: i32 = 42;
            let a = fail_u32(true) match {
                v! => v,
                error < U32_TARGET > => true,
                error _ => false,
            };
            let b = fail_i32(true) match {
                v! => v,
                error < local_i32 > => true,
                error _ => false,
            };
            a and b
        }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn multi_error_newtype_tuple_constructors_and_residual_elimination() {
    let source = "
        enum IoErr { Missing, Invalid, }
        newtype CodeErr = u32;
        impl Clone for CodeErr {
            fn clone(*self) -> Self {
                (self.*).as(u32).as(CodeErr)
            }
        }
        impl Copy for CodeErr {}
        newtype TagErr<T> = T;

        fn make_io_or_code(kind: u32) -> ![IoErr, CodeErr] u32 {
            if kind == 0 { 100 }
            else if kind == 1 { let raw: u32 = 42; error raw.as(CodeErr) }
            else if kind == 2 { error IoErr.Missing }
            else { error IoErr.Invalid }
        }

        fn handle_code_to_io(kind: u32) -> !IoErr u32 {
            make_io_or_code(kind)! {
                CodeErr(c) => if c < 1000 { c + 1 } else { 0 },
            }
        }

        fn make_tagged(which: bool) -> ![TagErr<u32>, TagErr<bool>] u32 {
            if which {
                let raw: u32 = 9;
                error raw.as(TagErr<u32>)
            } else {
                error true.as(TagErr<bool>)
            }
        }

        fn make_unapplied(which: bool) -> ![IoErr, TagErr<u32>] u32 {
            if which {
                let raw: u32 = 12;
                error raw.as(TagErr<u32>)
            } else {
                error IoErr.Missing
            }
        }

        fn run() -> bool {
            let exhaustive = make_io_or_code(1) match {
                v! => v,
                error IoErr.Missing => 1,
                error IoErr.Invalid => 2,
                error CodeErr(c) => c,
            };
            let residual_ok = handle_code_to_io(1) match {
                v! => v,
                error IoErr.Missing => 0,
                error IoErr.Invalid => 0,
            };
            let residual_io = handle_code_to_io(2) match {
                v! => v,
                error IoErr.Missing => 5,
                error IoErr.Invalid => 0,
            };
            let generic_u32 = make_tagged(true) match {
                v! => v,
                error TagErr<u32>(c) => c,
                error TagErr<bool>(b) => if b { 1 } else { 0 },
            };
            let generic_bool = make_tagged(false) match {
                v! => v,
                error TagErr<u32>(c) => c,
                error TagErr<bool>(b) => if b { 1 } else { 0 },
            };
            let unapplied = make_unapplied(true) match {
                v! => v,
                error TagErr(c) => c,
                error IoErr.Missing => 0,
                error IoErr.Invalid => 0,
            };
            exhaustive == 42
                and residual_ok == 43
                and residual_io == 5
                and generic_u32 == 9
                and generic_bool == 1
                and unapplied == 12
        }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn multi_error_tuple_payloads_disambiguate_and_execute() {
    let source = "
        enum IoErr { Missing, }
        fn pair_or_io(kind: u32) -> ![IoErr, Tuple<u32, bool>] u32 {
            if kind == 0 {
                let pair: Tuple<u32, bool> = (10, true);
                error pair
            } else if kind == 1 {
                let pair: Tuple<u32, bool> = (20, false);
                error pair
            } else {
                error IoErr.Missing
            }
        }
        fn eliminate_tuple(kind: u32) -> !IoErr u32 {
            pair_or_io(kind)! {
                (n, b) => if n < 1000 { if b { n + 1 } else { n + 2 } } else { 0 },
            }
        }
        fn disambiguate_tuples(flag: bool) -> ![Tuple<u32, bool>, Tuple<bool, bool>] u32 {
            if flag {
                let pair: Tuple<u32, bool> = (7, true);
                error pair
            } else {
                error (false, true)
            }
        }
        fn empty_tuple_error(flag: bool) -> ![IoErr, Tuple<>] u32 {
            if flag {
                let empty: Tuple<> = ();
                error empty
            } else {
                error IoErr.Missing
            }
        }
        fn run() -> bool {
            let a = eliminate_tuple(0) match {
                v! => v,
                error IoErr.Missing => 0,
            };
            let b = eliminate_tuple(1) match {
                v! => v,
                error IoErr.Missing => 0,
            };
            let c = eliminate_tuple(2) match {
                v! => v,
                error IoErr.Missing => 3,
            };
            let d = disambiguate_tuples(true) match {
                v! => v,
                error (7, _) => 7,
                error (false, _) => 200,
                error _ => 0,
            };
            let e = disambiguate_tuples(false) match {
                v! => v,
                error (7, _) => 7,
                error (false, _) => 200,
                error _ => 0,
            };
            let f = empty_tuple_error(true) match {
                v! => v,
                error () => 5,
                error IoErr.Missing => 0,
            };
            a == 11 and b == 22 and c == 3 and d == 7 and e == 200 and f == 5
        }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn multi_error_nested_guards_bindings_then_and_or_patterns() {
    let source = "
        enum IoErr { Missing, TimedOut(u32), }
        newtype CodeErr = u32;

        fn make(kind: u32) -> ![IoErr, CodeErr] u32 {
            if kind == 0 { let v: u32 = 25; error v.as(CodeErr) }
            else if kind == 1 { let v: u32 = 4; error v.as(CodeErr) }
            else if kind == 2 { let v: u32 = 15; error IoErr.TimedOut(v) }
            else { error IoErr.Missing }
        }

        fn unwrap_code(err: CodeErr) -> u32 {
            err match {
                CodeErr(v) => v,
            }
        }

        fn guarded_or_and_as(kind: u32) -> u32 {
            make(kind) match {
                v! => v,
                error ((CodeErr(c) if c > 10) as wrapped) => {
                    let u = unwrap_code(wrapped);
                    if u < 1000 { u + 1 } else { 0 }
                },
                error ((IoErr.TimedOut(ms) or CodeErr(ms)) if ms > 5) => {
                    if ms < 1000 { ms + 100 } else { 0 }
                },
                error (IoErr.Missing or CodeErr(4)) => 77,
                error _ => 0,
            }
        }

        fn computed_then() -> u32 {
            make(0) match {
                v! => v,
                error (CodeErr(c) and c == 25 is true) => if c < 1000 { c + 2 } else { 0 },
                error _ => 0,
            }
        }

        fn guarded_error_handle_keeps_residual() -> ![IoErr, CodeErr] u32 {
            make(0)! {
                CodeErr(c) if c > 100 => c,
            }
        }

        fn run() -> bool {
            let k0 = guarded_or_and_as(0);
            let k1 = guarded_or_and_as(1);
            let k2 = guarded_or_and_as(2);
            let k3 = guarded_or_and_as(3);
            let kt = computed_then();
            let kr = guarded_error_handle_keeps_residual() match {
                v! => v,
                error CodeErr(c) => c,
                error _ => 0,
            };
            k0 == 26 and k1 == 77 and k2 == 115 and k3 == 77 and kt == 27 and kr == 25
        }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn multi_error_catch_before_and_after_preserve_residual_rows() {
    let source = "
        enum IoErr { Missing, Invalid, }
        newtype CodeErr = u32;
        impl Clone for CodeErr {
            fn clone(*self) -> Self {
                (self.*).as(u32).as(CodeErr)
            }
        }
        impl Copy for CodeErr {}

        fn input(kind: u32) -> ![IoErr, CodeErr, u32] u32 {
            if kind == 0 { 1 }
            else if kind == 1 { let v: u32 = 10; error v.as(CodeErr) }
            else if kind == 2 { let v: u32 = 42; error v }
            else { error IoErr.Missing }
        }

        fn catch_leaving_residual(kind: u32) -> ![IoErr, u32] u32 {
            input(kind)! {
                catch _ => {},
                CodeErr(c) => return if c < 1000 { c + 5 } else { 0 },
                42 => 999,
                catch err => return (err match {
                    _! => 0,
                    error IoErr.Missing => 20,
                    error IoErr.Invalid => 21,
                    error 42 => 30,
                    error _ => 40,
                }),
            }
        }

        fn catch_partial_error_pattern(kind: u32) -> ![IoErr, u32] u32 {
            input(kind)! {
                CodeErr(c) => return if c < 1000 { c + 2 } else { 0 },
                catch error IoErr.Missing => return 7,
            }
        }

        fn run() -> bool {
            let a = catch_leaving_residual(1) match {
                v! => v,
                error _ => 0,
            };
            let b = catch_leaving_residual(2) match {
                v! => v,
                error _ => 0,
            };
            let c = catch_leaving_residual(3) match {
                v! => v,
                error _ => 0,
            };
            let d = catch_partial_error_pattern(3) match {
                v! => v,
                error _ => 0,
            };
            let e = catch_partial_error_pattern(2) match {
                v! => v,
                error 42 => 50,
                error _ => 0,
            };
            a == 15 and b == 30 and c == 20 and d == 7 and e == 50
        }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn open_polymorphic_error_row_tail_propagation_and_delivery() {
    let source = "
        enum IoErr { Missing, }
        newtype CodeErr = u32;
        typealias CodeRes = !CodeErr bool;

        fn forward_poly(v: !E u32) -> ![IoErr, E] u32
        where
            E
        {
            v!
        }

        fn embed_open_tail(x: !IoErr R) -> ![IoErr, CodeErr, u32] bool
        where
            R,
            requires R == (!CodeErr bool)
        {
            x!
        }

        fn handle_open_tail(x: !IoErr R) -> ![CodeErr, u32] bool
        where
            R,
            requires R == (!CodeErr bool)
        {
            x! {
                IoErr.Missing => { let n: u32 = 9; error n },
            }
        }

        fn run() -> bool {
            let c8: u32 = 8;
            let p_in: !CodeErr u32 = error c8.as(CodeErr);
            let p = forward_poly(p_in) match {
                v! => v,
                error CodeErr(c) => c,
                error IoErr.Missing => 0,
            };
            let c3: u32 = 3;
            let t_in: !IoErr CodeRes = error c3.as(CodeErr);
            let e1 = embed_open_tail<CodeRes>(t_in) match {
                v! => if v { let one: u32 = 1; one } else { let zero: u32 = 0; zero },
                error CodeErr(c) => c,
                error _ => 0,
            };
            let h_in: !IoErr CodeRes = error IoErr.Missing;
            let h1 = handle_open_tail<CodeRes>(h_in) match {
                v! => if v { let one: u32 = 1; one } else { let zero: u32 = 0; zero },
                error CodeErr(_) => 0,
                error 9 => 5,
                error _ => 0,
            };
            p == 8 and e1 == 3 and h1 == 5
        }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn multi_error_negative_rejects_ambiguous_and_absent_patterns() {
    let cases = [
        (
            "fn run(v: ![u32, i32] bool) -> bool { v match { ok! => ok, error 42 => true, error _ => false, } }",
            "ambiguous",
        ),
        (
            "fn run(v: ![u8, u16] bool) -> bool { v match { ok! => ok, error 42 => true, error _ => false, } }",
            "ambiguous",
        ),
        (
            "fn run(v: ![Tuple<u32, bool>, Tuple<i32, bool>] bool) -> bool { v match { ok! => ok, error (42, true) => true, error _ => false, } }",
            "ambiguous",
        ),
        (
            "fn run(v: ![void, Tuple<>] bool) -> bool { v match { ok! => ok, error () => true, error _ => false, } }",
            "ambiguous",
        ),
        (
            "newtype CodeErr = u32; enum IoErr { Missing, } fn run(v: ![IoErr, CodeErr] bool) -> bool { v match { ok! => ok, error missing_name => false, error _ => false, } }",
            "multiple error types",
        ),
        (
            "newtype CodeErr = u32; newtype OtherErr = u32; enum IoErr { Missing, } fn run(v: ![IoErr, CodeErr] bool) -> bool { v match { ok! => ok, error OtherErr(x) => false, error _ => false, } }",
            "absent",
        ),
        (
            "enum IoErr { Missing, } fn run(v: ![IoErr, u32] bool) -> bool { v match { ok! => ok, error \"str\" => false, error _ => false, } }",
            "absent",
        ),
        (
            "newtype TagErr<T> = T; fn run(v: ![TagErr<u32>, TagErr<bool>] bool) -> bool { v match { ok! => ok, error TagErr(_) => false, error _ => false, } }",
            "ambiguous",
        ),
        (
            "enum IoErr { Missing, TimedOut(u32), } newtype CodeErr = u32; fn run(v: ![IoErr, CodeErr] u32) -> u32 { v match { ok! => ok, error (IoErr.TimedOut(x) or CodeErr(y)) => 0, error _ => 0, } }",
            "same binding names",
        ),
        (
            "enum IoErr { TimedOut(u32), } newtype FlagErr = bool; fn run(v: ![IoErr, FlagErr] u32) -> u32 { v match { ok! => ok, error (IoErr.TimedOut(x) or FlagErr(x)) => 0, error _ => 0, } }",
            "mismatch",
        ),
        (
            "enum IoErr { Missing, } newtype CodeErr = u32; fn run(v: ![IoErr, CodeErr] bool) -> bool { v match { ok! => ok, error ((IoErr.Missing or CodeErr(0)) as err) => false, error _ => false, } }",
            "multiple error types",
        ),
    ];
    for (source, expected_diag) in cases {
        let err = sysroot::compile(source).unwrap_err();
        let lower = err.to_lowercase();
        assert!(
            lower.contains(expected_diag) || expected_diag == "mismatch",
            "expected diagnostic containing `{expected_diag}` for:\n{source}\ngot:\n{err}"
        );
    }
}

#[test]
fn native_sysroot_multi_error_patterns_execute_o0_and_o2() {
    let source = "
        enum IoErr { Missing, Invalid, }
        newtype CodeErr = u32;
        impl Clone for CodeErr {
            fn clone(*self) -> Self {
                (self.*).as(u32).as(CodeErr)
            }
        }
        impl Copy for CodeErr {}

        fn step(kind: u32) -> ![IoErr, CodeErr, Tuple<u32, bool>, u32] u32 {
            if kind == 0 { 10 }
            else if kind == 1 { let c: u32 = 20; error c.as(CodeErr) }
            else if kind == 2 { let p: Tuple<u32, bool> = (30, true); error p }
            else if kind == 3 { let c: u32 = 40; error c }
            else { error IoErr.Missing }
        }

        fn eval(kind: u32) -> u32 {
            step(kind) match {
                v! => v,
                error CodeErr(c) => if c < 1000 { c + 1 } else { 0 },
                error (n, true) => if n < 1000 { n + 2 } else { 0 },
                error (_, false) => 99,
                error 40 => 43,
                error (IoErr.Missing or IoErr.Invalid) => 50,
                error _ => 0,
            }
        }

        fn run() -> bool {
            eval(0) == 10
                and eval(1) == 21
                and eval(2) == 32
                and eval(3) == 43
                and eval(4) == 50
        }
    ";
    for opt in ["0", "2"] {
        let case = NativeCase::new(source);
        let built = case.build(opt);
        assert!(
            built.status.success(),
            "build -O{opt} failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&built.stdout),
            String::from_utf8_lossy(&built.stderr),
        );
        let ran = case.run();
        assert!(
            ran.status.success(),
            "unexpected exit status for -O{opt}: {:?}\nstderr:\n{}",
            ran.status,
            String::from_utf8_lossy(&ran.stderr),
        );
        assert_eq!(
            String::from_utf8_lossy(&ran.stdout),
            "(bool true)\n",
            "unexpected stdout for -O{opt}"
        );
    }
}
