#[path = "support/sysroot_execution.rs"]
mod sysroot;

use comptime_interpretor::Value;

#[test]
fn inline_and_pure_inline_function_types_execute_and_widen() {
    let source = r#"
inline fn step(x: i64) -> i64 {
    if x >= 0 and x <= 1000 { x + 3 } else { 0 }
}

pure inline fn pure_step(x: i64) -> i64 {
    if x >= 0 and x <= 1000 { x * 2 } else { 0 }
}

fn apply_inline(f: inline fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn apply_pure_inline(f: pure inline fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn apply_inline_pure(f: inline pure fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn apply_pure(f: pure fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn apply_plain(f: fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn apply_inline_unsafe(f: inline unsafe fn(*i64) -> i64, p: *i64) -> i64 {
    unsafe { f(p) }
}

unsafe fn deref_inc(p: *i64) -> i64 {
    let v = p.*;
    if v >= 0 and v <= 1000 { v + 1 } else { 0 }
}

pub fn run() -> i64 {
    let n: i64 = 10;
    let a = apply_inline(step, 4);
    let b = apply_pure_inline(pure_step, 5);
    let c = apply_inline_pure(pure_step, 6);
    let d = apply_pure(pure_step, 7);
    let e = apply_plain(step, 8);
    let f = apply_inline_unsafe(deref_inc, n.ref);
    if a >= 0 and a <= 1000 and b >= 0 and b <= 1000 and c >= 0 and c <= 1000 and d >= 0 and d <= 1000 and e >= 0 and e <= 1000 and f >= 0 and f <= 1000 {
        a + b + c + d + e + f
    } else {
        0
    }
}
"#;
    assert_eq!(
        sysroot::run(source),
        Value::SignedInteger(7 + 10 + 12 + 14 + 11 + 11)
    );
}

#[test]
fn comptime_inline_function_types_compile_and_invoke() {
    let source = r#"
comptime inline fn double(x: i64) -> i64 {
    if x >= 0 and x <= 1000 { x * 2 } else { 0 }
}

comptime fn apply(f: comptime inline fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

pub comptime fn run() -> i64 {
    apply(double, 21)
}
"#;
    assert_eq!(
        sysroot::invoke(source, "run").unwrap(),
        Value::SignedInteger(42)
    );
}

#[test]
fn rejected_function_type_modifiers_and_parameter_erasures() {
    for source in [
        "fn bad(f: spec fn(i64) -> i64) -> i64 { f(1) }",
        "fn bad(f: verified fn(i64) -> i64) -> i64 { f(1) }",
        "fn bad(f: spec unsafe fn(i64) -> i64) -> i64 { 0 }",
        "fn bad(f: verified unsafe fn(i64) -> i64) -> i64 { 0 }",
        "fn bad(f: pure unsafe fn(i64) -> i64) -> i64 { 0 }",
        "fn bad(f: spec verified fn(i64) -> i64) -> i64 { 0 }",
        "fn bad(f: fn(.x: i64) -> i64) -> i64 { 0 }",
        "fn bad(f: fn(...xs: i64) -> i64) -> i64 { 0 }",
        "fn bad(f: fn(implicit i64) -> i64) -> i64 { 0 }",
        "fn bad(f: fn(lambda i64) -> i64) -> i64 { 0 }",
    ] {
        let err = sysroot::compile(source).unwrap_err();
        assert!(
            !err.contains("FileParsingFailed") && !err.contains("[2001]"),
            "unexpected syntax failure for `{source}`: {err}"
        );
    }
}

#[test]
fn closure_explicit_environment_initialization_runs_for_closures_and_dyn_traits() {
    let closure_source = r#"
fn bounded_add(a: i64, b: i64) -> i64 {
    if a >= 0 and a <= 1000 and b >= 0 and b <= 1000 { a + b } else { 0 }
}

fn make_offset(base: i64) -> %{ bias: i64 = base } fn(i64) -> i64 {
    |x| bounded_add(base, x)
}

fn make_pure_offset(base: i64) -> %{ bias: i64 = base } pure fn(i64) -> i64 {
    |x| bounded_add(base, x)
}

fn make_quoted_offset() -> %{ step: i64 = delta } fn(i64) -> i64
where
    quote delta: lift i64,
{
    |x| bounded_add(delta, x)
}

fn compose(
    f: %e1 fn(B) -> C,
    g: %e2 fn(A) -> B,
) -> %{ first: %e1 fn(B) -> C = f, second: %e2 fn(A) -> B = g } fn(A) -> C
where
    A,
    B,
    C,
    e1: Type.vars.Closure,
    e2: Type.vars.Closure,
{
    |x| f(g(x))
}

pub fn run() -> i64 {
    let f1 = make_offset(10);
    let f2 = make_pure_offset(20);
    bounded_add(f1(1), f2(2))
}
"#;
    assert_eq!(sysroot::run(closure_source), Value::SignedInteger(11 + 22));

    let generic_and_quoted_closure_source = r#"
fn bounded_add(a: i64, b: i64) -> i64 {
    if a >= 0 and a <= 1000 and b >= 0 and b <= 1000 { a + b } else { 0 }
}

fn make_offset(base: i64) -> %{ bias: i64 = base } fn(i64) -> i64 {
    |x| bounded_add(base, x)
}

fn make_quoted_offset() -> %{ step: i64 = delta } fn(i64) -> i64
where
    quote delta: lift i64,
{
    |x| bounded_add(delta, x)
}

fn compose(
    f: %e1 fn(B) -> C,
    g: %e2 fn(A) -> B,
) -> %{ first: %e1 fn(B) -> C = f, second: %e2 fn(A) -> B = g } fn(A) -> C
where
    A,
    B,
    C,
    e1: Type.vars.Closure,
    e2: Type.vars.Closure,
{
    |x| f(g(x))
}

pub fn run() -> i64 {
    let f1 = make_offset(10);
    let f1b = make_offset(20);
    let f3 = make_quoted_offset<5>();
    let c = compose(f1b, f1);
    bounded_add(f3(3), c(4))
}
"#;
    let generic_package = sysroot::compile(generic_and_quoted_closure_source).unwrap();
    luna::instantiation::instantiate(&generic_package, "run", 1_000_000).unwrap();

    let dyn_source = r#"
trait Reading {
    fn value(*self) -> i64;
}

struct Scale {
    factor: i64,
}

struct Calibrated
where
    quote scale: lift *Scale,
{
    raw: i64,
}

impl Reading for Calibrated<scale>
where
    quote scale: lift *Scale,
{
    fn value(*self) -> i64 {
        let a = self.raw.*;
        let b = scale.factor.*;
        if a >= 0 and a <= 100 and b >= 0 and b <= 100 { a * b } else { 0 }
    }
}

fn relay(sensor: Calibrated<scale>) -> %{ _0: *Scale = scale } dyn Reading
where
    quote scale: lift *Scale,
{
    sensor.dyn(Reading)
}

pub fn run() -> i64 {
    var s = Scale { factor: 3 };
    let r = relay(Calibrated<s.ref> { raw: 3 });
    r.value()
}
"#;
    let package = sysroot::compile(dyn_source).unwrap();
    luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
}

#[test]
fn closure_explicit_environment_initialization_supports_comptime_closures() {
    let source = r#"
comptime fn make_adder(base: i64) -> %{ bias: i64 = base } comptime fn(i64) -> i64 {
    |x| if base >= 0 and base <= 1000 and x >= 0 and x <= 1000 { base + x } else { 0 }
}

pub comptime fn run() -> i64 {
    let add5 = make_adder(5);
    add5(37)
}
"#;
    assert_eq!(
        sysroot::invoke(source, "run").unwrap(),
        Value::SignedInteger(42)
    );
}

#[test]
fn closure_explicit_environment_initialization_rejects_invalid_sources() {
    let type_mismatch = r#"
fn bad(flag: bool) -> %{ bias: i64 = flag } fn(i64) -> i64 {
    |x| x
}
"#;
    let runtime_in_comptime_env = r#"
fn bad(base: i64) -> %{ bias: i64 = base } comptime fn(i64) -> i64 {
    |x| x
}
"#;
    let duplicate_field = r#"
fn bad(base: i64) -> %{ bias: i64 = base, bias: i64 = base } fn(i64) -> i64 {
    |x| if base >= 0 and base <= 1000 and x >= 0 and x <= 1000 { base + x } else { 0 }
}
"#;
    let missing_source = r#"
fn bad() -> %{ bias: i64 = missing } fn(i64) -> i64 {
    |x| x
}
"#;
    for source in [
        type_mismatch,
        runtime_in_comptime_env,
        duplicate_field,
        missing_source,
    ] {
        assert!(
            sysroot::compile(source).is_err(),
            "expected failure for:\n{source}"
        );
    }
}

#[test]
fn multi_error_row_or_patterns_exhaustiveness_bindings_and_guards() {
    let source = r#"
enum IoErr {
    NotFound,
    BadCode(i64),
    Pair(i64, i64),
}

enum ParseErr {
    Invalid,
    BadToken(i64),
}

enum AuthErr {
    Denied,
    RetryAfter(i64),
}

fn badd(a: i64, b: i64) -> i64 {
    if a >= 0 and a <= 1000000000 and b >= 0 and b <= 1000000000 { a + b } else { 0 }
}

fn bmul(a: i64, b: i64) -> i64 {
    if a >= 0 and a <= 10000 and b >= 0 and b <= 10000 { a * b } else { 0 }
}

fn produce_two(tag: i64) -> ![IoErr, ParseErr] i64 {
    if tag == 0 {
        100
    } else if tag == 1 {
        error IoErr.NotFound
    } else if tag == 2 {
        error ParseErr.Invalid
    } else if tag == 3 {
        error IoErr.BadCode(17)
    } else if tag == 4 {
        error ParseErr.BadToken(23)
    } else {
        error IoErr.Pair(4, 9)
    }
}

fn handle_two_exhaustive(tag: i64) -> i64 {
    produce_two(tag) match {
        ok! => ok,
        error (IoErr.NotFound or ParseErr.Invalid) => 1,
        error (IoErr.BadCode(code) or ParseErr.BadToken(code)) => code,
        error IoErr.Pair(a, b) => badd(a, b),
    }
}

fn produce_three(tag: i64) -> ![IoErr, ParseErr, AuthErr] i64 {
    if tag == 0 {
        error IoErr.NotFound
    } else if tag == 1 {
        error ParseErr.Invalid
    } else if tag == 2 {
        error AuthErr.Denied
    } else if tag == 3 {
        error IoErr.BadCode(15)
    } else if tag == 4 {
        error ParseErr.BadToken(5)
    } else if tag == 5 {
        error AuthErr.RetryAfter(12)
    } else if tag == 6 {
        error IoErr.Pair(7, 8)
    } else {
        50
    }
}

fn handle_three_nested_and_guarded(tag: i64) -> i64 {
    produce_three(tag) match {
        ok! => ok,
        error ((IoErr.NotFound or ParseErr.Invalid) or AuthErr.Denied) => 2,
        (error ((IoErr.BadCode(v) or ParseErr.BadToken(v)) or AuthErr.RetryAfter(v))) if v > 10 => bmul(v, 2),
        error (IoErr.Pair(a, _) or ParseErr.BadToken(a)) => badd(a, 1),
        error (IoErr.BadCode(a) or AuthErr.RetryAfter(a)) => a,
    }
}

fn handle_same_type_as(tag: i64) -> i64 {
    let res: ![IoErr, ParseErr] i64 = if tag == 0 {
        error IoErr.BadCode(31)
    } else if tag == 1 {
        error IoErr.Pair(11, 12)
    } else if tag == 2 {
        error IoErr.NotFound
    } else {
        error ParseErr.Invalid
    };
    res match {
        ok! => ok,
        error ((IoErr.BadCode(v) or IoErr.Pair(v, _)) as err) => {
            err match {
                IoErr.BadCode(x) => badd(x, v),
                IoErr.Pair(a, b) => badd(badd(a, b), v),
                IoErr.NotFound => 0,
            }
        },
        error (IoErr.NotFound or ParseErr.Invalid) => 9,
        error ParseErr.BadToken(t) => t,
    }
}

pub fn run() -> i64 {
    let s1 = badd(
        badd(badd(handle_two_exhaustive(0), handle_two_exhaustive(1)), badd(handle_two_exhaustive(2), handle_two_exhaustive(3))),
        badd(handle_two_exhaustive(4), handle_two_exhaustive(5)),
    );
    let s2 = badd(
        badd(
            badd(handle_three_nested_and_guarded(0), handle_three_nested_and_guarded(1)),
            badd(handle_three_nested_and_guarded(2), handle_three_nested_and_guarded(3)),
        ),
        badd(
            badd(handle_three_nested_and_guarded(4), handle_three_nested_and_guarded(5)),
            badd(handle_three_nested_and_guarded(6), handle_three_nested_and_guarded(7)),
        ),
    );
    let s3 = badd(
        badd(handle_same_type_as(0), handle_same_type_as(1)),
        badd(handle_same_type_as(2), handle_same_type_as(3)),
    );
    badd(badd(bmul(s1, 10000), bmul(s2, 100)), s3)
}
"#;
    let s1 = 100 + 1 + 1 + 17 + 23 + 13; // 155
    let s2 = 2 + 2 + 2 + 30 + 6 + 24 + 8 + 50; // 124
    let s3 = 62 + 34 + 9 + 9; // 114
    assert_eq!(
        sysroot::run(source),
        Value::SignedInteger(s1 * 10000 + s2 * 100 + s3)
    );
}

#[test]
fn multi_error_row_or_patterns_reject_invalid_cases() {
    let non_exhaustive = r#"
enum IoErr { NotFound, BadCode(i64), }
enum ParseErr { Invalid, BadToken(i64), }

fn produce() -> ![IoErr, ParseErr] i64 {
    error IoErr.NotFound
}

pub fn bad() -> i64 {
    produce() match {
        ok! => ok,
        error (IoErr.NotFound or ParseErr.Invalid) => 1,
        error IoErr.BadCode(c) => c,
    }
}
"#;
    let binding_name_mismatch = r#"
enum IoErr { BadCode(i64), }
enum ParseErr { BadToken(i64), }

fn produce() -> ![IoErr, ParseErr] i64 {
    error IoErr.BadCode(1)
}

pub fn bad() -> i64 {
    produce() match {
        ok! => ok,
        error (IoErr.BadCode(a) or ParseErr.BadToken(b)) => a,
    }
}
"#;
    let binding_type_mismatch = r#"
enum IoErr { BadCode(i64), }
enum FlagErr { Failed(bool), }

fn produce() -> ![IoErr, FlagErr] i64 {
    error IoErr.BadCode(1)
}

pub fn bad() -> i64 {
    produce() match {
        ok! => ok,
        error (IoErr.BadCode(x) or FlagErr.Failed(x)) => 0,
    }
}
"#;
    let missing_binding_in_branch = r#"
enum IoErr { BadCode(i64), }
enum ParseErr { Invalid, }

fn produce() -> ![IoErr, ParseErr] i64 {
    error ParseErr.Invalid
}

pub fn bad() -> i64 {
    produce() match {
        ok! => ok,
        error (IoErr.BadCode(x) or ParseErr.Invalid) => x,
    }
}
"#;
    for source in [
        non_exhaustive,
        binding_name_mismatch,
        binding_type_mismatch,
        missing_binding_in_branch,
    ] {
        assert!(
            sysroot::compile(source).is_err(),
            "expected failure for:\n{source}"
        );
    }
}
