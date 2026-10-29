//! Real LLVM assembly, native object emission, linking and execution.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
struct Work(PathBuf);
impl Work {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-host-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("source.fl"), source).unwrap();
        Self(path)
    }
    fn build(&self) -> Output {
        self.command()
            .args(["--build", "run", "--output", "native program"])
            .output()
            .unwrap()
    }
    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_luna"));
        c.current_dir(&self.0).arg("source.fl");
        c
    }
    fn execute(&self) -> Output {
        let mut command = Command::new(self.0.join("native program"));
        command.current_dir(&self.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            match command.output() {
                // Concurrent forks can briefly retain a writer before exec.
                // ETXTBSY means no child started: never retry a completed run
                // or an output-collection error, even when its exit is failure.
                Err(error)
                    if error.raw_os_error() == Some(26) && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(
                        std::time::Duration::from_millis(2)
                            .min(deadline.saturating_duration_since(std::time::Instant::now())),
                    );
                }
                result => return result.unwrap(),
            }
        }
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn success(out: &Output) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn native(source: &str, expected: &str) {
    let w = Work::new(source);
    success(&w.build());
    let out = w.execute();
    success(&out);
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}
#[test]
fn native_integer_contracts() {
    native(
        "typealias U=.u32'builtin; fn div(a:U,b:U)->r:U where requires b != 0, ensures r <= a { a / b } fn run()->U {div(84,2)}",
        "(unsigned-integer 42)\n",
    );
}

#[test]
fn short_circuit_preserves_current_partial_predicates() {
    for source in [
        "typealias U=.u32'builtin; fn check(n:U,d:U)->U {var seen=false;if d != 0 and do {assumes not seen;seen=true;n / d > 1} {assumes seen;n % d} else {if d == 0 {assumes not seen;} 0}} fn run()->bool {check(17,0) == 0 and check(17,3) == 2}",
        "typealias U=.u32'builtin; fn check(n:U,d:U)->U {if d != 0 and n / d > 1 {n % d} else {0}} fn run()->bool {check(17,0) == 0 and check(17,3) == 2}",
        "typealias U=.u32'builtin; fn check(n:U,d:U)->U {if d == 0 or n / d <= 1 {0} else {n % d}} fn run()->bool {check(17,0) == 0 and check(17,3) == 2}",
        "typealias U=.u32'builtin; fn check(n:U,d:U)->U {var divisor=d;while divisor != 0 and n / divisor > 1 {return n % divisor;} 0} fn run()->bool {check(17,0) == 0 and check(17,3) == 2}",
    ] {
        let work = Work::new(source);
        let reference = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for optimization in ["0", "3"] {
            success(
                &work
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        optimization,
                    ])
                    .output()
                    .unwrap(),
            );
            let result = work.execute();
            success(&result);
            assert_eq!(result.stdout, reference.stdout);
        }
    }
}

#[test]
fn short_circuit_partial_predicates_do_not_prove_opposite_or_stale_paths() {
    for source in [
        "typealias U=.u32'builtin; fn check(n:U,d:U)->U {if d != 0 and n / d > 1 {0} else {n % d}} fn run()->U {check(17,0)}",
        "typealias U=.u32'builtin; fn check(n:U,d:U)->U {if d == 0 or n / d <= 1 {n % d} else {0}} fn run()->U {check(17,0)}",
        "typealias U=.u32'builtin; fn check(n:U,d:U)->U {var divisor=d;if divisor != 0 and do {divisor=0;true} {n % divisor} else {0}} fn run()->U {check(17,3)}",
        "typealias U=.u32'builtin; fn check(n:U,d:U)->U {var divisor=d;if divisor == 0 or do {divisor=0;false} {0} else {n % divisor}} fn run()->U {check(17,3)}",
    ] {
        let work = Work::new(source);
        let output = work.build();
        assert!(!output.status.success(), "accepted {source}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("divisor is not proven nonzero"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!work.0.join("native program").exists());
    }
}
#[test]
fn native_boolean_control() {
    native(
        "fn skip()->bool {skip()} fn run()->bool {var active=true;while active {active=false;continue;skip();} not active and (true or skip())}",
        "(bool true)\n",
    );
}
#[test]
fn native_early_returns() {
    native(
        "fn choose(flag:bool)->bool {if flag {return true;} else {return false;}} fn run()->bool {choose(true)}",
        "(bool true)\n",
    );
}
#[test]
fn native_defaults_and_source_order() {
    native(
        "fn choose(a:bool,.b:bool=a)->bool {a and b} fn run()->bool {choose(true)}",
        "(bool true)\n",
    );
}
#[test]
fn native_checked_arithmetic() {
    native(
        "typealias U=.u32'builtin;fn add(a:U,b:U)->U where requires a <= 100, requires b <= 100 {a + b} fn run()->U {add(20,22)}",
        "(unsigned-integer 42)\n",
    );
}
#[test]
fn native_signed_arithmetic() {
    native(
        "typealias I=.i32'builtin;fn neg(a:I)->I where requires a >= 0 { -a } fn run()->I {neg(42)}",
        "(signed-integer -42)\n",
    );
}
#[test]
fn native_unit() {
    native("fn run() {}", "(unit)\n");
}
#[test]
fn unproven_overflow_is_rejected_before_codegen() {
    let w =
        Work::new("typealias U=.u8'builtin;fn add(a:U,b:U)->U {a + b} fn run()->U {add(255,1)}");
    let out = w.build();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("range is not proven"));
}
#[test]
fn predicate_failure_traps() {
    let w =
        Work::new("fn check(flag:bool)->bool {assumes flag;flag} fn run()->bool {check(false)}");
    success(&w.build());
    assert!(!w.execute().status.success());
}
#[test]
fn llvm_export_is_deterministic() {
    let w = Work::new("fn run()->bool {true}");
    let a = w.command().args(["--emit-llvm", "run"]).output().unwrap();
    success(&a);
    let b = w.command().args(["--emit-llvm", "run"]).output().unwrap();
    success(&b);
    assert_eq!(a.stdout, b.stdout);
    assert!(String::from_utf8_lossy(&a.stdout).contains("define i32 @main()"));
}
#[test]
fn missing_tool_preserves_output() {
    let w = Work::new("fn run()->bool {true}");
    fs::write(w.0.join("native program"), b"old\0artifact").unwrap();
    let out = w
        .command()
        .args(["--build", "run", "--output", "native program"])
        .env("LUNA_LLVM_CONFIG", w.0.join("missing-llvm"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(
        fs::read(w.0.join("native program")).unwrap(),
        b"old\0artifact"
    );
}
#[test]
fn source_cannot_be_overwritten() {
    let source = "fn run()->bool {true}";
    let w = Work::new(source);
    let out = w
        .command()
        .args(["--build", "run", "--output", "source.fl"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(fs::read_to_string(w.0.join("source.fl")).unwrap(), source);
}
#[test]
fn host_cli_rejects_conflicting_actions() {
    for args in [
        vec!["--build", "run"],
        vec!["--build", "run", "--output", "x", "--interpret", "run"],
        vec!["--emit-llvm", "run", "--emit", "nir"],
        vec!["--emit-llvm", "run", "--test"],
    ] {
        let w = Work::new("fn run() {}");
        let out = w.command().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2));
    }
}

#[test]
fn native_recursion_and_integer_loop() {
    native(
        "typealias U=.u32'builtin;fn count(n:U)->U {if n == 0 {42} else {count(n - 1)}} fn run()->U {var n:U=0;while n < 42 {n=n + 1;}count(n)}",
        "(unsigned-integer 42)\n",
    );
}
#[test]
fn native_named_arguments_evaluate_in_source_order() {
    native(
        "fn combine(.a:bool=false,.b:bool=false)->bool {a and b} fn run()->bool {var value=false;combine(.b=do {value=true;value},.a=value)}",
        "(bool true)\n",
    );
}
#[test]
fn unsupported_storage_preserves_output() {
    let w = Work::new("fn run()->bool {var value=true;let pointer=value.ref;pointer.*}");
    fs::write(w.0.join("native program"), b"old").unwrap();
    let out = w.build();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("host data pointers require paired Copy/Drop resource verification"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"old");
}
#[test]
fn zero_divisor_traps() {
    let w = Work::new(
        "typealias U=.u32'builtin;fn div(a:U,b:U)->U {assumes b != 0;a / b} fn run()->U {div(42,0)}",
    );
    success(&w.build());
    assert!(!w.execute().status.success());
}

#[test]
fn native_template_with_staged_type() {
    native(
        "typealias Type=.Type'builtin; template fn identity(comptime T:Type,value:T)->T; fn keep use identity {value} fn run()->bool {keep(bool,true)}",
        "(bool true)\n",
    );
}
#[test]
fn native_entry_default_wrapper() {
    native("fn run(.value:bool=true)->bool {value}", "(bool true)\n");
}
#[test]
fn native_partial_return() {
    native(
        "fn choose(flag:bool)->bool {if flag {return true;} false} fn run()->bool {choose(true) and not choose(false)}",
        "(bool true)\n",
    );
}

#[test]
fn native_ieee_float_arithmetic_and_bits() {
    for (ty, expected) in [
        ("f32", "(float32 (bits 42280000))\n"),
        ("f64", "(float64 (bits 4045000000000000))\n"),
    ] {
        native(
            &format!(
                "typealias F=.{ty}'builtin;fn calculate(a:F,b:F)->F {{(a + b) * b / b - b}} fn run()->F {{calculate(42.0,2.0)}}"
            ),
            expected,
        );
    }
}
#[test]
fn native_ieee_nan_and_infinity_comparisons() {
    for ty in ["f32", "f64"] {
        native(
            &format!(
                "typealias F=.{ty}'builtin;fn div(a:F,b:F)->F {{a / b}} fn run()->bool {{let nan=div(0.0,0.0);let inf=div(1.0,0.0);nan != nan and not (nan < inf) and not (nan >= inf) and inf > 1.0}}"
            ),
            "(bool true)\n",
        );
    }
}
#[test]
fn native_ieee_negative_zero() {
    for (ty, expected) in [
        ("f32", "(float32 (bits 80000000))\n"),
        ("f64", "(float64 (bits 8000000000000000))\n"),
    ] {
        native(
            &format!("typealias F=.{ty}'builtin;fn neg(a:F)->F {{ -a }} fn run()->F {{neg(0.0)}}"),
            expected,
        );
    }
}
#[test]
fn native_ieee_canonical_arithmetic_nan() {
    for (ty, expected) in [
        ("f32", "(float32 (bits 7fc00000))\n"),
        ("f64", "(float64 (bits 7ff8000000000000))\n"),
    ] {
        native(
            &format!(
                "typealias F=.{ty}'builtin;fn div(a:F,b:F)->F {{a / b}} fn run()->F {{div(0.0,0.0)}}"
            ),
            expected,
        );
    }
}

#[test]
fn native_ieee_precision_rounds_at_each_operation() {
    for (ty, threshold) in [("f32", "16777216.0"), ("f64", "9007199254740992.0")] {
        native(
            &format!(
                "typealias F=.{ty}'builtin;fn rounded(a:F,b:F)->F {{(a + b) - a}} fn run()->bool {{rounded({threshold},1.0) == 0.0}}"
            ),
            "(bool true)\n",
        );
    }
}
#[test]
fn native_ieee_subnormals_are_not_flushed() {
    for (ty, tiny, expected) in [
        (
            "f32",
            "1.401298464324817e-45",
            "(float32 (bits 00000001))\n",
        ),
        (
            "f64",
            "4.9406564584124654e-324",
            "(float64 (bits 0000000000000001))\n",
        ),
    ] {
        native(
            &format!(
                "typealias F=.{ty}'builtin;fn mul(a:F,b:F)->F {{a * b}} fn run()->F {{mul({tiny},1.0)}}"
            ),
            expected,
        );
    }
}
#[test]
fn native_ieee_comparisons_and_signed_zero_reciprocal() {
    for ty in ["f32", "f64"] {
        native(
            &format!(
                "typealias F=.{ty}'builtin;fn check(a:F,b:F)->bool {{a == b and a <= b and a >= b and not (a < b) and not (a > b) and 1.0 / a > 0.0 and 1.0 / b < 0.0}} fn run()->bool {{check(0.0,-0.0)}}"
            ),
            "(bool true)\n",
        );
    }
}

#[test]
fn shipped_floating_example_executes_natively() {
    native(
        include_str!("../../../development/examples/host-floating.fl"),
        "(bool true)\n",
    );
}

#[test]
fn native_i128_decimal_boundaries() {
    for (ty, value, kind) in [
        ("u128", "0", "unsigned"),
        (
            "u128",
            "340282366920938463463374607431768211455",
            "unsigned",
        ),
        ("i128", "0", "signed"),
        ("i128", "-1", "signed"),
        ("i128", "170141183460469231731687303715884105727", "signed"),
        ("i128", "-170141183460469231731687303715884105728", "signed"),
    ] {
        native(
            &format!(
                "typealias I=.{ty}'builtin;fn identity(value:I)->I {{value}} fn run()->I {{identity({value})}}"
            ),
            &format!("({kind}-integer {value})\n"),
        );
    }
}
#[test]
fn native_u128_arithmetic_crosses_machine_word() {
    native(
        "typealias U=.u128'builtin;fn double(value:U)->U where requires value <= 18446744073709551616 {value * 2} fn run()->U {double(18446744073709551616)}",
        "(unsigned-integer 36893488147419103232)\n",
    );
}
#[test]
fn native_u128_division_and_remainder() {
    native(
        "typealias U=.u128'builtin;fn div(value:U)->U {value / 3} fn run()->U {div(36893488147419103232)}",
        "(unsigned-integer 12297829382473034410)\n",
    );
    native(
        "typealias U=.u128'builtin;fn rem(value:U)->U {value % 10} fn run()->U {rem(340282366920938463463374607431768211455)}",
        "(unsigned-integer 5)\n",
    );
}

#[test]
fn shipped_wide_integer_example_executes_natively() {
    native(
        include_str!("../../../development/examples/host-wide-integer.fl"),
        "(unsigned-integer 36893488147419103232)\n",
    );
}

#[test]
fn native_function_parameter_and_return_value() {
    native(
        "fn invert(value:bool)->bool {not value} fn factory()->fn(bool)->bool {invert} fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)} fn run()->bool {apply(factory(),false)}",
        "(bool true)\n",
    );
}
#[test]
fn native_function_selection_and_reassignment() {
    native(
        "fn keep(value:bool)->bool {value} fn invert(value:bool)->bool {not value} fn choose(flag:bool)->fn(bool)->bool {if flag {keep} else {invert}} fn run()->bool {var callback=choose(true);let first=callback(true);callback=choose(false);first and callback(false)}",
        "(bool true)\n",
    );
}
#[test]
fn native_function_target_runs_before_arguments() {
    native(
        "fn keep(value:bool)->bool {value} fn invert(value:bool)->bool {not value} fn run()->bool {var state=false;(do {state=not state;keep})(do {let before=state;state=false;before}) and not state}",
        "(bool true)\n",
    );
}

#[test]
fn native_indirect_call_keeps_callee_checked_assumes() {
    let w = Work::new(
        "fn checked(value:bool)->bool {assumes value;value} fn apply(callback:fn(bool)->bool)->bool {callback(false)} fn run()->bool {apply(checked)}",
    );
    success(&w.build());
    assert!(!w.execute().status.success());
}
#[test]
fn native_indirect_void_call() {
    native(
        "fn unused(value:bool) {} fn apply(callback:fn(bool),value:bool) {callback(value);} fn run()->bool {apply(unused,true);true}",
        "(bool true)\n",
    );
}
#[test]
fn native_higher_order_function_default() {
    native(
        "fn keep(value:bool)->bool {value} fn apply(value:bool,.callback:fn(bool)->bool=keep)->bool {callback(value)} fn run()->bool {apply(true)}",
        "(bool true)\n",
    );
}
#[test]
fn native_indirect_wide_integer_and_float_abis() {
    native(
        "typealias U=.u128'builtin;fn keep(value:U)->U {value} fn apply(callback:fn(U)->U,value:U)->U {callback(value)} fn run()->U {apply(keep,340282366920938463463374607431768211455)}",
        "(unsigned-integer 340282366920938463463374607431768211455)\n",
    );
    native(
        "typealias F=.f64'builtin;fn neg(value:F)->F { -value } fn apply(callback:fn(F)->F,value:F)->F {callback(value)} fn run()->F {apply(neg,0.0)}",
        "(float64 (bits 8000000000000000))\n",
    );
}

#[test]
fn native_function_value_cannot_erase_preconditions() {
    let w = Work::new(
        "fn checked(value:bool)->bool where requires value {value} fn apply(callback:fn(bool)->bool)->bool {callback(false)} fn run()->bool {apply(checked)}",
    );
    let out = w.build();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("function value would erase program preconditions")
    );
}

#[test]
fn shipped_callback_example_executes_natively() {
    native(
        include_str!("../../../development/examples/host-callback.fl"),
        "(bool true)\n",
    );
}

#[test]
fn native_empty_lambda_as_parameter_and_return_value() {
    native(
        "fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)} fn factory()->fn(bool)->bool {|value| not value} fn run()->bool {apply(factory(),false)}",
        "(bool true)\n",
    );
}
#[test]
fn native_nested_empty_lambda() {
    native(
        "fn factory()->fn()->fn(bool)->bool {|| {|value| not value}} fn run()->bool {factory()()(false)}",
        "(bool true)\n",
    );
}
#[test]
fn native_lambda_return_does_not_return_from_enclosing_function() {
    native(
        "fn run()->bool {let callback:fn(bool)->bool=|value| {if value {return false;}true};not callback(true) and callback(false)}",
        "(bool true)\n",
    );
}

#[test]
fn native_lambda_does_not_inherit_parent_postcondition() {
    native(
        "fn run()->result:bool where ensures result {let callback:fn()->bool=||false;let ignored=callback();true}",
        "(bool true)\n",
    );
}
#[test]
fn native_lambda_calls_registered_function() {
    native(
        "fn invert(value:bool)->bool {not value} fn run()->bool {let callback:fn(bool)->bool=|value| invert(value);callback(false)}",
        "(bool true)\n",
    );
}
#[test]
fn empty_lambda_llvm_symbols_are_deterministic() {
    let w = Work::new(
        "fn factory()->fn()->fn(bool)->bool {|| {|value| not value}} fn run()->bool {factory()()(false)}",
    );
    let first = w.command().args(["--emit-llvm", "run"]).output().unwrap();
    success(&first);
    let second = w.command().args(["--emit-llvm", "run"]).output().unwrap();
    success(&second);
    assert_eq!(first.stdout, second.stdout);
    let ir = String::from_utf8_lossy(&first.stdout);
    assert!(ir.contains("@flurry_lambda_0"));
    assert!(ir.contains("@flurry_lambda_1"));
}
#[test]
fn native_captured_lambda_snapshots_and_preserves_mutation() {
    for source in [
        "typealias U=.u32'builtin;typealias Counter=%{value:U} fn()->U;fn make(start:U)->Counter {var value=start;|| {if value < 100 {value=value + 1;}value}} fn run()->bool {let next=make(40);next()==41 and next()==42}",
        "fn run()->bool {var bits:.u8'builtin=0;let write=|| {bits'bitvec[0..4]=3;bits'bitvec[4..8]=10;bits==163};write() and bits==0}",
        "fn run()->bool {var value=true;let callback=||value;value=false;callback() and not value}",
        "fn run()->bool {var value=false;let callback=|| {value=not value;value};let first=callback();let second=callback();first and not second and not value}",
        "typealias Counter=%{flag:bool} fn()->bool;fn make()->Counter {var flag=false;|| {flag=not flag;flag}} fn apply(counter:Counter)->bool {let first=counter();let second=counter();first and not second} fn run()->bool {apply(make())}",
        "fn run()->bool {let left=true;let right=false;let callback=|input:bool| {if input {return left;}right};callback(true) and not callback(false)}",
        "typealias Callback=%{flag:bool} fn()->bool;fn make(flag:bool)->Callback {||flag} fn run()->bool {let yes=make(true);let no=make(false);yes() and not no()}",
        "fn run()->bool {var flag=false;let next=|| {flag=not flag;flag};let nested=||next();let first=nested();let second=nested();first and not second}",
        "fn pack(...fields:T)->T where T {fields} fn run()->bool {var flag=false;let next=|| {flag=not flag;flag};let callbacks=pack {step:next};let first=callbacks.step();let second=callbacks.step();first and not second}",
    ] {
        let w = Work::new(source);
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        assert!(
            reference.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        assert_eq!(reference.stdout, b"(bool true)\n", "{source}");
        for level in ["0", "3"] {
            let output = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = w.execute();
            success(&output);
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                "(bool true)\n",
                "{source}"
            );
        }
    }
}

#[test]
fn native_captured_lambda_branch_and_argument_order() {
    for source in [
        "typealias Callback=%{flag:bool} fn()->bool;fn make(flag:bool,choose:bool)->Callback {if choose {||flag}else{||not flag}} fn run()->bool {make(true,true)() and make(false,false)()}",
        "fn run()->bool {var flag=false;let next=|ignored:bool| {flag=not flag;flag};let result=next(next(false));not result}",
        "fn pack(...fields:T)->T where T {fields} fn run()->bool {var flag=false;let next=|arg:bool| {flag=not flag;flag};var state=pack {step:next,other:false};let first=state.step(do {state.other=true;true});let second=state.step(false);first and not second and state.other}",
    ] {
        let w = Work::new(source);
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        assert!(
            reference.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        assert_eq!(reference.stdout, b"(bool true)\n", "{source}");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let out = w.execute();
            success(&out);
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                "(bool true)\n",
                "{source}"
            );
        }
    }
}

#[test]
fn native_captured_lambda_move_preserves_environment_state() {
    let source = "fn run()->bool {var flag=false;let next=|| {flag=not flag;flag};let first=next();let moved=next;let second=moved();first and not second}";
    let w = Work::new(source);
    let reference = w.command().args(["--interpret", "run"]).output().unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn native_captured_lambda_receiver_replacement_preserves_output() {
    let w = Work::new(
        "fn run()->bool {var flag=false;var action=|value:bool|flag;action(do {action=|value:bool|not flag;true})}",
    );
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let out = w.build();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("receiver is replaced"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn native_captured_lambda_symbols_are_deterministic() {
    let w = Work::new(
        "typealias Callback=%{flag:bool} fn()->bool;fn make(flag:bool)->Callback {||flag} fn run()->bool {make(true)()}",
    );
    let first = w.command().args(["--emit-llvm", "run"]).output().unwrap();
    success(&first);
    let second = w.command().args(["--emit-llvm", "run"]).output().unwrap();
    success(&second);
    assert_eq!(first.stdout, second.stdout);
    assert!(String::from_utf8_lossy(&first.stdout).contains("ptr %environment"));
}

#[test]
fn native_captured_lambda_checked_capture_transfer_executes() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Ticket {{}} impl Retire for Ticket {{fn drop(*self) {{}}}} fn run()->bool {{let ticket=Ticket {{}};let callback=|| {{ticket;true}};callback()}}"
    ));
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn shipped_captured_lambda_example_executes_natively() {
    native(
        include_str!("../../../development/examples/host-closure.fl"),
        "(bool true)\n",
    );
}

#[test]
fn shipped_lambda_example_executes_natively() {
    native(
        include_str!("../../../development/examples/host-lambda.fl"),
        "(bool true)\n",
    );
}

#[test]
fn native_scalar_match_bindings_and_priority() {
    native(
        "fn choose(value:bool)->bool {value match {true=>false,bound=>bound,}} fn run()->bool {not choose(true) and not choose(false)}",
        "(bool true)\n",
    );
}
#[test]
fn native_integer_range_match() {
    native(
        "typealias I=.i32'builtin;fn classify(value:I)->bool {value match { -10..0=>true,0..=10=>false,_=>true,}} fn run()->bool {classify(-3) and not classify(10) and classify(11)}",
        "(bool true)\n",
    );
}
#[test]
fn native_match_alternative_bindings() {
    native(
        "fn run()->bool {true match {(true as value) or (false as value)=>value,}}",
        "(bool true)\n",
    );
}
#[test]
fn native_float_range_keeps_nan_unordered() {
    native(
        "typealias F=.f64'builtin;fn check(value:F)->bool {value match {0.0..1.0=>false,_=>true,}} fn nan(a:F)->F {a / a} fn run()->bool {check(nan(0.0)) and not check(0.5)}",
        "(bool true)\n",
    );
}

#[test]
fn native_let_else_and_successful_binding() {
    native(
        "fn check(value:bool)->bool {let true as found=value else {return false;};found} fn run()->bool {check(true) and not check(false)}",
        "(bool true)\n",
    );
}
#[test]
fn native_failed_match_guard_preserves_external_writes() {
    native(
        "fn run()->bool {var state=false;true match {bound if do {state=bound;false}=>false,_=>state,}}",
        "(bool true)\n",
    );
}
#[test]
fn native_match_subject_once_and_later_guards_skipped() {
    native(
        "fn run()->bool {var state=false;(do {state=not state;true}) match {true=>state,_ if do {state=false;true}=>false,_=>false,}}",
        "(bool true)\n",
    );
}
#[test]
fn native_or_pattern_binding_uses_selected_alternative() {
    native(
        "fn choose(value:bool)->bool {value match {(true as chosen) or (false as chosen)=>chosen,}} fn run()->bool {choose(true) and not choose(false)}",
        "(bool true)\n",
    );
}
#[test]
fn native_computed_pattern_observes_bound_input() {
    native(
        "fn check(value:bool)->bool {value match {bound and not bound is result=>result,}} fn run()->bool {check(false) and not check(true)}",
        "(bool true)\n",
    );
}

#[test]
fn native_pattern_if_and_while_control() {
    native(
        "fn run()->bool {var active=true;var reached=false;while active is true do {active=false;reached=true;} if reached is true do {not active} else {false}}",
        "(bool true)\n",
    );
}
#[test]
fn native_let_else_continue_keeps_loop_state() {
    native(
        "fn run()->bool {var active=true;while active {active=false;let true=false else {continue;};return false;} not active}",
        "(bool true)\n",
    );
}
#[test]
fn native_match_return_from_guard_skips_remaining_candidates() {
    native(
        "fn run()->bool {true match {_ if do {return true;false}=>false,_=>false,}}",
        "(bool true)\n",
    );
}
#[test]
fn native_negated_float_range_matches_nan() {
    native(
        "typealias F=.f64'builtin;fn check(value:F)->bool {value match {not (0.0..1.0)=>true,_=>false,}} fn nan(value:F)->F {value / value} fn run()->bool {check(nan(0.0)) and not check(0.5)}",
        "(bool true)\n",
    );
}

#[test]
fn shipped_pattern_example_executes_natively() {
    native(
        include_str!("../../../development/examples/host-pattern.fl"),
        "(bool true)\n",
    );
}

#[test]
fn native_anonymous_record_fields_and_assignment() {
    native(
        "fn pack(...fields:T)->T where T {fields} fn run()->bool {var value=pack {flag:false,other:true};value.flag=true;value.flag and value.other}",
        "(bool true)\n",
    );
}
#[test]
fn native_nested_anonymous_record_return_and_projection() {
    native(
        "fn pack(...fields:T)->T where T {fields} fn keep(value:T)->T where T {value} fn run()->bool {var value=keep(pack {inner:pack {flag:false},other:true});value.inner.flag=true;value.inner.flag and value.other}",
        "(bool true)\n",
    );
}
#[test]
fn native_anonymous_record_pattern() {
    native(
        "fn pack(...fields:T)->T where T {fields} fn run()->bool {let value=pack {left:true,right:false};value match {{left:chosen,right:false}=>chosen,_=>false,}}",
        "(bool true)\n",
    );
}

#[test]
fn native_record_fields_evaluate_in_source_order() {
    native(
        "fn pack(...fields:T)->T where T {fields} fn run()->bool {var flag=false;let record=pack {first:do {flag=true;flag},second:do {let before=flag;flag=false;before}};record.first and record.second and not flag}",
        "(bool true)\n",
    );
}
#[test]
fn native_record_contains_unit_and_function_fields() {
    native(
        "fn pack(...fields:T)->T where T {fields} fn empty() {} fn run()->bool {let record=pack {unit:empty(),callback:|value:bool| not value};record.unit;record.callback(false)}",
        "(bool true)\n",
    );
}
#[test]
fn native_record_conditional_merge_and_independent_copies() {
    native(
        "fn pack(...fields:T)->T where T {fields} fn run()->bool {var original=pack {flag:false};let saved=original;original.flag=true;let selected=if original.flag {saved} else {original};not selected.flag and original.flag}",
        "(bool true)\n",
    );
}
#[test]
fn native_record_let_destructuring() {
    native(
        "fn pack(...fields:T)->T where T {fields} fn run()->bool {let {left:first,right:second}=pack {left:true,right:false};first and not second}",
        "(bool true)\n",
    );
}
#[test]
fn native_record_pointer_field_is_rejected_without_publishing() {
    let w = Work::new(
        "fn pack(...fields:T)->T where T {fields} fn run()->bool {var value=true;let record=pack {pointer:value.ref};record.pointer.*}",
    );
    fs::write(w.0.join("native program"), b"existing").unwrap();
    let out = w.build();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("host"));
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"existing");
}

#[test]
fn shipped_record_example_executes_natively() {
    native(
        include_str!("../../../development/examples/host-record.fl"),
        "(bool true)\n",
    );
}

const TUPLE_HEADER: &str = "pure comptime fn TupleElements<T> -> .Type'builtin {T} pure comptime fn Tuple<...Es:TupleElements<.Type'builtin>> -> .Type'builtin {bool}";
fn native_tuple(source: &str, expected: &str) {
    let w = Work::new(&format!("{TUPLE_HEADER}\n{source}"));
    let out = w
        .command()
        .args([
            "--meta-list-type",
            "input.TupleElements",
            "--tuple-type",
            "input.Tuple",
            "--build",
            "run",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    success(&out);
    let result = w.execute();
    success(&result);
    assert_eq!(String::from_utf8_lossy(&result.stdout), expected);
}
#[test]
fn native_tuple_return_and_let_destructuring() {
    native_tuple(
        "fn swap(pair:Tuple<A,B>)->Tuple<B,A> where A,B {let (a,b)=pair;(b,a)} fn run()->bool {let (a,b)=swap((true,false));not a and b}",
        "(bool true)\n",
    );
}
#[test]
fn native_nested_tuple_pattern_and_alternatives() {
    native_tuple(
        "fn check(pair:Tuple<Tuple<bool,bool>,bool>)->bool {pair match {((value,true),false) or ((true,value),true)=>value,_=>false,}} fn run()->bool {check(((true,true),false)) and not check(((true,false),true))}",
        "(bool true)\n",
    );
}
#[test]
fn native_empty_and_singleton_tuple_layouts() {
    native_tuple(
        "fn empty(value:Tuple<>)->Tuple<> {value} fn single(value:Tuple<bool>)->bool {let (flag,)=value;flag} fn run()->bool {let ()=empty(());single((true,))}",
        "(bool true)\n",
    );
}
#[test]
fn native_tuple_evaluation_order() {
    native_tuple(
        "fn run()->bool {var state=false;let (a,b)=(do {state=true;state},do {let old=state;state=false;old});a and b and not state}",
        "(bool true)\n",
    );
}

#[test]
fn native_variadic_tuple_collection_and_spread() {
    native_tuple(
        "fn take(...values:Tuple<bool,bool>)->bool {let (a,b)=values;a and not b} fn pair()->Tuple<bool,bool> {(true,false)} fn run()->bool {take(...pair())}",
        "(bool true)\n",
    );
}
#[test]
fn native_extended_variadic_tuple_and_record() {
    native_tuple(
        "comptime fn schema(...properties:T)->T where T {properties} const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;fn take(...items:Tuple<bool,bool>,...properties:Options)->bool {let (a,b)=items;a and not b and properties.flag} fn run()->bool {take {flag:true,...(true,false)}}",
        "(bool true)\n",
    );
}
#[test]
fn native_tuple_field_can_hold_empty_tuple_and_callback() {
    native_tuple(
        "fn run()->bool {let (empty,callback)=((),|value:bool| not value);let ()=empty;callback(false)}",
        "(bool true)\n",
    );
}
#[test]
fn tuple_head_is_retained_in_iir_before_codegen() {
    let w = Work::new(&format!(
        "{TUPLE_HEADER}\nfn pair()->Tuple<bool,bool> {{(true,false)}} fn run()->bool {{let (a,b)=pair();a and not b}}"
    ));
    let out = w
        .command()
        .args([
            "--meta-list-type",
            "input.TupleElements",
            "--tuple-type",
            "input.Tuple",
            "--instantiate",
            "run",
        ])
        .output()
        .unwrap();
    success(&out);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("(normal-form (head"), "{text}");
    assert!(text.contains("(type-list"), "{text}");
}

#[test]
fn shipped_tuple_example_executes_natively() {
    let w = Work::new(include_str!("../../../development/examples/host-tuple.fl"));
    let out = w
        .command()
        .args([
            "--meta-list-type",
            "input.TupleElements",
            "--tuple-type",
            "input.Tuple",
            "--build",
            "run",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    success(&out);
    let result = w.execute();
    success(&result);
    assert_eq!(result.stdout, b"(bool true)\n");
}

#[test]
fn optimization_levels_preserve_shipped_examples() {
    let examples = [
        (
            include_str!("../../../development/examples/host-arithmetic.fl"),
            "(unsigned-integer 42)\n",
        ),
        (
            include_str!("../../../development/examples/host-floating.fl"),
            "(bool true)\n",
        ),
        (
            include_str!("../../../development/examples/host-record.fl"),
            "(bool true)\n",
        ),
        (
            include_str!("../../../development/examples/host-pattern.fl"),
            "(bool true)\n",
        ),
        (
            include_str!("../../../development/examples/host-lambda.fl"),
            "(bool true)\n",
        ),
        (
            include_str!("../../../development/examples/host-wide-integer.fl"),
            "(unsigned-integer 36893488147419103232)\n",
        ),
    ];
    for level in ["0", "1", "2", "3"] {
        for (source, expected) in examples {
            let w = Work::new(source);
            let output = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&output);
            let result = w.execute();
            success(&result);
            assert_eq!(
                String::from_utf8_lossy(&result.stdout),
                expected,
                "level {level}"
            );
        }
    }
}
#[test]
fn optimization_keeps_runtime_verification_traps() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(
            "fn checked(value:bool)->bool {assumes value;value} fn run()->bool {checked(false)}",
        );
        let result = w
            .command()
            .args([
                "--build",
                "run",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&result);
        assert!(!w.execute().status.success(), "level {level}");
    }
}
#[test]
fn optimized_llvm_is_deterministic_and_changes_the_module() {
    let w = Work::new("fn run()->bool {var value=false;value=true;value}");
    let raw = w.command().args(["--emit-llvm", "run"]).output().unwrap();
    success(&raw);
    let first = w
        .command()
        .args(["--emit-llvm", "run", "--opt-level", "2"])
        .output()
        .unwrap();
    success(&first);
    let second = w
        .command()
        .args(["--emit-llvm", "run", "--opt-level", "2"])
        .output()
        .unwrap();
    success(&second);
    assert_eq!(first.stdout, second.stdout);
    assert_ne!(raw.stdout, first.stdout);
    assert!(String::from_utf8_lossy(&raw.stdout).contains("alloca i1"));
    assert!(!String::from_utf8_lossy(&first.stdout).contains("alloca i1"));
}
#[test]
fn optimization_options_are_validated() {
    for args in [
        vec!["--opt-level", "2"],
        vec!["--emit-llvm", "run", "--opt-level", "4"],
        vec!["--emit-llvm", "run", "--opt-level", "2", "--opt-level", "1"],
        vec!["--interpret", "run", "--opt-level", "0"],
    ] {
        let w = Work::new("fn run() {}");
        let result = w.command().args(args).output().unwrap();
        assert_eq!(result.status.code(), Some(2));
    }
}

#[test]
fn optimization_preserves_ieee_bit_boundaries() {
    let cases = [
        (
            "typealias F=.f32'builtin;fn neg(value:F)->F { -value } fn run()->F {neg(0.0)}",
            "(float32 (bits 80000000))\n",
        ),
        (
            "typealias F=.f64'builtin;fn mul(value:F)->F {value * 1.0} fn run()->F {mul(4.9406564584124654e-324)}",
            "(float64 (bits 0000000000000001))\n",
        ),
        (
            "typealias F=.f32'builtin;fn round(value:F)->F {(value + 1.0) - value} fn run()->F {round(16777216.0)}",
            "(float32 (bits 00000000))\n",
        ),
        (
            "typealias F=.f64'builtin;fn div(value:F)->F {value / value} fn run()->F {div(0.0)}",
            "(float64 (bits 7ff8000000000000))\n",
        ),
    ];
    for level in ["0", "1", "2", "3"] {
        for (source, expected) in cases {
            let w = Work::new(source);
            let output = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&output);
            let result = w.execute();
            success(&result);
            assert_eq!(String::from_utf8_lossy(&result.stdout), expected);
        }
    }
}
#[test]
fn optimized_dump_failure_preserves_previous_file() {
    let w = Work::new("fn run()->bool {true}");
    fs::write(w.0.join("dump.ll"), b"previous").unwrap();
    let output = w
        .command()
        .args([
            "--emit-llvm",
            "run",
            "--opt-level",
            "2",
            "--output",
            "dump.ll",
        ])
        .env("LUNA_LLVM_CONFIG", w.0.join("missing"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(w.0.join("dump.ll")).unwrap(), b"previous");
}

const RESOURCE_HEADER: &str =
    "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}";
fn resource_build(w: &Work) -> Output {
    w.command()
        .args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ])
        .output()
        .unwrap()
}
#[test]
fn native_resource_checked_scalar_entry() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} fn duplicate(value:bool)->bool {{value and value}} fn run()->bool {{duplicate(true)}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_resource_protocol_options_must_be_paired() {
    let w = Work::new(&format!("{RESOURCE_HEADER} fn run()->bool {{true}}"));
    for option in ["--copy-protocol", "--drop-protocol"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                option,
                "Duplicate",
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
    }
}
#[test]
fn native_resource_selection_failure_preserves_artifact() {
    let w = Work::new("fn run()->bool {true}");
    fs::write(w.0.join("native program"), b"old").unwrap();
    let out = resource_build(&w);
    assert!(!out.status.success());
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"old");
}
#[test]
fn native_resource_nominal_cleanup_is_not_silently_removed() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Token {{flag:bool}} impl Retire for Token {{fn drop(*self) {{}}}} fn run()->bool {{Token {{flag:true}};true}}"
    ));
    fs::write(w.0.join("native program"), b"old").unwrap();
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_resource_checked_loop_and_default_call() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} fn choose(a:bool,.b:bool=a)->bool {{a and b}} fn run()->bool {{var active=true;while active {{active=false;}}choose(not active)}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_resource_check_rejects_unproven_copy() {
    let w = Work::new(
        "trait Duplicate {} trait Retire {fn drop(*self);} struct Token {} impl Retire for Token {fn drop(*self) {}} fn consume(value:Token)->bool {true} fn run()->bool {let token=Token {};consume(token) and consume(token)}",
    );
    let out = resource_build(&w);
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(
        error.contains("cannot prepare resource-checked native compilation"),
        "{error}"
    );
    assert!(!w.0.join("native program").exists());
}

#[test]
fn shipped_resource_checked_example_executes_natively() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-resources.fl"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_checked_pointer_read_write_and_call() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} fn set(pointer:*bool) {{pointer.* = true;}} fn run()->bool {{var value=false;set(value.ref);let pointer=value.ref;pointer.*}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_pointer_nested_record_projection() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} fn pack(...fields:T)->T where T {{fields}} fn set(pointer:*bool) {{pointer.* = true;}} fn run()->bool {{var record=pack {{nested:pack {{flag:false}},other:true}};set(record.nested.flag.ref);record.nested.flag and record.other}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_pointer_to_pointer() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} fn set(pointer:**bool) {{pointer.*.* = true;}} fn run()->bool {{var flag=false;let pointer=flag.ref;set(pointer.ref);flag}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_pointer_local_escape_rejected() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} fn escape()->*bool {{let flag=true;flag.ref}} fn run()->bool {{escape().*}}"
    ));
    let out = resource_build(&w);
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(
        error.contains("cannot prepare resource-checked native compilation"),
        "{error}"
    );
    assert!(!w.0.join("native program").exists());
}

#[test]
fn native_checked_zero_sized_pointer_local_escape_rejected() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias Void=.void'builtin;fn empty()->Void {{}} fn escape()->*Void {{let value=empty();value.ref}} fn run() {{escape().*;}}"
    ));
    let out = resource_build(&w);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("cannot prepare resource-checked native compilation")
    );
    assert!(!w.0.join("native program").exists());
}

#[test]
fn native_checked_pointer_borrowed_return_and_alias_write() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} fn identity(pointer:*bool)->*bool {{pointer}} fn run()->bool {{var flag=false;let pointer=identity(flag.ref);pointer.* = true;flag}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_pointer_field_projection() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} comptime fn schema(...fields:T)->T where T {{fields}} const SAMPLE=schema {{flag:false,other:true}};typealias Options=SAMPLE'type;fn pack(...fields:Options)->Options {{fields}} fn set(pointer:*Options) {{pointer.flag.* = true;}} fn run()->bool {{var options=pack {{flag:false,other:true}};set(options.ref);options.flag and options.other}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_pointer_to_zero_sized_value() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias Void=.void'builtin;impl Duplicate for Void {{}} fn empty()->Void {{}} fn run()->bool {{var value=empty();let pointer=value.ref;let nested=pointer.ref;nested.*;pointer.*;pointer.* = empty();true}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn shipped_checked_pointer_example_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!(
            "../../../development/examples/host-pointer.fl"
        ));
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let result = w.execute();
        success(&result);
        assert_eq!(result.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_checked_struct_fields_and_assignment() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Flags {{first:bool,second:bool}} impl Duplicate for Flags {{}} fn run()->bool {{var flags=Flags {{first:false,second:true}};flags.first=true;flags.first and flags.second}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_generic_struct_return() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Cell where T {{value:T}} impl Duplicate for Cell<bool> {{}} fn make(value:T)->Cell<T> where T {{Cell<T> {{value:value}}}} fn run()->bool {{let cell=make(true);cell.value}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_struct_pointer_field_and_pattern() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Cell {{value:bool}} impl Duplicate for Cell {{}} fn set(pointer:*Cell) {{pointer.value.* = true;}} fn run()->bool {{var cell=Cell {{value:false}};set(cell.ref);let {{value:flag}}=cell;flag}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_checked_nested_struct_and_receiver_method() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Inner {{flag:bool}} impl Duplicate for Inner {{}} struct Outer {{inner:Inner,other:bool}} impl Duplicate for Outer {{}} impl Outer {{fn enable(*self) {{self.inner.flag.* = true;}}}} fn run()->bool {{var outer=Outer {{inner:Inner {{flag:false}},other:true}};outer.enable();let {{inner:{{flag}},other}}=outer;flag and other}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_empty_struct_has_zero_sized_storage() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Empty {{}} impl Duplicate for Empty {{}} fn keep(value:Empty)->Empty {{value}} fn run()->bool {{let value=keep(Empty {{}});let pointer=value.ref;let {{}}=pointer.*;true}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_struct_layout_does_not_merge_type_identities() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct A {{flag:bool}} struct B {{flag:bool}} impl Duplicate for A {{}} impl Duplicate for B {{}} fn take(value:A)->bool {{value.flag}} fn run()->bool {{take(B {{flag:true}})}}"
    ));
    let out = resource_build(&w);
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("elaboration failed"), "{error}");
    assert!(!w.0.join("native program").exists());
}
#[test]
fn native_struct_without_resource_verification_rejected() {
    let w =
        Work::new("struct Cell {flag:bool} fn run()->bool {let cell=Cell {flag:true};cell.flag}");
    let out = w.build();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("host nominal storage requires paired Copy/Drop resource verification")
    );
}

#[test]
fn shipped_checked_struct_example_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!("../../../development/examples/host-struct.fl"));
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let result = w.execute();
        success(&result);
        assert_eq!(result.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_checked_newtype_scalar_round_trip() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} newtype Flag=bool;impl Duplicate for Flag {{}} fn keep(value:Flag)->Flag {{value}} fn run()->bool {{keep(true.as(Flag)).as(bool)}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_newtype_nested_generic_representation() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} newtype Wrapped<T> = T;newtype Outer=Wrapped<bool>;impl Duplicate for Wrapped<bool> {{}} impl Duplicate for Outer {{}} fn run()->bool {{let value=true.as(Wrapped<bool>).as(Outer);value.as(Wrapped<bool>).as(bool)}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_newtype_struct_representation() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Cell {{flag:bool}} impl Duplicate for Cell {{}} newtype Wrapped=Cell;impl Duplicate for Wrapped {{}} fn run()->bool {{let wrapped=(Cell {{flag:true}}).as(Wrapped);let cell=wrapped.as(Cell);cell.flag}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_newtype_identity_cannot_be_bypassed_by_equal_layouts() {
    for source in [
        "newtype A=bool;newtype B=bool;impl Duplicate for A {} impl Duplicate for B {} fn accept(value:A)->bool {value.as(bool)} fn run()->bool {accept(true.as(B))}",
        "newtype A=bool;newtype B=A;impl Duplicate for A {} impl Duplicate for B {} fn run()->bool {true.as(B).as(bool)}",
    ] {
        let w = Work::new(&format!("{RESOURCE_HEADER} {source}"));
        let out = resource_build(&w);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("elaboration failed"));
    }
}

#[test]
fn native_checked_newtype_pointer_payload() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} newtype Handle=*bool;impl Duplicate for Handle {{}} fn enable(handle:Handle) {{let pointer=handle.as(*bool);pointer.* = true;}} fn run()->bool {{var flag=false;enable(flag.ref.as(Handle));flag}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_newtype_conversion_preserves_operand_effects() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} newtype Flag=bool;impl Duplicate for Flag {{}} fn run()->bool {{var changed=false;let value=(do {{changed=not changed;changed}}).as(Flag);value.as(bool) and changed}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_newtype_destructor_responsibility_is_not_erased() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} newtype Owned=bool;impl Retire for Owned {{fn drop(*self) {{}}}} fn run()->bool {{true.as(Owned);true}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn shipped_checked_newtype_example_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!(
            "../../../development/examples/host-newtype.fl"
        ));
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let result = w.execute();
        success(&result);
        assert_eq!(result.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_checked_fieldless_enum_construction_and_match() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} enum State {{off,on,}} impl Duplicate for State {{}} fn choose(flag:bool)->State {{if flag {{State.on}} else {{State.off}}}} fn read(value:State)->bool {{value match {{.off=>false,.on=>true,}}}} fn run()->bool {{read(choose(true)) and not read(choose(false))}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_checked_enum_assignment_through_pointer() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} enum State {{off,on,}} impl Duplicate for State {{}} fn enable(pointer:*State) {{pointer.* = State.on;}} fn run()->bool {{var value=State.off;enable(value.ref);value match {{.on=>true,.off=>false,}}}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_fixed_enum_uses_declared_bits_not_variant_indices() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias Byte=.u8'builtin;enum Encoded {{.base_type Byte,.width 8,.discrete_tag true,yes:0b10100101,no:0b01011010,}} impl Duplicate for Encoded {{}} fn yes()->Encoded {{Encoded.yes}} fn run()->bool {{yes() match {{.yes=>true,.no=>false,}}}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
    let ir = w
        .command()
        .args([
            "--emit-llvm",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&ir);
    let text = String::from_utf8_lossy(&ir.stdout);
    assert!(text.contains("ret i8 165"), "{text}");
}

#[test]
fn native_fixed_enum_preserves_full_128_bit_encoding() {
    let ones = "1".repeat(128);
    let zeros = "0".repeat(128);
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias Word=.u128'builtin;enum Wide {{.base_type Word,.width 128,.discrete_tag true,high:0b{ones},low:0b{zeros},}} impl Duplicate for Wide {{}} fn high()->Wide {{Wide.high}} fn run()->bool {{high() match {{.high=>true,.low=>false,}}}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
    let out = w
        .command()
        .args([
            "--emit-llvm",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&out);
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .contains("ret i128 340282366920938463463374607431768211455")
    );
}
#[test]
fn native_checked_enum_in_struct_pattern() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} enum State {{off,on,}} impl Duplicate for State {{}} struct Cell {{state:State}} impl Duplicate for Cell {{}} fn run()->bool {{var cell=Cell {{state:State.off}};cell.state=State.on;cell match {{{{state:.on}}=>true,_=>false,}}}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_fieldless_enum_or_pattern_and_guard() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} enum State {{off,on,unknown,}} impl Duplicate for State {{}} fn check(value:State)->bool {{value match {{.on or .unknown if true=>true,_=>false,}}}} fn run()->bool {{check(State.on) and check(State.unknown) and not check(State.off)}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_enum_payload_is_constructed_and_matched() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} enum Maybe {{none,some(bool),}} impl Duplicate for Maybe {{}} fn run()->bool {{Maybe.some(true) match {{.some(value)=>value,.none=>false,}}}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn shipped_checked_enum_example_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!("../../../development/examples/host-enum.fl"));
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let result = w.execute();
        success(&result);
        assert_eq!(result.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_fixed_enum_payload_encoding_and_extraction() {
    let source = format!(
        "{RESOURCE_HEADER} typealias Word=.u32'builtin;typealias Bits5=.BitVec'builtin<5>;enum Instruction {{.base_type Word,.width 32,.discrete_tag true,add:0b0000000 (rs2:Bits5) (rs1:Bits5) 000 (rd:Bits5) 0110011,sub:0b0100000 (rs2:Bits5) (rs1:Bits5) 000 (rd:Bits5) 0110011,}}impl Duplicate for Instruction {{}} fn destination(code:Instruction)->Word {{code match {{.add {{rd}}=>rd.as(Word),.sub {{rd}}=>rd.as(Word),}}}} fn run()->bool {{let code=Instruction.sub {{rd:1,rs2:3,rs1:2}};let bits=(code'bitvec).as(Word);bits == 1076953267 and destination(code) == 1}}"
    );
    let w = Work::new(&source);
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_fixed_enum_signed_payload() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias Signed=.i8'builtin;enum Packet {{.width 9,.discrete_tag true,data:0b1 (value:Signed),empty:0b000000000,}}impl Duplicate for Packet {{}}fn unpack(packet:Packet)->Signed {{packet match {{.data {{value}}=>value,.empty=>0,}}}}fn run()->bool {{unpack(Packet.data {{value: -128}}) == -128}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
#[test]
fn native_fixed_enum_float_payload_negative_zero() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias F=.f32'builtin;enum Packet {{.width 33,.discrete_tag true,data:0b1 (value:F),empty:0b000000000000000000000000000000000,}}impl Duplicate for Packet {{}}fn unpack(packet:Packet)->F {{packet match {{.data {{value}}=>value,.empty=>0.0,}}}}fn run()->F {{unpack(Packet.data {{value: -0.0}})}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(float32 (bits 80000000))\n");
}

#[test]
fn native_fixed_enum_named_payloads_keep_declaration_order() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias Byte=.u8'builtin;typealias Word=.u32'builtin;enum Packet {{.width 24,.discrete_tag true,data:0b10101010 (left:Byte) (right:Byte),}}impl Duplicate for Packet {{}}fn run()->bool {{var seen=false;let packet=Packet.data {{right:do {{seen=true;7}},left:do {{if seen {{9}} else {{0}}}}}};(packet'bitvec).as(Word) == 11141127}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn shipped_encoded_payload_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!(
            "../../../development/examples/host-encoded-payload.fl"
        ));
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let result = w.execute();
        success(&result);
        assert_eq!(result.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_fixed_enum_early_exit_skips_later_declared_payloads() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias Byte=.u8'builtin;enum Packet {{.width 24,.discrete_tag true,data:0b10101010 (left:Byte) (right:Byte),}}impl Duplicate for Packet {{}}fn run()->bool {{var seen=false;let packet=Packet.data {{right:do {{seen=true;7}},left:do {{return not seen;}}}};false}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_bitvec_algebra_and_slices() {
    native(
        "typealias Bits=.BitVec'builtin<8>;typealias Word=.u16'builtin;fn join(a:Bits,b:Bits)->Word {((a ++ b)[4..12]).as(Word)} fn run()->bool {join(0xAF,0x3C) == 0xF3}",
        "(bool true)\n",
    );
    native(
        "typealias Bits=.BitVec'builtin<8>;fn same(a:Bits,b:Bits)->bool {(not (a and b)) == ((not a) or (not b))} fn run()->bool {same(0xAF,0x3C)}",
        "(bool true)\n",
    );
}

#[test]
fn native_bitvec_logic_is_strict_and_source_ordered() {
    for op in ["and", "or"] {
        let source = format!(
            "typealias Bits=.BitVec'builtin<8>;fn run()->bool {{var seen=false;let bits:Bits=(do {{seen=true;0}}) {op} (do {{if seen {{7}} else {{9}}}});seen and bits == {expected}}}",
            expected = if op == "and" { 0 } else { 7 }
        );
        native(&source, "(bool true)\n");
        let source = format!(
            "typealias Bits=.BitVec'builtin<8>;fn run()->bool {{var seen=false;let bits:Bits=0 {op} (do {{seen=true;7}});seen}}"
        );
        native(&source, "(bool true)\n");
    }
}

#[test]
fn native_bitvec_dynamic_reads() {
    native(
        "typealias Word=.u16'builtin;typealias Byte=.u8'builtin;typealias Bits=.BitVec'builtin<16>;typealias Nibble=.BitVec'builtin<4>;pure fn select(bits:Bits,i:Byte)->Nibble where requires i <= 12 {bits[i..(i + 4)]} pure fn bit(bits:Bits,i:Byte)->.BitVec'builtin<1> where requires i < 16 {bits[i]}fn run()->bool {select(0xABCD,4).as(Word) == 12 and bit(0xABCD,15).as(Word) == 1}",
        "(bool true)\n",
    );
}

#[test]
fn native_bitvec_concat_128_and_full_width_slice() {
    native(
        "typealias Half=.BitVec'builtin<64>;typealias Full=.BitVec'builtin<128>;typealias Word=.u128'builtin;fn join(a:Half,b:Half)->Full {a ++ b}fn run()->Word {join(0xFFFFFFFFFFFFFFFF,0xFFFFFFFFFFFFFFFF)[0..128].as(Word)}",
        "(unsigned-integer 340282366920938463463374607431768211455)\n",
    );
}

#[test]
fn shipped_bitvec_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!("../../../development/examples/host-bitvec.fl"));
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_bitvec_early_exit_does_not_evaluate_later_operand() {
    native(
        "typealias Bits=.BitVec'builtin<8>;fn check(exit:bool)->bool {var seen=false;let other:Bits=7;let result:Bits=(do {if exit {return not seen;}other}) and (do {seen=true;other});false}fn run()->bool {check(true)}",
        "(bool true)\n",
    );
}

#[test]
fn native_bitvec_writeback_preserves_rhs_storage_changes() {
    native(
        "typealias Word=.u16'builtin;typealias Byte=.u8'builtin;fn run()->Word {var word:Word=0x1234;var i:Byte=4;word'bitvec[i..(i + 4)]=do {i=10;word=0xABCD;0b1010};word}",
        "(unsigned-integer 43949)\n",
    );
}

#[test]
fn native_bitvec_nested_and_overlapping_writeback() {
    native(
        "typealias Word=.u16'builtin;typealias Byte=.u8'builtin;pure fn nested(a:Byte,b:Byte)->Word where requires a <= 8, requires b <= 4 {var word:Word=0;word'bitvec[a..(a + 8)][b..(b + 4)]=0b1010;word}fn run()->Word {nested(4,2)}",
        "(unsigned-integer 640)\n",
    );
    native(
        "typealias Word=.u16'builtin;fn run()->Word {var word:Word=0x1234;word'bitvec[4..12]=word'bitvec[0..8];word}",
        "(unsigned-integer 4932)\n",
    );
}

#[test]
fn native_bitvec_writeback_to_signed_and_float_storage() {
    native(
        "typealias Signed=.i8'builtin;fn run()->Signed {var value:Signed=0;value'bitvec[7]=1;value}",
        "(signed-integer -128)\n",
    );
    native(
        "typealias F=.f32'builtin;fn run()->F {var value:F=0.0;value'bitvec[31]=1;value}",
        "(float32 (bits 80000000))\n",
    );
}

#[test]
fn native_bitvec_writeback_keeps_replaced_parent_fields() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias Word=.u16'builtin;typealias Byte=.u8'builtin;struct Packet {{word:Word,other:Word}}impl Duplicate for Packet {{}}fn run()->bool {{var packet=Packet {{word:0x1234,other:7}};var i:Byte=4;packet.word'bitvec[i..(i + 4)]=do {{i=10;packet=Packet {{word:0xABCD,other:9}};0b1010}};packet.word == 0xABAD and packet.other == 9}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_bitvec_pointer_writeback() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias Word=.u16'builtin;impl Duplicate for Word {{}}fn edit(pointer:*Word) {{pointer.*'bitvec[4..8]=do {{pointer.* = 0xABCD;0b1010}};}}fn run()->Word {{var word:Word=0x1234;edit(word.ref);word}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(unsigned-integer 43949)\n");
}

#[test]
fn native_bitvec_full_width_writeback() {
    native(
        "typealias Word=.u128'builtin;fn run()->Word {var value:Word=0;value'bitvec[0..128]=340282366920938463463374607431768211455;value}",
        "(unsigned-integer 340282366920938463463374607431768211455)\n",
    );
}

#[test]
fn native_bitvec_writeback_early_return() {
    native(
        "typealias Word=.u16'builtin;typealias Byte=.u8'builtin;fn write(exit:bool)->Word {var word:Word=0;let i:Byte=4;word'bitvec[i]=do {if exit {return 9;}1};word}fn run()->Word {write(true)}",
        "(unsigned-integer 9)\n",
    );
}

#[test]
fn native_empty_bitvec_values_and_calls() {
    native(
        "typealias Empty=.BitVec'builtin<0>;fn keep(value:Empty)->Empty {not value}fn run()->Empty {keep(0)}",
        "(unsigned-integer 0)\n",
    );
    native(
        "typealias Empty=.BitVec'builtin<0>;typealias Bits=.BitVec'builtin<8>;fn join(empty:Empty,bits:Bits)->bool {(empty ++ bits) == bits and (bits ++ empty) == bits and (empty and empty) == (empty or empty)}fn run()->bool {join(0,0xAF)}",
        "(bool true)\n",
    );
}

#[test]
fn native_empty_bitvec_windows_and_writeback_keep_effects() {
    native(
        "typealias Word=.u16'builtin;typealias Byte=.u8'builtin;fn run()->Word {var word:Word=0x1234;let i:Byte=16;word'bitvec[i..i]=do {word=0xABCD;0};word}",
        "(unsigned-integer 43981)\n",
    );
    native(
        "typealias Empty=.BitVec'builtin<0>;typealias Bits=.BitVec'builtin<8>;typealias Byte=.u8'builtin;pure fn empty(bits:Bits,i:Byte)->Empty where requires i <= 8 {bits[i..i]}fn run()->Byte {empty(255,8).as(Byte)}",
        "(unsigned-integer 0)\n",
    );
}

#[test]
fn native_empty_bitvec_operations_keep_both_operands() {
    for operation in ["and", "or", "++"] {
        native(
            &format!(
                "typealias Empty=.BitVec'builtin<0>;fn run()->bool {{var seen=false;let a:Empty=0;let b:Empty=a {operation} (do {{seen=true;a}});seen and b == a}}"
            ),
            "(bool true)\n",
        );
    }
    native(
        "typealias Empty=.BitVec'builtin<0>;fn slice(value:Empty)->Empty {value[0..0]}fn run()->Empty {slice(0)}",
        "(unsigned-integer 0)\n",
    );
}

#[test]
fn native_empty_bitvec_out_of_bounds_still_traps() {
    for level in ["0", "3"] {
        let w = Work::new(
            "typealias Byte=.u8'builtin;typealias Bits=.BitVec'builtin<8>;typealias Empty=.BitVec'builtin<0>;fn read(i:Byte)->Empty {assumes i <= 8;let bits:Bits=255;bits[i..i]}fn run()->Empty {read(9)}",
        );
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        assert!(!w.execute().status.success());
    }
}

#[test]
fn native_wide_bitvec_values_and_writeback() {
    native(
        "typealias Bits=.BitVec'builtin<256>;typealias Half=.BitVec'builtin<128>;typealias Byte=.u8'builtin;fn run()->bool {var bits:Bits=0;let i:Byte=200;bits[i..(i + 4)]=0b1010;bits[200..204] == 10 and bits[0..128] == 0}",
        "(bool true)\n",
    );
    native(
        "typealias Half=.BitVec'builtin<128>;typealias Full=.BitVec'builtin<256>;fn join(a:Half,b:Half)->Full {a ++ b}fn run()->bool {let value=join(340282366920938463463374607431768211455,1);value[128..256] == 340282366920938463463374607431768211455 and value[0..128] == 1}",
        "(bool true)\n",
    );
    native(
        "typealias Bits=.BitVec'builtin<256>;fn invert(value:Bits)->Bits {not value}fn run()->bool {let value=invert(0);var target:Bits=0;target[0..256]=value;target == value}",
        "(bool true)\n",
    );
}

#[test]
fn native_wide_bitvec_constants_and_odd_widths() {
    native(
        "typealias Bits=.BitVec'builtin<257>;fn mask()->Bits {not 0}fn run()->bool {let value=mask();value[256] == 1 and value[128..256] == 340282366920938463463374607431768211455 and value[0..128] == 340282366920938463463374607431768211455}",
        "(bool true)\n",
    );
    native(
        "typealias Bits=.BitVec'builtin<1024>;typealias Position=.u16'builtin;fn write(i:Position)->Bits where requires i <= 768 {var bits:Bits=0;bits[i..(i + 256)]=not 0;bits}fn run()->bool {let bits=write(768);bits[768..1024] == (not 0) and bits[0..768] == 0}",
        "(bool true)\n",
    );
}

#[test]
fn shipped_wide_bitvec_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!(
            "../../../development/examples/host-wide-bitvec.fl"
        ));
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn shipped_wide_enum_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!(
            "../../../development/examples/host-wide-enum.fl"
        ));
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_wide_enum_expression_encoding() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias Bits=.BitVec'builtin<257>;comptime fn high()->Bits {{not 0}}enum Mask {{.width 257,.discrete_tag true,empty=0,full=high(),}}impl Duplicate for Mask {{}}fn check(value:Mask)->bool {{let bits=value'bitvec;value match {{.full=>bits[256] == 1 and bits[0..256] == (not 0),.empty=>bits == 0,}}}}fn run()->bool {{check(Mask.full) and check(Mask.empty)}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn shipped_payload_enum_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!(
            "../../../development/examples/host-payload-enum.fl"
        ));
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_enum_mixed_payloads_and_nested_variants() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias F=.f32'builtin;typealias Empty=.BitVec'builtin<0>;enum Inner {{flag(bool),number(F),empty(Empty)}}impl Duplicate for Inner {{}}enum Outer {{none,wrapped(Inner)}}impl Duplicate for Outer {{}}fn read(value:Outer)->bool {{value match {{.none=>false,.wrapped(inner)=>inner match {{.flag(flag)=>flag,.number(n)=>n == 1.5,.empty(e)=>e == 0,}},}}}}fn run()->bool {{read(Outer.wrapped(Inner.number(1.5))) and read(Outer.wrapped(Inner.empty(0))) and read(Outer.wrapped(Inner.flag(true)))}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_enum_payload_pointer_reassignment() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} enum Choice {{none,value(bool)}}impl Duplicate for Choice {{}}fn set(pointer:*Choice) {{pointer.* = Choice.value(true);}}fn run()->bool {{var choice=Choice.none;set(choice.ref);choice match {{.none=>false,.value(flag)=>flag,}}}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_enum_named_payload_initializes_in_declaration_order() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} enum Choice {{none,pair {{left:bool,right:bool}}}}impl Duplicate for Choice {{}}fn run()->bool {{var seen=false;let choice=Choice.pair {{right:seen,left:do {{seen=true;true}}}};choice match {{.none=>false,.pair {{left,right}}=>left and right,}}}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_enum_payload_destructor_is_not_discarded() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Token {{}}impl Retire for Token {{fn drop(*self) {{}}}}enum Choice {{none,value(Token)}}fn run()->bool {{let choice=Choice.value(Token {{}});true}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn shipped_bit_pattern_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!(
            "../../../development/examples/host-bit-pattern.fl"
        ));
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_bit_pattern_alternatives_and_negation() {
    native(
        "typealias Bits=.BitVec'builtin<4>;typealias Tail=.BitVec'builtin<2>;fn extract(value:Bits)->Tail {value match {0b(head:Tail) 11 or 0b00 (head:Tail)=>head,_=>0,}}fn outside(value:Bits)->bool {value match {not (0b10 (_:Tail))=>true,0b10 (_:Tail)=>false,}}fn run()->bool {extract(7) == 1 and extract(2) == 2 and extract(3) == 0 and outside(15) and not outside(8)}",
        "(bool true)\n",
    );
}

#[test]
fn native_wide_bit_pattern_bindings() {
    native(
        "typealias Bits=.BitVec'builtin<257>;typealias Payload=.BitVec'builtin<256>;fn check(value:Bits)->bool {value match {0b1 (data:Payload)=>data == (not 0),_=>false,}}fn run()->bool {check(not 0) and not check(0)}",
        "(bool true)\n",
    );
}

#[test]
fn native_bit_pattern_guard_fallthrough() {
    native(
        "typealias Bits=.BitVec'builtin<4>;typealias Tail=.BitVec'builtin<2>;fn check(value:Bits)->bool {var seen=false;value match {0b10 (data:Tail) if do {seen=true;false}=>false,0b10 (_:Tail)=>seen,_=>not seen,}}fn run()->bool {check(9) and check(1)}",
        "(bool true)\n",
    );
}

#[test]
fn native_bit_pattern_signed_and_float_fields() {
    native(
        "typealias Bits=.BitVec'builtin<8>;typealias Signed=.i8'builtin;fn unpack(value:Bits)->Signed {value match {0b(number:Signed)=>number}}fn run()->Signed {unpack(128)}",
        "(signed-integer -128)\n",
    );
    native(
        "typealias Bits=.BitVec'builtin<32>;typealias F=.f32'builtin;fn unpack(value:Bits)->F {value match {0b(number:F)=>number}}fn run()->F {unpack(0x80000000)}",
        "(float32 (bits 80000000))\n",
    );
}

#[test]
fn native_bit_pattern_subject_is_evaluated_once() {
    native(
        "typealias Bits=.BitVec'builtin<4>;typealias Tail=.BitVec'builtin<2>;fn run()->bool {var seen=false;let subject:Bits=9;(do {seen=not seen;subject}) match {0b00 (_:Tail)=>false,0b10 (part:Tail)=>seen and part == 1,_=>false,}}",
        "(bool true)\n",
    );
}

#[test]
fn native_bit_pattern_width_error_preserves_output() {
    let w = Work::new(
        "typealias Bits=.BitVec'builtin<4>;typealias Tail=.BitVec'builtin<3>;fn check(value:Bits)->bool {value match {0b10 (_:Tail)=>true,_=>false,}}fn run()->bool {check(9)}",
    );
    fs::write(w.0.join("native program"), b"old").unwrap();
    let out = w.build();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("elaboration failed"));
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"old");
}

#[test]
fn native_error_results_success_and_failure() {
    native(
        "fn choose(failed:bool)->!bool bool {if failed {error true} else {false}}fn recover(value:!bool bool)->bool {value match {ok! =>not ok,error failed=>failed,}}fn run()->bool {recover(choose(false)) and recover(choose(true))}",
        "(bool true)\n",
    );
}

#[test]
fn native_error_rows_reorder_by_type_identity() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} enum A {{bad(bool)}}enum B {{bad(bool)}}impl Duplicate for A {{}}impl Duplicate for B {{}}fn source(failed:bool)->![A,B] bool {{if failed {{error A.bad(true)}} else {{error B.bad(false)}}}}fn widen(failed:bool)->![B,A] bool {{source(failed)}}fn recover(value:![B,A] bool)->bool {{value match {{ok! =>ok,error A.bad(flag)=>flag,error B.bad(flag)=>not flag,}}}}fn run()->bool {{recover(widen(true)) and recover(widen(false))}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_error_propagation_and_handler() {
    native(
        "fn check(failed:bool)->!bool bool {if failed {error false} else {true}}fn forward(failed:bool)->!bool bool {check(failed)!}fn recover(failed:bool)->bool {forward(failed)! {false=>true,true=>false,}}fn run()->bool {recover(false) and recover(true)}",
        "(bool true)\n",
    );
}

#[test]
fn shipped_error_results_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!("../../../development/examples/host-errors.fl"));
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_error_rows_widen_and_keep_success() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} enum A {{bad}}enum B {{bad}}impl Duplicate for A {{}}impl Duplicate for B {{}}fn small(failed:bool)->!A bool {{if failed {{error A.bad}} else {{true}}}}fn wide(failed:bool)->![B,A] bool {{small(failed)}}fn recover(value:![B,A] bool)->bool {{value match {{ok! =>ok,error A.bad=>true,error B.bad=>false,}}}}fn run()->bool {{recover(wide(true)) and recover(wide(false))}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_error_payload_destructor_is_not_discarded() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Token {{}}impl Retire for Token {{fn drop(*self) {{}}}}fn fail()->!Token bool {{error Token {{}}}}fn run()->bool {{let result=fail();true}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_error_product_transport_preserves_tuple_head_and_fields() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER}{TUPLE_HEADER} enum Failure where T {{Bad(T)}}enum Other {{Bad}}impl Duplicate for Failure<bool> {{}}impl Duplicate for Other {{}}fn sample()->![Other,Failure<bool>] bool {{error Failure<bool>.Bad(true)}}fn packet()->Tuple<Tuple<![Other,Failure<bool>] bool,bool>,bool> {{((sample(),true),true)}}fn forward(value:Tuple<Tuple<![Failure<T>,Other] bool,bool>,bool>)->bool where T {{let ((result,first),second)=value;result! {{Failure<T>.Bad(_)=>first and second,Other.Bad=>false}}}}fn run()->bool {{forward(packet())}}"
    ));
    let out = w
        .command()
        .args([
            "--build",
            "run",
            "--meta-list-type",
            "input.TupleElements",
            "--tuple-type",
            "input.Tuple",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    success(&out);
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_error_product_transport_in_record() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} fn record(...fields:T)->T where T {{fields}}enum Failure where T {{Bad(T)}}enum Other {{Bad}}impl Duplicate for Failure<bool> {{}}impl Duplicate for Other {{}}fn sample()->![Other,Failure<bool>] bool {{error Failure<bool>.Bad(true)}}fn expected()->![Failure<T>,Other] bool where T {{true}}fn forward(value:(record {{result:expected<T>(),flag:true}})'type)->bool where T {{value.result! {{Failure<T>.Bad(_)=>value.flag,Other.Bad=>false}}}}fn run()->bool {{forward(record {{result:sample(),flag:true}})}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn shipped_error_product_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!(
            "../../../development/examples/host-error-product.fl"
        ));
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

fn prepared_host(source: &str) -> luna::host::Prepared {
    let w = Work::new(source);
    let command = luna::cli::parse([
        w.0.join("source.fl").into_os_string(),
        "--build".into(),
        "run".into(),
        "--output".into(),
        "native program".into(),
        "--copy-protocol".into(),
        "Duplicate".into(),
        "--drop-protocol".into(),
        "Retire".into(),
    ])
    .unwrap();
    let luna::cli::Command::Host { options, .. } = command else {
        panic!("host command")
    };
    let (artifact, bindings) = luna::frontend::compile_bound_with_resource_protocols(
        &options,
        Some("Duplicate"),
        Some("Retire"),
    )
    .unwrap();
    let luna::frontend::Artifact::Nir(package) = artifact else {
        panic!("NIR")
    };
    luna::host::prepare_with_protocols(
        &package,
        "run",
        100_000,
        bindings.copy.unwrap(),
        bindings.drop.unwrap(),
    )
    .unwrap()
}

#[test]
fn host_preparation_retains_destructor_schedule_for_inspection() {
    let prepared = prepared_host(&format!(
        "{RESOURCE_HEADER} struct Token {{}}impl Retire for Token {{fn drop(*self) {{}}}}enum Choice {{none,some(Token)}}fn run()->bool {{let value=Choice.some(Token {{}});true}}"
    ));
    let dump = prepared.to_string();
    assert!(dump.contains("cleanup-invoke"), "{dump}");
    assert!(!prepared.schedule().functions.is_empty());
    assert!(!prepared.graph().functions.is_empty());
    let ir = luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
    assert!(ir.contains("call void @flurry_"), "{ir}");
    assert_eq!(prepared.to_string(), dump);
}

#[test]
fn host_prepared_input_can_be_lowered_repeatedly_without_mutation() {
    let prepared = prepared_host(&format!("{RESOURCE_HEADER} fn run()->bool {{true}}"));
    let before = prepared.to_string();
    assert!(luna::host::llvm_ir_prepared(&prepared, 0).is_err());
    let a = luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
    let b = luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
    assert_eq!(a, b);
    assert_eq!(before, prepared.to_string());
}

#[test]
fn native_direct_local_destructor_observes_real_storage() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = true;}}}}fn run()->bool {{var seen=false;do {{let watch=Watch {{target:seen.ref}};}};seen}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_direct_destructors_run_in_reverse_order() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct First {{first:*bool,second:*bool}}struct Second {{second:*bool}}impl Retire for First {{fn drop(*self) {{self.first.*.* = self.second.*.*;}}}}impl Retire for Second {{fn drop(*self) {{self.second.*.* = true;}}}}fn run()->bool {{var first=false;var second=false;do {{let a=First {{first:first.ref,second:second.ref}};let b=Second {{second:second.ref}};}};first and second}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_direct_destructor_runs_once_per_loop_iteration() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} typealias Byte=.u8'builtin;struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn run()->bool {{var seen=false;var i:Byte=0;while i < 3 {{let watch=Watch {{target:seen.ref}};i=i + 1;}};seen}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_direct_empty_destructor_on_return() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Token {{}}impl Retire for Token {{fn drop(*self) {{}}}}fn run()->bool {{let token=Token {{}};return true;}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn shipped_direct_destructor_at_each_optimization_level() {
    for level in ["0", "1", "2", "3"] {
        let w = Work::new(include_str!(
            "../../../development/examples/host-destructor.fl"
        ));
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_direct_parameter_destructor_at_function_end() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn consume(watch:Watch) {{}}fn run()->bool {{var seen=false;consume(Watch {{target:seen.ref}});seen}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_direct_return_does_not_repeat_function_cleanup() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn consume(watch:Watch) {{return;}}fn run()->bool {{var seen=false;consume(Watch {{target:seen.ref}});seen}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_direct_loop_exits_cleanup_once() {
    for control in ["break", "continue"] {
        let w = Work::new(&format!(
            "{RESOURCE_HEADER} typealias Byte=.u8'builtin;struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn run()->bool {{var seen=false;var i:Byte=0;while i < 1 {{let watch=Watch {{target:seen.ref}};i=i + 1;{control};}};seen}}"
        ));
        success(&resource_build(&w));
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_direct_return_value_precedes_destructor() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = true;}}}}fn leave(flag:*bool)->bool {{let watch=Watch {{target:flag}};return flag.*;}}fn run()->bool {{var seen=false;let before=leave(seen.ref);not before and seen}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_discarded_destructor_runs_before_next_statement() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn make(target:*bool)->Watch {{Watch {{target:target}}}}fn run()->bool {{var seen=false;make(seen.ref);let first=seen;Watch {{target:seen.ref}};first and not seen}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_discard_skips_unconstructed_value_after_return() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn maybe(target:*bool,leave:bool) {{do {{if leave {{return;}}Watch {{target:target}}}};}}fn run()->bool {{var seen=false;maybe(seen.ref,true);let untouched=not seen;maybe(seen.ref,false);untouched and seen}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_replacement_drops_old_value_after_rhs_before_store() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn run()->bool {{var seen=false;var old_seen=false;var replaced=false;do {{var watch=Watch {{target:seen.ref}};watch=do {{old_seen=seen;Watch {{target:seen.ref}}}};replaced=seen;}};not old_seen and replaced and not seen}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_nested_struct_fields_are_destroyed() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}struct Pair {{left:Watch,right:Watch}}struct Outer {{pair:Pair}}fn run()->bool {{var left=false;var right=false;do {{let value=Outer {{pair:Pair {{left:Watch {{target:left.ref}},right:Watch {{target:right.ref}}}}}};}};left and right}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_replacement_of_field_keeps_sibling_alive() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}struct Pair {{left:Watch,right:Watch}}fn run()->bool {{var old=false;var next=false;var sibling=false;var during=false;do {{var pair=Pair {{left:Watch {{target:old.ref}},right:Watch {{target:sibling.ref}}}};pair.left=Watch {{target:next.ref}};during=old and not next and not sibling;}};during and old and next and sibling}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_parent_destructor_precedes_field_destructor() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}struct Parent {{child:Watch}}impl Retire for Parent {{fn drop(*self) {{self.child.target.*.* = false;}}}}fn run()->bool {{var seen=true;do {{let parent=Parent {{child:Watch {{target:seen.ref}}}};}};seen}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_replacement_rhs_return_only_runs_exit_cleanup() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn replace(flag:*bool,leave:bool) {{var watch=Watch {{target:flag}};watch=do {{if leave {{return;}}Watch {{target:flag}}}};}}fn run()->bool {{var seen=false;replace(seen.ref,true);seen}}"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_predicate_failure_lowers_cleanup_before_propagation() {
    let source = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn check(flag:bool,target:*bool) {{let watch=Watch {{target:target}};assumes flag;}}fn run()->bool {{var seen=false;check(false,seen.ref);seen}}"
    );
    let w = Work::new(&source);
    let out = w
        .command()
        .args([
            "--emit-llvm",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&out);
    let ir = String::from_utf8(out.stdout).unwrap();
    let lines = ir.lines().collect::<Vec<_>>();
    assert!(
        lines.iter().enumerate().any(|(index, line)| {
            line.trim_start().starts_with("call void @flurry_")
                && line.contains("(ptr ")
                && lines[index + 1..]
                    .iter()
                    .take_while(|line| !line.trim_start().starts_with("ret "))
                    .any(|line| line.trim() == "call void @flurry_check_failure()")
        }),
        "{ir}"
    );
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        assert!(!output.status.success());
        let diagnostic = String::from_utf8(output.stderr).unwrap();
        let start = source.find("assumes flag;").unwrap() + "assumes ".len();
        assert!(
            diagnostic.contains(&format!("(predicate {start} {})", start + 4)),
            "{diagnostic}"
        );
        assert_eq!(
            diagnostic.matches("runtime-check-failure").count(),
            1,
            "{diagnostic}"
        );
    }
    let pass = Work::new(&source.replace("check(false,seen.ref)", "check(true,seen.ref)"));
    success(&resource_build(&pass));
    let out = pass.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
    assert!(out.stderr.is_empty());
}

#[test]
fn native_function_contracts_retain_direct_cleanup() {
    for (contract, body) in [
        ("requires flag", "flag"),
        ("requires flag, ensures result", "flag"),
        ("requires flag, ensures result", "return flag;"),
        (
            "requires flag, ensures result",
            "if flag {return true;}false",
        ),
    ] {
        let source = format!(
            "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn check(watch:Watch,flag:bool)->result:bool where {contract} {{{body}}}fn run()->bool {{var seen=false;let result=check(Watch {{target:seen.ref}},true);result and seen}}"
        );
        let w = Work::new(&source);
        let out = w
            .command()
            .args([
                "--emit-llvm",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&out);
        let ir = String::from_utf8(out.stdout).unwrap();
        let lines = ir.lines().collect::<Vec<_>>();
        let failures = lines
            .windows(2)
            .filter(|pair| {
                pair[0].trim() == "call void @flurry_check_failure()"
                    && pair[1].trim() == "store i1 true, ptr @flurry_failed"
            })
            .collect::<Vec<_>>();
        let expected = if !contract.contains("ensures") {
            1
        } else if body.starts_with("if") {
            3
        } else {
            2
        };
        assert_eq!(failures.len(), expected, "{contract}: {body}\n{ir}");
        assert!(
            failures
                .iter()
                .all(|pair| pair[0].trim_start().starts_with("call void @flurry_")),
            "{contract}: {body}\n{ir}"
        );
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n", "{contract}: {body}");
        }
    }
}

#[test]
fn native_variant_cleanup_only_drops_active_payload() {
    let watch = "struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}";
    for (declarations, expression) in [
        (
            "enum Choice {none,some(Watch)} fn make(flag:bool,target:*bool)->Choice {if flag {Choice.some(Watch {target:target})}else{Choice.none}}",
            "make(flag,seen.ref)",
        ),
        (
            "enum Inner {none,some(Watch)}enum Outer {none,some(Inner)}fn make(flag:bool,target:*bool)->Outer {Outer.some(if flag {Inner.some(Watch {target:target})}else{Inner.none})}",
            "make(flag,seen.ref)",
        ),
        (
            "fn make(flag:bool,target:*bool)->!Watch bool {if flag {error Watch {target:target}}else{true}}",
            "make(flag,seen.ref)",
        ),
    ] {
        let source = format!(
            "{RESOURCE_HEADER}{watch}{declarations}fn check(flag:bool)->bool {{var seen=false;do {{let value={expression};}};seen == flag}}fn run()->bool {{check(false) and check(true)}}"
        );
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(
                out.stdout,
                b"(bool true)
",
                "{source}"
            );
        }
    }
}

#[test]
fn native_variant_replacement_and_discard_preserve_cleanup_order() {
    let source = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}enum Choice {{none,some(Watch)}}fn make(flag:bool,target:*bool)->Choice {{if flag {{Choice.some(Watch {{target:target}})}}else{{Choice.none}}}}fn check(flag:bool)->bool {{var first=false;var second=false;var replaced=false;do {{var value=make(flag,first.ref);value=if flag {{Choice.some(Watch {{target:second.ref}})}}else{{Choice.none}};replaced=first == flag and not second;}};let ended=second == flag;make(flag,second.ref);replaced and ended and not second}}fn run()->bool {{check(false) and check(true)}}"
    );
    let prepared = prepared_host(&source);
    if let Err(error) = luna::host::llvm_ir_prepared(&prepared, 100_000) {
        panic!("{error}\n{prepared}");
    }
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_call_failure_cleanup_is_admitted() {
    let source = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}enum Choice {{none,some(Watch)}}fn make(flag:bool,target:*bool)->Choice {{if flag {{Choice.some(Watch {{target:target}})}}else{{Choice.none}}}}fn check(flag:bool)->bool {{var seen=false;var value=make(flag,seen.ref);value=make(flag,seen.ref);true}}fn run()->bool {{check(true)}}"
    );
    let prepared = prepared_host(&source);
    let dump = prepared.to_string();
    assert!(
        dump.contains("predicate-failure") && dump.contains("cleanup-invoke"),
        "{dump}"
    );
    luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
    assert_eq!(prepared.to_string(), dump);
    let w = Work::new(&source);
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_sparse_variant_cleanup_selects_before_dropping() {
    let source = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}enum Choice {{unused,left(Watch),right(Watch)}}fn check(flag:bool)->bool {{var left=false;var right=false;do {{let value=if flag {{Choice.left(Watch {{target:left.ref}})}}else{{Choice.right(Watch {{target:right.ref}})}};}};left == flag and right == not flag}}fn run()->bool {{check(false) and check(true)}}"
    );
    let prepared = prepared_host(&source);
    let dump = prepared.to_string();
    assert!(dump.contains("cleanup-select"), "{dump}");
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_nested_select_checks_outer_tag_before_payload() {
    let source = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}enum Inner {{unused,left(Watch),right(Watch)}}enum Outer {{none,some(Inner)}}fn check(empty:bool,left:bool)->bool {{var seen=false;do {{let value=if empty {{Outer.none}}else{{Outer.some(if left {{Inner.left(Watch {{target:seen.ref}})}}else{{Inner.right(Watch {{target:seen.ref}})}})}};}};seen == not empty}}fn run()->bool {{check(true,false) and check(false,false) and check(false,true)}}"
    );
    let dump = prepared_host(&source).to_string();
    assert!(dump.contains("cleanup-select"), "{dump}");
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_conditional_whole_move_drops_exactly_once() {
    let source = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn check(flag:bool)->bool {{var seen=false;do {{let watch=Watch {{target:seen.ref}};if flag {{watch;}}}};seen}}fn run()->bool {{check(false) and check(true)}}"
    );
    let dump = prepared_host(&source).to_string();
    assert!(dump.contains("cleanup-if-constructed"), "{dump}");
    if let Err(error) = luna::host::llvm_ir_prepared(&prepared_host(&source), 100_000) {
        panic!("{error}\n{dump}");
    }
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_conditional_move_state_resets_on_assignment_and_loop_entry() {
    let header = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}"
    );
    for body in [
        "var seen=false;do {var watch=Watch {target:seen.ref};if flag {watch;}watch=Watch {target:seen.ref};};not seen",
        "var seen=false;var again=true;while true {let watch=Watch {target:seen.ref};if flag {watch;}if again {again=false;continue;}break;};not seen",
    ] {
        let source = format!(
            "{header}fn check(flag:bool)->bool {{{body}}}fn run()->bool {{check(false) and check(true)}}"
        );
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n", "{body}");
        }
    }
}

#[test]
fn native_partial_move_preserves_sibling_cleanup() {
    let header = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}struct Pair {{a:Watch,b:Watch}}struct Wrapper {{pair:Pair}}"
    );
    for (initialize, moved, reassignment, expected) in [
        (
            "Pair {a:Watch {target:left.ref},b:Watch {target:right.ref}}",
            "pair",
            "pair.a=Watch {target:left.ref};",
            "not left and right",
        ),
        (
            "Pair {a:Watch {target:left.ref},b:Watch {target:right.ref}}",
            "pair",
            "pair.a=Watch {target:left.ref};pair.b=Watch {target:right.ref};",
            "not left and not right",
        ),
        (
            "Pair {a:Watch {target:left.ref},b:Watch {target:right.ref}}",
            "pair.a",
            "",
            "left and right",
        ),
        (
            "Pair {a:Watch {target:left.ref},b:Watch {target:right.ref}}",
            "pair.a",
            "pair.a=Watch {target:left.ref};",
            "not left and right",
        ),
        (
            "Wrapper {pair:Pair {a:Watch {target:left.ref},b:Watch {target:right.ref}}}",
            "pair.pair.a",
            "",
            "left and right",
        ),
    ] {
        let source = format!(
            "{header}fn check(flag:bool)->bool {{var left=false;var right=false;do {{var pair={initialize};if flag {{{moved};}}{reassignment}}};{expected}}}fn run()->bool {{check(false) and check(true)}}"
        );
        let prepared = prepared_host(&source);
        let dump = prepared.to_string();
        assert!(dump.contains("cleanup-if-constructed"), "{dump}");
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n", "{source}");
        }
    }
}

#[test]
fn native_pattern_residual_and_bound_field_are_dropped_once() {
    let header = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}struct Pair {{a:Watch,b:Watch}}"
    );
    for body in [
        "let {a:kept,b:_}=Pair {a:Watch {target:left.ref},b:Watch {target:right.ref}};",
        "Pair {a:Watch {target:left.ref},b:Watch {target:right.ref}} match {{a:kept,b:_}=>{kept;},};",
        "Pair {a:Watch {target:left.ref},b:Watch {target:right.ref}} match {{a:kept,b:_} if flag=>{kept;}, {a:_,b:kept}=>{kept;},};",
    ] {
        let source = format!(
            "{header}fn check(flag:bool)->bool {{var left=false;var right=false;do {{{body}}};left and right}}fn run()->bool {{check(false) and check(true)}}"
        );
        let prepared = prepared_host(&source);
        let dump = prepared.to_string();
        assert!(dump.contains("temporary-target"), "{dump}");
        if let Err(error) = luna::host::llvm_ir_prepared(&prepared, 100_000) {
            panic!("{error}\n{dump}");
        }
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n", "{body}");
        }
    }
}

#[test]
fn native_let_else_retains_failed_subject_until_return_cleanup() {
    let source = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}struct Packet {{flag:bool,a:Watch,b:Watch}}fn consume(flag:bool,left:*bool,right:*bool) {{let {{flag:true,a:kept,b:_}}=Packet {{flag:flag,a:Watch {{target:left}},b:Watch {{target:right}}}} else {{return;}};}}fn check(flag:bool)->bool {{var left=false;var right=false;consume(flag,left.ref,right.ref);left and right}}fn run()->bool {{check(false) and check(true)}}"
    );
    let prepared = prepared_host(&source);
    if let Err(error) = luna::host::llvm_ir_prepared(&prepared, 100_000) {
        panic!("{error}\n{prepared}");
    }
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_or_pattern_tracks_actual_moved_field() {
    let source = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}struct Packet {{a:Watch,b:Watch,flag:bool}}fn check(flag:bool)->bool {{var left=false;var right=false;var middle=false;Packet {{flag:flag,a:Watch {{target:left.ref}},b:Watch {{target:right.ref}}}} match {{{{flag:true,a:chosen,b:_}} or {{flag:false,a:_,b:chosen}}=>{{chosen;middle=left == flag and right == not flag;}},}};middle and left and right}}fn run()->bool {{check(false) and check(true)}}"
    );
    let prepared = prepared_host(&source);
    let dump = prepared.to_string();
    assert!(dump.contains("cleanup-if-constructed"), "{dump}");
    if let Err(error) = luna::host::llvm_ir_prepared(&prepared, 100_000) {
        panic!("{error}\n{dump}");
    }
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
    let conditional = Work::new(&source.replace(
        "chosen;middle=left == flag and right == not flag;",
        "if flag {chosen;}middle=true;",
    ));
    success(&resource_build(&conditional));
    let out = conditional.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_or_pattern_cleanup_uses_selected_binding_order() {
    let source = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool,value:bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = self.value.*;}}}}struct Packet {{flag:bool,a:Watch,b:Watch}}fn check(flag:bool)->bool {{var seen=true;Packet {{flag:flag,a:Watch {{target:seen.ref,value:false}},b:Watch {{target:seen.ref,value:true}}}} match {{{{flag:true,a:x,b:y}} or {{flag:false,a:y,b:x}}=>{{}},}};not seen}}fn run()->bool {{check(false) and check(true)}}"
    );
    let prepared = prepared_host(&source);
    let dump = prepared.to_string();
    assert!(dump.contains("(pattern-order "), "{dump}");
    if let Err(error) = luna::host::llvm_ir_prepared(&prepared, 100_000) {
        panic!("{error}\n{dump}");
    }
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_partial_construction_cleans_completed_fields_on_return() {
    let header = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}"
    );
    for (declaration, constructor) in [
        ("struct Packet {first:Watch,last:bool}", "Packet"),
        (
            "enum Packet {none,some {first:Watch,last:bool}}",
            "Packet.some",
        ),
    ] {
        let source = format!(
            "{header}{declaration}fn consume(stop:bool,target:*bool) {{let packet={constructor} {{first:Watch {{target:target}},last:do {{if stop {{return;}}false}}}};}}fn run()->bool {{var seen=false;consume(true,seen.ref);let aborted=seen;seen=false;consume(false,seen.ref);aborted and seen}}"
        );
        let prepared = prepared_host(&source);
        let dump = prepared.to_string();
        assert!(dump.contains("prepared-target"), "{dump}");
        if let Err(error) = luna::host::llvm_ir_prepared(&prepared, 100_000) {
            panic!("{error}\n{dump}");
        }
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_nested_partial_construction_unwinds_inner_before_outer() {
    let source = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool,value:bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = self.value.*;}}}}struct Inner {{first:Watch,last:bool}}struct Outer {{first:Watch,last:Inner}}fn consume(stop:bool,target:*bool) {{let value=Outer {{first:Watch {{target:target,value:false}},last:Inner {{first:Watch {{target:target,value:true}},last:do {{if stop {{return;}}false}}}}}};}}fn run()->bool {{var seen=true;consume(true,seen.ref);not seen}}"
    );
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_partial_construction_cleans_before_loop_exit() {
    for edge in ["break", "continue"] {
        let source = format!(
            "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}struct Packet {{first:Watch,last:bool}}fn check(stop:bool)->bool {{var seen=false;var active=true;while active {{active=false;let value=Packet {{first:Watch {{target:seen.ref}},last:do {{if stop {{{edge};}}false}}}};}}seen}}fn run()->bool {{check(false) and check(true)}}"
        );
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n", "{edge}");
        }
    }
}

#[test]
fn native_argument_preparation_cleans_completed_values_on_return() {
    let header = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}"
    );
    for (sink, call) in [
        (
            "fn sink(first:Watch,last:bool) {}",
            "sink(Watch {target:target},do {if stop {return;}false})",
        ),
        (
            "fn sink(target:*bool,.last:bool=false,.first:Watch=Watch {target:target}) {}",
            "sink(target,.first=Watch {target:target},.last=do {if stop {return;}false})",
        ),
    ] {
        let source = format!(
            "{header}{sink}fn consume(stop:bool,target:*bool) {{{call};}}fn run()->bool {{var seen=false;consume(true,seen.ref);let aborted=seen;seen=false;consume(false,seen.ref);aborted and seen}}"
        );
        let prepared = prepared_host(&source);
        let dump = prepared.to_string();
        assert!(dump.contains("prepared-target"), "{dump}");
        if let Err(error) = luna::host::llvm_ir_prepared(&prepared, 100_000) {
            panic!("{error}\n{dump}");
        }
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_named_argument_unwind_uses_reverse_evaluation_order() {
    let source = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool,value:bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = self.value.*;}}}}fn sink(target:*bool,.b:Watch=Watch {{target:target,value:true}},.a:Watch=Watch {{target:target,value:false}},.last:bool=false) {{}}fn consume(stop:bool,target:*bool) {{sink(target,.a=Watch {{target:target,value:false}},.b=Watch {{target:target,value:true}},.last=do {{if stop {{return;}}false}});}}fn run()->bool {{var seen=true;consume(true,seen.ref);let reversed=not seen;seen=false;consume(false,seen.ref);reversed and seen}}"
    );
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_argument_preparation_cleans_on_break_and_continue() {
    for edge in ["break", "continue"] {
        let source = format!(
            "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn sink(first:Watch,last:bool) {{}}fn check(stop:bool)->bool {{var seen=false;var active=true;while active {{active=false;sink(Watch {{target:seen.ref}},do {{if stop {{{edge};}}false}});}}seen}}fn run()->bool {{check(false) and check(true)}}"
        );
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n", "{edge}");
        }
    }
}

#[test]
fn native_postcondition_retains_owned_result_until_delivery() {
    let header = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}"
    );
    for body in [
        "Watch {target:target}",
        "return Watch {target:target};",
        "if flag {return Watch {target:target};}Watch {target:target}",
    ] {
        let source = format!(
            "{header}fn make(flag:bool,target:*bool)->Watch where requires flag, ensures flag {{{body}}}fn run()->bool {{var seen=false;make(true,seen.ref);seen}}"
        );
        let prepared = prepared_host(&source);
        let dump = prepared.to_string();
        assert!(
            dump.contains("prepared-target") && dump.contains("postcondition-failure"),
            "{dump}"
        );
        let ir = luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
        let drops_before_propagation = ir
            .lines()
            .collect::<Vec<_>>()
            .windows(3)
            .filter(|lines| {
                // Control-state suspension now sits between the destructor
                // and its failure check. The owned-result live flag must
                // still clear before propagating the predicate failure.
                lines[0].trim() == "call void @flurry_check_failure()"
                    && lines[1].trim_start().starts_with("store i1 false")
                    && lines[2].trim() == "store i1 true, ptr @flurry_failed"
            })
            .count();
        assert_eq!(
            drops_before_propagation,
            if body.starts_with("if") { 2 } else { 1 },
            "{ir}"
        );
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_owned_error_rows_preserve_cleanup_responsibility() {
    let header = format!(
        "{RESOURCE_HEADER} struct Fault {{target:*bool}}impl Retire for Fault {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}enum Other {{bad}}impl Duplicate for Other {{}}fn small(failed:bool,target:*bool)->!Fault bool {{if failed {{error Fault {{target:target}}}}else{{true}}}}"
    );
    for (functions, operation) in [
        (
            "fn wide(failed:bool,target:*bool)->![Other,Fault] bool {small(failed,target)}",
            "wide(failed,seen.ref);",
        ),
        (
            "fn record(...fields:T)->T where T {fields}fn sample(failed:bool,target:*bool)->![Other,Fault<bool>] bool {small(failed,target)}fn narrow()->![Other,Fault<bool>] bool {true}fn expected()->![Fault<T>,Other] bool where T {true}fn pack(failed:bool,target:*bool)->(record {left:narrow(),right:true})'type {record {left:sample(failed,target),right:true}}fn accept(value:(record {left:expected<T>(),right:true})'type) where T {}",
            "accept(pack(failed,seen.ref));",
        ),
    ] {
        let header = if operation.starts_with("accept") {
            header
                .replace(
                    "struct Fault {target:*bool}",
                    "struct Fault where T {target:*bool,marker:T}",
                )
                .replace("impl Retire for Fault", "impl Retire for Fault<T> where T")
                .replace("->!Fault bool", "->!Fault<bool> bool")
                .replace(
                    "error Fault {target:target}",
                    "error Fault<bool> {target:target,marker:true}",
                )
        } else {
            header.clone()
        };
        let source = format!(
            "{header}{functions}fn check(failed:bool)->bool {{var seen=false;{operation}seen == failed}}fn run()->bool {{check(false) and check(true)}}"
        );
        let dump = prepared_host(&source).to_string();
        assert!(dump.contains("(conversion embed)"), "{dump}");
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n", "{operation}");
        }
    }
}

#[test]
fn native_default_context_cleans_locals_and_delivers_owned_value() {
    let source = format!(
        "{RESOURCE_HEADER} struct Token {{}}impl Retire for Token {{fn drop(*self) {{}}}}struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}fn sink(target:*bool,.last:Watch=do {{let local=Token {{}};Watch {{target:target}}}}) {{}}fn run()->bool {{var seen=false;sink(seen.ref);let once=seen;sink(seen.ref);once and not seen}}"
    );
    let prepared = prepared_host(&source);
    let dump = prepared.to_string();
    assert!(
        dump.contains("default-context") && dump.contains("cleanup-invoke"),
        "{dump}"
    );
    if let Err(error) = luna::host::llvm_ir_prepared(&prepared, 100_000) {
        panic!("{error}\n{dump}");
    }
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_default_runtime_predicate_remains_rejected_by_p3() {
    let w =
        Work::new("fn sink(flag:bool,.value:bool=do {assumes flag;true}) {}fn run(){sink(true);}");
    let out = w.build();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("default expression requires P3"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn native_staged_callable_identity_dispatch() {
    for source in [
        include_str!("../../../development/examples/host-staged-callback.fl"),
        "typealias Callback=fn(comptime bool,bool)->bool;
         fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
         fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
         fn apply(callback:Callback,value:bool)->bool {
             let first=callback(true,value);let second=callback(false,value);
             first and not second
         }
         fn trial(start:bool)->bool {
             let callback=if start {keep} else {invert};
             let result=apply(callback,not start);
             not result
         }
         fn run()->bool {trial(true) and trial(false)}",
        "typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:T)->bool;
         fn selected(comptime T:Type,value:T)->bool where answer:bool {answer}
         fn trial(flag:bool)->bool {
             var callback:Callback=if flag {selected<true>} else {selected<false>};
             let alias=callback;
             callback=selected<false>;
             alias(bool,true) == flag and not callback(bool,true)
         }
         fn run()->bool {trial(true) and trial(false)}",
    ] {
        let w = Work::new(source);
        let ir = w.command().args(["--emit-llvm", "run"]).output().unwrap();
        success(&ir);
        assert!(
            String::from_utf8_lossy(&ir.stdout).contains("icmp eq i64"),
            "staged identity dispatch must remain in emitted LLVM"
        );
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_staged_callable_preserves_selection_and_argument_effects() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-staged-callback-order.fl"
    ));
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_trait_callable_dispatch() {
    for source in [
        "trait Select {fn select(value:bool)->bool;}
         struct Host {} impl Select for Host {fn select(value:bool)->bool {not value}}
         fn apply(callback:fn(bool)->bool)->bool {callback(false)}
         fn run()->bool {apply(Host.as(Select).select)}",
        "trait Select where enabled:bool {
             fn select(comptime flag:bool,value:bool)->bool {
                 if flag {value == enabled} else {value != enabled}
             }
         }
         struct Host {} impl Select<true> for Host {} impl Select<false> for Host {}
         fn apply(callback:fn(comptime bool,bool)->bool)->bool {
             let a=callback(true,true);let b=callback(false,true);a and not b
         }
         fn trial(start:bool)->bool {
             let yes=Host.as(Select<true>).select;
             let no=Host.as(Select<false>).select;
             let callback=if start {yes} else {no};
             apply(callback) == start
         }
         fn run()->bool {trial(true) and trial(false)}",
        "trait Select {fn select(self,comptime flag:bool,value:bool)->bool;}
         impl Select for bool {
             fn select(self,comptime flag:bool,value:bool)->bool {
                 if flag {value == self} else {value != self}
             }
         }
         fn apply(callback:fn(bool,comptime bool,bool)->bool,host:bool)->bool {
             callback(host,true,true)
         }
         fn trial(start:bool)->bool {apply(bool.as(Select).select,start) == start}
         fn run()->bool {trial(true) and trial(false)}",
        "trait Select {fn select(comptime flag:bool,value:bool)->bool;}
         struct Host {} impl Select for Host {
             fn select(comptime flag:bool,value:bool)->bool {value}
         }
         fn invert(comptime flag:bool,value:bool)->bool {not value}
         fn apply(callback:fn(comptime bool,bool)->bool)->bool {callback(true,true)}
         fn trial(start:bool)->bool {
             let callback=if start {Host.as(Select).select} else {invert};
             apply(callback) == start
         }
         fn run()->bool {trial(true) and trial(false)}",
    ] {
        let w = Work::new(source);
        let first = w.command().args(["--emit-llvm", "run"]).output().unwrap();
        let repeated = w.command().args(["--emit-llvm", "run"]).output().unwrap();
        success(&first);
        success(&repeated);
        assert_eq!(
            first.stdout, repeated.stdout,
            "callable identity numbering must be deterministic"
        );
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_staged_trait_receiver_retains_resource_validation() {
    let source =
        include_str!("../../../development/repros/host-trait-callback-resource-receiver.fl");
    let prepared = prepared_host(source);
    let mut preparations = 0;
    for function in &prepared.graph().functions {
        nir::visit::expressions(&function.function, &mut 100_000, |expression, _| {
            if let nir::program::ExprKind::CallPreparation { statements, .. } = &expression.kind {
                preparations += 1;
                assert!(
                    statements
                        .iter()
                        .all(|statement| !matches!(statement.kind, nir::program::ExprKind::Unit))
                );
            }
            Ok(())
        })
        .unwrap();
    }
    assert!(preparations > 0);
    let w = Work::new(source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_staged_trait_receiver_is_dropped_after_call() {
    let source = format!("{RESOURCE_HEADER}
        trait Select {{fn select(self,comptime flag:bool,value:bool)->bool;}}
        struct Host {{enabled:bool, retired:*bool}}
        impl Retire for Host {{fn drop(*self) {{self.retired.*.* = not self.retired.*.*;}}}}
        impl Select for Host {{
            fn select(self,comptime flag:bool,value:bool)->bool {{
                if flag {{value == self.enabled}} else {{value != self.enabled}}
            }}
        }}
        fn apply(callback:fn(Host,comptime bool,bool)->bool,host:Host)->bool {{callback(host,true,true)}}
        fn trial(start:bool)->bool {{
            var retired=false;
            let answer=apply(Host.as(Select).select,Host {{enabled:start,retired:retired.ref}});
            answer == start and retired
        }}
        fn run()->bool {{trial(true) and trial(false)}}");
    let prepared = prepared_host(&source);
    assert!(prepared.to_string().contains("cleanup-invoke"));
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_mutable_staged_aliases_keep_unique_cleanup_boundaries() {
    let source =
        include_str!("../../../development/repros/host-staged-callback-resource-boundary.fl");
    let prepared = prepared_host(source);
    luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
    let w = Work::new(source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_ordinary_trait_callbacks_retain_resource_evidence() {
    for source in [
        "trait Select {fn select(value:bool)->bool;}
         struct Host {} impl Select for Host {fn select(value:bool)->bool {not value}}
         fn apply(callback:fn(bool)->bool)->bool {callback(false)}
         fn run()->bool {let callback=Host.as(Select).select;apply(callback)}",
        "trait Select where inverted:bool {fn select(value:bool)->bool {value != inverted}}
         struct Host {} impl Select<true> for Host {} impl Select<false> for Host {}
         fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)}
         fn trial(flag:bool)->bool {
             let callback=if flag {Host.as(Select<true>).select} else {Host.as(Select<false>).select};
             apply(callback,false) == flag
         }
         fn run()->bool {trial(true) and trial(false)}",
        "trait Write {fn set(target:*bool);}
         struct Host {} impl Write for Host {fn set(target:*bool) {target.* = true;}}
         fn apply(callback:fn(*bool),target:*bool) {callback(target);}
         fn run()->bool {var state=false;apply(Host.as(Write).set,state.ref);state}",
    ] {
        let source = format!("{RESOURCE_HEADER}{source}");
        let prepared = prepared_host(&source);
        luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(&w.command().args([
                "--build", "run", "--copy-protocol", "Duplicate", "--drop-protocol", "Retire",
                "--opt-level", level, "--output", "native program",
            ]).output().unwrap());
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_trait_callback_cannot_drop_interface_predicates() {
    let source = format!(
        "{RESOURCE_HEADER}
        trait Select {{fn select(value:bool)->bool where requires value;}}
        struct Host {{}} impl Select for Host {{fn select(value:bool)->bool {{value}}}}
        fn apply(callback:fn(bool)->bool)->bool {{callback(false)}}
        fn run()->bool {{apply(Host.as(Select).select)}}"
    );
    let w = Work::new(&source);
    fs::write(w.0.join("native program"), b"previous output").unwrap();
    let out = w
        .command()
        .args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(
        error.contains("trait method value would erase program contracts")
            || error.contains("composed memory contract"),
        "{error}"
    );
    assert_eq!(
        fs::read(w.0.join("native program")).unwrap(),
        b"previous output"
    );
}

#[test]
fn native_conditional_sum_ownership_cleans_active_payload_once() {
    let header = format!(
        "{RESOURCE_HEADER}
        struct Watch {{target:*bool}}
        impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}"
    );
    for (declarations, construction) in [
        (
            "enum Packet {none,some(Watch)} fn make(full:bool,target:*bool)->Packet {if full {Packet.some(Watch {target:target})} else {Packet.none}}",
            "make(full,seen.ref)",
        ),
        (
            "fn make(full:bool,target:*bool)->!Watch bool {if full {error Watch {target:target}} else {true}}",
            "make(full,seen.ref)",
        ),
    ] {
        let source = format!("{header}{declarations}
            fn check(full:bool,moved:bool)->bool {{
                var seen=false;
                do {{let packet={construction};if moved {{packet;}}}};
                seen == full
            }}
            fn run()->bool {{check(false,false) and check(false,true) and check(true,false) and check(true,true)}}");
        let prepared = prepared_host(&source);
        assert!(prepared.to_string().contains("cleanup-if-constructed"));
        luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_conditional_sum_fields_and_replacement_restore_ownership() {
    let header = format!(
        "{RESOURCE_HEADER}
        struct Watch {{target:*bool}}
        impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}
        enum Packet {{none,some(Watch)}}
        struct Pair {{packet:Packet,other:Watch}}"
    );
    for body in [
        "var left=false;var right=false;
         do {let pair=Pair {packet:Packet.some(Watch {target:left.ref}),other:Watch {target:right.ref}};
             if moved {pair.packet;}};
         left and right",
        "var left=false;var right=false;
         do {var packet=Packet.some(Watch {target:left.ref});if moved {packet;}
             packet=Packet.some(Watch {target:right.ref});};
         left and right",
        "var seen=false;var again=true;
         while true {let packet=Packet.some(Watch {target:seen.ref});if moved {packet;}
             if again {again=false;continue;}break;};
         not seen",
    ] {
        let source = format!("{header}fn check(moved:bool)->bool {{{body}}}fn run()->bool {{check(false) and check(true)}}");
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(&w.command().args([
                "--build", "run", "--copy-protocol", "Duplicate", "--drop-protocol", "Retire",
                "--opt-level", level, "--output", "native program",
            ]).output().unwrap());
            let out = w.execute();success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_source_test_cli_reports_failures_and_filters_without_overwriting_output() {
    let w = Work::new("test passing {} test failing {assumes false;} test passing {}");
    let selected = w
        .command()
        .args([
            "--test-native",
            "--opt-level",
            "3",
            "--test-filter",
            "passing",
        ])
        .output()
        .unwrap();
    success(&selected);
    let text = String::from_utf8_lossy(&selected.stdout);
    assert!(
        text.contains("(count 2) (failures 0) (filtered-out 1)"),
        "{text}"
    );
    fs::write(w.0.join("report"), b"previous report").unwrap();
    let failed = w
        .command()
        .args(["--test-native", "--output", "report"])
        .output()
        .unwrap();
    assert!(!failed.status.success());
    let text = String::from_utf8_lossy(&failed.stderr);
    assert!(
        text.contains("(count 3) (failures 1) (filtered-out 0)"),
        "{text}"
    );
    assert_eq!(fs::read(w.0.join("report")).unwrap(), b"previous report");
    for args in [
        vec!["--test-native", "--test"],
        vec!["--test-native", "--list-tests"],
        vec!["--test-native", "--inline-functions"],
    ] {
        assert!(!w.command().args(args).output().unwrap().status.success());
    }
}

#[test]
fn native_source_test_cli_preserves_resource_cleanup() {
    let source = format!(
        "{RESOURCE_HEADER}
        struct Watch {{target:*bool}}
        impl Retire for Watch {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}
        test cleanup {{var seen=false;do {{let watch=Watch {{target:seen.ref}};}};assumes seen;}}"
    );
    let w = Work::new(&source);
    let output = w
        .command()
        .args([
            "--test-native",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("(count 1) (failures 0)"));
}

#[test]
fn native_source_test_packages_select_primary_identity() {
    let w = Work::new("test shared {}");
    fs::write(w.0.join("dependency.fl"), "test shared {assumes false;}").unwrap();
    for (primary, expected) in [("1", true), ("2", false)] {
        let out = Command::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&w.0)
            .args([
                "--packages-v1",
                "--package",
                "1",
                "701",
                "app",
                "source.fl",
                "--package",
                "2",
                "901",
                "dependency",
                "dependency.fl",
                "--primary",
                primary,
                "--test-native",
                "--opt-level",
                "3",
                "--test-filter",
                "shared",
            ])
            .output()
            .unwrap();
        assert_eq!(
            out.status.success(),
            expected,
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(if expected { &out.stdout } else { &out.stderr });
        assert!(text.contains("(count 1)"), "{text}");
        assert!(
            text.contains(if expected {
                "(package 701)"
            } else {
                "(package 901)"
            }),
            "{text}"
        );
    }
}

#[test]
fn shipped_native_source_tests_execute_at_o0_and_o3() {
    let w = Work::new(include_str!("../../../development/examples/host-tests.fl"));
    for level in ["0", "3"] {
        let out = w
            .command()
            .args(["--test-native", "--opt-level", level])
            .output()
            .unwrap();
        success(&out);
        assert!(String::from_utf8_lossy(&out.stdout).contains("(count 2) (failures 0)"));
    }
}

#[test]
fn native_source_test_cli_runtime_deadline_is_configurable() {
    let w = Work::new("test endless {while true {}} test following {}");
    let normal = w
        .command()
        .args(["--test-native", "--test-timeout-ms", "200"])
        .output()
        .unwrap();
    let packaged = Command::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&w.0)
        .args([
            "--packages-v1",
            "--package",
            "1",
            "701",
            "app",
            "source.fl",
            "--primary",
            "1",
            "--test-timeout-ms",
            "200",
            "--test-native",
        ])
        .output()
        .unwrap();
    for output in [normal, packaged] {
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("timed out after 200ms"), "{error}");
        assert!(error.contains("(count 2) (failures 1)"), "{error}");
        assert!(error.contains("(name (string \"following\"))"), "{error}");
    }
}

#[test]
fn native_source_test_cli_rejects_invalid_runtime_deadlines() {
    let w = Work::new("test example {}");
    for value in ["0", "-1", "+1", "1.5", "x", "18446744073709551616", ""] {
        let output = w
            .command()
            .args(["--test-native", "--test-timeout-ms", value])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("--test-timeout-ms"));
    }
    for args in [
        vec!["--test", "--test-timeout-ms", "1"],
        vec!["--list-tests", "--test-timeout-ms", "1"],
        vec!["--test-native", "--test-timeout-ms"],
        vec![
            "--test-native",
            "--test-timeout-ms",
            "1",
            "--test-timeout-ms",
            "2",
        ],
    ] {
        assert_eq!(
            w.command().args(args).output().unwrap().status.code(),
            Some(2)
        );
    }
}

#[test]
fn native_uncalled_closure_environment_cleanup() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        typealias Action=%{watch:Watch} fn()->Watch;
        fn make(target:*bool)->Action {let watch=Watch {target:target};||watch}
        fn consume(action:Action) {}
        fn pack(...fields:T)->T where T {fields}";
    for body in [
        "let seen=false;do {let watch=Watch {target:seen.ref};let action=||watch;};seen",
        "let seen=false;do {let watch=Watch {target:seen.ref};let action=||watch;let moved=action;};seen",
        "let seen=false;make(seen.ref);seen",
        "let seen=false;consume(make(seen.ref));seen",
        "let first=false;let second=false;consume(make(first.ref));consume(make(second.ref));first and second",
        "let seen=false;do {let watch=Watch {target:seen.ref};let inner=||watch;let outer=||inner;};seen",
        "let seen=false;do {let watch=Watch {target:seen.ref};let action=||watch;let product=pack {callback:action};};seen",
        "let first=false;let second=false;var replaced=false;do {let watch=Watch {target:first.ref};var action=||watch;let replacement=Watch {target:second.ref};action=||replacement;replaced=first and not second;};replaced and first and second",
        "let seen=false;do {let action=make(seen.ref);};seen",
        "let seen=false;while true {let action=make(seen.ref);break;};seen",
    ] {
        let source = format!("{header} fn run()->bool {{{body}}}");
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        assert!(
            reference.status.success(),
            "{body}: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        assert_eq!(reference.stdout, b"(bool true)\n", "{body}");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{body}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout, "{body}");
        }
    }
}

#[test]
fn native_uncalled_closure_captures_drop_in_reverse_order_without_calling_body() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Set {target:*bool}
        impl Retire for Set {fn drop(*self) {self.target.*.* = true;}}
        struct Observe {target:*bool, observed:*bool}
        impl Retire for Observe {fn drop(*self) {self.observed.*.* = self.target.*.*;}}
        fn run()->bool {
            let target=false;let observed=true;let invoked=false;let body=invoked.ref;
            do {
                let first=Set {target:target.ref};
                let last=Observe {target:target.ref,observed:observed.ref};
                let action=|| {first;last;body.* = true;};
            };
            target and not observed and not invoked
        }";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn native_uncalled_closure_conditional_move_cleans_exactly_once() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        fn exercise(flag:bool)->bool {let seen=false;do {let watch=Watch {target:seen.ref};let action=||watch;if flag {let moved=action;}};seen}
        fn run()->bool {exercise(true) and exercise(false)}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn native_closure_caller_failure_unwind_is_admitted() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        typealias Action=%{watch:Watch} fn()->Watch;
        fn make(target:*bool)->Action {let watch=Watch {target:target};||watch}
        fn run()->bool {let first=false;let second=false;var action=make(first.ref);action=make(second.ref);true}";
    let w = Work::new(source);
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn shipped_closure_environment_cleanup_executes_natively() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-closure-cleanup.fl"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_checked_closure_calls_clean_parameters_and_locals() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}";
    for body in [
        "let first=false;let second=false;let destination=second.ref;var watch=Watch {target:first.ref};var replaced=false;do {let action=|| {watch=Watch {target:destination};};action();replaced=first and not second;};replaced and first and second",
        "let seen=false;do {let watch=Watch {target:seen.ref};let action=||watch.target.*;let before=action();if before {return false;}};seen",
        "let flag=true;let action=||flag;action()",
        "let seen=false;let pointer=seen.ref;let action=|| {let watch=Watch {target:pointer};};action();let first=seen;action();first and not seen",
        "let seen=false;let pointer=seen.ref;let action=|early:bool| {let watch=Watch {target:pointer};if early {return;} };action(true);let first=seen;action(false);first and not seen",
        "let seen=false;let pointer=seen.ref;let action=|| {pointer.* = not pointer.*;pointer.*};let first=action();let second=action();first and not second",
    ] {
        let w = Work::new(&format!("{header} fn run()->bool {{{body}}}"));
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        assert!(
            reference.status.success(),
            "{body}: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        assert_eq!(reference.stdout, b"(bool true)\n", "{body}");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{body}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout, "{body}");
        }
    }
}

#[test]
fn native_checked_empty_lambda_parameter_drop_is_executed() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);} struct Ticket {}
        impl Retire for Ticket {fn drop(*self) {assumes false;}}
        fn run()->bool {let action=|ticket:Ticket|true;action(Ticket {})}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    assert!(!reference.status.success());
    assert!(
        String::from_utf8_lossy(&reference.stderr).contains("ProgramContractRejected"),
        "{}",
        String::from_utf8_lossy(&reference.stderr)
    );
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        assert!(!w.execute().status.success());
    }
}

#[test]
fn native_checked_closure_nested_contexts_execute() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        fn run()->bool {let seen=false;let pointer=seen.ref;let inner=|| {let watch=Watch {target:pointer};};let outer=||inner();outer();seen}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn shipped_checked_closure_calls_execute_natively() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-closure-call.fl"
    ));
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_checked_returned_closure_preserves_environment_and_cleanup() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        typealias Action=%{target:*bool} fn()->.void'builtin;
        fn make(target:*bool)->Action {|| {let watch=Watch {target:target};}}
        fn run()->bool {let seen=false;let action=make(seen.ref);action();let first=seen;action();first and not seen}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn native_checked_closure_distinct_instantiations_do_not_share_source_only_plan() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        fn make(value:T)->%{value:T} fn()->T where T {||value}
        fn run()->bool {let number:.u8'builtin=1;let first=make(true);let second=make(number);first() and second()==1}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn native_generic_closure_instances_preserve_distinct_owned_cleanup() {
    for (first, second) in [(true, false), (false, false), (true, true)] {
        let w = Work::new(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
             impl Duplicate for bool {{}}
             struct First {{valid:bool}} struct Second {{valid:bool}}
             impl Retire for First {{fn drop(*self) {{assumes self.valid.*;}}}}
             impl Retire for Second {{fn drop(*self) {{assumes not self.valid.*;}}}}
             fn make(value:T)->%{{value:T}} fn()->T where T {{||value}}
             fn run()->bool {{
                 let a=make(First {{valid:{first}}});
                 let b=make(Second {{valid:{second}}});
                 a();b();true
             }}",
        ));
        let expected = first && !second;
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        assert_eq!(
            reference.status.success(),
            expected,
            "{}",
            String::from_utf8_lossy(&reference.stderr)
        );
        if expected {
            assert_eq!(reference.stdout, b"(bool true)\n");
        }
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            assert_eq!(
                out.status.success(),
                expected,
                "first={first}, second={second}, O{level}: {out:?}"
            );
            if expected {
                assert_eq!(out.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn native_checked_nested_callable_scopes_share_only_matching_plans() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        typealias Action=%{target:*bool} fn()->.void'builtin;
        fn apply(action:Action) {action();}
        fn relay(action:Action) {apply(action);}";
    for body in [
        "let seen=false;let target=seen.ref;let action=|| {let watch=Watch {target:target};};apply(action);seen",
        "let seen=false;let target=seen.ref;let action=|| {let watch=Watch {target:target};};relay(action);seen",
        "let seen=false;let target=seen.ref;let outer=|| {let inner=|| {let watch=Watch {target:target};};relay(inner);};outer();let first=seen;outer();first and not seen",
        "let seen=false;let target=seen.ref;let inner=|| {let watch=Watch {target:target};};let middle=||inner();let outer=||middle();outer();let first=seen;outer();first and not seen",
    ] {
        let w = Work::new(&format!("{header} fn run()->bool {{{body}}}"));
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        assert!(
            reference.status.success(),
            "{body}: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        assert_eq!(reference.stdout, b"(bool true)\n", "{body}");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{body}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout, "{body}");
        }
    }
}

#[test]
fn native_checked_closure_nested_named_parameter_cleanup_is_executed() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);} struct Ticket {}
        impl Retire for Ticket {fn drop(*self) {assumes false;}}
        fn consume(ticket:Ticket) {}
        fn run()->bool {let action=||consume(Ticket {});action();true}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    assert!(!reference.status.success());
    assert!(
        String::from_utf8_lossy(&reference.stderr).contains("ProgramContractRejected"),
        "{}",
        String::from_utf8_lossy(&reference.stderr)
    );
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        assert!(!w.execute().status.success());
    }
}

#[test]
fn native_checked_nested_closure_transfers_capture() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);} struct Ticket {}
        impl Retire for Ticket {fn drop(*self) {}}
        fn run()->bool {let ticket=Ticket {};let inner=|| {ticket;};let outer=||inner();outer();true}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn shipped_nested_checked_closures_execute_natively() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-nested-closure.fl"
    ));
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_recursive_callable_cleanup_aliases_execute() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        typealias Action=%{target:*bool} fn()->.void'builtin;
        fn repeat(action:Action,again:bool) {action();if again {repeat(action,false);}}
        fn run()->bool {let seen=false;let target=seen.ref;let action=|| {let watch=Watch {target:target};};repeat(action,true);not seen}";
    let prepared = prepared_host(source);
    assert!(!prepared.schedule().aliases.is_empty(), "{prepared}");
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn native_recursive_cleanup_counts_each_callback_in_nested_and_mutual_calls() {
    let header="trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Count=.u32'builtin;
        struct Counted {target:*Count}
        impl Retire for Counted {fn drop(*self) {let value=self.target.*.*;if value < 100 {self.target.*.* = value + 1;}}}
        typealias Action=%{target:*Count} fn()->.void'builtin;
        fn repeat(action:Action,remaining:Count) {action();if remaining != 0 {repeat(action,remaining - 1);}}
        fn left(action:Action,remaining:Count) {action();if remaining != 0 {right(action,remaining - 1);}}
        fn right(action:Action,remaining:Count) {action();if remaining != 0 {left(action,remaining - 1);}}";
    for body in [
        "let count:Count=0;let target=count.ref;let action=|| {let counted=Counted {target:target};};repeat(action,3);count==4",
        "let count:Count=0;let target=count.ref;let action=|| {let counted=Counted {target:target};};left(action,4);count==5",
        "let count:Count=0;let target=count.ref;let outer=|| {let inner=|| {let counted=Counted {target:target};};repeat(inner,3);};outer();outer();count==8",
    ] {
        let source = format!("{header} fn run()->bool {{{body}}}");
        let prepared = prepared_host(&source);
        assert!(!prepared.schedule().aliases.is_empty(), "{prepared}");
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        assert!(
            reference.status.success(),
            "{body}: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        assert_eq!(reference.stdout, b"(bool true)\n", "{body}");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{body}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout, "{body}");
        }
    }
}

#[test]
fn shipped_recursive_checked_closure_executes_natively() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-recursive-closure.fl"
    ));
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(unsigned-integer 4)\n");
    }
}

#[test]
fn native_checked_closure_default_arguments_keep_their_cleanup_frames() {
    let header="trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Token {} impl Retire for Token {fn drop(*self) {}}
        struct Watch {target:*bool} impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        typealias Action=%{target:*bool} fn()->.void'builtin;
        fn apply(action:Action) {action();}
        fn sink(target:*bool,.last:Watch=do {let local=Token {};Watch {target:target}}) {}";
    for body in [
        "let seen=false;let target=seen.ref;let action=||sink(target);action();let first=seen;action();first and not seen",
        "let seen=false;let target=seen.ref;let action=||sink(target);apply(action);seen",
        "let seen=false;let target=seen.ref;let outer=|| {let inner=||sink(target);apply(inner);};outer();let first=seen;outer();first and not seen",
    ] {
        let source = format!("{header} fn run()->bool {{{body}}}");
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        assert!(
            reference.status.success(),
            "{body}: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        assert_eq!(reference.stdout, b"(bool true)\n", "{body}");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{body}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout, "{body}");
        }
    }
}

#[test]
fn native_checked_closure_nested_defaults_deliver_and_clean_owned_values() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Token {} impl Retire for Token {fn drop(*self) {}}
        struct Watch {target:*bool} impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        fn make(target:*bool,.watch:Watch=do {let temporary=Token {};Watch {target:target}})->Watch {watch}
        fn sink(target:*bool,.watch:Watch=make(target)) {}
        fn run()->bool {let seen=false;let target=seen.ref;let action=||sink(target);action();let first=seen;action();first and not seen}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn native_checked_default_constructed_closure_survives_its_default_frame() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Watch {target:*bool} impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        typealias Action=%{target:*bool} fn()->.void'builtin;
        fn consume(target:*bool,.action:Action=|| {let watch=Watch {target:target};}) {action();}
        fn run()->bool {let seen=false;let target=seen.ref;let outer=||consume(target);outer();let first=seen;outer();first and not seen}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn shipped_default_closure_uses_each_calls_argument() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-default-closure.fl"
    ));
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn native_checked_default_closure_calls_recursive_function() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Token {} impl Retire for Token {fn drop(*self) {}}
        fn recur(again:bool)->bool {let token=Token {};if again {recur(false)} else {true}}
        typealias Action=%{again:bool} fn()->bool;
        fn sink(again:bool,.action:Action=||recur(again))->bool {action()}
        fn run()->bool {let action=||sink(true);action() and action()}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_recursive_default_requires_termination_evidence() {
    let w = Work::new(
        "fn recur(again:bool)->bool {if again {recur(false)} else {true}}
        fn sink(.value:bool=recur(true))->bool {value}
        fn run()->bool {sink()}",
    );
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let out = w.build();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("default expression requires P3 stability/termination evidence")
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn native_checked_default_closure_recursive_destructors_execute() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Count=.u32'builtin;
        struct Token {target:*Count}
        impl Retire for Token {fn drop(*self) {let n=self.target.*.*;if n < 100 {self.target.*.* = n + 1;}}}
        fn recur(target:*Count,again:bool) {if again {recur(target,false);}let token=Token {target:target};}
        typealias Action=%{target:*Count} fn()->.void'builtin;
        fn sink(target:*Count,.action:Action=||recur(target,true)) {action();}
        fn run()->bool {let count:Count=0;let target=count.ref;let action=||sink(target);action();action();count==4}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn native_recursive_live_resource_executes() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Count=.u32'builtin;
        struct Token {target:*Count}
        impl Retire for Token {fn drop(*self) {let n=self.target.*.*;if n < 100 {self.target.*.* = n + 1;}}}
        fn recur(target:*Count,again:bool) {let token=Token {target:target};if again {recur(target,false);}}
        fn run()->bool {let count:Count=0;recur(count.ref,true);count==2}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, reference.stdout);
}

#[test]
fn native_recursive_failure_unwinds_each_live_frame() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Count=.u32'builtin;
        struct Token {target:*Count}
        impl Retire for Token {fn drop(*self) {let n=self.target.*.*;if n < 100 {self.target.*.* = n + 1;}}}
        fn trial(target:*Count,again:bool) {let token=Token {target:target};if again {trial(target,false);} else {assumes false;}}
        fn run()->bool {let count:Count=0;trial(count.ref,true);false}";
    assert_native_failure_cleans_two_frames(source);
}

#[test]
fn native_closure_failure_unwinds_callee_and_caller() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Count=.u32'builtin;
        struct Token {target:*Count}
        impl Retire for Token {fn drop(*self) {let n=self.target.*.*;if n < 100 {self.target.*.* = n + 1;}}}
        fn trial(target:*Count,again:bool) {let token=Token {target:target};let action=|| {let inner=Token {target:target};assumes not again;};action();target.* = 99;}
        fn run()->bool {let count:Count=0;trial(count.ref,true);false}";
    assert_native_failure_cleans_two_frames(source);
}

fn assert_native_failure_cleans_two_frames(source: &str) {
    let prepared = prepared_host(source);
    let ir = luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
    assert_native_failure_count_ir(&prepared, &ir);
    let w = Work::new(source);
    success(&resource_build(&w));
    assert!(
        !w.execute().status.success(),
        "entry must terminate on propagated failure"
    );
}

fn assert_native_failure_count_ir(prepared: &luna::host::Prepared, ir: &str) {
    let index = prepared
        .graph()
        .functions
        .iter()
        .position(|f| f.function.name == "trial")
        .unwrap();
    // Replace only the development entry adapter. Keep all generated machine
    // bodies and cleanup code intact; observe both frames' externally stored count.
    let (module, _) = ir.split_once("define i32 @main()").unwrap();
    let ir = format!(
        "{module}define i32 @main() {{
entry:
  %count = alloca i32
  store i32 0, ptr %count
  call void @flurry_{index}(ptr %count, i1 true)
  %n = load i32, ptr %count
  %twice = icmp eq i32 %n, 2
  %failed = load i1, ptr @flurry_failed
  %ok = and i1 %twice, %failed
  %exit = select i1 %ok, i32 0, i32 1
  ret i32 %exit
}}"
    );
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn native_failed_postcondition_keeps_result_cleanup_in_callee() {
    for (result, body) in [
        ("Token", "Token {target:target}"),
        ("Token", "return Token {target:target};"),
        ("value:Token", "Token {target:target}"),
        ("value:Token", "return Token {target:target};"),
    ] {
        let source=format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            typealias Count=.u32'builtin;
            struct Token {{target:*Count}}
            impl Retire for Token {{fn drop(*self) {{let n=self.target.*.*;if n < 100 {{self.target.*.* = n + 1;}}}}}}
            fn make(target:*Count,flag:bool)->{result} where requires flag, ensures flag {{{body}}}
            fn trial(target:*Count,flag:bool) {{assumes flag;let outer=Token {{target:target}};let result=make(target,flag);target.* = 99;}}
            fn run()->bool {{let count:Count=0;trial(count.ref,true);false}}");
        assert_injected_contract_failure(&source, "make", 1);
    }
}

#[test]
fn native_call_argument_failure_cleans_undelivered_resource() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Count=.u32'builtin;
        struct Token {target:*Count}
        impl Retire for Token {fn drop(*self) {let n=self.target.*.*;if n < 100 {self.target.*.* = n + 1;}}}
        fn fail(flag:bool)->bool {assumes not flag;true}
        fn consume(token:Token,ok:bool) {}
        fn trial(target:*Count,flag:bool) {let outer=Token {target:target};consume(Token {target:target},fail(flag));target.* = 99;}
        fn run()->bool {let count:Count=0;trial(count.ref,true);false}";
    assert_native_failure_cleans_two_frames(source);
}

#[test]
fn native_precondition_failure_cleans_delivered_parameter() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Count=.u32'builtin;
        struct Token {target:*Count}
        impl Retire for Token {fn drop(*self) {let n=self.target.*.*;if n < 100 {self.target.*.* = n + 1;}}}
        fn consume(token:Token,flag:bool) where requires flag {}
        fn trial(target:*Count,flag:bool) {assumes flag;let outer=Token {target:target};consume(Token {target:target},flag);target.* = 99;}
        fn run()->bool {let count:Count=0;trial(count.ref,true);false}";
    assert_injected_contract_failure(source, "consume", 0);
}

// Valid programs establish their contracts before execution. Force only the
// retained runtime guard to fail, to exercise the backend's defensive path.
fn assert_injected_contract_failure(source: &str, name: &str, guard: usize) {
    let prepared = prepared_host(source);
    let index = prepared
        .graph()
        .functions
        .iter()
        .position(|f| f.function.name == name)
        .unwrap();
    let mut ir = luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
    let symbol = format!("@flurry_{index}(");
    let start = ir
        .lines()
        .scan(0, |offset, line| {
            let here = *offset;
            *offset += line.len() + 1;
            Some((here, line))
        })
        .find(|(_, line)| line.starts_with("define internal ") && line.contains(&symbol))
        .unwrap()
        .0;
    let end = start + ir[start..].find("\n}\n").unwrap();
    let body = &ir[start..end];
    let branch = body.match_indices("br i1 ").nth(guard).unwrap().0;
    let condition_end = branch + body[branch..].find(',').unwrap();
    let mut modified = body.to_owned();
    modified.replace_range(branch..condition_end, "br i1 false");
    ir.replace_range(start..end, &modified);
    assert_native_failure_count_ir(&prepared, &ir);
}

#[test]
fn native_failure_cleanup_can_call_ordinary_helpers() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Count=.u32'builtin;
        fn increment(target:*Count) {let n=target.*;if n < 100 {target.* = n + 1;}}
        struct Token {target:*Count}
        impl Retire for Token {fn drop(*self) {increment(self.target.*);}}
        fn trial(target:*Count,again:bool) {let token=Token {target:target};if again {trial(target,false);} else {assumes false;}}
        fn run()->bool {let count:Count=0;trial(count.ref,true);false}";
    assert_native_failure_cleans_two_frames(source);
}

#[test]
fn native_second_failure_stops_remaining_cleanup() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Count=.u32'builtin;
        fn stop() {assumes false;}
        struct Later {target:*Count}
        impl Retire for Later {fn drop(*self) {self.target.*.* = 99;}}
        struct Failing {target:*Count}
        impl Retire for Failing {fn drop(*self) {self.target.*.* = 1;stop();self.target.*.* = 77;}}
        fn trial(target:*Count,flag:bool) {let later=Later {target:target};let failing=Failing {target:target};assumes not flag;}
        fn run()->bool {let count:Count=0;trial(count.ref,true);false}";
    let prepared = prepared_host(source);
    let index = prepared
        .graph()
        .functions
        .iter()
        .position(|f| f.function.name == "trial")
        .unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
    let (module, _) = ir.split_once("define i32 @main()").unwrap();
    // Observe the state at the terminal failure boundary, without changing any
    // generated function, cleanup ordering, or failure propagation check.
    let module = module.replace(
        "call void @llvm.trap()",
        "call void @observe_terminal_failure()",
    );
    let ir = format!(
        "{module}
@observed = internal global i32 0
declare void @exit(i32) noreturn
define internal void @observe_terminal_failure() {{
entry:
  %n = load i32, ptr @observed
  %first = icmp eq i32 %n, 1
  %failed = load i1, ptr @flurry_failed
  %ok = and i1 %first, %failed
  %code = select i1 %ok, i32 0, i32 1
  call void @exit(i32 %code)
  unreachable
}}
define i32 @main() {{
entry:
  call void @flurry_{index}(ptr @observed, i1 true)
  ret i32 1
}}"
    );
    for flag in ["true", "false"] {
        // Exercise the unmodified executable as well as the terminal-state
        // observer below. Distinct checks must report in execution order;
        // propagation through stop/Drop/trial must not duplicate diagnostics.
        let runtime_source =
            source.replace("trial(count.ref,true)", &format!("trial(count.ref,{flag})"));
        let w = Work::new(&runtime_source);
        let first = runtime_source.find("assumes not flag;").unwrap() + "assumes ".len();
        let second = runtime_source.find("assumes false;").unwrap() + "assumes ".len();
        let expected = if flag == "true" {
            vec![(first, first + 8), (second, second + 5)]
        } else {
            vec![(second, second + 5)]
        };
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            assert!(!output.status.success());
            let diagnostic = String::from_utf8(output.stderr).unwrap();
            let lines = diagnostic.lines().collect::<Vec<_>>();
            assert_eq!(lines.len(), expected.len(), "{diagnostic}");
            for (line, (start, end)) in lines.iter().zip(&expected) {
                assert!(line.starts_with("(runtime-check-failure "), "{diagnostic}");
                assert!(
                    line.contains(&format!("(predicate {start} {end})")),
                    "{diagnostic}"
                );
            }
        }
        let ir = ir.replace(
            &format!("call void @flurry_{index}(ptr @observed, i1 true)"),
            &format!("call void @flurry_{index}(ptr @observed, i1 {flag})"),
        );
        for optimization in [
            luna::host::OptimizationLevel::O0,
            luna::host::OptimizationLevel::O3,
        ] {
            luna::host::Toolchain {
                optimization,
                ..Default::default()
            }
            .execute_test(&ir, std::time::Duration::from_secs(5))
            .unwrap();
        }
    }
}

#[test]
fn native_closure_pointer_call_requires_borrowed_callable_contract() {
    let w = Work::new(include_str!(
        "../../../development/repros/host-borrowed-closure-pointer.fl"
    ));
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let result = resource_build(&w);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("pointer reads of non-intrinsic values require Copy evidence or a memory transfer contract"),"{error}");
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn native_empty_effect_computation_executes() {
    for source in [
        "fn value()->#[] bool {true} fn run()->bool {value()# {}}",
        "fn value()->#[] bool {return true;} fn run()->bool {(value()# {}) and (value()# {})}",
    ] {
        native(source, "(bool true)\n");
    }
    native(
        include_str!("../../../development/examples/host-empty-effects.fl"),
        "(unsigned-integer 42)\n",
    );
}

#[test]
fn native_empty_effect_rows_keep_calls_and_cleanup() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        fn value(target:*bool)->#[] bool {let watch=Watch {target:target};true}
        fn run()->bool {let seen=false;let first=value(seen.ref)# {};let after=seen;let second=value(seen.ref)# {};first and second and after and not seen}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let result = w.execute();
        success(&result);
        assert_eq!(result.stdout, reference.stdout);
    }
}

#[test]
fn native_cross_function_direct_return_requires_checked_cleanup() {
    let w = Work::new(
        "effect read()->bool;fn subject()->#read bool {read()#}fn run()->bool {subject()# {read()=>do {return true;},}}",
    );
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let result = w.build();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("host outlined direct return requires checked cleanup")
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn native_empty_effect_rows_preserve_delivered_origins() {
    let header="trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        typealias Pointer=*bool;
        typealias Action=%{flag:bool} fn()->bool;
        fn pointer(target:*bool)->#[] Pointer {target}
        fn owned(target:*bool)->#[] Watch {Watch {target:target}}
        fn closure(flag:bool)->#[] Action {||flag}";
    for body in [
        "let seen=false;let target=pointer(seen.ref)# {};target.* = true;seen",
        "let seen=false;do {let resource=owned(seen.ref)# {};};seen",
        "let action=closure(true)# {};action()",
    ] {
        let source = format!("{header} fn run()->bool {{{body}}}");
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n", "{body}");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout, "{body}");
        }
    }
}

#[test]
fn native_empty_effect_row_does_not_hide_pointer_escape() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {} impl Duplicate for *T where T {}
        typealias Pointer=*bool;
        fn bad()->#[] Pointer {let local=false;local.ref}
        fn run()->bool {let value=bad()# {};value.*}";
    let w = Work::new(source);
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let out = resource_build(&w);
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(
        error.contains("outlive") || error.contains("escape") || error.contains("local storage"),
        "{error}"
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn native_lexical_direct_effect_handlers_execute() {
    for source in [
        include_str!("../../../development/examples/host-direct-effect.fl"),
        "effect read()->bool;fn run()->bool {(read()# {read()=>read()#,})# {read()=>true,}}",
        "effect read()->bool;fn run()->bool {let flag=false;(read()# {read()=>not read()#,})# {read()=>flag,}}",
        "effect read()->bool;fn run()->bool {read()# {read()=>true,}}",
        "effect echo(value:bool)->bool;fn run()->bool {echo(true)# {echo(value)=>value,}}",
        "effect echo(value:bool)->bool;fn run()->bool {echo(false)# {echo(true)=>false,echo(false)=>true,}}",
        "effect read()->bool;fn run()->bool {let flag=true;read()# {read()=>flag,}}",
        "effect read()->bool;fn run()->bool {var flag=false;let result=read()# {read()=>do {flag=true;true},};result and flag}",
        "effect echo(value:bool)->bool;fn run()->bool {var seen=false;let result=echo(do {seen=true;false})# {echo(value)=>seen and not value,};result and seen}",
    ] {
        let w = Work::new(source);
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n", "{source}");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout, "{source}");
        }
    }
}

#[test]
fn native_direct_effect_resource_frames_execute() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {} effect read()->bool;fn run()->bool {read()# {read()=>true,}}";
    let w = Work::new(source);
    let out = resource_build(&w);
    success(&out);
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_checked_direct_handler_cleanup_matches_reference() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {} struct Watch {valid:bool} impl Retire for Watch {fn drop(*self) {assumes self.valid.*;}} effect get()->bool;";
    for body in [
        "fn run()->bool {get()# {get()=>do {let watch=Watch {valid:true};true},}}",
        "fn run()->bool {(get()# {get()=>do {let watch=Watch {valid:true};get()#},})# {get()=>true,}}",
        "fn run()->bool {(get()# {get()=>do {let watch=Watch {valid:true};get()#},})# {get()=>do {return true;},};false}",
        "fn run()->bool {(get()# {get() if false=>false,})# {get()=>do {let watch=Watch {valid:true};resume true;},}}",
        "effect consume(value:Watch)->bool;fn run()->bool {consume(Watch {valid:true})# {consume(value)=>true,}}",
        "^attrs.effects.direct_passing effect setting()->bool;fn run()->bool {(setting()# and setting()#)# {setting()=>do {let watch=Watch {valid:true};resume true;},}}",
        "^attrs.effects.direct_passing effect setting()->bool;fn run()->bool {setting()# {setting()=>do {let watch=Watch {valid:true};return true;},};false}",
        "fn run()->bool {get()# {get()=>do {let watch=Watch {valid:true};resume true;},}}",
        "fn run()->bool {get()# {get()=>do {let watch=Watch {valid:true};return true;},};false}",
        "fn run()->bool {while true {get()# {get()=>do {let watch=Watch {valid:true};break;},};}true}",
        "fn run()->bool {var again=true;while again {again=false;get()# {get()=>do {let watch=Watch {valid:true};continue;},};}true}",
    ] {
        for fails in [false, true] {
            let body = if fails {
                body.replace("valid:true", "valid:false")
            } else {
                body.to_owned()
            };
            let w = Work::new(&format!("{header}{body}"));
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            if fails {
                assert!(!reference.status.success(), "{body}");
            } else {
                success(&reference);
                assert_eq!(reference.stdout, b"(bool true)\n", "{body}");
            }
            for level in ["0", "3"] {
                let out = w
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap();
                success(&out);
                let out = w.execute();
                if fails {
                    assert!(!out.status.success(), "Drop predicate must execute: {body}");
                } else {
                    success(&out);
                    assert_eq!(out.stdout, reference.stdout, "{body}");
                }
            }
        }
    }
}

#[test]
fn direct_passing_preserves_only_stable_installation_facts() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {} ^attrs.effects.direct_passing effect setting()->bool;";
    for computation in ["setting()#", "false and setting()#"] {
        let w = Work::new(&format!(
            "{header}fn keep(flag:bool)->result:bool where requires flag, ensures result {{let ignored=({computation})# {{setting()=>true,}};flag}}fn run()->bool {{keep(true)}}"
        ));
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
    for (setup, mutation) in [
        ("var flag=true;", "flag=false;"),
        ("let flag=true;let pointer=flag.ref;", "pointer.* = false;"),
    ] {
        // The subject skips its request, but installation still invokes the
        // provider. Its write invalidates the pre-installation flag fact.
        let w = Work::new(&format!(
            "{header}fn run()->result:bool where ensures result {{{setup}let ignored=(false and setting()#)# {{setting()=>do {{{mutation}true}},}};flag}}"
        ));
        let out = w.command().args(["--emit-llvm", "run"]).output().unwrap();
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("postcondition is not proven"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn handler_postcondition_does_not_reuse_mutated_capture_facts() {
    for body in [
        "var flag=true;(do {flag=false;get()#})# {get()=>do {return flag;},};true",
        "let flag=true;let pointer=flag.ref;(do {pointer.* = false;get()#})# {get()=>do {return flag;},};true",
    ] {
        let w = Work::new(&format!(
            "effect get()->bool;fn run()->result:bool where ensures result {{{body}}}"
        ));
        let out = w.command().args(["--emit-llvm", "run"]).output().unwrap();
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("postcondition is not proven"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn native_handler_stable_capture_arithmetic_survives_repeated_requests() {
    for declaration in ["let", "var"] {
        let w = Work::new(&format!(
            "typealias Byte=.u8'builtin;effect read()->Byte;
             fn source()->#read Byte {{let first=read()#;let second=read()#;second}}
             fn run()->Byte {{{declaration} captured:Byte=41;
                 source()# {{read()=>captured + 1,}}}}"
        ));
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(unsigned-integer 42)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_checked_handler_return_delivers_owned_result_after_postcondition() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {} struct Watch {valid:bool}impl Retire for Watch {fn drop(*self) {assumes self.valid.*;}}effect get()->bool;";
    for result in ["Watch", "answer:Watch"] {
        for nested in [false, true] {
            let body = if nested {
                "(get()# {get()=>get()#,})# {get()=>do {return Watch {valid:flag};},};Watch {valid:flag}"
            } else {
                "get()# {get()=>do {return Watch {valid:flag};},};Watch {valid:flag}"
            };
            let w = Work::new(&format!(
                "{header}fn make(flag:bool)->{result} where requires flag, ensures flag {{{body}}}fn run()->bool {{make(true);true}}"
            ));
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                let out = w
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap();
                success(&out);
                let out = w.execute();
                success(&out);
                assert_eq!(out.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn native_checked_handler_guard_cleans_private_nonrejecting_values() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {} struct Watch {} impl Retire for Watch {fn drop(*self) {}} effect get()->bool;";
    for body in [
        "fn run()->bool {get()# {get() if do {let watch=Watch {};true}=>true,get()=>false,}}",
        "fn run()->bool {(get()# {get() if do {let watch=Watch {};false}=>false,})# {get()=>true,}}",
        "fn run()->bool {get()# {get() if do {let first=Watch {};let second=Watch {};false}=>false,get() if do {let third=Watch {};true}=>true,get()=>false,}}",
    ] {
        let w = Work::new(&format!("{header}{body}"));
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        let ir = w
            .command()
            .args([
                "--emit-llvm",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&ir);
        // The only void callee is Watch.drop. Even an empty Drop is a selected
        // cleanup invocation before optimization, not permission to omit it.
        let ir = String::from_utf8(ir.stdout).unwrap();
        assert_eq!(
            ir.lines()
                .filter(|line| line
                    .trim()
                    .strip_prefix("call void @flurry_")
                    .is_some_and(|name| name.as_bytes().first().is_some_and(u8::is_ascii_digit)))
                .count(),
            body.matches("Watch {}").count(),
            "guard cleanup invocation count: {ir}"
        );
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout, "{body}");
        }
    }
}

#[test]
fn checked_handler_guard_fallible_cleanup_requires_failure_transfer() {
    let w = Work::new(
        "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {} struct Watch {valid:bool}impl Retire for Watch {fn drop(*self) {assumes self.valid.*;}}effect get()->bool;fn run()->bool {(get()# {get() if do {let watch=Watch {valid:true};false}=>false,})# {get()=>true,}}",
    );
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let out = resource_build(&w);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("observation cleanup needs failure-path resource transfer"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn native_checked_direct_passing_installs_once() {
    let source = format!(
        "trait Duplicate {{}} trait Retire {{fn drop(*self);}} impl Duplicate for bool {{}} {}",
        include_str!("../../../development/examples/host-direct-passing.fl")
    );
    let w = Work::new(&source);
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn shipped_direct_effect_cleanup_executes() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-direct-effect-cleanup.fl"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_checked_handler_pointer_parameter_supports_cleanup() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {} impl Duplicate for *T where T {} struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}effect get(target:*bool)->bool;";
    for computation in [
        "get(seen.ref)#",
        "(get(seen.ref)# {get(target) if false=>false,})#",
    ] {
        for finish in ["true", "resume true;"] {
            let w = Work::new(&format!(
                "{header}fn run()->bool {{let seen=false;let result={computation} {{get(target)=>do {{let watch=Watch {{target:target}};{finish}}},}};result and seen}}"
            ));
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                let out = w
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap();
                success(&out);
                let out = w.execute();
                success(&out);
                assert_eq!(out.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn native_checked_handler_cleanup_distinguishes_pointer_input_fields() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {} impl Duplicate for *T where T {} struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}} struct Pair {first:*bool,second:*bool}impl Duplicate for Pair {}";
    for body in [
        "effect get(pair:Pair)->bool;fn run()->bool {let first=false;let second=false;let result=get(Pair {first:first.ref,second:second.ref})# {get(pair)=>do {let a=Watch {target:pair.first};let b=Watch {target:pair.second};true},};result and first and second}",
        "effect get(first:*bool,second:*bool)->bool;fn run()->bool {let first=false;let second=false;let result=get(first.ref,second.ref)# {get(p,q)=>do {let a=Watch {target:p};let b=Watch {target:q};resume true;},};result and first and second}",
        "effect get(first:*bool,second:*bool)->bool;fn run()->bool {let seen=false;let result=get(seen.ref,seen.ref)# {get(p,q)=>do {let a=Watch {target:p};let b=Watch {target:q};true},};result and not seen}",
    ] {
        let w = Work::new(&format!("{header}{body}"));
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_checked_handler_cleanup_respects_enum_pointer_payloads() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {} impl Duplicate for *T where T {} struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}enum Choice {none,left(*bool),right(*bool)}impl Duplicate for Choice {}effect get(choice:Choice)->bool;";
    for forwarded in [false, true] {
        let computation = if forwarded {
            "(get(choice)# {get(.none)=>true,})#"
        } else {
            "get(choice)#"
        };
        let w = Work::new(&format!(
            "{header}fn check(empty:bool,left:bool)->bool {{let first=false;let second=false;let choice=if empty {{Choice.none}}else{{if left {{Choice.left(first.ref)}}else{{Choice.right(second.ref)}}}};let result={computation} {{get(.none)=>true,get(.left(target))=>do {{let watch=Watch {{target:target}};true}},get(.right(target))=>do {{let watch=Watch {{target:target}};resume true;}},}};result and first == (not empty and left) and second == (not empty and not left)}}fn run()->bool {{check(true,false) and check(false,true) and check(false,false)}}"
        ));
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout);
        }
    }
}

#[test]
fn handler_cleanup_summary_does_not_authorize_observing_calls() {
    let w = Work::new(
        "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {} impl Duplicate for *T where T {} struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}effect get(target:*bool)->bool;fn touch(p:*bool)->bool {get(p)# {get(target)=>do {let watch=Watch {target:target};true},}}fn run()->bool {let seen=false;true match {_ if touch(seen.ref)=>true,_=>false}} ",
    );
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let out = resource_build(&w);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(
            "guard pointer observation cannot write caller memory or run unverified cleanup"
        ),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn checked_handler_cleanup_does_not_extend_pointer_argument_lifetime() {
    let w = Work::new(
        "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {} impl Duplicate for *T where T {} struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}effect get(target:*bool)->bool;fn run()->bool {get(do {let dead=false;dead.ref})# {get(target)=>do {let watch=Watch {target:target};true},}}",
    );
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let out = resource_build(&w);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("pointer target unavailable outside its local scope (escaping provenance)"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn checked_handler_pointer_provenance_gaps_preserve_existing_artifacts() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {} impl Duplicate for *T where T {} struct Watch {target:*bool} impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}";
    for (body, diagnostic) in [(
        "effect get()->bool;fn run()->bool {let seen=false;get()# {get()=>do {let watch=Watch {target:seen.ref};true},}}",
        "handler lexical capture address exposure requires a memory contract",
    )] {
        let w = Work::new(&format!("{header}{body}"));
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let out = resource_build(&w);
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(diagnostic),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_checked_handler_stable_pointer_capture_observes_cleanup() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {} impl Duplicate for *T where T {} struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}effect get()->bool;";
    for computation in [
        "get()#",
        "(get()# {get() if false=>false,})#",
        "((get()# {get() if false=>false,})# {get() if false=>false,})#",
    ] {
        for finish in ["true", "resume true;"] {
            let w = Work::new(&format!(
                "{header}fn run()->bool {{let seen=false;let target=seen.ref;let result={computation} {{get()=>do {{let watch=Watch {{target:target}};{finish}}},}};result and seen}}"
            ));
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                let out = w
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap();
                success(&out);
                let out = w.execute();
                success(&out);
                assert_eq!(out.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn native_direct_effect_partial_handlers_forward_once() {
    for source in [
        "effect echo(value:bool)->bool;fn run()->bool {(echo(false)# {echo(value) if value=>false,})# {echo(value)=>not value,}}",
        "effect echo(value:bool)->bool;fn run()->bool {var seen=false;let result=(echo(false)# {echo(value) if do {seen=true;value}=>false,})# {echo(value)=>seen and not value,};result and seen}",
        "effect echo(value:bool)->bool;fn run()->bool {(echo(false)# {echo(true)=>false,})# {echo(value)=>not value,}}",
        "effect echo(value:bool)->bool;fn run()->bool {(echo(true)# {echo(true)=>true,})# {echo(value)=>false,}}",
        "effect echo(value:bool)->bool;fn run()->bool {((echo(false)# {echo(true)=>false,})# {echo(true)=>false,})# {echo(value)=>not value,}}",
        "effect echo(value:bool)->bool;fn run()->bool {var seen=false;let result=(echo(do {seen=not seen;false})# {echo(true)=>false,})# {echo(value)=>seen and not value,};result and seen}",
    ] {
        let w = Work::new(source);
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n", "{source}");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout, "{source}");
        }
    }
}

#[test]
fn native_direct_handler_resume_delivers_to_its_request() {
    for source in [
        "effect read()->bool;fn run()->bool {read()# {read()=>do {var active=true;while active {active=false;continue;}resume not active;},}}",
        "effect read()->bool;fn run()->bool {read()# {read()=>do {while true {break;}resume true;},}}",
        "effect read()->bool;fn run()->bool {read()# {read()=>true match {true=>do {resume true;},false=>false,},}}",
        "effect read()->bool;fn run()->bool {read()# {read()=>do {while true {resume true;}false},}}",
        "effect echo(value:bool)->bool;fn run()->bool {echo(true)# {echo(value) if do {resume value;}=>false,echo(value)=>false,}}",
        "effect read()->bool;fn run()->bool {read()# {read()=>do {resume true;false},}}",
        "effect echo(value:bool)->bool;fn run()->bool {echo(true)# {echo(value)=>do {if value {resume true;}false},}}",
        "effect echo(value:bool)->bool;fn run()->bool {echo(false)# {echo(value)=>do {if value {resume false;}true},}}",
        "effect read()->bool;fn run()->bool {var seen=false;let first=read()# {read()=>do {resume true;seen=true;false},};let second=read()# {read()=>do {resume true;},};first and second and not seen}",
        "effect read()->bool;fn run()->bool {(read()# {read()=>do {let value=read()#;resume not value;},})# {read()=>do {resume false;},}}",
        "effect echo(value:bool)->bool;fn run()->bool {(echo(false)# {echo(true)=>do {resume false;},})# {echo(value)=>do {resume not value;},}}",
    ] {
        let w = Work::new(source);
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n", "{source}");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout, "{source}");
        }
    }
}

#[test]
fn native_direct_handler_function_return_is_not_resume() {
    let source =
        "effect read()->bool;fn run()->bool {read()# {read()=>do {return true;false},};false}";
    let w = Work::new(source);
    let reference = w.command().args(["--interpret", "run"]).output().unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--build",
                "run",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&out);
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_handler_preserves_lexical_control_targets() {
    for source in [
        "effect read()->bool;fn run()->bool {read()# {read()=>do {let flag=true;let local=|| {return flag;};resume local();},}}",
        "effect echo(value:bool)->bool;fn run()->bool {(echo(false)# {echo(true)=>false,})# {echo(value)=>do {return not value;false},};false}",
        "effect read()->bool;fn run()->bool {while true {read()# {read()=>do {break;false},};return false;}true}",
        "effect read()->bool;fn run()->bool {var active=true;while active {read()# {read()=>do {active=false;continue;false},};return false;}true}",
        "effect read()->bool;fn run()->bool {(read()# {read()=>do {return true;false},})# {};false}",
        "effect echo(value:bool)->bool;fn run()->bool {echo(true)# {echo(value) if do {return value;}=>false,echo(value)=>false,};false}",
        "effect read()->bool;fn run()->bool {read()# {read()=>do {let local=|| {return true;};resume local();},}}",
    ] {
        let w = Work::new(source);
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n", "{source}");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout, "{source}");
        }
    }
}

#[test]
fn native_direct_passing_installs_once_before_computation() {
    for source in [
        include_str!("../../../development/examples/host-direct-passing.fl"),
        "^attrs.effects.direct_passing effect setting()->bool;fn run()->bool {var flag=true;do {flag=false;setting()#}# {setting()=>flag,}}",
        "^attrs.effects.direct_passing effect setting()->bool;fn run()->bool {do {let first=setting()#;let second=setting()# {setting()=>false,};let third=setting()#;first and not second and third}# {setting()=>true,}}",
        "^attrs.effects.direct_passing effect setting()->bool;fn run()->bool {var seen=false;var active=true;var phase=false;while active {setting()# {setting()=>do {seen=not seen;true},};if phase {active=false;}phase=true;}not seen}",
        "^attrs.effects.direct_passing effect setting()->bool;fn run()->bool {(setting()# {setting() if false=>false,})# {setting()=>true,}}",
        "^attrs.effects.direct_passing effect setting()->bool;fn run()->bool {setting()# {setting()=>true,}}",
        "^attrs.effects.direct_passing effect setting()->bool;fn run()->bool {var seen=false;let result=(setting()# and setting()#)# {setting()=>do {seen=not seen;true},};result and seen}",
        "^attrs.effects.direct_passing effect setting()->bool;fn run()->bool {var seen=false;let result=(false and setting()#)# {setting()=>do {seen=true;true},};not result and seen}",
        "^attrs.effects.direct_passing effect setting()->bool;fn run()->bool {(setting()# {setting()=>not setting()#,})# {setting()=>false,}}",
        "^attrs.effects.direct_passing effect setting()->bool;fn run()->bool {var seen=false;(setting()# {setting()=>do {return true;seen=true;false},});false}",
    ] {
        let w = Work::new(source);
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n", "{source}");
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout, "{source}");
        }
    }
}

#[test]
fn native_direct_passing_rejects_unimplemented_slot_layouts() {
    let w = Work::new(
        "^attrs.effects.direct_passing effect callback()->fn()->bool;fn run()->bool {(callback()# {callback()=>||true,})()}",
    );
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let out = w.build();
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(
        error.contains(
            "host direct-passing slots currently require a zero-argument pointer-free data signature"
        ),
        "{error}"
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn native_inline_effects_rebuild_checked_resource_cleanup() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {}impl Duplicate for *T where T {}
        struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        inline effect get(target:*bool)->bool;
        inline fn subject(target:*bool)->#get bool {get(target)#}
        fn run()->bool {let seen=false;let result=subject(seen.ref)# {get(target)=>do {let watch=Watch {target:target};true},};result and seen}";
    for source in [
        source.to_owned(),
        source.replace(";true},", ";resume true;},"),
        include_str!("../../../development/examples/host-inline-effect-cleanup.fl").to_owned(),
    ] {
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--inline-functions",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        let dump = w
            .command()
            .args([
                "--emit-llvm",
                "run",
                "--inline-functions",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&dump);
        assert!(String::from_utf8_lossy(&dump.stdout).contains("define i32 @main()"));
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--inline-functions",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_inlining_requires_protocols_and_routes_unexpanded_direct_effects() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {}
        inline effect read()->bool;
        fn subject()->#read bool {read()#}
        fn run()->bool {subject()# {read()=>true,}}";
    let w = Work::new(source);
    fs::write(w.0.join("native program"), b"previous").unwrap();
    for args in [
        vec![
            "--build",
            "run",
            "--inline-functions",
            "--output",
            "native program",
        ],
        vec!["--emit-llvm", "run", "--inline-functions"],
    ] {
        let out = w.command().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("--copy-protocol and --drop-protocol")
        );
    }
    let out = w
        .command()
        .args([
            "--build",
            "run",
            "--inline-functions",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    success(&out);
    let output = w.execute();
    success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn native_source_tests_inline_effects_with_cleanup_and_continue_after_failure() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {}impl Duplicate for *T where T {}
        struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        inline effect get(target:*bool)->bool;
        inline fn subject(target:*bool)->#get bool {get(target)#}
        test first {let seen=false;let result=subject(seen.ref)# {get(target)=>do {let watch=Watch {target:target};resume true;},};assumes result and seen;}
        test fails {assumes false;}
        test last {let seen=false;let result=subject(seen.ref)# {get(target)=>do {let watch=Watch {target:target};true},};assumes result and seen;}";
    let w = Work::new(source);
    fs::write(w.0.join("report"), b"previous").unwrap();
    let empty = w
        .command()
        .args([
            "--test-native",
            "--inline-functions",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--test-filter",
            "absent",
        ])
        .env("LUNA_LLVM_CONFIG", "/definitely/missing/llvm-config")
        .output()
        .unwrap();
    success(&empty);
    assert!(
        String::from_utf8_lossy(&empty.stdout).contains("(count 0) (failures 0) (filtered-out 3)")
    );
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--test-native",
                "--inline-functions",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "report",
            ])
            .output()
            .unwrap();
        assert!(!out.status.success());
        let report = String::from_utf8_lossy(&out.stderr);
        assert!(report.contains("(count 3) (failures 1)"), "{report}");
        assert_eq!(fs::read(w.0.join("report")).unwrap(), b"previous");
        let filtered = w
            .command()
            .args([
                "--test-native",
                "--inline-functions",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--test-filter",
                "first",
            ])
            .output()
            .unwrap();
        success(&filtered);
        assert!(
            String::from_utf8_lossy(&filtered.stdout)
                .contains("(count 1) (failures 0) (filtered-out 2)")
        );
    }
}

#[test]
fn shipped_inline_effect_cleanup_runs_as_native_test() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-inline-effect-cleanup.fl"
    ));
    for level in ["0", "3"] {
        let out = w
            .command()
            .args([
                "--test-native",
                "--inline-functions",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
            ])
            .output()
            .unwrap();
        success(&out);
        assert!(String::from_utf8_lossy(&out.stdout).contains("(count 1) (failures 0)"));
    }
}

#[test]
fn native_never_return_channels_preserve_control_flow() {
    for level in ["0", "3"] {
        for source in [
            "fn stop()->.noreturn'builtin {while true {}}fn choose(flag:bool)->bool {if flag {true}else {stop()}}fn run()->bool {choose(true)}",
            "fn stop()->.noreturn'builtin {while true {}}fn choose(f:fn()->.noreturn'builtin,flag:bool)->bool {if flag {true}else {f()}}fn run()->bool {choose(stop,true)}",
            "fn choose(f:fn()->.noreturn'builtin,flag:bool)->bool {if flag {true}else {f()}}fn run()->bool {choose(|| {while true {}},true)}",
        ] {
            let w = Work::new(source);
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
        for entry in ["run", "stop"] {
            let w = Work::new(
                "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}fn stop()->.noreturn'builtin {assumes false;while true {}}fn run()->bool {stop()}",
            );
            success(
                &w.command()
                    .args([
                        "--build",
                        entry,
                        "--opt-level",
                        level,
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
        }
    }
}

#[test]
fn native_entry_prints_structural_products_without_erasing_tuple_identity() {
    let source = include_str!("../../../development/examples/host-product-result.fl");
    for level in ["0", "3"] {
        let w = Work::new(source);
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--tuple-type",
                    "input.Tuple",
                    "--meta-list-type",
                    "input.Elements",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "(anonymous-record (fields (field (string \"ready\") (bool true)) (field (string \"data\") (tuple (unsigned-integer 340282366920938463463374607431768211455) (float32 (bits 3f800000)))) (field (string \"empty\") (unit))))\n"
        );
    }
}

#[test]
fn native_product_entry_rejects_owned_nominal_results_without_losing_artifacts() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}struct Watch {value:bool}impl Retire for Watch {fn drop(*self) {}}fn record(...fields:T)->T where T {fields}typealias Result=(record {owned:Watch {value:true}})'type;fn run()->Result {record {owned:Watch {value:true}}}",
    );
    fs::write(w.0.join("native program"), b"previous executable").unwrap();
    let output = w
        .command()
        .args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("owned nominal values need an entry cleanup adapter"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(w.0.join("native program")).unwrap(),
        b"previous executable"
    );
}

#[test]
fn native_product_entry_checks_failure_before_display() {
    for level in ["0", "3"] {
        for failed in [false, true] {
            let source = format!(
                "pure comptime fn Elements<T>->.Type'builtin {{T}}pure comptime fn Tuple<...Es:Elements<.Type'builtin>>->.Type'builtin {{bool}}trait Duplicate {{}}trait Retire {{fn drop(*self);}}impl Duplicate for bool {{}}fn run()->Tuple<bool,.void'builtin> {{assumes {}; (true,())}}",
                !failed
            );
            let w = Work::new(&source);
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--tuple-type",
                        "input.Tuple",
                        "--meta-list-type",
                        "input.Elements",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            if failed {
                assert!(!output.status.success());
                assert!(output.stdout.is_empty());
            } else {
                success(&output);
                assert_eq!(output.stdout, b"(tuple (bool true) (unit))\n");
            }
        }
    }
}

#[test]
fn native_discard_result_retires_owned_entry_values() {
    for level in ["0", "3"] {
        for valid in [true, false] {
            let source = format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}impl Duplicate for bool {{}}struct Watch {{valid:bool}}impl Retire for Watch {{fn drop(*self) {{assumes self.valid.*;self.valid.* = false;}}}}fn run(.valid:bool={valid})->Watch {{Watch {{valid:valid}}}}"
            );
            let w = Work::new(&source);
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--discard-result",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            if valid {
                success(&output);
                assert_eq!(output.stdout, b"(unit)\n");
            } else {
                assert!(!output.status.success());
                assert!(output.stdout.is_empty());
            }
        }
    }
}

#[test]
fn native_discard_result_requires_explicit_checked_host_action() {
    let w = Work::new("fn run()->bool {true}");
    fs::write(w.0.join("native program"), b"previous").unwrap();
    for args in [
        vec![
            "--discard-result",
            "--build",
            "run",
            "--output",
            "native program",
        ],
        vec!["--discard-result", "--interpret", "run"],
        vec![
            "--discard-result",
            "--discard-result",
            "--build",
            "run",
            "--output",
            "native program",
        ],
    ] {
        let output = w.command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_string_values_preserve_utf8_and_escaped_output() {
    for level in ["0", "3"] {
        for (literal, expected) in [
            (
                r#""a\0b\x{7f}\x{85}""#,
                "(string \"a\\u0000b\\u007f\\u0085\")\n",
            ),
            (r#""""#, "(string \"\")\n"),
            (r#""hello 世界""#, "(string \"hello 世界\")\n"),
            (r#""a\n\t\"\\z""#, "(string \"a\\n\\t\\\"\\\\z\")\n"),
        ] {
            let source = format!(
                "typealias Text=.str'builtin;fn keep(value:Text)->Text {{value}}fn choose(flag:bool)->Text {{if flag {{keep({literal})}}else {{\"unused\"}}}}fn run()->Text {{choose(true)}}"
            );
            let w = Work::new(&source);
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
        }
    }
}

#[test]
fn native_string_patterns_compare_content_and_length() {
    let source = r#"typealias Text=.str'builtin;
        fn same()->Text {"same"}
        fn match_text(text:Text)->bool {text match {""=>false,"same"=>true,_=>false}}
        fn run()->bool {match_text(same()) and not match_text("sam") and not match_text("same!") and not match_text("")}"#;
    for level in ["0", "3"] {
        for checked in [false, true] {
            let source = if checked {
                format!(
                    "trait Duplicate {{}}trait Retire {{fn drop(*self);}}impl Duplicate for Text {{}}impl Duplicate for bool {{}}{source}"
                )
            } else {
                source.into()
            };
            let w = Work::new(&source);
            let mut command = w.command();
            command.args([
                "--build",
                "run",
                "--opt-level",
                level,
                "--output",
                "native program",
            ]);
            if checked {
                command.args(["--copy-protocol", "Duplicate", "--drop-protocol", "Retire"]);
            }
            let built = command.output().unwrap();
            if checked {
                assert!(!built.status.success());
                assert!(
                    String::from_utf8_lossy(&built.stderr)
                        .contains("string-view memory provenance is not implemented")
                );
            } else {
                success(&built);
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, b"(bool true)\n");
            }
        }
    }
}

#[test]
fn shipped_native_string_selects_utf8_literal() {
    let w = Work::new(include_str!("../../../development/examples/host-string.fl"));
    let reference = w.command().args(["--interpret", "run"]).output().unwrap();
    success(&reference);
    assert_eq!(reference.stdout, "(string \"你好，Flurry！\")\n".as_bytes());
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_product_string_results_share_escaping_and_evaluate_fields_once() {
    let source = include_str!("../../../development/examples/host-product-string-result.fl");
    for level in ["0", "3"] {
        let w = Work::new(source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--tuple-type",
                "input.Tuple",
                "--meta-list-type",
                "input.Elements",
            ])
            .output()
            .unwrap();
        success(&reference);
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--tuple-type",
                    "input.Tuple",
                    "--meta-list-type",
                    "input.Elements",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "(anonymous-record (fields (field (string \"label\") (string \"中文 100%\")) (field (string \"data\") (tuple (string \"\") (string \"a\\u0000b\\n\\\"\\\\\"))) (field (string \"seen\") (bool true))))\n"
        );
    }
}

#[test]
fn native_wide_bitvec_entry_prints_complete_decimal_values() {
    for width in [65, 127, 129, 256, 1024] {
        let mut digits = vec![0u8];
        for _ in 0..width {
            let mut carry = 1;
            for digit in &mut digits {
                let n = *digit * 2 + carry;
                *digit = n % 10;
                carry = n / 10;
            }
            if carry != 0 {
                digits.push(carry);
            }
        }
        let expected = digits
            .iter()
            .rev()
            .map(|d| (b'0' + d) as char)
            .collect::<String>();
        for level in ["0", "3"] {
            let w = Work::new(&format!(
                "typealias Bits=.BitVec'builtin<{width}>;fn run()->Bits {{not 0}}"
            ));
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                format!("(unsigned-integer {expected})\n")
            );
        }
    }
}

#[test]
fn native_wide_bitvec_product_prints_zero_and_partial_limbs() {
    for level in ["0", "3"] {
        let source = "pure comptime fn Elements<T>->.Type'builtin {T}pure comptime fn Tuple<...Es:Elements<.Type'builtin>>->.Type'builtin {bool}typealias A=.BitVec'builtin<129>;typealias B=.BitVec'builtin<256>;typealias C=.BitVec'builtin<1024>;fn run()->Tuple<A,B,C,A> {(0,42,18446744073709551616,36893488147419103233)}";
        let w = Work::new(source);
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--tuple-type",
                    "input.Tuple",
                    "--meta-list-type",
                    "input.Elements",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout,b"(tuple (unsigned-integer 0) (unsigned-integer 42) (unsigned-integer 18446744073709551616) (unsigned-integer 36893488147419103233))\n");
    }
}

#[test]
fn native_unsafe_scope_retains_audit_without_waiving_body_checks() {
    let sources = [
        "fn run()->bool {unsafe {let value=unsafe {true};value}}",
        "pure fn keep(value:bool)->bool {unsafe {value}}fn run()->bool {keep(true)}",
        "fn keep(value:T)->T where T {unsafe {value}}fn run()->bool {keep(true)}",
        "fn run()->bool {unsafe {return true;}false}",
    ];
    for source in sources {
        let w = Work::new(source);
        let dump = w.command().args(["--emit", "nir"]).output().unwrap();
        success(&dump);
        assert!(String::from_utf8_lossy(&dump.stdout).contains("(unsafe-scope "));
        let graph = w.command().args(["--instantiate", "run"]).output().unwrap();
        success(&graph);
        assert!(String::from_utf8_lossy(&graph.stdout).contains("(unsafe-scope "));
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        success(&w.build());
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_unsafe_scopes_preserve_owned_cleanup_and_transfers() {
    for body in [
        "let seen=false;unsafe {let watch=Watch {target:seen.ref};};seen",
        "let seen=false;let watch=unsafe {Watch {target:seen.ref}};watch;seen",
    ] {
        let w = Work::new(&format!(
            "{RESOURCE_HEADER}struct Watch {{target:*bool}}impl Retire for Watch {{fn drop(*self) {{self.target.*.* = true;}}}}fn run()->bool {{{body}}}"
        ));
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn unsafe_scope_does_not_authorize_overflow_or_local_pointer_escape() {
    for source in [
        "typealias Byte=.u8'builtin;fn run()->Byte {unsafe {let x:Byte=255;x + 1}}",
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}impl Duplicate for *T where T {}fn escape()->*bool {unsafe {let value=true;value.ref}}fn run()->bool {escape().*}",
    ] {
        let w = Work::new(source);
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let mut command = w.command();
        command.args(["--build", "run", "--output", "native program"]);
        if source.starts_with("trait") {
            command.args(["--copy-protocol", "Duplicate", "--drop-protocol", "Retire"]);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_unsafe_scope_preserves_pointer_and_callable_origins() {
    for (helper, body) in [
        ("", "let flag=true;let pointer=unsafe {flag.ref};pointer.*"),
        (
            "fn forward(value:*bool)->*bool {unsafe {value}}",
            "let flag=true;forward(flag.ref).*",
        ),
        (
            "",
            "let callback=unsafe {|value:bool| not value};callback(false)",
        ),
    ] {
        let w = Work::new(&format!(
            "{RESOURCE_HEADER}{helper}fn run()->bool {{{body}}}"
        ));
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn unsafe_audit_scope_cannot_forge_a_different_iir_result_type() {
    let prepared = prepared_host(&format!(
        "{RESOURCE_HEADER}fn run()->bool {{unsafe {{true}}}}"
    ));
    let mut graph = prepared.graph().clone();
    let mut changed = false;
    for instance in &mut graph.functions {
        nir::visit::expression_mut(&mut instance.function.body, &mut 100_000, |expr, _| {
            if matches!(expr.kind, nir::program::ExprKind::UnsafeScope(_)) {
                expr.ty = nir::Term::Primitive(nir::Primitive::U32);
                changed = true;
            }
            Ok(())
        })
        .unwrap();
    }
    assert!(changed);
    let error = graph.validate(&mut 100_000).unwrap_err();
    assert!(
        error.contains("unsafe scope changes its body type"),
        "{error}"
    );
}

#[test]
fn native_unsafe_scope_survives_optional_inlining_and_effect_resume() {
    for functions in [
        "inline fn choose(flag:bool)->bool {unsafe {if flag {return true;}false}}fn run()->bool {choose(true)}",
        "inline effect get()->bool;inline fn worker()->#get bool {unsafe {get()#}}fn run()->bool {worker()# {get()=>unsafe {resume true;},}}",
    ] {
        let w = Work::new(&format!("{RESOURCE_HEADER}{functions}"));
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--inline-functions",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn iir_rejects_unqualified_unsafe_entry_and_malformed_telescope() {
    let prepared = prepared_host(&format!("{RESOURCE_HEADER}fn run()->bool {{true}}"));
    for legacy_wrapper in [false, true] {
        let mut graph = prepared.graph().clone();
        let instance = graph
            .functions
            .iter_mut()
            .find(|instance| instance.function.name == "run")
            .unwrap();
        if legacy_wrapper {
            instance.function.signature =
                nir::Term::UnsafeFunction(Box::new(instance.function.signature.clone()));
        } else {
            instance.function.is_unsafe = true;
        }
        let error = graph.validate(&mut 100_000).unwrap_err();
        assert!(
            error.contains(if legacy_wrapper {
                "unqualified parameter/result telescope"
            } else {
                "unsafe entry requires an explicit safe wrapper"
            }),
            "{error}"
        );
    }
}

#[test]
fn native_unsafe_function_calls_require_and_preserve_lexical_boundary() {
    for functions in [
        "unsafe fn raw(value:bool)->bool {value}fn run()->bool {unsafe {raw(true)}}",
        "unsafe fn raw(value:bool)->bool {value}unsafe fn forward(value:bool)->bool {raw(value)}fn run()->bool {unsafe {forward(true)}}",
        "unsafe fn raw(value:bool)->bool {value}fn invoke(callback:unsafe fn(bool)->bool)->bool {unsafe {callback(true)}}fn run()->bool {invoke(raw)}",
    ] {
        let w = Work::new(functions);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_unsafe_calls_survive_templates_defaults_inlining_and_closures() {
    for source in [
        "struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}}unsafe fn consume(value:Watch) {}fn run()->bool {let seen=false;unsafe {consume(Watch {target:seen.ref});};seen}",
        "unsafe fn raw(value:bool)->bool {value}fn invoke(callback:unsafe fn(bool)->bool)->bool {unsafe {callback(true)}}fn run()->bool {invoke(raw)}",
        "trait Raw {unsafe fn get(value:bool)->bool;}struct S {}impl Raw for S {unsafe fn get(value:bool)->bool {value}}fn run()->bool {unsafe {S.as(Raw).get(true)}}",
        "unsafe fn raw(value:T)->T where T {value}fn run()->bool {unsafe {raw(true)}}",
        "inline unsafe fn raw(value:bool)->bool {value}fn run()->bool {unsafe {raw(true)}}",
        "unsafe fn raw(.value:bool=true)->bool {value}fn run()->bool {unsafe {raw()}}",
        "unsafe fn raw(value:bool)->bool {value}fn run()->bool {let callback=unsafe {||raw(true)};callback()}",
    ] {
        let w = Work::new(&format!("{RESOURCE_HEADER}{source}"));
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--inline-functions",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn unsafe_call_admission_cannot_leak_through_values_helpers_or_defaults() {
    for source in [
        "unsafe fn raw()->bool {true}fn run()->bool {raw()}",
        "unsafe fn raw()->bool {true}fn run()->bool {let callback=unsafe {raw};callback()}",
        "unsafe fn raw()->bool {true}fn run()->bool {unsafe {raw()};raw()}",
        "unsafe fn raw()->bool {true}fn helper()->bool {raw()}fn run()->bool {unsafe {helper()}}",
        "unsafe fn raw()->bool {true}fn helper(.value:bool=raw())->bool {value}fn run()->bool {unsafe {helper()}}",
        "unsafe fn run()->bool {true}",
        "comptime unsafe fn raw()->bool {true}fn run()->bool {true}",
    ] {
        let w = Work::new(source);
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let output = w.build();
        assert!(!output.status.success(), "unexpected success: {source}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!error.contains("panicked"), "{source}: {error}");
        assert!(error.contains("unsafe"), "{source}: {error}");
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_unsafe_call_repository_example() {
    native(
        include_str!("../../../development/examples/host-unsafe-call.fl"),
        "(bool true)\n",
    );
}

#[test]
fn native_staged_runtime_callback_splice_preserves_phase_and_unsafe_type() {
    let source = include_str!("../../../development/examples/host-staged-unsafe-callback.fl");
    for source in [
        source.to_owned(),
        source
            .replace("unsafe { PICKED(true) }", "PICKED(true)")
            .replace("unsafe ", ""),
    ] {
        let w = Work::new(&format!(
            "{RESOURCE_HEADER}{source}impl Duplicate for Raw {{}}"
        ));
        for level in ["0", "3"] {
            for checked in [false, true] {
                let mut command = w.command();
                command.args([
                    "--splice-protocol",
                    "Splice",
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ]);
                if checked {
                    command.args(["--copy-protocol", "Duplicate", "--drop-protocol", "Retire"]);
                }
                success(&command.output().unwrap());
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, b"(bool true)\n");
            }
        }
    }
}

#[test]
fn native_frozen_runtime_callback_instances_keep_distinct_arguments() {
    let example = include_str!("../../../development/examples/host-staged-unsafe-callback.fl");
    let header = example.split_once("unsafe fn raw").unwrap().0;
    let w = Work::new(&format!(
        "{RESOURCE_HEADER}{header}impl Duplicate for Raw {{}}
        unsafe fn raw(value:bool)->bool where flag:bool {{if flag {{value}} else {{not value}}}}
        comptime fn choose(comptime flag:bool)->Raw {{if flag {{raw<true>}} else {{raw<false>}}}}
        const FIRST=choose(true);const SECOND=choose(false);
        fn run()->bool {{unsafe {{FIRST(true) and not SECOND(true)}}}}"
    ));
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--splice-protocol",
                    "Splice",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn frozen_runtime_callback_cannot_be_invoked_at_comptime_or_weakened_by_splice() {
    let example = include_str!("../../../development/examples/host-staged-unsafe-callback.fl");
    let bad_phase=example.replace("const PICKED = select();", "comptime fn bad()->bool {let callback=select();unsafe {callback(true)}}const TRIGGER=bad();const PICKED=select();");
    let bad_type = example
        .replace(
            "assoc RuntimeTarget: .Type'builtin = Raw;",
            "assoc RuntimeTarget: .Type'builtin = fn(bool)->bool;",
        )
        .replace(
            "fn splice(comptime data: Raw) -> Raw",
            "fn splice(comptime data: Raw) -> fn(bool)->bool",
        );
    for source in [bad_phase, bad_type] {
        let w = Work::new(&source);
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let output = w
            .command()
            .args([
                "--splice-protocol",
                "Splice",
                "--build",
                "run",
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!error.contains("panicked"), "{error}");
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_trait_aggregate_result_retains_contracts_and_rejects_local_escape() {
    let cases = [
        ("struct View {pointer:*bool} impl Duplicate for View {}
          trait Inspect {fn view(*self,allowed:bool)->Self where requires allowed;}
          impl Inspect for View {fn view(*self,allowed:bool)->Self {self.*}}
          fn run()->bool {let flag=true;let original=View {pointer:flag.ref};let result=original.as(Inspect).view(false);result.pointer.*}",
          "precondition"),
        ("struct View {pointer:*bool} impl Duplicate for View {}
          trait Factory {fn make()->View;} struct FactoryHost {}
          impl Factory for FactoryHost {fn make()->View {let flag=true;View {pointer:flag.ref}}}
          fn run()->bool {let result=FactoryHost.as(Factory).make();result.pointer.*}",
          "local"),
        ("struct Owner {flag:bool} struct View {pointer:*bool} impl Duplicate for View {}
          trait Borrow {fn borrow(*self)->View;}
          impl Borrow for Owner {fn borrow(*self)->View {View {pointer:self.*.flag.ref}}}
          fn run()->bool {let original=Owner {flag:true};let view=original.as(Borrow).borrow();let moved=original;view.pointer.*}",
          "unavailable"),
    ];
    for (source, reason) in cases {
        let w = Work::new(&format!("{RESOURCE_HEADER}{source}"));
        fs::write(w.0.join("native program"), b"previous output").unwrap();
        let output = resource_build(&w);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(reason), "{error}");
        assert_eq!(
            fs::read(w.0.join("native program")).unwrap(),
            b"previous output"
        );
        if reason == "precondition" {
            fs::write(
                w.0.join("source.fl"),
                format!(
                    "{RESOURCE_HEADER}{}",
                    source.replace("view(false)", "view(true)")
                ),
            )
            .unwrap();
            // Even a satisfied source predicate still needs a composed memory
            // contract. Do not select behavior-only code and erase this boundary.
            let output = resource_build(&w);
            assert!(!output.status.success());
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("interface predicates require a composed memory contract")
            );
            assert_eq!(
                fs::read(w.0.join("native program")).unwrap(),
                b"previous output"
            );
        }
    }
}

#[test]
fn native_trait_aggregate_result_preserves_nested_generic_pointer_fields() {
    let source = format!("{RESOURCE_HEADER}
        struct View where T {{pointer:*T}}
        impl Duplicate for View<T> where T {{}}
        struct Nested where T {{view:View<T>}}
        impl Duplicate for Nested<T> where T {{}}
        trait Snapshot {{fn snapshot(*self)->Self;}}
        impl Snapshot for Nested<T> where T {{fn snapshot(*self)->Self {{self.*}}}}
        fn run()->bool {{var flag=false;let original=Nested<bool> {{view:View<bool> {{pointer:flag.ref}}}};
            let result=original.as(Snapshot).snapshot();result.view.pointer.* = true;original.view.pointer.* and flag}}"
    );
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_staged_trait_contract_aligns_results_and_entry_snapshots() {
    for implementation in [
        "fn keep(comptime enabled:bool,value:bool)->result:bool where ensures result == value'old {value}",
        "fn keep(comptime enabled:bool,value:bool)->bool {value}",
    ] {
        let source = format!("{RESOURCE_HEADER}
            trait Keep {{fn keep(comptime enabled:bool,value:bool)->answer:bool
                where requires enabled, ensures answer == value'old;}}
            struct Host {{}} impl Keep for Host {{{implementation}}}
            fn run()->bool {{Host.as(Keep).keep(true,true) and not Host.as(Keep).keep(true,false)}}");
        let w = Work::new(&source);
        success(&resource_build(&w));
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
        fs::write(
            w.0.join("source.fl"),
            source.replace("keep(true,true)", "keep(false,true)"),
        )
        .unwrap();
        let old = fs::read(w.0.join("native program")).unwrap();
        let output = resource_build(&w);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("precondition") || error.contains("predicate"),
            "{error}"
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), old);
    }
}

#[test]
fn native_trait_entry_predicates_use_prepared_arguments_once() {
    native(
        "trait Gate {fn enter(first:bool,.second:bool=first)->bool where requires first and second;}
        struct Host {} impl Gate for Host {fn enter(first:bool,.second:bool=first)->bool {first and second}}
        fn run()->bool {var seen=false;
            let value=Host.as(Gate).enter(do {assumes not seen;seen=true;true});
            value and seen}",
        "(bool true)\n",
    );
}

#[test]
fn native_trait_exit_predicates_bind_each_actual_result_and_old_value() {
    native(
        "trait Echo {fn echo(flag:bool)->answer:bool where ensures answer == flag'old;}
        struct Host {} impl Echo for Host {fn echo(flag:bool)->result:bool where ensures result == flag'old {flag}}
        fn run()->bool {Host.as(Echo).echo(true) and not Host.as(Echo).echo(false)}",
        "(bool true)\n",
    );
}

#[test]
fn native_resource_checked_trait_exit_publishes_result_boundary() {
    let w=Work::new(&format!("{RESOURCE_HEADER}
        trait Echo {{fn echo(flag:bool)->answer:bool where ensures answer == flag'old;}}
        struct Host {{}} impl Echo for Host {{fn echo(flag:bool)->result:bool where ensures result == flag'old {{flag}}}}
        fn run()->bool {{Host.as(Echo).echo(true) and not Host.as(Echo).echo(false)}}"));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_resource_checked_trait_exit_keeps_caller_and_pending_owners() {
    for body in [
        "let owner=Token {};Host.as(Echo).echo(true)",
        "consume(Token {},Host.as(Echo).echo(true))",
        "Host.as(Echo).finish(true);true",
        "let owner=Token {};var again=true;while again {again=false;Host.as(Echo).echo(true);} true",
        "let owner=Token {};delayed()",
    ] {
        let w = Work::new(&format!(
            "{RESOURCE_HEADER}
            struct Token {{}} impl Retire for Token {{fn drop(*self) {{}}}}
            trait Echo {{
                fn echo(flag:bool)->answer:bool where ensures answer == flag'old;
                fn finish(flag:bool) where requires flag, ensures flag'old;
            }}
            struct Host {{}} impl Echo for Host {{
                fn echo(flag:bool)->result:bool where ensures result == flag'old {{flag}}
                fn finish(flag:bool) {{}}
            }}
            fn consume(token:Token,flag:bool)->bool {{flag}}
            fn delayed(.flag:bool=Host.as(Echo).echo(true))->bool {{flag}}
            fn run()->bool {{{body}}}"
        ));
        success(&resource_build(&w));
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_resource_checked_trait_owned_result_is_delivered_once() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-interface-contracts.fl"
    ));
    success(&resource_build(&w));
    let out = w.execute();
    success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn native_resource_checked_interface_forwards_callback_inputs() {
    for callback in ["yes", "|| true"] {
        let w=Work::new(&format!("{RESOURCE_HEADER}
            fn yes()->bool {{true}}
            trait Apply {{fn apply(f:fn()->bool,flag:bool)->bool where requires flag, ensures flag'old;}}
            struct Host {{}} impl Apply for Host {{fn apply(f:fn()->bool,flag:bool)->bool {{f()}}}}
            fn run()->bool {{Host.as(Apply).apply({callback},true)}}"));
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn resource_checked_interface_tracks_affine_callback_environment() {
    for twice in [false, true] {
        let body = if twice { "f();f()" } else { "f()" };
        let w=Work::new(&format!("{RESOURCE_HEADER}
            struct Token {{}} impl Retire for Token {{fn drop(*self) {{}}}}
            fn consume(token:Token) {{}}
            trait Apply {{fn apply(f:%{{token:Token}} fn()->bool,flag:bool)->bool where requires flag, ensures flag'old;}}
            struct Host {{}} impl Apply for Host {{fn apply(f:%{{token:Token}} fn()->bool,flag:bool)->bool {{{body}}}}}
            fn run()->bool {{let token=Token {{}};Host.as(Apply).apply(|| {{consume(token);true}},true)}}"));
        let build = resource_build(&w);
        if twice {
            assert!(!build.status.success());
            let error = String::from_utf8_lossy(&build.stderr);
            assert!(
                error.contains("unavailable") || error.contains("move"),
                "{error}"
            );
        } else {
            let prepared = prepared_host(&fs::read_to_string(w.0.join("source.fl")).unwrap());
            let mut vm =
                comptime_interpretor::Interpreter::from_iir(prepared.graph(), Default::default())
                    .unwrap()
                    .with_cleanup_schedule(prepared.schedule())
                    .unwrap();
            let entry = prepared
                .graph()
                .functions
                .iter()
                .find(|f| f.function.name == "run")
                .unwrap();
            assert_eq!(
                vm.invoke_instance(&entry.key, vec![]).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
            success(&build);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_interface_factory_delivers_owned_closure_after_postchecks() {
    for body in [
        "let task=Host.as(Factory).make(Token {},true);task()",
        "Host.as(Factory).make(Token {},true)()",
    ] {
        let w=Work::new(&format!("{RESOURCE_HEADER}
        struct Token {{}} impl Retire for Token {{fn drop(*self) {{}}}}
        fn consume(token:Token) {{}}
        typealias Task=%{{token:Token}} fn()->bool;
        trait Factory {{fn make(token:Token,flag:bool)->Task where requires flag, ensures flag'old;}}
        struct Host {{}} impl Factory for Host {{fn make(token:Token,flag:bool)->Task {{|| {{consume(token);true}}}}}}
        fn run()->bool {{{body}}}"));
        success(&resource_build(&w));
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_closure_capture_state_survives_conditional_consumption() {
    let source = include_str!("../../../development/examples/host-closure-consume.fl");
    for (invocation, reject) in [
        ("callback(flag);", false),
        ("(do {callback})(flag);", false),
        ("callback(flag);finish(callback);", true),
        ("let outer=|take:bool| callback(take);outer(flag);", false),
    ] {
        let source = source.replace("callback(flag);", invocation);
        let w = Work::new(&source);
        if reject {
            // Persisting state does not make a partially consumed owner whole.
            let rejected = resource_build(&w);
            assert!(!rejected.status.success());
            assert!(String::from_utf8_lossy(&rejected.stderr).contains("use of unavailable value"));
            continue;
        }
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_temporary_closure_reinitializes_capture_state_each_iteration() {
    let source = include_str!("../../../development/examples/host-closure-consume.fl")
        .replace(
            "    do {",
            "    var rounds:Count=0; var take=flag; while rounds < 3 {",
        )
        .replace(
            "callback(flag);",
            "(do {callback})(take);take=not take;rounds=rounds + 1;",
        )
        .replace("count == 2", "count == 6");
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_platform_integer_cli_selects_host_target() {
    let source = include_str!("../../../development/examples/host-platform-integers.fl");
    let work = Work::new(source);
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(unsigned-integer 42)\n");
    }
    let output = work
        .command()
        .args(["--emit-llvm", "run"])
        .output()
        .unwrap();
    success(&output);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("!flurry.target")
    );
}

#[test]
fn native_package_platform_integer_cli_selects_host_target() {
    let work = Work::new(include_str!(
        "../../../development/examples/host-platform-integers.fl"
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&work.0)
        .args([
            "--packages-v1",
            "--package",
            "1",
            "701",
            "app",
            "source.fl",
            "--primary",
            "1",
            "--build",
            "run",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    success(&output);
    let output = work.execute();
    success(&output);
    assert_eq!(output.stdout, b"(unsigned-integer 42)\n");
}

#[test]
fn native_source_tests_accept_platform_integers_in_single_and_multi_package_modes() {
    let work = Work::new(
        "typealias Word=.usize'builtin; test platform {let count:Word=42; assumes count == 42;}",
    );
    for level in ["0", "3"] {
        let output = work
            .command()
            .args(["--test-native", "--opt-level", level])
            .output()
            .unwrap();
        success(&output);
        assert!(String::from_utf8_lossy(&output.stdout).contains("(count 1) (failures 0)"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&work.0)
        .args([
            "--packages-v1",
            "--package",
            "1",
            "701",
            "app",
            "source.fl",
            "--primary",
            "1",
            "--test-native",
        ])
        .output()
        .unwrap();
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("(count 1) (failures 0)"));
}

#[test]
fn native_source_tests_missing_toolchain_preserves_empty_selection() {
    let work = Work::new("test selected {assumes true;}");
    for multi in [false, true] {
        for filter in ["absent", "selected"] {
            let mut command = if multi {
                let mut command = Command::new(env!("CARGO_BIN_EXE_luna"));
                command.current_dir(&work.0).args([
                    "--packages-v1",
                    "--package",
                    "1",
                    "701",
                    "app",
                    "source.fl",
                    "--primary",
                    "1",
                ]);
                command
            } else {
                work.command()
            };
            let output = command
                .args(["--test-native", "--test-filter", filter])
                .env("LUNA_LLVM_CONFIG", "/definitely/missing/llvm-config")
                .output()
                .unwrap();
            if filter == "absent" {
                success(&output);
                assert!(String::from_utf8_lossy(&output.stdout).contains("(count 0) (failures 0)"));
            } else {
                assert!(!output.status.success());
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("cannot start /definitely/missing/llvm-config")
                );
            }
        }
    }
}

#[test]
fn native_platform_pointer_counter_preserves_contracts() {
    let work = Work::new(
        "trait Duplicate {} trait Retire {fn drop(*self);} typealias Word=.usize'builtin; impl Duplicate for Word {} fn advance(p:*Word) where requires p.* < 100 { p.* = p.* + 1; } fn initialize(p:*Word) where ensures p.* < 100 {p.* = 41;} fn run()->Word { let count:Word=0; initialize(count.ref); advance(count.ref); count }",
    );
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(unsigned-integer 42)\n");
    }
    let source = fs::read_to_string(work.0.join("source.fl")).unwrap();
    let invalidated = source
        .replace("fn run()", "fn overwrite(p:*Word) {p.* = 100;} fn run()")
        .replace(
            "initialize(count.ref); advance",
            "initialize(count.ref); overwrite(count.ref); advance",
        );
    let rejected = Work::new(&invalidated);
    let output = rejected
        .command()
        .args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostic.contains("flow-error"), "{diagnostic}");
}

#[test]
fn native_platform_bit_index_checks_bounds() {
    let source = "typealias Index=.usize'builtin; typealias Bits=.BitVec'builtin<8>; typealias One=.BitVec'builtin<1>; fn choose(bits:Bits,index:Index)->One where requires index < 8 {bits[index]} fn run()->bool {choose(0b10000001,0) == 1 and choose(0b10000001,7) == 1 and choose(0b10000001,1) == 0}";
    let work = Work::new(source);
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
    let rejected = Work::new(&source.replace("choose(0b10000001,7)", "choose(0b10000001,8)"));
    let output = rejected.build();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("flow-error"));
}

#[test]
fn native_platform_bit_window_requires_signed_lower_and_upper_bounds() {
    let source = "typealias Index=.isize'builtin; typealias Bits=.BitVec'builtin<8>; typealias Nibble=.BitVec'builtin<4>; fn select(bits:Bits,index:Index)->Nibble where requires index >= 0, requires index <= 4 {bits[index..(index + 4)]} fn run()->bool {select(0xA5,0) == 5 and select(0xA5,4) == 10}";
    for source in [
        source.to_owned(),
        source.replace(".isize'builtin", ".usize'builtin"),
    ] {
        let work = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &work
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
    for invalid in [
        source.replace("requires index >= 0, ", ""),
        source.replace("select(0xA5,0)", "select(0xA5,-1)"),
        source.replace("select(0xA5,4)", "select(0xA5,5)"),
    ] {
        let rejected = Work::new(&invalid);
        let output = rejected.build();
        assert!(!output.status.success());
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains("flow-error"), "{diagnostic}");
    }
}

#[test]
fn native_platform_bit_update_preserves_endpoint_snapshot() {
    let source = include_str!("../../../development/examples/host-bitvec.fl").replace(
        "typealias Byte = .u8'builtin;",
        "typealias Byte = .usize'builtin;",
    );
    let work = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
    for invalid in [
        source.replace("replace(4)", "replace(13)"),
        source.replace("let end: Byte = 16;", "let end: Byte = 17;"),
    ] {
        let rejected = Work::new(&invalid);
        let output = rejected.build();
        assert!(!output.status.success());
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains("flow-error"), "{diagnostic}");
    }
}

#[test]
fn native_platform_bit_representation_roundtrip_and_writeback() {
    let work = Work::new(
        r#"
typealias U = .usize'builtin;
typealias I = .isize'builtin;
pure fn signed(value: I) -> I { value'bitvec.as(I) }
pure fn change(value: U, index: U) -> U where requires index <= 4 {
    var word: U = value;
    word'bitvec[index..(index + 4)] = 0b1010;
    word
}
fn run() -> bool { signed(-1) == -1 and change(0,4) == 160 }
"#,
    );
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_platform_bit_pointer_writeback() {
    let work = Work::new(
        r#"
trait Duplicate {} trait Retire { fn drop(*self); }
typealias I = .isize'builtin;
impl Duplicate for I {}
fn clear_low(p: *I) { p.*'bitvec[0..4] = 0; }
fn run() -> I { let value: I = -1; clear_low(value.ref); value }
"#,
    );
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(signed-integer -16)\n");
    }
}

#[test]
fn native_platform_enum_payload_encoding() {
    let work = Work::new(include_str!(
        "../../../development/examples/host-platform-enum.fl"
    ));
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_platform_bit_pattern_partition() {
    let work = Work::new(
        r#"
typealias Word=.isize'builtin;
fn classify(word: Word)->bool { word'bitvec match {
    0b(value: Word) if value == -1 => true,
    0b(value: Word) if -1 != value => false,
} }
fn run()->bool { classify(-1) and not classify(42) }
"#,
    );
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_platform_range_patterns_and_complements() {
    let work = Work::new(
        r#"
typealias I=.isize'builtin;
typealias U=.usize'builtin;
fn negative(word: I)->bool { word match { ..0 => true, 0.. => false, } }
fn small(word: U)->bool { word match { 0..10 => true, not (0..10) => false, } }
fn run()->bool { negative(-1) and not negative(0) and small(0) and small(9) and not small(10) }
"#,
    );
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_platform_nominal_value_indices() {
    let work = Work::new(
        r#"
trait Duplicate {} trait Retire {fn drop(*self);}
typealias U=.usize'builtin;
impl Duplicate for U {}
struct Tagged where n:U {value:U}
enum Choice where n:U {none,some(Tagged<n>)}
fn read(value:Choice<42>)->U {value match {.none=>0,.some(item)=>item.value}}
fn run()->U {read(Choice<42>.some(Tagged<42>{value:42}))}
"#,
    );
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(unsigned-integer 42)\n");
    }
}

#[test]
fn native_platform_trait_value_index_dispatch() {
    let work = Work::new(
        "typealias U=.usize'builtin; trait Read where tag:U {fn read()->U;} impl Read<tag> for bool where tag:U {fn read()->U {tag}} fn run()->U {bool.as(Read<42>).read()}",
    );
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(unsigned-integer 42)\n");
    }
}

#[test]
fn native_generic_pure_trait_and_default_methods() {
    let work = Work::new(
        "trait Flip {pure fn flip(self)->Self; pure fn twice(self)->Self {self.flip().flip()}} impl Flip for bool {fn flip(self)->Self {not self}} pure fn generic(value:T)->T where T:-Flip {value.flip()} pure fn twice(value:T)->T where T:-Flip {value.twice()} fn run()->bool {generic(false) and twice(true)}",
    );
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_symbol_encoded_enums_preserve_codes_and_variant_names() {
    let source = include_str!("../../../development/examples/host-symbol-enum.fl");
    let w = Work::new(source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(String::from_utf8_lossy(&output.stdout), "(bool true)\n");
    }
}

#[test]
fn native_enum_tag_names_preserve_groups_payloads_and_evaluation_count() {
    for definition in [
        "enum Event {group.{ready(bool)}, closed}",
        "enum Event {.base_type .u16'builtin,.width 16,.discrete_tag true,group.{ready:0b00000000 (flag:.u8'builtin)},closed=256}",
        "enum Event {.base_type .Symbol'builtin,.discrete_tag true,group.{ready=.wire_ready},closed=.wire_closed}",
    ] {
        let symbol = definition.contains("Symbol");
        let fixed = definition.contains(".width");
        let construct = if symbol {
            "Event.group.ready"
        } else if fixed {
            "Event.group.ready {flag:7}"
        } else {
            "Event.group.ready(true)"
        };
        let pattern = if symbol {
            ".group.ready"
        } else if fixed {
            ".group.ready {flag: 7}"
        } else {
            ".group.ready(true)"
        };
        let source = format!(
            "{RESOURCE_HEADER} impl Duplicate for .u8'builtin {{}} impl Duplicate for Event {{}} {definition}
            fn run()->bool {{
                var seen=false;
                let event={construct};
                let name=(do {{assumes not seen;seen=true;event}})'tag_name;
                seen and name == \"group.ready\" and (event match {{{pattern}=>true,_=>false}})
                    and Event.closed'tag_name == \"closed\"
            }}"
        );
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_enum_tag_name_retires_temporary_payload_once() {
    for body in [
        "var seen=false;let name=make(seen.ref)'tag_name;seen and name == \"held\"",
        "var seen=false;do {let event=make(seen.ref);let first=event'tag_name;let second=event'tag_name;let matched=first == \"held\" and second == \"held\";assumes matched;assumes not seen;};seen",
    ] {
        let w=Work::new(&format!("{RESOURCE_HEADER}
        struct Held {{seen:*bool}}
        impl Retire for Held {{fn drop(*self) {{let previous=self.seen.*.*;assumes not previous;self.seen.*.* = true;}}}}
        enum Event {{held(Held),empty}}
        fn make(seen:*bool)->Event {{Event.held(Held {{seen:seen}})}}
        fn run()->bool {{{body}}}
    "));
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_runtime_symbols_preserve_type_values_and_names() {
    let source = "typealias Symbol=.Symbol'builtin;typealias str=.str'builtin;
        fn identity(value:Symbol)->Symbol {value}
        fn choose(value:Symbol)->bool {value match {.ready=>true,_=>false}}
        fn run()->bool {var seen=false;let value=identity(.ready);
            let name=(do {assumes not seen;seen=true;value})'tag_name;
            seen and choose(value) and not choose(.waiting) and value == .ready
                and value != .waiting and name == \"ready\"}
        fn raw()->Symbol {identity(.ready)}
        fn early()->str {(do {return \"early\";})'tag_name}";
    let w = Work::new(source);
    for level in ["0", "3"] {
        for (entry, expected) in [
            ("run", "(bool true)\n"),
            ("raw", "(symbol \"ready\")\n"),
            ("early", "(string \"early\")\n"),
        ] {
            success(
                &w.command()
                    .args([
                        "--build",
                        entry,
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
        }
    }
}

#[test]
fn native_checked_symbol_storage() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        typealias Symbol=.Symbol'builtin;impl Duplicate for Symbol {}
        struct Entry {tag:Symbol} impl Duplicate for Entry {}
        enum Choice {none,item(Symbol)} impl Duplicate for Choice {}
        fn get(value:Choice)->Symbol {value match {.none=>.missing,.item(tag)=>tag}}
        fn run()->bool {var entry=Entry {tag:.ready};let pointer=entry.tag.ref;
            pointer.* = .waiting;let chosen=get(Choice.item(entry.tag));
            chosen == .waiting and (entry matches {tag:.waiting})}";
    let w = Work::new(source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_symbol_and_string_product_results_remain_distinct() {
    let w = Work::new(
        "fn record(...fields:T)->T where T {fields}
        typealias Result=(record {tag:.ready,text:\"ready\"})'type;
        fn run()->Result {record {tag:.ready,text:\"ready\"}}",
    );
    let emitted = w.command().args(["--emit-llvm", "run"]).output().unwrap();
    success(&emitted);
    let ir = String::from_utf8(emitted.stdout).unwrap();
    assert_eq!(
        ir.lines()
            .filter(|line| line.starts_with("@flurry_string_")
                && line.contains(" = private unnamed_addr constant"))
            .count(),
        1,
        "symbol and string retain distinct types but share identical immutable bytes: {ir}"
    );
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "(anonymous-record (fields (field (string \"tag\") (symbol \"ready\")) (field (string \"text\") (string \"ready\"))))\n"
        );
    }
}

#[test]
fn native_symbol_closure_transfers_state_and_retires_environment() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-symbol-closure.fl"
    ));
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
    let previous = fs::read(w.0.join("native program")).unwrap();
    let invalid = include_str!("../../../development/examples/host-symbol-closure.fl").replace(
        "let valid = close(transition);",
        "let valid = close(transition);transition(.closed);",
    );
    fs::write(w.0.join("source.fl"), invalid).unwrap();
    let rejected = resource_build(&w);
    assert!(!rejected.status.success());
    let diagnostic = String::from_utf8_lossy(&rejected.stderr);
    assert!(diagnostic.contains("move"), "{diagnostic}");
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), previous);
}

#[test]
fn native_generic_name_views_keep_symbol_and_enum_domains() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for .Symbol'builtin {}
        typealias Symbol=.Symbol'builtin;typealias str=.str'builtin;
        enum Event where T {network.{ready(T)},closed}
        impl Duplicate for Event<T> where T:-Duplicate {}
        fn symbol_name(value:T)->str where T, requires T == Symbol {value'tag_name}
        fn enum_name(value:T)->str where T, requires T == Event<bool> {value'tag_name}
        fn run()->bool {let a=symbol_name<Symbol>(.ready);let b=enum_name<Event<bool>>(Event<bool>.network.ready(true));
            a == \"ready\" and b == \"network.ready\"}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
    let previous = fs::read(w.0.join("native program")).unwrap();
    for invalid in [
        source.replace("where T, requires T == Symbol", "where T"),
        format!(
            "pure comptime fn Identity<T> -> .Type'builtin {{T}}\n{}",
            source.replace("T == Symbol", "T == Identity<Symbol>")
        ),
    ] {
        fs::write(w.0.join("source.fl"), invalid).unwrap();
        let output = resource_build(&w);
        assert!(!output.status.success());
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("tag_name")
                || diagnostic.contains("structural type view remains unresolved"),
            "{diagnostic}"
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), previous);
    }
}

#[test]
fn native_character_results_keep_their_logical_type() {
    for literal in ["'a'", "'中'", "'🦀'", "'\\0'", "'\\n'"] {
        let w = Work::new(&format!("fn run()->.char'builtin {{{literal}}}"));
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        success(&reference);
        assert!(String::from_utf8_lossy(&reference.stdout).starts_with("(char "));
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
    let w = Work::new(
        "fn record(...fields:T)->T where T {fields}
        typealias Result=(record {character:'a',integer:do {let value:.u32'builtin=97;value}})'type;
        fn run()->Result {record {character:'a',integer:do {let value:.u32'builtin=97;value}}}",
    );
    let reference = w.command().args(["--interpret", "run"]).output().unwrap();
    success(&reference);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "(anonymous-record (fields (field (string \"character\") (char 97)) (field (string \"integer\") (unsigned-integer 97))))\n"
        );
    }
    let checked = Work::new(&format!(
        "{RESOURCE_HEADER} impl Duplicate for .char'builtin {{}} fn run()->.char'builtin {{'中'}}"
    ));
    success(&resource_build(&checked));
    let output = checked.execute();
    success(&output);
    assert_eq!(output.stdout, b"(char 20013)\n");
}

#[test]
fn native_error_result_entries_print_only_the_active_payload() {
    for (signature, body) in [
        ("![Symbol,bool] Char", "'中'"),
        ("![Symbol,bool] Char", "error .ready"),
        ("![Symbol,bool] Char", "error true"),
        ("!Symbol Never", "error .ready"),
        ("!Symbol Void", "error .ready"),
    ] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;typealias Char=.char'builtin;typealias Void=.void'builtin;typealias Never=.noreturn'builtin;fn run()->{signature} {{{body}}}"
        );
        let w = Work::new(&source);
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        success(&reference);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_error_result_printer_does_not_erase_owned_payloads() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER} struct Held {{}} impl Retire for Held {{fn drop(*self) {{}}}} fn run()->!Held bool {{error Held {{}}}}"
    ));
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let rejected = resource_build(&w);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("owned nominal"));
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    success(
        &w.command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--discard-result",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap(),
    );
    let output = w.execute();
    success(&output);
    assert_eq!(output.stdout, b"(unit)\n");
}

#[test]
fn native_error_result_product_fields_evaluate_once() {
    for failed in [false, true] {
        let source=format!("{RESOURCE_HEADER} impl Duplicate for Symbol {{}} impl Duplicate for Char {{}} typealias Symbol=.Symbol'builtin;typealias Char=.char'builtin;
            typealias Response = !Symbol Char;
            fn record(...fields:T)->T where T {{fields}}
            fn schema()->Response {{'A'}}
            typealias Packet=(record {{result:schema(),seen:false}})'type;
            fn sample(failed:bool,seen:*bool)->Response {{assumes not seen.*;seen.* = true;if failed {{error .missing}} else {{'A'}}}}
            fn run()->Packet {{var seen=false;record {{result:sample({failed},seen.ref),seen:seen}}}}");
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
            let text = String::from_utf8_lossy(&output.stdout);
            assert!(text.contains("(bool true)"), "{text}");
            assert_eq!(text.matches("(error-result ").count(), 1);
            if failed {
                assert!(text.contains("(symbol \"missing\")"), "{text}");
                assert!(!text.contains("(char 65)"));
            } else {
                assert!(text.contains("(char 65)"), "{text}");
                assert!(!text.contains("(symbol \"missing\")"));
            }
        }
    }
}

#[test]
fn native_copysign_preserves_magnitude_bits_and_uses_the_sign_bit() {
    // Includes quiet/signalling NaN payloads, subnormals, infinities and signed zeros.
    // The expected result is a bit operation, not host floating-point arithmetic.
    for (width, magnitudes, signs) in [
        (
            32,
            vec![0, 0x80000000, 1, 0xff800000, 0x7fc12345, 0x7f800001],
            vec![0, 0x80000000, 0xffc00001],
        ),
        (
            64,
            vec![
                0,
                0x8000000000000000,
                1,
                0xfff0000000000000,
                0x7ff8123456789abc,
                0x7ff0000000000001,
            ],
            vec![0, 0x8000000000000000, 0xfff8000000000001],
        ),
    ] {
        let sign_mask = 1u64 << (width - 1);
        let mut cases = String::new();
        for magnitude in &magnitudes {
            for sign in &signs {
                let expected = (magnitude & !sign_mask) | (sign & sign_mask);
                cases.push_str(&format!(
                    "do {{
                    var magnitude:F=0.0; magnitude'bitvec[0..{width}]={magnitude};
                    var sign:F=0.0; sign'bitvec[0..{width}]={sign};
                    let result=copy(magnitude,sign);
                    let bits=result'bitvec;
                    assumes bits == {expected};
                }};"
                ));
            }
        }
        let source = format!(
            "{RESOURCE_HEADER}
            typealias F=.f{width}'builtin;
            impl Duplicate for F {{}}
            impl Duplicate for .BitVec'builtin<{width}> {{}}
            ^intrinsic_enum enum Ops {{sign}}
            fn copy(magnitude:F,sign:F)->F {{unsafe {{Ops.sign'intrinsic(magnitude,sign).as(F)}}}}
            fn run()->bool {{{cases} true}}"
        );
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--host-copysign",
                        "input.Ops.sign",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_void_error_success_and_exit_cleanup() {
    for (early, failed) in [(false, false), (true, false), (false, true)] {
        let w = Work::new(&format!(
            "{RESOURCE_HEADER}
            typealias Void=.void'builtin;
            struct Held {{closed:*bool}}
            impl Retire for Held {{fn drop(*self) {{
                let previous=self.closed.*.*;
                assumes not previous;
                self.closed.*.* = true;
            }}}}
            fn finish(early:bool,failed:bool,closed:*bool)->!bool Void {{
                let held=Held {{closed:closed}};
                if failed {{return error true;}}
                if early {{return;}}
            }}
            fn run()->bool {{
                var closed=false;
                let outcome=finish({early},{failed},closed.ref);
                let correct=outcome match {{_! =>not {failed},error code=>{failed} and code,}};
                closed and correct
            }}"
        ));
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
    for body in ["", "do {}", "return;", "if true {}"] {
        let w = Work::new(&format!(
            "typealias Void=.void'builtin;fn run()->!bool Void {{{body}}}"
        ));
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        success(&reference);
        assert!(
            String::from_utf8_lossy(&reference.stdout).contains("(branch success) (value (unit))")
        );
        success(&w.build());
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_void_error_propagation_and_catches_preserve_order() {
    for failed in [false, true] {
        let w=Work::new(&format!("{RESOURCE_HEADER}
            typealias Void=.void'builtin;
            fn step(failed:bool)->!bool Void {{if failed {{return error true;}}}}
            fn forward(failed:bool,continued:*bool)->!bool Void {{
                step(failed)!;
                continued.* = true;
            }}
            fn run()->bool {{
                var continued=false;
                var before=false;
                var arm=false;
                var after=false;
                forward({failed},continued.ref)! {{
                    catch input=>{{assumes not before and not arm and not after; before=true;}},
                    true=>{{assumes before and not arm and not after; arm=true;}},
                    catch input=>{{assumes before and arm and not after; after=true;}},
                }};
                continued == not {failed} and before == {failed} and arm == {failed} and after == {failed}
            }}"));
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_void_error_catch_exits_release_local_owners() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-void-error-flow.fl"
    ));
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_rounding_intrinsics_keep_ieee_boundaries() {
    for (operation, expected) in [
        (
            "floor",
            [-3.0f64, -2.0, -1.0, 0.0, -0.0, 0.0, 1.0, 2.0, 3.0],
        ),
        ("ceil", [-2.0f64, -1.0, -0.0, 0.0, -0.0, 1.0, 2.0, 3.0, 4.0]),
        (
            "trunc",
            [-2.0f64, -1.0, -0.0, 0.0, -0.0, 0.0, 1.0, 2.0, 3.0],
        ),
        (
            "roundeven",
            [-2.0f64, -2.0, -0.0, 0.0, -0.0, 0.0, 2.0, 2.0, 4.0],
        ),
    ] {
        for width in [32, 64] {
            let encode = |value: f64| {
                if width == 32 {
                    u64::from((value as f32).to_bits())
                } else {
                    value.to_bits()
                }
            };
            let sign_mask = 1u64 << (width - 1);
            let mut cases = Vec::new();
            for (input, output) in [-2.5f64, -1.5, -0.5, 0.0, -0.0, 0.5, 1.5, 2.5, 3.5]
                .into_iter()
                .zip(expected)
            {
                cases.push((encode(input), encode(output)));
            }
            cases.extend([
                (encode(f64::INFINITY), encode(f64::INFINITY)),
                (encode(f64::NEG_INFINITY), encode(f64::NEG_INFINITY)),
                (1, encode(if operation == "ceil" { 1.0 } else { 0.0 })),
                (
                    sign_mask | 1,
                    encode(if operation == "floor" { -1.0 } else { -0.0 }),
                ),
            ]);
            let mut checks = String::new();
            for (input, expected) in cases {
                checks.push_str(&format!(
                    "do {{var input:F=0.0;input'bitvec[0..{width}]={input};
                    let output=apply(input);let bits=output'bitvec;assumes bits == {expected};}};"
                ));
            }
            // NaN payload/sign are not constrained for rounding operations.
            for bits in if width == 32 {
                vec![0x7fc12345u64, 0x7f800001]
            } else {
                vec![0x7ff8123456789abcu64, 0x7ff0000000000001]
            } {
                checks.push_str(&format!(
                    "do {{var input:F=0.0;input'bitvec[0..{width}]={bits};
                    let output=apply(input);assumes output != output;}};"
                ));
            }
            let w = Work::new(&format!(
                "{RESOURCE_HEADER} typealias F=.f{width}'builtin;
                impl Duplicate for F {{}} impl Duplicate for .BitVec'builtin<{width}> {{}}
                ^intrinsic_enum enum Ops {{round}}
                fn apply(input:F)->F {{unsafe {{Ops.round'intrinsic(input).as(F)}}}}
                fn run()->bool {{{checks} true}}"
            ));
            let binding = format!("{operation}=input.Ops.round");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--host-intrinsic",
                            &binding,
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--build",
                            "run",
                            "--output",
                            "native program",
                            "--opt-level",
                            level,
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, b"(bool true)\n");
            }
        }
    }
}

#[test]
fn native_zero_sized_pointer_assignment_keeps_address_and_value_evaluation() {
    let source = format!(
        r#"{RESOURCE_HEADER}
        typealias Void=.void'builtin;
        impl Duplicate for Void {{}}
        fn empty()->Void {{}}
        fn locate(value:*Void, seen:*bool)->*Void {{
            let previous=seen.*; assumes not previous;
            seen.* = true;
            value
        }}
        fn produce(address_seen:*bool, value_seen:*bool)->Void {{
            let address=address_seen.*;
            let previous=value_seen.*;
            assumes address and not previous;
            value_seen.* = true;
        }}
        fn run()->bool {{
            var value=empty();
            var address_seen=false;
            var value_seen=false;
            locate(value.ref,address_seen.ref).* = produce(address_seen.ref,value_seen.ref);
            let nested=value.ref;
            nested.*;
            address_seen and value_seen
        }}
    "#
    );
    for source in [
        source.clone(),
        source.replace(
            "produce(address_seen.ref,value_seen.ref)",
            "do {return address_seen and not value_seen; produce(address_seen.ref,value_seen.ref)}",
        ),
    ] {
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn zero_sized_pointer_storage_still_requires_a_live_owner() {
    for body in [
        "var value=empty();value.ref",
        "do {var value=empty();value.ref}",
    ] {
        let w = Work::new(&format!(
            "{RESOURCE_HEADER} typealias Void=.void'builtin;impl Duplicate for Void {{}} fn empty()->Void {{}} fn leak()->*Void {{{body}}} fn run()->bool {{let pointer=leak();pointer.*;true}}"
        ));
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        assert!(!reference.status.success());
        let diagnostic = String::from_utf8_lossy(&reference.stderr);
        assert!(
            diagnostic.contains("local memory target cannot escape"),
            "{diagnostic}"
        );
        let native = resource_build(&w);
        assert!(!native.status.success());
        let diagnostic = String::from_utf8_lossy(&native.stderr);
        assert!(
            diagnostic.contains("local memory target cannot escape"),
            "{diagnostic}"
        );
        assert!(!w.0.join("native program").exists());
    }
}

#[test]
fn native_fixed_enum_empty_payloads_keep_evaluation_and_extraction() {
    for encoding in [
        "0b(empty:Empty) 10101010",
        "0b1010 (empty:Empty) 1010",
        "0b10101010 (empty:Empty)",
    ] {
        let source = format!(
            "{RESOURCE_HEADER}
            typealias Empty=.BitVec'builtin<0>;typealias Byte=.u8'builtin;
            enum Packet {{.width 8,.discrete_tag true,data:{encoding},}}
            impl Duplicate for Packet {{}}
            fn extract(packet:Packet)->Empty {{packet match {{.data {{empty}}=>empty}}}}
            fn run()->bool {{var seen=false;
                let packet=Packet.data {{empty:do {{seen=true;0}}}};
                seen and extract(packet) == 0 and (packet'bitvec).as(Byte) == 170}}"
        );
        for level in ["0", "3"] {
            let w = Work::new(&source);
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_fixed_enum_empty_payload_exit_skips_remaining_fields() {
    let source = format!(
        "{RESOURCE_HEADER}
        typealias Empty=.BitVec'builtin<0>;typealias Byte=.u8'builtin;
        enum Packet {{.width 8,.discrete_tag true,data:0b(empty:Empty) (rest:Byte),}}
        impl Duplicate for Packet {{}}
        fn build(seen:*bool)->bool {{
            let packet=Packet.data {{empty:do {{return true;0}},rest:do {{seen.* = true;7}}}};
            false
        }}
        fn run()->bool {{var seen=false;build(seen.ref) and not seen}}"
    );
    for level in ["0", "3"] {
        let w = Work::new(&source);
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let out = w.execute();
        success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_object_can_be_delivered_without_a_linker_and_linked_later() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-arithmetic.fl"
    ));
    let emitted = w.command().args(["--emit-llvm", "run"]).output().unwrap();
    success(&emitted);
    let ir = String::from_utf8(emitted.stdout).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let toolchain = luna::host::Toolchain {
            linker: w.0.join("linker-is-not-installed"),
            optimization,
            ..Default::default()
        };
        let object = luna::host::object(&ir, &toolchain).unwrap();
        assert!(!object.is_empty());
        let object_path = w.0.join("separate object.o");
        fs::write(&object_path, object).unwrap();
        let linked = Command::new(luna::host::Toolchain::default().linker)
            .arg(&object_path)
            .arg("-lm")
            .arg("-o")
            .arg(w.0.join("native program"))
            .output()
            .unwrap();
        success(&linked);
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(unsigned-integer 42)\n");
    }
}

#[test]
fn native_object_keeps_unresolved_symbols_for_the_link_step() {
    let w = Work::new("");
    let ir = "declare i32 @provided_later()\ndefine i32 @main() {\n %value = call i32 @provided_later()\n ret i32 %value\n}\n";
    let toolchain = luna::host::Toolchain::default();
    let object = luna::host::object(ir, &toolchain).unwrap();
    fs::write(w.0.join("module.o"), object).unwrap();
    fs::write(
        w.0.join("support.c"),
        "int provided_later(void) { return 0; }\n",
    )
    .unwrap();
    let link = |support: bool| {
        let mut command = Command::new(&toolchain.linker);
        command.current_dir(&w.0).arg("module.o");
        if support {
            command.arg("support.c");
        }
        command.args(["-o", "native program"]).output().unwrap()
    };
    assert!(!link(false).status.success());
    success(&link(true));
    success(&w.execute());
    assert!(luna::host::object("this is not LLVM IR", &toolchain).is_err());
    assert!(
        luna::host::object(
            "target datalayout = \"E-p:32:32\"\ndefine i32 @main() {ret i32 0}",
            &toolchain
        )
        .unwrap_err()
        .contains("mismatch")
    );
}

#[test]
fn emit_object_cli_supports_sources_and_explicit_packages() {
    for packages in [false, true] {
        let w = Work::new("fn run()->bool {true}");
        for level in ["0", "3"] {
            let mut command = if packages {
                let mut command = Command::new(env!("CARGO_BIN_EXE_luna"));
                command.current_dir(&w.0).args([
                    "--packages-v1",
                    "--package",
                    "7",
                    "901",
                    "app",
                    "source.fl",
                    "--primary",
                    "7",
                ]);
                command
            } else {
                w.command()
            };
            let out = command
                .args([
                    "--emit-object",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "separate object.o",
                ])
                .env("LUNA_CC", w.0.join("missing-linker"))
                .output()
                .unwrap();
            success(&out);
            assert!(out.stdout.is_empty());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(w.0.join("separate object.o"))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o111,
                    0
                );
            }
            let linked = Command::new(luna::host::Toolchain::default().linker)
                .current_dir(&w.0)
                .args(["separate object.o", "-lm", "-o", "native program"])
                .output()
                .unwrap();
            success(&linked);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn emit_object_cli_rejects_conflicts_and_preserves_existing_outputs() {
    let w = Work::new("fn run()->bool {true}");
    for args in [
        vec!["--emit-object", "run"],
        vec![
            "--emit-object",
            "run",
            "--build",
            "run",
            "--output",
            "module.o",
        ],
        vec![
            "--emit-object",
            "run",
            "--emit-llvm",
            "run",
            "--output",
            "module.o",
        ],
        vec![
            "--emit-object",
            "run",
            "--interpret",
            "run",
            "--output",
            "module.o",
        ],
    ] {
        let out = w.command().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(!w.0.join("module.o").exists());
    }
    fs::write(w.0.join("module.o"), b"previous object").unwrap();
    let out = w
        .command()
        .args(["--emit-object", "run", "--output", "module.o"])
        .env("LUNA_LLVM_CONFIG", w.0.join("missing-llvm"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(fs::read(w.0.join("module.o")).unwrap(), b"previous object");
    let original = fs::read(w.0.join("source.fl")).unwrap();
    fs::hard_link(w.0.join("source.fl"), w.0.join("alias.o")).unwrap();
    for output in ["source.fl", "alias.o"] {
        let out = w
            .command()
            .args(["--emit-object", "run", "--output", output])
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert_eq!(fs::read(w.0.join("source.fl")).unwrap(), original);
    }
}

#[test]
fn emit_object_package_output_protects_dependency_sources() {
    let w = Work::new("fn run()->bool {true}");
    fs::write(w.0.join("dependency.fl"), "fn other()->bool {false}").unwrap();
    let original = fs::read(w.0.join("dependency.fl")).unwrap();
    for output in ["dependency.fl", "module.o"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_luna"));
        command.current_dir(&w.0).args([
            "--packages-v1",
            "--package",
            "1",
            "701",
            "app",
            "source.fl",
            "--package",
            "2",
            "901",
            "dep",
            "dependency.fl",
            "--primary",
            "1",
            "--emit-object",
            "run",
            "--output",
            output,
        ]);
        if output == "module.o" {
            command.arg("--with-inputs");
        }
        assert!(!command.output().unwrap().status.success());
        assert_eq!(fs::read(w.0.join("dependency.fl")).unwrap(), original);
        assert!(!w.0.join("module.o").exists());
    }
}

#[test]
fn emit_object_preserves_intrinsic_bindings_and_checked_closure_cleanup() {
    for (source, flags, expected) in [
        (
            include_str!("../../../development/examples/host-intrinsic-fma.fl"),
            vec!["--host-intrinsic", "fma=input.machine.Scalar.fused"],
            b"(float32 (bits 41880000))\n".as_slice(),
        ),
        (
            include_str!("../../../development/examples/host-closure-cleanup.fl"),
            vec![
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--inline-functions",
            ],
            b"(bool true)\n".as_slice(),
        ),
    ] {
        let w = Work::new(source);
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--emit-object",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "module.o",
                ])
                .args(&flags)
                .env("LUNA_CC", w.0.join("missing-linker"))
                .output()
                .unwrap();
            success(&out);
            let linked = Command::new(luna::host::Toolchain::default().linker)
                .current_dir(&w.0)
                .args(["module.o", "-lm", "-o", "native program"])
                .output()
                .unwrap();
            success(&linked);
            let out = w.execute();
            success(&out);
            assert_eq!(out.stdout, expected);
        }
    }
}

#[test]
fn emit_object_packages_preserve_discarded_result_cleanup_and_failure() {
    for valid in [true, false] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}impl Duplicate for bool {{}}
            struct Watch {{valid:bool}}
            impl Retire for Watch {{fn drop(*self) {{assumes self.valid.*;self.valid.* = false;}}}}
            fn run(.valid:bool={valid})->Watch {{Watch {{valid:valid}}}}"
        );
        let w = Work::new(&source);
        for level in ["0", "3"] {
            let out = Command::new(env!("CARGO_BIN_EXE_luna"))
                .current_dir(&w.0)
                .args([
                    "--packages-v1",
                    "--package",
                    "4",
                    "901",
                    "app",
                    "source.fl",
                    "--primary",
                    "4",
                    "--emit-object",
                    "run",
                    "--discard-result",
                    "--inline-functions",
                    "--copy-protocol",
                    "app.Duplicate",
                    "--drop-protocol",
                    "app.Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "module.o",
                ])
                .env("LUNA_CC", w.0.join("missing-linker"))
                .output()
                .unwrap();
            success(&out);
            let linked = Command::new(luna::host::Toolchain::default().linker)
                .current_dir(&w.0)
                .args(["module.o", "-lm", "-o", "native program"])
                .output()
                .unwrap();
            success(&linked);
            let out = w.execute();
            assert_eq!(out.status.success(), valid);
            assert_eq!(
                out.stdout,
                if valid {
                    b"(unit)\n".as_slice()
                } else {
                    b"".as_slice()
                }
            );
        }
    }
}

#[test]
fn native_minimum_maximum_preserve_nan_and_signed_zero() {
    for operation in ["minimum", "maximum", "minimumnum", "maximumnum"] {
        for width in [32, 64] {
            let encode = |v: f64| {
                if width == 32 {
                    u64::from((v as f32).to_bits())
                } else {
                    v.to_bits()
                }
            };
            let mut checks = String::new();
            for (a, b, low, high) in [
                (0.0, -0.0, -0.0, 0.0),
                (-0.0, 0.0, -0.0, 0.0),
                (-3.0, 2.0, -3.0, 2.0),
                (2.0, -3.0, -3.0, 2.0),
                (
                    f64::NEG_INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::INFINITY,
                ),
            ] {
                let expected = encode(if matches!(operation, "minimum" | "minimumnum") {
                    low
                } else {
                    high
                });
                let a = encode(a);
                let b = encode(b);
                checks.push_str(&format!("do {{var a:F=0.0;var b:F=0.0;a'bitvec[0..{width}]={a};b'bitvec[0..{width}]={b};let output=apply(a,b);assumes output'bitvec == {expected};}};"));
            }
            for bits in if width == 32 {
                vec![0x7fc12345u64, 0x7f800001]
            } else {
                vec![0x7ff8123456789abcu64, 0x7ff0000000000001]
            } {
                let numeric = matches!(operation, "minimumnum" | "maximumnum");
                let expected = if numeric {
                    "assumes first == 1.0;assumes second == 1.0;"
                } else {
                    "assumes first != first;assumes second != second;"
                };
                checks.push_str(&format!("do {{var nan:F=0.0;nan'bitvec[0..{width}]={bits};let first=apply(nan,1.0);let second=apply(1.0,nan);{expected}let both=apply(nan,nan);assumes both != both;}};"));
            }
            let w = Work::new(&format!(
                "{RESOURCE_HEADER}typealias F=.f{width}'builtin;impl Duplicate for F {{}}impl Duplicate for .BitVec'builtin<{width}> {{}}^intrinsic_enum enum Ops {{bound}}fn apply(a:F,b:F)->F {{unsafe {{Ops.bound'intrinsic(a,b).as(F)}}}}fn run()->bool {{{checks}true}}"
            ));
            let binding = format!("{operation}=input.Ops.bound");
            for level in ["0", "3"] {
                let out = w
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--host-intrinsic",
                        &binding,
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap();
                success(&out);
                let out = w.execute();
                assert!(
                    out.status.success(),
                    "{operation} f{width} O{level}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                assert_eq!(out.stdout, b"(bool true)\n");
            }
        }
    }
}

#[test]
fn native_integer_math_wrappers_cover_widths_bits_and_once_only_inputs() {
    let mut body = String::new();
    for width in [8, 16, 32, 64, 128] {
        let mask = u128::MAX >> (128 - width);
        let values = [
            0,
            1,
            mask,
            1 << (width - 1),
            0x96 & mask,
            0x0123456789abcdef_fedcba9876543210 & mask,
        ];
        for value in values {
            let reversed = value.reverse_bits() >> (128 - width);
            if width >= 16 {
                // Independent byte-array oracle, distinct from LLVM's intrinsic.
                let bytes = value.to_le_bytes();
                let swapped = bytes[..(width / 8) as usize]
                    .iter()
                    .fold(0u128, |result, byte| (result << 8) | u128::from(*byte));
                body.push_str(&format!("do {{var seen=false;let value:.u{width}'builtin={value};let swapped=math.byte_swap(do {{assumes not seen;seen=true;value}});assumes seen and swapped == {swapped};let recovered = math.byte_swap(swapped);assumes recovered == value;}};"));
            }
            let count = value.count_ones();
            let leading = value.leading_zeros() - (128 - width);
            let trailing = value.trailing_zeros().min(width);
            body.push_str(&format!("do {{
                var seen = false;
                let reversed = math.bit_reverse(do {{ assumes not seen; seen = true; let value:.u{width}'builtin={value}; value }});
                assumes seen and reversed == {reversed};
                seen = false;
                let counted = math.count_ones<.u{width}'builtin>(do {{ assumes not seen; seen = true; let value:.u{width}'builtin={value}; value }});
                assumes seen and counted == {count};
                seen = false;
                let leading = math.leading_zeros(do {{ assumes not seen; seen = true; let value:.u{width}'builtin={value}; value }});
                assumes seen and leading == {leading};
                seen = false;
                let trailing = math.trailing_zeros<.u{width}'builtin>(do {{ assumes not seen; seen = true; let value:.u{width}'builtin={value}; value }});
                assumes seen and trailing == {trailing};
            }};"));
        }
    }
    let source = format!(
        "mod math {{ {} }}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        impl Duplicate for bool {{}}
        impl Duplicate for .u8'builtin {{}} impl Duplicate for .u16'builtin {{}}
        impl Duplicate for .u32'builtin {{}} impl Duplicate for .u64'builtin {{}}
        impl Duplicate for .u128'builtin {{}}
        fn run()->bool {{{body} true}}",
        include!("support/math_source.rs")
    );
    let w = Work::new(&source);
    let mut dump = w.command();
    dump.args([
        "--emit-llvm",
        "run",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
        "--fuel",
        "1000000",
    ]);
    for operation in nir::intrinsics::Operation::ALL {
        let name = operation.name();
        dump.args([
            "--host-intrinsic",
            &format!("{name}=input.math.target.Scalar.{name}"),
        ]);
    }
    let dumped = dump.output().unwrap();
    success(&dumped);
    let ir = String::from_utf8_lossy(&dumped.stdout);
    for width in [16, 32, 64, 128] {
        assert!(ir.contains(&format!("@llvm.bswap.i{width}(")));
    }
    assert!(!ir.contains("@llvm.bswap.i8("));
    for level in ["0", "3"] {
        let mut command = w.command();
        command.args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--opt-level",
            level,
            "--output",
            "native program",
            "--fuel",
            "1000000",
        ]);
        for operation in nir::intrinsics::Operation::ALL {
            let name = operation.name();
            command.args([
                "--host-intrinsic",
                &format!("{name}=input.math.target.Scalar.{name}"),
            ]);
        }
        success(&command.output().unwrap());
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_direct_passing_cross_function_calls_preserve_caller_arguments() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-passing-calls.fl"
    ));
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let build = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        success(&build);
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_passing_routes_branches_and_overrides() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        ^attrs.effects.direct_passing effect enabled()->bool;
        ^attrs.effects.direct_passing effect ready()->bool;
        fn read(value:bool)->#ready bool {value and ready()#}
        fn other(value:bool)->#ready bool {value or ready()#}
        fn branch(flag:bool)->#[enabled,ready] bool {
            if flag and (enabled()#) {read(true)} else {other(true)}
        }
        fn matched(flag:bool)->#[enabled,ready] bool {
            flag match {true=>read(enabled()#),false=>other(enabled()#),}
        }
        fn reversed()->#[ready,enabled] bool {enabled()# and not ready()#}
        fn overridden()->#[enabled,ready] bool {
            let before = ready()#;
            let inside = read(enabled()#)# {ready()=>enabled()#,};
            not before and inside and not ready()#
        }
        fn subject()->#[enabled,ready] bool {
            let c = branch(true);
            let d = branch(false);
            let e = matched(true);
            let f = matched(false);
            not c and d and not e and f and reversed() and overridden()
        }
        fn run()->bool {subject()# {enabled()=>true,ready()=>false,}}";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_passing_indirect_abi_with_resource_admission() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        ^attrs.effects.direct_passing effect enabled()->bool;
        ^attrs.effects.direct_passing effect ready()->bool;
        fn read(value:bool)->#ready bool {value and ready()#}
        fn other(value:bool)->#ready bool {value or ready()#}
        fn indirect(flag:bool)->#[enabled,ready] bool {
            let target = if flag {read} else {other}; target(enabled()#)
        }
        fn run()->bool {
            let a = indirect(true)# {enabled()=>true,ready()=>false,};
            let b = indirect(false)# {enabled()=>true,ready()=>false,};
            not a and b
        }";
    let w = Work::new(source);
    let reference = w.command().args(["--interpret", "run"]).output().unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
    let checked_reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&checked_reference);
    assert_eq!(checked_reference.stdout, reference.stdout);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_passing_calls_keep_owned_parameter_cleanup() {
    let w = Work::new(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        ^attrs.effects.direct_passing effect ready()->bool;
        fn release(value:Watch)->#ready bool {ready()#}
        fn run()->bool {
            let seen = false; let target = seen.ref;
            let result = release(Watch {target:target})# {ready()=>true,};
            result and seen
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_passing_machine_parameters_keep_wide_float_and_void_layouts() {
    let w = Work::new(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias U=.u128'builtin; typealias F=.f64'builtin;
        impl Duplicate for bool {} impl Duplicate for U {} impl Duplicate for F {}
        ^attrs.effects.direct_passing effect signal();
        ^attrs.effects.direct_passing effect wide()->U;
        ^attrs.effects.direct_passing effect real()->F;
        fn values(value:U)->#[signal,wide,real] bool {
            signal()#; signal()#;
            value == (wide()#) and (real()#) == 3.5
        }
        fn run()->bool {
            var installed = false;
            let result = values(170141183460469231731687303715884105729)# {
                signal()=>do {assumes not installed; installed = true;},
                wide()=>170141183460469231731687303715884105729,
                real()=>3.5,
            };
            result and installed
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    let ir = w
        .command()
        .args([
            "--emit-llvm",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&ir);
    let text = String::from_utf8_lossy(&ir.stdout);
    assert!(
        text.contains("i128 %arg0, i128 %effect1, double %effect2"),
        "{text}"
    );
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_passing_closure_environment_and_parameters_remain_distinct() {
    let w = Work::new(
        "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        ^attrs.effects.direct_passing effect ready()->bool;
        typealias Action=%{flag:bool} fn(bool)->#ready bool;
        fn make(flag:bool)->Action {|value| flag and value and ready()#}
        fn apply(action:Action,value:bool)->#ready bool {action(value)}
        fn run()->bool {
            let action = make(true);
            let a = action(true)# {ready()=>true,};
            let b = action(false)# {ready()=>true,};
            let c = apply(action,true)# {ready()=>false,};
            let other = make(false);
            let d = apply(other,true)# {ready()=>true,};
            a and not b and not c and not d
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_passing_generic_operation_instances_keep_abi_identity() {
    let w = Work::new(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias U=.u128'builtin;
        impl Duplicate for bool {} impl Duplicate for U {}
        ^attrs.effects.direct_passing effect setting()->T where T;
        fn fetch()->#setting<T> T where T {setting<T>()#}
        fn combine()->#[setting<bool>,setting<U>] bool {
            let flag:bool = fetch<bool>();
            let value:U = fetch<U>();
            flag and value == 170141183460469231731687303715884105729
        }
        fn run()->bool {combine()# {
            setting<bool>()=>true,
            setting<U>()=>170141183460469231731687303715884105729,
        }}",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_passing_owned_closure_cleans_invoked_and_uninvoked_captures() {
    let w = Work::new("trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        ^attrs.effects.direct_passing effect ready()->bool;
        typealias Action=%{watch:Watch} fn()->#ready bool;
        fn consume(value:Watch) {}
        fn make(target:*bool)->Action {let watch=Watch {target:target}; || {consume(watch);ready()#}}
        fn invoke(action:Action)->#ready bool {action()}
        fn scope(called:bool)->bool {
            let seen=false; let target=seen.ref;
            do {
                let action=make(target);
                if called {let result=invoke(action)# {ready()=>true,}; assumes result;}
            };
            seen
        }
        fn run()->bool {scope(true) and scope(false)}");
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
    let source = fs::read_to_string(w.0.join("source.fl")).unwrap();
    let twice = source.replace(
        "fn invoke(action:Action)->#ready bool {action()}",
        "fn invoke(action:Action)->#ready bool {let first:bool=action();action()}",
    );
    assert_ne!(source, twice);
    fs::write(w.0.join("source.fl"), twice).unwrap();
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let rejected = w
        .command()
        .args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    let error = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        error.contains("cannot prepare resource-checked") && error.contains("unavailable"),
        "{error}"
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");

    // Both ABIs allow the closed invocation, with the same payload cleanup.
    // Neither permits invoking the consumed affine environment a second time.
    let ordinary = source.replace("^attrs.effects.direct_passing effect ready", "effect ready");
    fs::write(w.0.join("source.fl"), &ordinary).unwrap();
    let accepted = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&accepted);
    assert_eq!(accepted.stdout, reference.stdout);
    let twice = ordinary.replace(
        "fn invoke(action:Action)->#ready bool {action()}",
        "fn invoke(action:Action)->#ready bool {let first:bool=action();action()}",
    );
    assert_ne!(ordinary, twice);
    fs::write(w.0.join("source.fl"), twice).unwrap();
    let rejected = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    let error = String::from_utf8_lossy(&rejected.stderr);
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn native_unused_direct_passing_row_retains_declared_operation_abi() {
    native(
        "^attrs.effects.direct_passing effect setting()->bool;
        fn advertised()->#setting bool {true}
        fn run()->bool {let action=advertised; true}",
        "(bool true)\n",
    );
}

#[test]
fn native_direct_passing_copy_aggregate_crosses_calls_and_overrides() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for .u128'builtin {}
        struct Config {enabled:bool, mask:.u128'builtin}
        impl Duplicate for Config {}
        ^attrs.effects.direct_passing effect config()->Config;
        fn read()->#config bool {
            let first=config()#; let second=config()#;
            first.enabled and second.mask==340282366920938463463374607431768211455
        }
        fn forward()->#config bool {read()}
        fn nested()->#config bool {
            let outer=forward();
            let inner=forward()# {config()=>Config {enabled:false,mask:0},};
            outer and not inner and forward()
        }
        fn run()->bool {
            var installed=false;
            let answer=nested()# {config()=>{assumes not installed;installed=true;
                Config {enabled:true,mask:340282366920938463463374607431768211455}},};
            answer and installed
        }";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
    // Identical machine layout does not provide Copy permission.
    let affine = source.replace("impl Duplicate for Config {}", "");
    fs::write(w.0.join("source.fl"), affine).unwrap();
    let rejected = w
        .command()
        .args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("Copy permission"),
        "{rejected:?}"
    );
    let old = w.execute();
    success(&old);
    assert_eq!(old.stdout, reference.stdout);
}

#[test]
fn native_direct_passing_copy_enum_preserves_variants_and_payloads() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for .u128'builtin {}
        enum Choice {none,some(bool,.u128'builtin),}
        impl Duplicate for Choice {}
        ^attrs.effects.direct_passing effect choice()->Choice;
        fn inspect()->#choice bool {
            let first=choice()#;let second=choice()#;
            first match {.some(flag,value)=>flag and value==340282366920938463463374607431768211455 and
                (second match {.some(other,word)=>other and word==value,.none=>false,}),.none=>false,}
        }
        fn run()->bool {
            let yes=inspect()# {choice()=>Choice.some(true,340282366920938463463374607431768211455),};
            let no=inspect()# {choice()=>Choice.none,};
            yes and not no
        }";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_passing_copy_newtypes_preserve_nominal_delivery() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for .u128'builtin {}
        struct Pair {flag:bool,word:.u128'builtin}
        impl Duplicate for Pair {}
        newtype Wrapped<T> = T;
        impl Duplicate for Wrapped<Pair> {}
        newtype Outer = Wrapped<Pair>;
        impl Duplicate for Outer {}
        ^attrs.effects.direct_passing effect config()->Outer;
        fn read()->#config bool {
            let first=config()#.as(Wrapped<Pair>).as(Pair);
            let second=config()#.as(Wrapped<Pair>).as(Pair);
            first.flag and second.word==340282366920938463463374607431768211455
        }
        fn forward()->#config bool {read()}
        fn run()->bool {
            forward()# {config()=>Pair {flag:true,word:340282366920938463463374607431768211455}.as(Wrapped<Pair>).as(Outer),}
        }";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
    fs::write(
        w.0.join("source.fl"),
        source.replace("impl Duplicate for Outer {}", ""),
    )
    .unwrap();
    let rejected = w
        .command()
        .args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("Copy permission"),
        "{rejected:?}"
    );
    let output = w.execute();
    success(&output);
    assert_eq!(output.stdout, reference.stdout);
}

#[test]
fn native_direct_passing_generic_aggregates_keep_identity_through_higher_order_calls() {
    let source = include_str!("../../../development/examples/host-passing-aggregates.fl");
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_passing_error_results_preserve_success_and_error_payloads() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for .u128'builtin {}
        enum Failure {bad(.u128'builtin),}
        impl Duplicate for Failure {}
        impl Duplicate for !Failure bool {}
        ^attrs.effects.direct_passing effect answer()->!Failure bool;
        fn read()->#answer bool {
            let first=answer()#;let second=answer()#;
            let a=first! {Failure.bad(word)=>word==340282366920938463463374607431768211455,};
            let b=second! {Failure.bad(word)=>word==340282366920938463463374607431768211455,};
            a and b
        }
        fn forward()->#answer bool {read()}
        fn run()->bool {
            var installed=false;
            let failed=forward()# {answer()=>{assumes not installed;installed=true;
                error Failure.bad(340282366920938463463374607431768211455)},};
            let success=forward()# {answer()=>true,};
            let no=forward()# {answer()=>false,};
            failed and success and not no and installed
        }";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
    fs::write(
        w.0.join("source.fl"),
        source.replace("impl Duplicate for !Failure bool {}", ""),
    )
    .unwrap();
    let rejected = w
        .command()
        .args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("Copy permission"),
        "{rejected:?}"
    );
    let old = w.execute();
    success(&old);
    assert_eq!(old.stdout, reference.stdout);
}

#[test]
fn native_cross_function_direct_handlers_outline_scalar_captures() {
    native(
        "effect read()->bool;fn subject()->#read bool {read()#}fn relay()->#read bool {subject()}fn run()->bool {let captured=true;relay()# {read()=>captured,}}",
        "(bool true)\n",
    );
}

#[test]
fn native_cross_function_direct_handlers_preserve_outer_entries_and_fallback() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        effect read()->bool;
        ^attrs.effects.direct_passing effect bias()->bool;
        fn subject()->#read bool {read()#}
        fn inner()->#[read,bias] bool {subject()# {read() if false=>false,}}
        fn via_outer()->#[read,bias] bool {subject()# {read()=>read()# and bias()#,}}
        fn run()->bool {
            let a=inner()# {read()=>true,bias()=>true,};
            let b=via_outer()# {read()=>true,bias()=>true,};
            a and b
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let result = w.execute();
        success(&result);
        assert_eq!(result.stdout, reference.stdout);
    }
}

#[test]
fn native_cross_function_direct_handlers_share_captures_and_clean_each_resume() {
    let w = Work::new(include_str!(
        "../../../development/examples/host-direct-calls.fl"
    ));
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let result = w.execute();
        success(&result);
        assert_eq!(result.stdout, reference.stdout);
    }
}

#[test]
fn native_cross_function_direct_handlers_capture_write_only_places() {
    native(
        "typealias Void=.void'builtin;effect signal();fn subject()->#signal Void {signal()#}fn run()->bool {var seen=false;subject()# {signal()=>do {seen=true;},};seen}",
        "(bool true)\n",
    );
}

#[test]
fn native_cross_function_direct_handlers_preserve_void_and_wide_arguments() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias U=.u128'builtin;typealias Void=.void'builtin;
        impl Duplicate for U {}
        effect change(ignored:Void,value:U)->U;
        fn unit() {}
        fn subject(value:U)->#change U {change(unit(),value)#}
        fn run()->bool {
            let value:U=36893488147419103232;
            let result=subject(value)# {change(_,input)=>input,};
            result==value
        }",
    );
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_cross_function_direct_handler_local_loops_clean_break_and_continue() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        effect read()->bool;
        fn subject()->#read bool {read()#}
        fn run()->bool {
            let seen=false;let target=seen.ref;
            let result=subject()# {read()=>do {
                var again=true;
                while true {
                    let owner=Watch {target:target};
                    if again {again=false;continue;}
                    break;
                }
                resume not again;
            },};
            result and not seen
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_cross_function_direct_handler_closure_return_keeps_its_own_boundary() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        effect read()->bool;
        fn subject()->#read bool {read()#}
        fn run()->bool {
            let flag=true;
            let result=subject()# {read()=>do {
                let local=|| {return flag;};
                let first=local();
                resume not first;
            },};
            not result
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_cross_function_direct_handlers_require_checked_loop_targets() {
    for control in ["break", "continue"] {
        let source = format!(
            "effect read()->bool;fn subject()->#read bool {{read()#}}
            fn run()->bool {{
                var active=true;
                while active {{
                    active=false;
                    subject()# {{read()=>do {{while false {{}} {control};}},}};
                }}
                true
            }}"
        );
        let w = Work::new(&source);
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let result = w.build();
        assert!(!result.status.success());
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(
            error.contains("host outlined direct loop exit requires checked target evidence"),
            "{error}"
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_cross_function_direct_handlers_borrow_complete_aggregates() {
    for (declarations, initialization, handler, result) in [
        (
            "struct State {first:bool,seen:bool}impl Duplicate for State {}",
            "State {first:true,seen:false}",
            "let local=|| {return state.first;};let first=local();state.first=false;state.seen=true;first",
            "state.seen and not state.first",
        ),
        (
            "enum State {off,on(bool)}impl Duplicate for State {}",
            "State.off",
            "let first=state match {.off=>true,.on(flag)=>not flag,};state=State.on(true);first",
            "state match {.off=>false,.on(flag)=>flag,}",
        ),
        (
            "newtype State=bool;impl Duplicate for State {}",
            "true.as(State)",
            "let first=state.as(bool);state=false.as(State);first",
            "not state.as(bool)",
        ),
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}impl Duplicate for bool {{}}
            {declarations}
            effect read()->bool;
            fn subject()->#read bool {{let first=read()#;let second=read()#;first and not second}}
            fn run()->bool {{
                var state={initialization};
                let result=subject()# {{read()=>do {{{handler}}},}};
                result and ({result})
            }}"
        );
        for copy in [true, false] {
            let source = if copy {
                source.clone()
            } else {
                source.replace("impl Duplicate for State {}", "")
            };
            let w = Work::new(&source);
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn native_cross_function_direct_handlers_keep_aggregate_pointer_provenance() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        struct State {target:*bool,next:bool}impl Duplicate for State {}
        effect touch()->bool;
        fn twice()->#touch bool {let first=touch()#;let second=touch()#;not first and second}
        fn run()->bool {
            var flag=false;var state=State {target:flag.ref,next:false};
            let result=twice()# {touch()=>do {
                state.target.* = state.next;state.next=not state.next;state.target.*
            },};
            result and flag and not state.next
        }";
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
    let invalid = source.replace("state.next=not state.next;", "state.target=state.target;");
    fs::write(w.0.join("source.fl"), invalid).unwrap();
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let rejected = w
        .command()
        .args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    let error = String::from_utf8_lossy(&rejected.stderr);
    assert!(error.contains("pointer slot rebinding"), "{error}");
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn native_cross_function_direct_return_unwinds_callers() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        effect read()->bool;
        fn subject(target:*bool)->#read bool {let owner=Watch {target:target};read()#}
        fn relay(target:*bool)->#read bool {let owner=Watch {target:target};subject(target)}
        fn run()->bool {
            let seen=false;
            relay(seen.ref)# {read()=>do {return true;},};
            false
        }";
    for source in [
        source.to_string(),
        source.replace(";subject(target)", ";let callback=subject;callback(target)"),
    ] {
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let built = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&built);
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_cross_function_direct_return_preserves_destination_type_and_cleanup_order() {
    let w=Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        typealias U=.u128'builtin;
        struct Watch {target:*bool,previous:*bool}
        impl Retire for Watch {fn drop(*self) {
            let previous=self.previous.*.*;let already=self.target.*.*;assumes previous;assumes not already;self.target.*.* = true;
        }}
        effect read()->U;
        fn subject(second:*bool,first:*bool)->#read U {
            let owner=Watch {target:second,previous:first};read()#
        }
        fn relay(third:*bool,second:*bool,first:*bool)->#read U {
            let owner=Watch {target:third,previous:second};subject(second,first)
        }
        fn install(seed:*bool,first:*bool,second:*bool,third:*bool,last:*bool)->bool {
            let owner=Watch {target:last,previous:third};
            relay(third,second,first)# {read()=>do {
                let handler_owner=Watch {target:first,previous:seed};return true;
            },};
            false
        }
        fn run()->bool {
            let seed=true;let first=false;let second=false;let third=false;let last=false;
            let result=install(seed.ref,first.ref,second.ref,third.ref,last.ref);
            result and first and second and third and last
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_cross_function_direct_return_checks_only_destination_postconditions() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        effect read()->bool;
        fn subject()->result:#read bool where ensures result {let value=read()#;assumes value;value}
        fn install(flag:bool)->result:bool where ensures result==flag {
            subject()# {read()=>do {return flag;},};flag
        }
        fn run()->bool {not install(false) and install(true)}",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_cross_function_direct_returns_select_recursive_installation_instances() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        effect read()->bool;
        fn subject()->#read bool {read()#}
        fn install(outer:bool)->bool {
            if outer {let inner=install(false);assumes inner;}
            subject()# {read()=>do {return not outer;},};false
        }
        fn run()->bool {not install(true)}",
    );
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_cross_function_direct_return_transfers_owned_result_to_caller() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}
        struct Watch {valid:bool}
        impl Retire for Watch {fn drop(*self) {assumes self.valid.*;}}
        effect read()->bool;
        fn subject()->#read bool {read()#}
        fn make(flag:bool)->Watch {
            subject()# {read()=>do {return Watch {valid:flag};},};
            Watch {valid:flag}
        }
        fn run()->bool {do {let owner=make(true);};true}",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    let source = fs::read_to_string(w.0.join("source.fl")).unwrap();
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
    fs::write(
        w.0.join("source.fl"),
        source.replace("make(true)", "make(false)"),
    )
    .unwrap();
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        assert!(
            !w.execute().status.success(),
            "returned owner destructor must run"
        );
    }
}

#[test]
fn native_cross_function_direct_return_crosses_nested_handler_entries() {
    for inner in ["second()", "inner()#"] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}impl Duplicate for bool {{}}
            effect outer()->bool;effect inner()->bool;
            fn first()->#outer bool {{outer()#}}
            fn second()->#inner bool {{inner()#}}
            fn run()->bool {{
                first()# {{outer()=>do {{
                    {inner}# {{inner()=>do {{return true;}},}};
                    false
                }},}};
                false
            }}"
        );
        // A direct request is already a computation, without a second '#'.
        let source = source.replace("inner()##", "inner()#");
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_cross_function_direct_loop_exits_clean_each_iteration() {
    let w=Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias U=.u32'builtin;
        impl Duplicate for bool {}impl Duplicate for U {}impl Duplicate for *T where T {}
        struct Watch {count:*U}
        impl Retire for Watch {fn drop(*self) {
            let previous=self.count.*.*;assumes previous < 100;self.count.*.* = previous + 1;
        }}
        effect read()->bool;
        fn subject(count:*U)->#read bool {while true {let owner=Watch {count:count};return read()#;}}
        fn relay(count:*U)->#read bool {subject(count)}
        fn run()->bool {
            var again=true;let count:U=0;let pointer=count.ref;
            let outside_owner=Watch {count:pointer};
            while true {
                let owner=Watch {count:pointer};
                relay(pointer)# {read()=>do {
                    let handler_owner=Watch {count:pointer};
                    if again {again=false;continue;}
                    break;
                },};
                return false;
            }
            not again and count == 6
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_cross_function_direct_loop_exits_target_an_enclosing_handler_loop() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        effect read()->bool;effect step()->bool;
        fn first()->#read bool {read()#}
        fn second()->#step bool {step()#}
        fn run()->bool {
            let result=first()# {read()=>do {
                var again=true;
                while true {
                    second()# {step()=>do {
                        if again {again=false;continue;}
                        break;
                    },};
                    return false;
                }
                resume not again;
            },};
            result
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_cross_function_direct_loop_exits_unwind_lexical_handler_frames() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias U=.u32'builtin;
        impl Duplicate for bool {}impl Duplicate for U {}impl Duplicate for *T where T {}
        struct Watch {count:*U}
        impl Retire for Watch {fn drop(*self) {
            let previous=self.count.*.*;assumes previous < 100;self.count.*.* = previous + 1;
        }}
        effect read()->bool;
        fn subject(count:*U)->#read bool {let owner=Watch {count:count};read()#}
        fn run()->bool {
            var again=true;let count:U=0;let pointer=count.ref;
            while true {
                let owner=Watch {count:pointer};
                read()# {read()=>do {
                    let lexical_owner=Watch {count:pointer};
                    subject(pointer)# {read()=>do {
                        let machine_owner=Watch {count:pointer};
                        if again {again=false;continue;}
                        break;
                    },};
                    return false;
                },};
                return false;
            }
            not again and count == 8
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_control_during_arguments_cleans_prepared_values_and_skips_later_work() {
    for (control, expected) in [
        ("return true;", 1),
        ("break;", 1),
        ("if again {again=false;continue;}break;", 2),
    ] {
        let source=format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            typealias U=.u32'builtin;
            impl Duplicate for bool {{}}impl Duplicate for U {{}}impl Duplicate for *T where T {{}}
            struct Watch {{count:*U}}
            impl Retire for Watch {{fn drop(*self) {{
                let previous=self.count.*.*;assumes previous < 100;self.count.*.* = previous + 1;
            }}}}
            effect read()->bool;
            fn subject()->#read bool {{read()#}}
            fn consume(owner:Watch,value:bool,last:bool,entered:*bool)->bool {{entered.* = true;value and last}}
            fn install(count:*U,late:*bool,entered:*bool)->bool {{
                var again=true;
                while true {{
                    consume(Watch {{count:count}},subject()# {{read()=>do {{{control}}},}},do {{late.* = true;true}},entered);
                    return false;
                }}
                true
            }}
            fn run()->bool {{
                let count:U=0;let late=false;let entered=false;
                let result=install(count.ref,late.ref,entered.ref);
                result and count == {expected} and not late and not entered
            }}"
        );
        let named = source
            .replace(
                "fn consume(owner:Watch,value:bool,last:bool,entered:*bool)",
                "fn consume(.last:bool=false,owner:Watch,.value:bool=false,entered:*bool)",
            )
            .replace(
                "consume(Watch {count:count},subject()#",
                "consume(Watch {count:count},.value=subject()#",
            )
            .replace(
                "},},do {late.* = true;true},entered)",
                "},},.last=do {late.* = true;true},entered)",
            );
        assert_ne!(named, source);
        for source in [&source, &named] {
            let w = Work::new(source);
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn native_direct_control_during_arguments_releases_uninvoked_owned_callee() {
    for (control, expected) in [
        ("return true;", 1),
        ("if again {again=false;continue;}break;", 2),
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            typealias U=.u32'builtin;
            impl Duplicate for bool {{}}impl Duplicate for U {{}}impl Duplicate for *T where T {{}}
            struct Watch {{count:*U}}
            impl Retire for Watch {{fn drop(*self) {{
                let previous=self.count.*.*;assumes previous < 100;self.count.*.* = previous + 1;
            }}}}
            effect read()->bool;
            fn subject()->#read bool {{read()#}}
            typealias Action=%{{watch:Watch,entered:*bool}} fn(bool)->bool;
            fn consume(owner:Watch) {{}}
            fn make(count:*U,entered:*bool)->Action {{
                let watch=Watch {{count:count}};
                |value| {{consume(watch);entered.* = true;value}}
            }}
            fn install(count:*U,entered:*bool)->bool {{
                var again=true;
                while true {{
                    make(count,entered)(subject()# {{read()=>do {{{control}}},}});
                    return false;
                }}
                true
            }}
            fn run()->bool {{
                let count:U=0;let entered=false;
                let result=install(count.ref,entered.ref);
                result and count == {expected} and not entered
            }}"
        );
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_effect_rows_do_not_authorize_effectful_defaults_even_when_overridden() {
    for call in ["request()", "request(.value=true)"] {
        let w = Work::new(&format!(
            "effect read()->bool;
            fn request(.value:bool=read()#)->#read bool {{value}}
            fn run()->bool {{{call}# {{read()=>true,}}}}"
        ));
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let rejected = w.build();
        assert!(
            !rejected.status.success(),
            "a function body row must not authorize its default"
        );
        let error = String::from_utf8_lossy(&rejected.stderr);
        assert!(
            error.contains("effect request requires an explicit allowed effect row"),
            "{error}"
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_direct_return_destination_belongs_to_the_installing_closure() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        effect read()->bool;
        fn subject()->#read bool {read()#}
        fn run()->bool {
            let flag=false;
            let action=|| {
                subject()# {read()=>do {return flag;},};true
            };
            not action()
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_loop_destination_belongs_to_the_installing_closure() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        effect read()->bool;
        fn subject()->#read bool {read()#}
        fn run()->bool {
            let flag=false;
            let action=|| {
                var again=true;
                while true {
                    subject()# {read()=>do {
                        if again {again=false;continue;}
                        break;
                    },};
                    return true;
                }
                again or flag
            };
            not action()
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_nonlocal_error_return_keeps_error_result_abi() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        enum Failure {bad(bool)} impl Duplicate for Failure {}
        effect read()->bool;
        fn subject()->#read bool {read()#}
        fn relay()->#read bool {subject()}
        fn install(failed:bool)->!Failure bool {
            relay()# {read()=>do {
                if failed {return error Failure.bad(false);}
                return true;
            },};false
        }
        fn recover(failed:bool)->bool {install(failed)! {Failure.bad(value)=>value,}}
        fn run()->bool {not recover(true) and recover(false)}",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_discarded_error_value_drops_owned_payload_without_never_storage() {
    for discarded in [
        "error Failure.bad(Watch {seen:seen.ref});",
        "let pending=error Failure.bad(Watch {seen:seen.ref}); pending;",
        "record {failure:error Failure.bad(Watch {seen:seen.ref}), flag:true};",
    ] {
        let source = "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        impl Duplicate for *T where T {}
        struct Watch {seen:*bool}
        impl Retire for Watch {fn drop(*self) {
            let previous=self.seen.*.*;
            assumes not previous;
            self.seen.*.* = true;
        }}
        fn record(...fields:T)->T where T {fields}
        enum Failure {bad(Watch)}
        fn run()->bool {
            var seen=false;
            DISCARDED_ERROR
            seen
        }"
        .replace("DISCARDED_ERROR", discarded);
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_cross_function_direct_error_return_transfers_owned_payload_through_embedding() {
    for consumer in [
        "let result=forward(VALID, FAILED); true",
        "let result=forward(VALID, FAILED)! {Failure.bad(owner)=>false, Other.bad=>false,}; result == not FAILED",
    ] {
        for failed in [false, true] {
            for valid in [false, true] {
                let source = "trait Duplicate {}trait Retire {fn drop(*self);}
                    impl Duplicate for bool {}
                    struct Watch {valid:bool}
                    impl Retire for Watch {fn drop(*self) {assumes self.valid.*;}}
                    enum Failure {bad(Watch)} enum Other {bad}
                    effect read()->bool;
                    fn subject()->#read bool {read()#}
                    fn make(valid:bool,failed:bool)->!Failure bool {
                        subject()# {read()=>do {
                            if failed {return error Failure.bad(Watch {valid:valid});}
                            return true;
                        },};false
                    }
                    fn forward(valid:bool,failed:bool)->![Other,Failure] bool {make(valid,failed)!}
                    fn run()->bool {CONSUMER}"
                    .replace("CONSUMER", consumer)
                    .replace("VALID", if valid { "true" } else { "false" })
                    .replace("FAILED", if failed { "true" } else { "false" });
                let w = Work::new(&source);
                let reference = w
                    .command()
                    .args([
                        "--interpret",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                    ])
                    .output()
                    .unwrap();
                let passes = valid || !failed;
                assert_eq!(
                    reference.status.success(),
                    passes,
                    "{source}\n{}",
                    String::from_utf8_lossy(&reference.stderr)
                );
                if passes {
                    assert_eq!(reference.stdout, b"(bool true)\n");
                }
                for level in ["0", "3"] {
                    success(
                        &w.command()
                            .args([
                                "--build",
                                "run",
                                "--copy-protocol",
                                "Duplicate",
                                "--drop-protocol",
                                "Retire",
                                "--opt-level",
                                level,
                                "--output",
                                "native program",
                            ])
                            .output()
                            .unwrap(),
                    );
                    let actual = w.execute();
                    assert_eq!(
                        actual.status.success(),
                        passes,
                        "returned error payload destructor must run only on the error path: {source}"
                    );
                    if passes {
                        assert_eq!(actual.stdout, reference.stdout);
                    }
                }
            }
        }
    }
}

#[test]
fn native_direct_handler_borrows_caller_parameter_storage() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        impl Duplicate for *T where T {}
        effect read()->bool;
        fn subject()->#read bool {read()#}
        fn update(pointer:*bool)->bool {
            subject()# {read()=>do {pointer.* = true;true},}
        }
        fn run()->bool {var seen=false;let result=update(seen.ref);result and seen}",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_handler_returns_installer_pointer_provenance() {
    let w = Work::new(include_str!(
        "../../../development/repros/host-handler-borrowed-return.fl"
    ));
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_handler_returns_owned_pointer_payload() {
    let w=Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        impl Duplicate for *T where T {}
        struct Watch {seen:*bool}
        impl Retire for Watch {fn drop(*self) {let previous=self.seen.*.*;assumes not previous;self.seen.*.* = true;}}
        effect read()->bool;fn subject()->#read bool {read()#}
        fn make(pointer:*bool)->Watch {
            subject()# {read()=>do {return Watch {seen:pointer};},};Watch {seen:pointer}
        }
        fn run()->bool {var seen=false;do {let owner=make(seen.ref);};seen}",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_handler_pointer_return_rejects_retiring_targets() {
    for body in [
        "var local=false;let borrowed=local.ref;subject()# {read()=>do {return borrowed;},};pointer",
        "subject()# {read()=>do {var local=false;return local.ref;},};pointer",
        "var local=false;let borrowed=local.ref;subject()# {read()=>do {if flag {return pointer;}return borrowed;},};pointer",
    ] {
        let source=format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
            effect read()->bool;fn subject()->#read bool {{read()#}}
            fn invalid(pointer:*bool,flag:bool)->*bool {{{body}}}
            fn run()->bool {{var value=false;let returned=invalid(value.ref,true);returned.*}}");
        let w = Work::new(&source);
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let rejected = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        assert!(!rejected.status.success(), "{source}");
        let error = String::from_utf8_lossy(&rejected.stderr);
        assert!(
            error.contains("escaping provenance")
                || error.contains("local memory target cannot escape"),
            "{error}"
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_direct_handler_returns_joined_pointer_provenance() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        impl Duplicate for *T where T {}
        effect read()->bool;fn subject()->#read bool {read()#}
        fn choose(left:*bool,right:*bool,flag:bool)->*bool {
            subject()# {read()=>do {if flag {return left;}return right;},};left
        }
        fn trial(flag:bool)->bool {
            var left=false;var right=false;
            let selected=choose(left.ref,right.ref,flag);selected.* = true;
            left == flag and right == not flag
        }
        fn run()->bool {trial(true) and trial(false)}",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_handler_returns_pointer_through_nested_installations() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        impl Duplicate for *T where T {}
        effect read()->bool;fn subject()->#read bool {read()#}
        fn forward(pointer:*bool,continued:*bool)->*bool {
            subject()# {read()=>do {
                subject()# {read()=>do {return pointer;},};false
            },};
            continued.* = true;
            pointer
        }
        fn run()->bool {
            var seen=false;var continued=false;
            let result=forward(seen.ref,continued.ref);result.* = true;
            seen and not continued
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_handler_conditional_pointer_return_keeps_absent_payload_absent() {
    let w=Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        impl Duplicate for *T where T {}
        struct Watch {seen:*bool}
        impl Retire for Watch {fn drop(*self) {let previous=self.seen.*.*;assumes not previous;self.seen.*.* = true;}}
        enum Choice {none,some(Watch)}
        effect read()->bool;fn subject()->#read bool {read()#}
        fn make(pointer:*bool,flag:bool)->result:Choice where ensures true {
            subject()# {read()=>do {
                if flag {return Choice.some(Watch {seen:pointer});}
                return Choice.none;
            },};Choice.none
        }
        fn trial(flag:bool)->bool {var seen=false;do {let owner=make(seen.ref,flag);};seen == flag}
        fn run()->bool {trial(true) and trial(false)}",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_direct_handler_return_variant_only_drops_its_selected_pointer_target() {
    let w=Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        impl Duplicate for *T where T {} typealias Tag=.u8'builtin;impl Duplicate for Tag {}
        struct Watch {seen:*bool}
        impl Retire for Watch {fn drop(*self) {let previous=self.seen.*.*;assumes not previous;self.seen.*.* = true;}}
        enum Choice {none,left(Watch),right(Watch)}
        effect read()->bool;fn subject()->#read bool {read()#}
        fn make(left:*bool,right:*bool,tag:Tag)->result:Choice where ensures true {
            subject()# {read()=>do {
                if tag == 1 {return Choice.left(Watch {seen:left});}
                if tag == 2 {return Choice.right(Watch {seen:right});}
                return Choice.none;
            },};Choice.none
        }
        fn trial(tag:Tag)->bool {
            var left=tag != 1;var right=tag != 2;
            do {let owner=make(left.ref,right.ref,tag);};
            left and right
        }
        fn aliased(tag:Tag)->bool {
            var seen=tag == 0;
            do {let owner=make(seen.ref,seen.ref,tag);};
            seen
        }
        fn run()->bool {trial(0) and trial(1) and trial(2) and aliased(0) and aliased(1) and aliased(2)}",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_generic_closure_pointer_payload_cleanup_observes_indirect_slots() {
    let source =
        include_str!("../../../development/repros/host-generic-closure-pointer-cleanup.fl");
    for valid in [true, false] {
        let source = if valid {
            source.to_owned()
        } else {
            source.replace("var first = false;", "var first = true;")
        };
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        assert_eq!(
            reference.status.success(),
            valid,
            "{}",
            String::from_utf8_lossy(&reference.stderr)
        );
        if valid {
            assert_eq!(reference.stdout, b"(bool true)\n");
        }
        for level in ["0", "3"] {
            let out = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&out);
            let out = w.execute();
            assert_eq!(out.status.success(), valid, "{out:?}");
            if valid {
                assert_eq!(out.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn native_pointer_then_patterns_preserve_indirect_targets() {
    let w=Work::new("trait Duplicate {}trait Retire {fn drop(*self);}
        fn selected(p:***bool)->bool {let _ and p.* is slot and slot.* is target = true;target.*}
        fn trial(value:bool)->bool {let pointer=value.ref;let slot=pointer.ref;selected(slot.ref)==value}
        fn run()->bool {trial(true) and trial(false)}");
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_pointer_then_patterns_select_composed_results() {
    for expression in [
        "if flag {p.*} else {q.*}",
        "do {let selected=if flag {p.*} else {q.*};selected}",
        "flag match {true => p.*,false => q.*}",
        "choose(flag,p,q)",
    ] {
        let w=Work::new(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            fn choose(flag:bool,p:**bool,q:**bool)->*bool {{if flag {{p.*}} else {{q.*}}}}
            fn selected(flag:bool,p:**bool,q:**bool)->bool {{let _ and ({expression}) is target = true;target.*}}
            fn trial(flag:bool)->bool {{let yes=true;let no=false;let p=yes.ref;let q=no.ref;selected(flag,p.ref,q.ref)==flag}}
            fn run()->bool {{trial(true) and trial(false)}}"));
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout, "{expression}");
        }
    }
}

#[test]
fn native_handler_parameter_access_survives_lexical_data_return() {
    let source =
        include_str!("../../../development/repros/host-handler-parameter-scalar-return.fl");
    for request in ["subject(pointer)#", "inspect(pointer)#"] {
        for (returned, expectation) in [
            ("value.*", "result and seen"),
            ("do {value.* = true; value.*}", "result and seen"),
            (
                "do {value.* = not value.*; value.*}",
                "not result and not seen",
            ),
        ] {
            let source = source
                .replace("subject(pointer)#", request)
                .replace("return value.*;", &format!("return {returned};"))
                .replace("result and seen", expectation);
            let w = Work::new(&source);
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn checked_handler_parameter_pointer_return_requires_provenance_before_publication() {
    let source = include_str!("../../../development/repros/host-handler-parameter-return.fl");
    for direct_request in [false, true] {
        let source = if direct_request {
            source.replace("subject(pointer)# {", "lend(pointer)# {")
        } else {
            source.to_owned()
        };
        for aggregate in [false, true] {
            let source = if aggregate {
                format!(
                    "struct Holder {{pointer:*bool}}\n{}",
                    source
                        .replace(
                            "fn forward(pointer: *bool) -> *bool",
                            "fn forward(pointer: *bool) -> Holder"
                        )
                        .replace("return value;", "return Holder {pointer:value};")
                        .replace("    pointer\n}", "    Holder {pointer:pointer}\n}")
                        .replace("result.* = true", "result.pointer.* = true")
                )
            } else {
                source.clone()
            };
            let w = Work::new(&source);
            // Establish runtime behavior with the unchecked reference interpreter.
            let reference = w.command().args(["--interpret", "run"]).output().unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            let report = w
                .command()
                .args([
                    "--analyze-resources",
                    "forward",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            assert!(!report.status.success());
            assert!(
                String::from_utf8_lossy(&report.stderr)
                    .contains("handler pointer return lacks request-side provenance evidence"),
                "{}",
                String::from_utf8_lossy(&report.stderr)
            );
            // Temporary implementation boundary, not a language rejection rule.
            fs::write(w.0.join("native program"), b"previous").unwrap();
            let checked = resource_build(&w);
            assert!(!checked.status.success());
            let diagnostic = String::from_utf8_lossy(&checked.stderr);
            assert!(
                diagnostic
                    .contains("handler pointer return lacks request-side provenance evidence")
                    || diagnostic.contains("pointer target provenance is not established")
                    || diagnostic.contains("pointer write target unavailable: MissingPlace"),
                "{diagnostic}"
            );
            assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
        }
    }
}

#[test]
fn native_handler_lexical_return_preserves_other_branch_parameter_completion() {
    for computation in ["select(first)#", "subject(first)#"] {
        for arms in [
            "select(value)=>do {if early {return second;}value},",
            "select(value)=>do {if early {return identity(second);}value},",
            "select(value)=>do {if early {return second;}resume value;},",
            "select(value) if early=>do {return second;},select(value)=>value,",
        ] {
            let w = Work::new(&format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
                fn identity(pointer:*bool)->*bool {{pointer}}
                effect select(pointer:*bool)->*bool;
                fn subject(pointer:*bool)->#select *bool {{select(pointer)#}}
                fn forward(first:*bool,second:*bool,early:bool)->*bool {{{computation} {{{arms}}}}}
                fn trial(early:bool)->bool {{
                    var first=false;var second=false;
                    let selected=forward(first.ref,second.ref,early);selected.* = true;
                    first==not early and second==early
                }}
                fn run()->bool {{trial(false) and trial(true)}}"
            ));
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout, "{computation}/{arms}");
            }
        }
    }
}

#[test]
fn native_handler_constructed_lexical_result_keeps_parameter_completion_separate() {
    for returned in ["Watch {target:second}", "make_watch(second)"] {
        let w=Work::new(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
            struct Watch {{target:*bool}}
            impl Retire for Watch {{fn drop(*self) {{assumes not self.target.*.*;self.target.*.* = true;}}}}
            fn make_watch(target:*bool)->Watch {{Watch {{target:target}}}}
            effect select(pointer:*bool)->*bool;
            fn subject(pointer:*bool)->#select *bool {{select(pointer)#}}
            fn make(first:*bool,second:*bool,early:bool)->Watch {{
                let chosen=subject(first)# {{select(value)=>do {{if early {{return {returned};}}value}},}};
                Watch {{target:chosen}}
            }}
            fn trial(early:bool)->bool {{
                var first=false;var second=false;
                make(first.ref,second.ref,early);
                first==not early and second==early
            }}
            fn run()->bool {{trial(false) and trial(true)}}"));
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout, "{returned}");
        }
    }
}

#[test]
fn native_lexical_only_handler_uses_outer_parameter_forwarding_fallback() {
    for computation in ["select(first)#", "subject(first)#"] {
        let w=Work::new(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
            effect select(pointer:*bool)->*bool;
            fn subject(pointer:*bool)->#select *bool {{select(pointer)#}}
            fn forward(first:*bool,second:*bool,early:bool)->*bool {{
                ({computation} {{select(value) if early=>do {{return second;}},}})# {{select(value)=>value,}}
            }}
            fn trial(early:bool)->bool {{
                var first=false;var second=false;
                let selected=forward(first.ref,second.ref,early);selected.* = true;
                first==not early and second==early
            }}
            fn run()->bool {{trial(false) and trial(true)}}"));
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_fallback_pointer_completion_and_lexical_return_retire_all_scopes_in_order() {
    let source = include_str!("../../../development/repros/host-fallback-cleanup-order.fl");
    checked_ordered_effect_cleanup(
        source,
        "trial(false) and trial(true)",
        &["trial(false)", "trial(true)"],
    );
    let aggregate = source
        .replace("effect select(pointer: *bool) -> *bool;", "struct Choice {target: *bool} impl Duplicate for Choice {} effect select(pointer: Choice) -> Choice;")
        .replace("fn subject(pointer: *bool, counter: *Stage) -> #select *bool", "fn subject(pointer: Choice, counter: *Stage) -> #select Choice")
        .replace("early: bool) -> *bool", "early: bool) -> Choice")
        .replace("subject(first, counter)", "subject(Choice {target: first}, counter)")
        .replace("return second;", "return Choice {target: second};")
        .replace("selected.* = true;", "selected.target.* = true;");
    checked_ordered_effect_cleanup(
        &aggregate,
        "trial(false) and trial(true)",
        &["trial(false)", "trial(true)"],
    );
}

#[test]
fn native_pointer_completion_and_lexical_loop_exits_retire_all_scopes_in_order() {
    let source = include_str!("../../../development/repros/host-pointer-loop-cleanup-order.fl");
    checked_ordered_effect_cleanup(
        source,
        "trial(false, false) and trial(true, false) and trial(true, true)",
        &[
            "trial(false, false)",
            "trial(true, false)",
            "trial(true, true)",
        ],
    );
    let start = source.find("subject(first.ref, counter)# {").unwrap();
    let end = source[start..].find(";\n            selected.*").unwrap() + start;
    let mut fallback = source.to_owned();
    fallback.replace_range(
        start..end,
        "(subject(first.ref, counter)# {
        select(value) if early => do {
            let arm = Step {counter: counter, expected: 0, next: 1};
            if again {continue;}
            break;
        },
    })# {
        select(value) => do {
            let arm = Step {counter: counter, expected: 0, next: 1};
            value
        },
    }",
    );
    checked_ordered_effect_cleanup(
        &fallback,
        "trial(false, false) and trial(true, false) and trial(true, true)",
        &[
            "trial(false, false)",
            "trial(true, false)",
            "trial(true, true)",
        ],
    );
}

fn checked_ordered_effect_cleanup(source: &str, entry: &str, cases: &[&str]) {
    checked_ordered_effect_cleanup_with_counter(
        source,
        entry,
        cases,
        ("expected: 3, next: 4", "expected: 2, next: 4"),
    );
}

fn checked_ordered_effect_cleanup_with_counter(
    source: &str,
    entry: &str,
    cases: &[&str],
    invalid: (&str, &str),
) {
    assert!(source.contains(entry));
    assert!(source.contains(invalid.0));
    for early in cases {
        for valid in [true, false] {
            let mut source = source.replace(entry, early);
            if !valid {
                // The caller must observe every inner destructor. This
                // intentionally wrong contract must fail in both engines.
                source = source.replace(invalid.0, invalid.1);
            }
            let w = Work::new(&source);
            let protocols = ["--copy-protocol", "Duplicate", "--drop-protocol", "Retire"];
            let reference = w
                .command()
                .args(["--interpret", "run"])
                .args(protocols)
                .output()
                .unwrap();
            assert_eq!(
                reference.status.success(),
                valid,
                "early={early}, valid={valid}: {}",
                String::from_utf8_lossy(&reference.stderr)
            );
            if valid {
                assert_eq!(reference.stdout, b"(bool true)\n");
            } else {
                assert!(
                    String::from_utf8_lossy(&reference.stderr).contains("ProgramContractRejected")
                );
            }
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args(["--build", "run"])
                        .args(protocols)
                        .args(["--opt-level", level, "--output", "native program"])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                assert_eq!(
                    output.status.success(),
                    valid,
                    "early={early}, valid={valid}, O{level}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                if valid {
                    assert_eq!(output.stdout, reference.stdout);
                }
            }
        }
    }
}

#[test]
fn native_pointer_effect_callbacks_preserve_ordered_nonlocal_cleanup() {
    let source = include_str!("../../../development/repros/host-fallback-cleanup-order.fl");
    let closure =
        include_str!("../../../development/examples/host-effect-callback-cleanup.fl").to_owned();
    let indirect = source
        .replace(
            "(subject(first, counter)#",
            "(selected_subject(first, counter)#",
        )
        .replace(
            "let installer =",
            "let selected_subject = subject;\n    let installer =",
        );
    let uninstalled = format!(
        "{closure}\nfn uninstalled()->#select bool {{var target=false;var stage:Stage=0;let result=subject(target.ref,stage.ref);result.*}}"
    );
    let no_installer = Work::new(&uninstalled)
        .command()
        .args([
            "--analyze-resources",
            "uninstalled",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    assert!(!no_installer.status.success());
    assert!(
        String::from_utf8_lossy(&no_installer.stderr).contains("requires a memory contract"),
        "{}",
        String::from_utf8_lossy(&no_installer.stderr)
    );
    let returned = "            value\n        },";
    assert!(closure.contains(returned));
    let invalid = Work::new(&closure.replace(
        returned,
        "            do {var local=false; local.ref}\n        },",
    ));
    fs::write(invalid.0.join("native program"), b"previous").unwrap();
    let rejected = invalid
        .command()
        .args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert_eq!(
        fs::read(invalid.0.join("native program")).unwrap(),
        b"previous"
    );
    let selected = indirect
        .replace("fn forward(", "fn relay(pointer:*bool, counter:*Stage)->#select *bool {subject(pointer,counter)}\nfn forward(")
        .replace("let selected_subject = subject;", "let selected_subject = if early {subject} else {relay};");
    for source in [closure, indirect, selected] {
        checked_ordered_effect_cleanup(
            &source,
            "trial(false) and trial(true)",
            &["trial(false)", "trial(true)"],
        );
    }
}

#[test]
fn native_higher_order_pointer_effects_retire_five_scopes_in_order() {
    let source = include_str!("../../../development/repros/host-higher-order-pointer-effect.fl");
    let generic = source.replace(
        "fn apply(action: Action, counter: *Stage) -> #select *bool {",
        "fn apply(action: F, counter: *Stage) -> #select *bool where F, requires F == Action {",
    );
    assert_ne!(generic, source);
    let start = source.find("fn subject(").unwrap();
    let end = source[start..].find("\nfn forward(").unwrap() + start;
    let mut factory = source.to_owned();
    factory.replace_range(
        start..end,
        "fn make(pointer: *bool, counter: *Stage) -> Action {
        || {
            let called = Step {counter: counter, expected: 1, next: 2};
            select(pointer)#
        }
    }
    fn subject(pointer: *bool, counter: *Stage) -> #select *bool {
        apply(make(pointer, counter), counter)
    }
    ",
    );
    for case in [source, generic.as_str(), factory.as_str()] {
        checked_ordered_effect_cleanup_with_counter(
            case,
            "trial(false) and trial(true)",
            &["trial(false)", "trial(true)"],
            ("expected: 4, next: 5", "expected: 3, next: 5"),
        );
    }
}

#[test]
fn native_repeated_pointer_effect_callback_keeps_each_invocation_cleanup_separate() {
    let source = include_str!("../../../development/repros/host-repeated-pointer-effect.fl");
    checked_ordered_effect_cleanup_with_counter(
        source,
        "trial(false) and trial(true)",
        &["trial(false)", "trial(true)"],
        ("expected: 6, next: 7", "expected: 5, next: 7"),
    );
}

#[test]
fn native_nested_and_empty_anonymous_records_match_reference_display() {
    for expression in [
        "record {}",
        "record {empty:record {},nested:record {flag:true,text:\"中\"}}",
    ] {
        let source = format!(
            "fn record(...fields:T)->T where T {{fields}} typealias Result=({expression})'type;fn run()->Result {{{expression}}}"
        );
        let w = Work::new(&source);
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        success(&reference);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout, "{expression}, O{level}");
        }
    }
}

#[test]
fn native_empty_tuple_and_error_product_display_keep_logical_types() {
    for body in [
        "fn run()->Tuple<> {()}",
        "fn run()->Tuple<Tuple<>,Tuple<bool>,.void'builtin> {((),(true,),())}",
        "fn run()->!Tuple<> Tuple<bool> {(true,)}",
        "fn run()->!Tuple<> Tuple<bool> {error ()}",
        "fn record(...fields:T)->T where T {fields} typealias R=(record {empty:empty(),unit:do {}})'type;fn run()->!bool R {record {empty:empty(),unit:do {}}}",
        "fn record(...fields:T)->T where T {fields} typealias R=(record {empty:empty(),unit:do {}})'type;fn run()->!R bool {error record {empty:empty(),unit:do {}}}",
    ] {
        let w = Work::new(&format!(
            "{TUPLE_HEADER} fn empty()->Tuple<> {{()}}\n{body}"
        ));
        let bindings = [
            "--tuple-type",
            "input.Tuple",
            "--meta-list-type",
            "input.TupleElements",
        ];
        let reference = w
            .command()
            .args(bindings)
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        success(&reference);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args(bindings)
                    .args([
                        "--build",
                        "run",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout, "{body}, O{level}");
        }
    }
}

#[test]
fn native_funnel_shifts_and_rotations_cover_modulo_width_and_argument_order() {
    // Independent oracle: index bits in the concatenated pair, without a wide shift.
    fn funnel(a: u128, b: u128, amount: u128, width: u32, left: bool) -> u128 {
        let shift = (amount % u128::from(width)) as u32;
        (0..width).fold(0, |result, bit| {
            let index = if left {
                width + bit - shift
            } else {
                bit + shift
            };
            let value = if index < width {
                b >> index
            } else {
                a >> (index - width)
            };
            result | ((value & 1) << bit)
        })
    }
    let mut body = String::new();
    for width in [8, 16, 32, 64, 128] {
        let mask = u128::MAX >> (128 - width);
        let a = 0x0123456789abcdef_fedcba9876543296 & mask;
        let b = (!a) & mask;
        for amount in [
            0,
            1,
            u128::from(width - 1),
            u128::from(width),
            u128::from(width + 1),
            mask,
        ] {
            for (direction, left) in [("left", true), ("right", false)] {
                let expected = funnel(a, b, amount, width, left);
                let rotated = funnel(a, a, amount, width, left);
                body.push_str(&format!("do {{
                    var first=false;var second=false;var third=false;
                    let result=math.funnel_shift_{direction}<.u{width}'builtin>(
                        do {{assumes not first and not second and not third;first=true;let x:.u{width}'builtin={a};x}},
                        do {{assumes first and not second and not third;second=true;let x:.u{width}'builtin={b};x}},
                        do {{assumes first and second and not third;third=true;let x:.u{width}'builtin={amount};x}});
                    assumes first and second and third and result == {expected};
                    first=false;second=false;
                    let rotated=math.rotate_{direction}<.u{width}'builtin>(
                        do {{assumes not first and not second;first=true;let x:.u{width}'builtin={a};x}},
                        do {{assumes first and not second;second=true;let x:.u{width}'builtin={amount};x}});
                    assumes first and second and rotated == {rotated};
                }};"));
            }
        }
    }
    let source = format!(
        "mod math {{ {} }}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        impl Duplicate for bool {{}}
        impl Duplicate for .u8'builtin {{}} impl Duplicate for .u16'builtin {{}}
        impl Duplicate for .u32'builtin {{}} impl Duplicate for .u64'builtin {{}}
        impl Duplicate for .u128'builtin {{}}
        fn run()->bool {{{body} true}}",
        include!("support/math_source.rs")
    );
    let w = Work::new(&source);
    let mut dump = w.command();
    dump.args([
        "--emit-llvm",
        "run",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
        "--fuel",
        "1000000",
    ]);
    for operation in nir::intrinsics::Operation::ALL {
        let name = operation.name();
        dump.args([
            "--host-intrinsic",
            &format!("{name}=input.math.target.Scalar.{name}"),
        ]);
    }
    let dumped = dump.output().unwrap();
    success(&dumped);
    let ir = String::from_utf8_lossy(&dumped.stdout);
    for width in [8, 16, 32, 64, 128] {
        for op in ["fshl", "fshr"] {
            assert!(ir.contains(&format!("@llvm.{op}.i{width}(")));
        }
    }
    for level in ["0", "3"] {
        let mut command = w.command();
        command.args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--opt-level",
            level,
            "--output",
            "native program",
            "--fuel",
            "1000000",
        ]);
        for operation in nir::intrinsics::Operation::ALL {
            let name = operation.name();
            command.args([
                "--host-intrinsic",
                &format!("{name}=input.math.target.Scalar.{name}"),
            ]);
        }
        success(&command.output().unwrap());
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_contract_diagnostics_locate_original_predicates() {
    for (source, needle) in [
        (
            "fn check(flag:bool) {assumes flag;} fn relay() {check(false);} fn run() {relay();}",
            "assumes flag",
        ),
        (
            "fn run() {let check=|flag:bool| {assumes flag;};check(false);}",
            "assumes flag",
        ),
    ] {
        let w = Work::new(source);
        let start = source.find(needle).unwrap() + needle.find(' ').unwrap() + 1;
        let end = source.find(needle).unwrap() + needle.len();
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            assert!(!output.status.success());
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(
                stderr.contains(&format!("(predicate {start} {end})")),
                "{source}: {stderr}"
            );
            assert_eq!(
                stderr.matches("runtime-check-failure").count(),
                1,
                "{stderr}"
            );
        }
    }
}

#[test]
fn native_inlined_predicate_diagnostic_recovers_source_provenance() {
    let source = format!(
        "{RESOURCE_HEADER} inline fn check(flag:bool) {{assumes flag;}} fn run() {{check(false);}}"
    );
    let start = source.find("assumes flag;").unwrap() + "assumes ".len();
    let w = Work::new(&source);
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--inline-functions",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains(&format!("(predicate {start} {})", start + 4)),
            "{stderr}"
        );
        assert_eq!(
            stderr.matches("runtime-check-failure").count(),
            1,
            "{stderr}"
        );
    }
}

#[test]
fn native_scalar_newtype_entry_requires_ownership_adapter() {
    for (representation, value) in [
        ("bool", "true"),
        (".u32'builtin", "42"),
        (".f64'builtin", "1.5"),
        (".void'builtin", "()"),
        (".BitVec'builtin<0>", "0"),
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}newtype Token = {representation};impl Retire for Token {{fn drop(*self) {{assumes false;}}}}fn run()->Token {{let value: {representation} = {value};value.as(Token)}}"
        );
        let w = Work::new(&source);
        fs::write(w.0.join("native program"), b"previous executable").unwrap();
        for action in ["--build", "--emit-object", "--emit-llvm"] {
            let output = w
                .command()
                .args([
                    action,
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            assert!(
                !output.status.success(),
                "newtype result lost ownership: {source}"
            );
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("owned nominal values need an entry cleanup adapter"),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                fs::read(w.0.join("native program")).unwrap(),
                b"previous executable"
            );
        }
        for level in ["0", "3"] {
            for valid in [false, true] {
                fs::write(
                    w.0.join("source.fl"),
                    source.replace("assumes false", &format!("assumes {valid}")),
                )
                .unwrap();
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--discard-result",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                if valid {
                    success(&output);
                    assert_eq!(output.stdout, b"(unit)\n");
                    assert!(output.stderr.is_empty());
                    continue;
                }
                assert!(!output.status.success(), "Drop was skipped: {source}");
                assert!(output.stdout.is_empty());
                let diagnostic = String::from_utf8(output.stderr).unwrap();
                assert_eq!(
                    diagnostic.matches("runtime-check-failure").count(),
                    1,
                    "{diagnostic}"
                );
            }
        }
    }
}

#[test]
fn native_discard_entry_selects_only_the_live_owned_error_payload() {
    let original = include_str!("../../../development/examples/host-owned-result.fl");
    for wrap_error in [false, true] {
        let original = if wrap_error {
            original.replace("fn run(", "newtype WrappedFailure = Failure; impl Retire for WrappedFailure {fn drop(*self) {}}\nfn run(")
          .replace("-> !Failure Ticket", "-> !WrappedFailure Ticket")
          .replace("error Failure.expired(ticket)", "error Failure.expired(ticket).as(WrappedFailure)")
          .replace("error Failure.denied(ticket.as(Envelope))", "error Failure.denied(ticket.as(Envelope)).as(WrappedFailure)")
        } else {
            original.to_owned()
        };
        for (failed, expired) in [(false, false), (true, false), (true, true)] {
            for valid in [true, false] {
                let source = original
                    .replace(".failed: bool = true", &format!(".failed: bool = {failed}"))
                    .replace(
                        ".expired: bool = false",
                        &format!(".expired: bool = {expired}"),
                    )
                    .replace(".valid: bool = true", &format!(".valid: bool = {valid}"));
                let w = Work::new(&source);
                let reference = w
                    .command()
                    .args([
                        "--interpret",
                        "reference",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                    ])
                    .output()
                    .unwrap();
                assert_eq!(
                    reference.status.success(),
                    valid,
                    "{}",
                    String::from_utf8_lossy(&reference.stderr)
                );
                for level in ["0", "3"] {
                    success(
                        &w.command()
                            .args([
                                "--build",
                                "run",
                                "--discard-result",
                                "--copy-protocol",
                                "Duplicate",
                                "--drop-protocol",
                                "Retire",
                                "--opt-level",
                                level,
                                "--output",
                                "native program",
                            ])
                            .output()
                            .unwrap(),
                    );
                    let output = w.execute();
                    assert_eq!(
                        output.status.success(),
                        valid,
                        "failed={failed} expired={expired} valid={valid}: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    if valid {
                        assert_eq!(output.stdout, reference.stdout);
                        assert_eq!(output.stdout, b"(unit)\n");
                        assert!(output.stderr.is_empty());
                    } else {
                        assert!(output.stdout.is_empty());
                        let diagnostic = String::from_utf8(output.stderr).unwrap();
                        let start = source.find("self.ready.*;").unwrap();
                        assert!(
                            diagnostic.contains(&format!(
                                "(predicate {start} {})",
                                start + "self.ready.*".len()
                            )),
                            "{diagnostic}"
                        );
                        assert_eq!(
                            diagnostic.matches("runtime-check-failure").count(),
                            1,
                            "{diagnostic}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn native_nested_newtype_cleanup_survives_moves_returns_and_captures() {
    let header = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {assumes not self.target.*.*;self.target.*.* = true;}}
        newtype Inner=Watch;impl Retire for Inner {fn drop(*self) {}}
        newtype Outer=Inner;impl Retire for Outer {fn drop(*self) {}}
        fn make(target:*bool)->Outer {Watch {target:target}.as(Inner).as(Outer)}
        fn consume(value:Outer,early:bool) {if early {return;}}
        fn pack(...fields:T)->T where T {fields}";
    for body in [
        "let seen=false;do {let value=make(seen.ref);};seen",
        "let seen=false;make(seen.ref);seen",
        "let seen=false;consume(make(seen.ref),false);seen",
        "let seen=false;consume(make(seen.ref),true);seen",
        "let seen=false;do {let value=make(seen.ref);let action=||value;};seen",
        "let seen=false;do {let value=make(seen.ref);let action=||value;let outer=||action;};seen",
        "let seen=false;do {let value=make(seen.ref);let product=pack {owned:value};};seen",
        "let first=false;let second=false;var replaced=false;do {var value=make(first.ref);value=make(second.ref);replaced=first and not second;};replaced and first and second",
    ] {
        let source = format!("{header} fn run()->bool {{{body}}}");
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n", "{body}");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout, "{body}");
            assert!(output.stderr.is_empty());
        }
    }
}

#[test]
fn native_computed_newtype_candidates_retire_nested_payloads() {
    for flag in [false, true] {
        for inline in [false, true] {
            let prefix = if inline { "inline " } else { "" };
            let source = format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}impl Duplicate for bool {{}}
                struct Item {{ready:bool}} impl Retire for Item {{fn drop(*self) {{assumes self.ready.*;self.ready.* = false;}}}}
                newtype Inner=Item;impl Retire for Inner {{fn drop(*self) {{}}}}
                newtype Outer=Inner;impl Retire for Outer {{fn drop(*self) {{}}}}
                {prefix}fn make()->Outer {{Item {{ready:true}}.as(Inner).as(Outer)}}
                fn run()->bool {{let (_ and make() is (_ if {flag})) = true else {{return false;}};true}}");
            let rejected = Work::new(&source);
            fs::write(rejected.0.join("native program"), b"previous").unwrap();
            let refusal = resource_build(&rejected);
            assert!(!refusal.status.success());
            assert!(
                String::from_utf8_lossy(&refusal.stderr)
                    .contains("observation cleanup needs failure-path resource transfer"),
                "{}",
                String::from_utf8_lossy(&refusal.stderr)
            );
            assert_eq!(
                fs::read(rejected.0.join("native program")).unwrap(),
                b"previous"
            );
            let source = source.replace("assumes self.ready.*;self.ready.* = false;", "");
            // Empty destructors have no observable runtime side effect. Inspect
            // the unoptimized handoff too, so a passing Bool is not the only
            // evidence that each nominal cleanup responsibility was emitted.
            let prepared = prepared_host(&source);
            let ir = luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
            let destructors = prepared
                .graph()
                .functions
                .iter()
                .enumerate()
                .filter(|(_, function)| function.function.name == "drop")
                .collect::<Vec<_>>();
            assert_eq!(destructors.len(), 3);
            for (index, function) in destructors {
                assert!(
                    ir.contains(&format!("call void @flurry_{index}(ptr ")),
                    "missing {}: {ir}",
                    function.key
                );
            }
            let w = Work::new(&source);
            let mut reference = w.command();
            reference.args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ]);
            if inline {
                reference.arg("--inline-functions");
            }
            let reference = reference.output().unwrap();
            success(&reference);
            assert_eq!(reference.stdout, format!("(bool {flag})\n").as_bytes());
            for level in ["0", "3"] {
                let mut build = w.command();
                build.args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ]);
                if inline {
                    build.arg("--inline-functions");
                }
                success(&build.output().unwrap());
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn native_generic_newtype_instances_keep_distinct_cleanup_subjects() {
    let header = "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        struct Set {target:*bool}impl Retire for Set {fn drop(*self) {assumes not self.target.*.*;self.target.*.* = true;}}
        struct Clear {target:*bool}impl Retire for Clear {fn drop(*self) {assumes self.target.*.*;self.target.*.* = false;}}
        newtype Wrapped<T> = T;impl Retire for Wrapped<T> where T {fn drop(*self) {}}
        fn wrap(value:T)->Wrapped<T> where T {value.as(Wrapped<T>)}
        fn hold(value:T)->%{value:T} fn()->T where T {||value}";
    for nested in [false, true] {
        for captured in [false, true] {
            let wrapped = |value: &str| {
                let value = format!("wrap({value})");
                let value = if nested {
                    format!("wrap({value})")
                } else {
                    value
                };
                if captured {
                    format!("hold({value})")
                } else {
                    value
                }
            };
            let set = wrapped("Set {target:seen.ref}");
            let clear = wrapped("Clear {target:seen.ref}");
            let source = format!(
                "{header} fn run()->bool {{
                let seen=false;
                do {{let pending={set};}};
                let first=seen;
                do {{let pending={clear};}};
                first and not seen
            }}"
            );
            let w = Work::new(&source);
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
                assert!(output.stderr.is_empty());
            }
        }
    }
}

#[test]
fn native_linker_failure_reports_both_streams_and_preserves_artifact() {
    use std::os::unix::fs::PermissionsExt;
    let w = Work::new("fn run()->bool {true}");
    let linker = w.0.join("diagnostic linker");
    fs::write(
        &linker,
        "#!/bin/sh\nprintf 'link explanation\\033'\nprintf 'link failure\\n' >&2\nexit 7\n",
    )
    .unwrap();
    fs::set_permissions(&linker, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(w.0.join("native program"), b"previous executable").unwrap();
    let output = w
        .command()
        .args(["--build", "run", "--output", "native program"])
        .env("LUNA_CC", &linker)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(
        diagnostic.contains("stdout: \"link explanation\\u{1b}\""),
        "{diagnostic}"
    );
    assert!(
        diagnostic.contains("stderr: \"link failure\\n\""),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains('\x1b'));
    assert_eq!(
        fs::read(w.0.join("native program")).unwrap(),
        b"previous executable"
    );
    success(&w.build());
    let recovered = w.execute();
    success(&recovered);
    assert_eq!(recovered.stdout, b"(bool true)\n");
}

#[test]
fn native_intrinsic_predicates_preserve_runtime_failure() {
    for valid in [true, false] {
        let source = include_str!("../../../development/examples/host-intrinsic-predicate.fl");
        let source = if valid {
            source.to_owned()
        } else {
            source.replace(
                "check64(3.0, 4.0, 5.0, 17.0)",
                "check64(3.0, 4.0, 5.0, 18.0)",
            )
        };
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--host-intrinsic",
                        "fma=input.Ops.fma",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            assert_eq!(
                output.status.success(),
                valid,
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            if valid {
                assert_eq!(output.stdout, b"(bool true)\n");
                assert!(output.stderr.is_empty());
            } else {
                assert!(output.stdout.is_empty());
                assert_eq!(
                    String::from_utf8_lossy(&output.stderr)
                        .matches("runtime-check-failure")
                        .count(),
                    1
                );
            }
        }
    }
}

#[test]
fn intrinsic_predicates_do_not_authorize_operand_writes() {
    let w = Work::new(
        "typealias F=.f32'builtin;^intrinsic_enum enum Ops {fma}fn run()->bool {var value:F=0.0;let b:F=4.0;let c:F=5.0;assumes unsafe {Ops.fma'intrinsic(do {value=3.0;value},b,c).as(F)} == 17.0;true}",
    );
    fs::write(w.0.join("native program"), b"previous executable").unwrap();
    let output = w
        .command()
        .args([
            "--build",
            "run",
            "--host-intrinsic",
            "fma=input.Ops.fma",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("assumes requires a supported effect-free program predicate"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(w.0.join("native program")).unwrap(),
        b"previous executable"
    );
}

#[test]
fn native_integer_intrinsic_contract_ranges_and_exact_results() {
    for width in [8u32, 16, 32, 64, 128] {
        let maximum = u128::MAX >> (128 - width);
        let high = 1u128 << (width - 1);
        let mut declarations = String::new();
        let mut calls = String::new();
        let mut cases = vec![
            ("ctpop", "0".to_owned(), 0),
            ("ctpop", maximum.to_string(), u128::from(width)),
            ("ctlz", "0".to_owned(), u128::from(width)),
            ("ctlz", "1".to_owned(), u128::from(width - 1)),
            ("cttz", "0".to_owned(), u128::from(width)),
            ("cttz", high.to_string(), u128::from(width - 1)),
            ("bitreverse", "1".to_owned(), high),
            ("fshl", format!("1,0,{}", width + 1), 2),
            ("fshr", format!("0,2,{}", width + 1), 1),
            ("fshl", format!("1,2,{width}"), 1),
            ("fshr", format!("1,2,{width}"), 2),
        ];
        if width >= 16 {
            cases.push(("bswap", "1".to_owned(), 1u128 << (width - 8)));
        }
        for (index, (op, arguments, expected)) in cases.iter().enumerate() {
            let values = arguments.split(',').collect::<Vec<_>>();
            let locals = values
                .iter()
                .enumerate()
                .map(|(i, value)| format!("let x{i}:U={value};"))
                .collect::<String>();
            let args = (0..values.len())
                .map(|i| format!("x{i}"))
                .collect::<Vec<_>>()
                .join(",");
            declarations.push_str(&format!("fn constant{index}()->r:U where ensures r == {expected} {{{locals}unsafe {{Ops.{op}'intrinsic({args}).as(U)}}}}"));
            calls.push_str(&format!(
                "test constant_{index} {{let actual=constant{index}();assumes actual == {expected};}}"
            ));
        }
        for op in ["ctpop", "ctlz", "cttz"] {
            declarations.push_str(&format!("fn bounded_{op}(a:U)->r:U where requires unsafe {{Ops.{op}'intrinsic(a).as(U)}} <= {width}, ensures r <= {width} {{unsafe {{Ops.{op}'intrinsic(a).as(U)}}}}"));
            calls.push_str(&format!(
                "test bounded_{op} {{let count=bounded_{op}({maximum});assumes count <= {width};}}"
            ));
        }
        for op in ["ctpop", "ctlz", "cttz"] {
            let bound = if op == "ctpop" {
                "> 0".to_owned()
            } else {
                format!("< {width}")
            };
            declarations.push_str(&format!("fn nonzero_{op}(a:U)->r:U where requires a > 0, ensures r {bound} {{unsafe {{Ops.{op}'intrinsic(a).as(U)}}}}"));
            calls.push_str(&format!(
                "test nonzero_{op} {{let count=nonzero_{op}({maximum});assumes count {bound};}}"
            ));
        }
        declarations.push_str(&format!("fn ordered()->r:U where ensures r == 3 {{var a:U=1;let shift:U=1;unsafe {{Ops.fshl'intrinsic(a,do {{a={high};a}},shift).as(U)}}}}"));
        calls.push_str("test ordered_arguments {let value=ordered();assumes value == 3;}");
        let source = format!(
            "{RESOURCE_HEADER}typealias U=.u{width}'builtin;impl Duplicate for U {{}}^intrinsic_enum enum Ops {{ctpop,ctlz,cttz,bitreverse,bswap,fshl,fshr}}{declarations}{calls}"
        );
        let w = Work::new(&source);
        for level in ["0", "3"] {
            let mut command = w.command();
            command.args([
                "--test-native",
                "--fuel",
                "10000000",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
            ]);
            for op in [
                "ctpop",
                "ctlz",
                "cttz",
                "bitreverse",
                "bswap",
                "fshl",
                "fshr",
            ] {
                command.args(["--host-intrinsic", &format!("{op}=input.Ops.{op}")]);
            }
            let output = command.output().unwrap();
            success(&output);
            let report = String::from_utf8_lossy(&output.stdout);
            assert!(
                report.contains(&format!("(count {}) (failures 0)", cases.len() + 7)),
                "{report}"
            );
        }
    }
}

#[test]
fn integer_intrinsic_contracts_reject_false_and_stale_bounds() {
    for (op, parameter, preparation) in [
        ("ctpop", "a:U", "let value=a;"),
        ("ctlz", "a:U", "let value=a;"),
        ("cttz", "a:U", "let value=a;"),
        ("ctlz", "a:U", "var value=a;assumes value > 0;value=0;"),
        ("cttz", "a:U", "var value=a;assumes value > 0;value=0;"),
    ] {
        let source = format!(
            "typealias U=.u8'builtin;^intrinsic_enum enum Ops {{{op}}}fn invalid({parameter})->r:U where ensures r < 8 {{{preparation}unsafe {{Ops.{op}'intrinsic(value).as(U)}}}}fn run()->U {{invalid(255)}}"
        );
        let w = Work::new(&source);
        fs::write(w.0.join("native program"), b"previous executable").unwrap();
        let output = w
            .command()
            .args([
                "--build",
                "run",
                "--host-intrinsic",
                &format!("{op}=input.Ops.{op}"),
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains("postcondition"), "{diagnostic}");
        assert_eq!(
            fs::read(w.0.join("native program")).unwrap(),
            b"previous executable"
        );
    }
}

#[test]
fn native_mixed_intrinsic_contract_chain_reuses_current_relations() {
    let w = Work::new(include_str!(
        "../../../development/repros/host-intrinsic-mixed-contract-chain.fl"
    ));
    for level in ["0", "3"] {
        let mut command = w.command();
        command.args([
            "--build",
            "run",
            "--fuel",
            "1000000",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--opt-level",
            level,
            "--output",
            "native program",
        ]);
        for op in [
            "ctpop",
            "ctlz",
            "cttz",
            "bitreverse",
            "bswap",
            "fshl",
            "fshr",
        ] {
            command.args(["--host-intrinsic", &format!("{op}=input.Ops.{op}")]);
        }
        success(&command.output().unwrap());
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn native_text_constants_share_storage_across_functions_closures_and_handlers() {
    let w = Work::new(
        r#"
        typealias Text = .str'builtin;
        effect text() -> Text;
        fn request() -> #text Text { text()# }
        fn literal() -> Text { "ready" }
        fn is_ready(value: Text) -> bool { value match { "ready" => true, _ => false } }
        fn run() -> bool {
            let callback = || { "ready" };
            let value = request()# { text() => do { resume "ready"; }, };
            is_ready(callback()) and is_ready(literal()) and is_ready(value)
        }
    "#,
    );
    let first = w.command().args(["--emit-llvm", "run"]).output().unwrap();
    success(&first);
    let second = w.command().args(["--emit-llvm", "run"]).output().unwrap();
    success(&second);
    assert_eq!(first.stdout, second.stdout);
    let ir = String::from_utf8(first.stdout).unwrap();
    assert_eq!(
        ir.lines()
            .filter(|line| line.starts_with("@flurry_string_")
                && line.contains(" = private unnamed_addr constant"))
            .count(),
        1,
        "{ir}"
    );
    assert!(
        ir.contains("@flurry_handler_"),
        "expected an outlined handler"
    );
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
#[cfg(target_os = "linux")]
fn native_output_errors_return_failure() {
    let sources = [
        ("fn run()->bool {true}".to_owned(), "(bool true)\n".to_owned()),
        ("fn run()->.str'builtin {\"small\"}".to_owned(), "(string \"small\")\n".to_owned()),
        (format!("fn run()->.str'builtin {{\"{}\"}}", "x".repeat(10000)),
            format!("(string \"{}\")\n", "x".repeat(10000))),
        ("fn record(...fields:T)->T where T {fields} typealias R=(record {flag:true,text:\"value\"})'type; fn run()->R {record {flag:true,text:\"value\"}}".to_owned(),
            "(anonymous-record (fields (field (string \"flag\") (bool true)) (field (string \"text\") (string \"value\"))))\n".to_owned()),
    ];
    for (source, expected) in sources {
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--fuel",
                        "1000000",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let normal = w.execute();
            success(&normal);
            assert_eq!(normal.stdout, expected.as_bytes());
            let full = fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap();
            let failed = Command::new(w.0.join("native program"))
                .stdout(full)
                .output()
                .unwrap();
            assert_eq!(
                failed.status.code(),
                Some(1),
                "failed output was reported as success at O{level}"
            );
            assert!(failed.stderr.is_empty());
        }
    }
}

#[test]
fn emit_asm_cli_round_trips_single_and_multiple_packages_without_linker() {
    for packages in [false, true] {
        let w = Work::new(include_str!(
            "../../../development/examples/host-direct-calls.fl"
        ));
        let command = || {
            let mut command = if packages {
                let mut command = Command::new(env!("CARGO_BIN_EXE_luna"));
                command.current_dir(&w.0).args([
                    "--packages-v1",
                    "--package",
                    "7",
                    "901",
                    "app",
                    "source.fl",
                    "--primary",
                    "7",
                ]);
                command
            } else {
                w.command()
            };
            command.args(["--copy-protocol", "Duplicate", "--drop-protocol", "Retire"]);
            command
        };
        for level in ["0", "3"] {
            let mut first = command();
            first
                .args(["--emit-asm", "run", "--opt-level", level])
                .env("LUNA_CC", w.0.join("missing-linker"));
            let output = first.output().unwrap();
            success(&output);
            let again = first.output().unwrap();
            success(&again);
            assert_eq!(output.stdout, again.stdout);
            assert!(!output.stdout.is_empty());
            let file = first
                .args(["--output", "separate assembly.s"])
                .output()
                .unwrap();
            success(&file);
            assert!(file.stdout.is_empty());
            assert_eq!(
                fs::read(w.0.join("separate assembly.s")).unwrap(),
                output.stdout
            );
            let linked = Command::new(luna::host::Toolchain::default().linker)
                .current_dir(&w.0)
                .args(["separate assembly.s", "-lm", "-o", "native program"])
                .output()
                .unwrap();
            success(&linked);
            let run = w.execute();
            success(&run);
            assert_eq!(run.stdout, b"(bool true)\n");
        }
        if packages {
            let wrapped = command()
                .args(["--emit-asm", "run", "--with-inputs"])
                .output()
                .unwrap();
            success(&wrapped);
            let text = String::from_utf8(wrapped.stdout).unwrap();
            assert!(text.starts_with("(compilation-result (version 1) (result (host-assembly "));
        }
    }
}

#[test]
fn emit_asm_rejects_conflicts_and_preserves_sources_and_existing_output() {
    let w = Work::new("fn run()->bool {true}");
    for extra in [
        vec!["--emit-object", "run"],
        vec!["--emit-llvm", "run"],
        vec!["--interpret", "run"],
        vec!["--test"],
        vec!["--emit-asm", "run"],
    ] {
        let output = w
            .command()
            .args(["--emit-asm", "run", "--output", "module.s"])
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(!w.0.join("module.s").exists());
    }
    fs::write(w.0.join("module.s"), b"previous assembly").unwrap();
    let failed = w
        .command()
        .args(["--emit-asm", "run", "--output", "module.s"])
        .env("LUNA_LLVM_CONFIG", w.0.join("missing-llvm"))
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert_eq!(
        fs::read(w.0.join("module.s")).unwrap(),
        b"previous assembly"
    );
    let original = fs::read(w.0.join("source.fl")).unwrap();
    fs::hard_link(w.0.join("source.fl"), w.0.join("alias.s")).unwrap();
    for name in ["source.fl", "alias.s"] {
        let failed = w
            .command()
            .args(["--emit-asm", "run", "--output", name])
            .output()
            .unwrap();
        assert!(!failed.status.success());
        assert_eq!(fs::read(w.0.join("source.fl")).unwrap(), original);
    }
}

#[test]
fn native_float_classification_generic_library_preserves_ieee_classes() {
    for width in [32, 64] {
        let patterns: Vec<(u64, bool, bool)> = if width == 32 {
            vec![
                (0, false, false),
                (0x80000000, false, false),
                (1, false, false),
                (0x80000001, false, false),
                (0x007fffff, false, false),
                (0x807fffff, false, false),
                (0xff7fffff, false, false),
                (0x00800000, false, false),
                (0x80800000, false, false),
                (0x3f800000, false, false),
                (0x7f7fffff, false, false),
                (0x7f800000, false, true),
                (0xff800000, false, true),
                (0x7f800001, true, false),
                (0xff800001, true, false),
                (0x7fc00000, true, false),
                (0xffc00000, true, false),
            ]
        } else {
            vec![
                (0, false, false),
                (0x8000000000000000, false, false),
                (1, false, false),
                (0x8000000000000001, false, false),
                (0x000fffffffffffff, false, false),
                (0x800fffffffffffff, false, false),
                (0xffefffffffffffff, false, false),
                (0x0010000000000000, false, false),
                (0x8010000000000000, false, false),
                (0x3ff0000000000000, false, false),
                (0x7fefffffffffffff, false, false),
                (0x7ff0000000000000, false, true),
                (0xfff0000000000000, false, true),
                (0x7ff0000000000001, true, false),
                (0xfff0000000000001, true, false),
                (0x7ff8000000000000, true, false),
                (0xfff8000000000000, true, false),
            ]
        };
        let fraction_mask = if width == 32 {
            0x007fffff
        } else {
            0x000fffffffffffff
        };
        let magnitude_mask = (1u64 << (width - 1)) - 1;
        let exponent_mask = magnitude_mask ^ fraction_mask;
        let checks = patterns.iter().map(|(bits,nan,inf)| format!(
            "do {{var value:F=0.0;value'bitvec[0..{width}]={bits};if not verify(value,{nan},{inf},{finite},{normal},{subnormal},{sign}) {{return false;}}}};",
            finite = !nan && !inf, normal = bits & exponent_mask != 0 && bits & exponent_mask != exponent_mask, subnormal = bits & exponent_mask == 0 && bits & fraction_mask != 0, sign = bits & (1u64 << (width-1)) != 0)).collect::<String>();
        let source = format!("{RESOURCE_HEADER}mod math {{
{}
}} typealias F=.f{width}'builtin;impl Duplicate for F {{}}impl Duplicate for .BitVec'builtin<{width}> {{}}
            fn verify(value:F,n:bool,i:bool,f:bool,normal:bool,subnormal:bool,sign:bool)->bool {{math.isnan(value)==n and math.isinf(value)==i and math.isfinite(value)==f and math.isnormal(value)==normal and math.issubnormal(value)==subnormal and math.signbit(value)==sign}}
            fn run()->bool {{{checks}true}}", include!("support/math_source.rs"));
        let w = Work::new(&source);
        for level in ["0", "3"] {
            let mut command = w.command();
            command.args([
                "--build",
                "run",
                "--fuel",
                "1000000",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "native program",
            ]);
            for operation in nir::intrinsics::Operation::ALL {
                command.args([
                    "--host-intrinsic",
                    &format!("{0}=input.math.target.Scalar.{0}", operation.name()),
                ]);
            }
            success(&command.output().unwrap());
            let result = w.execute();
            success(&result);
            assert_eq!(result.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn float_classification_rejects_unproved_domains_and_wrong_results() {
    for body in [
        "fn classify(x:T)->bool where T {unsafe {Ops.classify'intrinsic(x).as(bool)}}",
        "fn classify(x:T)->bool where T, requires T == .u32'builtin {unsafe {Ops.classify'intrinsic(x).as(bool)}}",
        "fn classify(x:.f32'builtin)->.f32'builtin {unsafe {Ops.classify'intrinsic(x).as(.f32'builtin)}}",
        "fn classify(x:bool)->bool {unsafe {Ops.classify'intrinsic(x).as(bool)}}",
    ] {
        let w = Work::new(&format!(
            "^intrinsic_enum enum Ops {{classify}}{body}fn run()->bool {{true}}"
        ));
        fs::write(w.0.join("previous.ll"), b"previous artifact").unwrap();
        let output = w
            .command()
            .args([
                "--emit-llvm",
                "run",
                "--host-intrinsic",
                "isnan=input.Ops.classify",
                "--output",
                "previous.ll",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            fs::read(w.0.join("previous.ll")).unwrap(),
            b"previous artifact"
        );
    }
}

#[test]
fn native_float_classification_predicates_check_runtime_values_and_evaluate_once() {
    for width in [32, 64] {
        let nan_bits = if width == 32 {
            "2139095041"
        } else {
            "9218868437227405313"
        };
        for (operation, bits, expected) in [
            ("isnan", nan_bits, true),
            ("isinf", nan_bits, false),
            ("isfinite", nan_bits, false),
            ("isnan", "0", false),
            ("isinf", "0", false),
            ("isfinite", "0", true),
            ("isnormal", "0", false),
            ("issubnormal", "1", true),
            ("signbit", "0", false),
            ("isnormal", nan_bits, false),
            ("issubnormal", nan_bits, false),
            ("signbit", nan_bits, false),
        ] {
            for valid in [true, false] {
                let expected = if valid { expected } else { !expected };
                let source = format!("{RESOURCE_HEADER}
                    typealias F=.f{width}'builtin;impl Duplicate for F {{}}impl Duplicate for .BitVec'builtin<{width}> {{}}
                    ^intrinsic_enum enum Ops {{classify}}
                    fn check(value:F) {{assumes unsafe {{Ops.classify'intrinsic(value).as(bool)}} == {expected};}}
                    fn once(value:F)->bool {{
                        var visited=false;
                        let result=unsafe {{Ops.classify'intrinsic(do {{assumes not visited;visited=true;value}}).as(bool)}};
                        assumes visited;
                        result
                    }}
                    fn run()->bool {{var value:F=0.0;value'bitvec[0..{width}]={bits};let observed=once(value);check(value);observed == {expected}}}");
                let w = Work::new(&source);
                for level in ["0", "3"] {
                    success(
                        &w.command()
                            .args([
                                "--build",
                                "run",
                                "--copy-protocol",
                                "Duplicate",
                                "--drop-protocol",
                                "Retire",
                                "--host-intrinsic",
                                &format!("{operation}=input.Ops.classify"),
                                "--opt-level",
                                level,
                                "--output",
                                "native program",
                            ])
                            .output()
                            .unwrap(),
                    );
                    let output = w.execute();
                    assert_eq!(
                        output.status.success(),
                        valid,
                        "{operation} f{width} O{level}: {output:?}"
                    );
                    if valid {
                        assert_eq!(output.stdout, b"(bool true)\n");
                        assert!(output.stderr.is_empty());
                    } else {
                        assert!(output.stdout.is_empty());
                        assert_eq!(
                            String::from_utf8_lossy(&output.stderr)
                                .matches("runtime-check-failure")
                                .count(),
                            1
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn invalid_llvm_tool_directory_preserves_artifact_and_recovers() {
    use std::os::unix::fs::PermissionsExt;
    let w = Work::new("fn run()->bool {true}");
    let config = w.0.join("invalid llvm config");
    for (body, expected) in [
        ("printf ' \\n\\t'", "LLVM bindir is empty"),
        (
            "printf '%s\\n' ./missing-llvm-directory",
            "cannot resolve LLVM bindir",
        ),
        (
            "printf '%s\\n' './invalid llvm config'",
            "is not a directory",
        ),
    ] {
        fs::write(&config, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(w.0.join("native program"), b"previous executable").unwrap();
        let output = w
            .command()
            .args(["--build", "run", "--output", "native program"])
            .env("LUNA_LLVM_CONFIG", &config)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{output:?}"
        );
        assert_eq!(
            fs::read(w.0.join("native program")).unwrap(),
            b"previous executable"
        );
    }
    success(&w.build());
    let recovered = w.execute();
    success(&recovered);
    assert_eq!(recovered.stdout, b"(bool true)\n");
}

#[test]
fn successful_linker_must_deliver_nonempty_regular_artifact() {
    use std::os::unix::fs::PermissionsExt;
    let w = Work::new("fn run()->bool {true}");
    let linker = w.0.join("incomplete linker");
    for (action, diagnostic) in [
        (
            ": > \"$destination\"",
            "linked executable artifact is empty",
        ),
        ("true", "cannot inspect linked executable artifact"),
        (
            "mkdir \"$destination\"",
            "linked executable artifact is not a regular file",
        ),
        (
            "mkfifo \"$destination\"",
            "linked executable artifact is not a regular file",
        ),
    ] {
        fs::write(
            &linker,
            format!("#!/bin/sh\nfor destination in \"$@\"; do :; done\n{action}\n"),
        )
        .unwrap();
        fs::set_permissions(&linker, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(w.0.join("native program"), b"previous executable").unwrap();
        let output = w
            .command()
            .args(["--build", "run", "--output", "native program"])
            .env("LUNA_CC", &linker)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "{action}: incomplete artifact was published"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "{output:?}"
        );
        assert_eq!(
            fs::read(w.0.join("native program")).unwrap(),
            b"previous executable"
        );
    }
    success(&w.build());
    let recovered = w.execute();
    success(&recovered);
    assert_eq!(recovered.stdout, b"(bool true)\n");
}

#[test]
fn emit_bitcode_cli_round_trips_checked_single_and_multiple_packages() {
    use std::os::unix::fs::PermissionsExt;
    let tools = luna::host::Toolchain::default();
    let bindir = Command::new(&tools.llvm_config)
        .arg("--bindir")
        .output()
        .unwrap();
    success(&bindir);
    let disassembler =
        PathBuf::from(String::from_utf8(bindir.stdout).unwrap().trim()).join("llvm-dis");
    for packages in [false, true] {
        let w = Work::new(include_str!(
            "../../../development/examples/host-direct-calls.fl"
        ));
        let command = || {
            if packages {
                let mut c = Command::new(env!("CARGO_BIN_EXE_luna"));
                c.current_dir(&w.0).args([
                    "--packages-v1",
                    "--package",
                    "7",
                    "901",
                    "app",
                    "source.fl",
                    "--primary",
                    "7",
                ]);
                c
            } else {
                w.command()
            }
        };
        for level in ["0", "3"] {
            let mut c = command();
            c.args([
                "--emit-bitcode",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--opt-level",
                level,
                "--output",
                "module.bc",
            ])
            .env("LUNA_CC", w.0.join("missing-linker"));
            let result = c.output().unwrap();
            success(&result);
            assert!(result.stdout.is_empty());
            let bitcode = fs::read(w.0.join("module.bc")).unwrap();
            assert!(bitcode.starts_with(b"BC\xc0\xde"));
            success(&c.output().unwrap());
            assert_eq!(fs::read(w.0.join("module.bc")).unwrap(), bitcode);
            assert_eq!(
                fs::metadata(w.0.join("module.bc"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111,
                0
            );
            success(
                &Command::new(&disassembler)
                    .current_dir(&w.0)
                    .args(["module.bc", "-o", "roundtrip.ll"])
                    .output()
                    .unwrap(),
            );
            let ir = fs::read_to_string(w.0.join("roundtrip.ll")).unwrap();
            assert!(ir.contains("target triple ="));
            assert!(ir.contains("target datalayout ="));
            assert!(ir.contains("!flurry.target"));
            let bytes = luna::host::executable(&ir, &tools).unwrap();
            fs::write(w.0.join("native program"), bytes).unwrap();
            fs::set_permissions(
                w.0.join("native program"),
                fs::Permissions::from_mode(0o700),
            )
            .unwrap();
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
        if packages {
            let result = command()
                .args([
                    "--emit-bitcode",
                    "run",
                    "--output",
                    "module.bc",
                    "--with-inputs",
                ])
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(2));
        }
    }
}

#[test]
fn emit_bitcode_requires_binary_destination_and_preserves_existing_files() {
    let w = Work::new("fn run()->bool {true}");
    for args in [
        vec!["--emit-bitcode", "run"],
        vec![
            "--emit-bitcode",
            "run",
            "--build",
            "run",
            "--output",
            "module.bc",
        ],
        vec![
            "--emit-bitcode",
            "run",
            "--emit-asm",
            "run",
            "--output",
            "module.bc",
        ],
    ] {
        assert_eq!(
            w.command().args(args).output().unwrap().status.code(),
            Some(2)
        );
        assert!(!w.0.join("module.bc").exists());
    }
    fs::write(w.0.join("module.bc"), b"previous bitcode").unwrap();
    let result = w
        .command()
        .args(["--emit-bitcode", "run", "--output", "module.bc"])
        .env("LUNA_LLVM_CONFIG", w.0.join("missing-llvm"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(
        fs::read(w.0.join("module.bc")).unwrap(),
        b"previous bitcode"
    );
    let source = fs::read(w.0.join("source.fl")).unwrap();
    fs::hard_link(w.0.join("source.fl"), w.0.join("alias.bc")).unwrap();
    for name in ["source.fl", "alias.bc"] {
        let result = w
            .command()
            .args(["--emit-bitcode", "run", "--output", name])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert_eq!(fs::read(w.0.join("source.fl")).unwrap(), source);
    }
}

#[test]
fn native_saturating_integer_math_covers_all_widths_and_signs() {
    for width in [8, 16, 32, 64, 128] {
        for signed in [false, true] {
            let mut cases = Vec::new();
            if signed {
                let max = if width == 128 {
                    i128::MAX
                } else {
                    (1i128 << (width - 1)) - 1
                };
                let min = -max - 1;
                for (a, b) in [
                    (0, 0),
                    (max, 1),
                    (min, -1),
                    (max, min),
                    (min, max),
                    (max, -1),
                    (min, 1),
                    (max, max),
                    (min, min),
                ] {
                    cases.push(format!(
                        "verify({a},{b},{},{})",
                        a.saturating_add(b).clamp(min, max),
                        a.saturating_sub(b).clamp(min, max)
                    ));
                }
            } else {
                let max = if width == 128 {
                    u128::MAX
                } else {
                    (1u128 << width) - 1
                };
                for (a, b) in [
                    (0, 0),
                    (max, 1),
                    (max, max),
                    (0, max),
                    (1, max),
                    (max, 0),
                    (max - 1, 1),
                    (2, 1),
                ] {
                    cases.push(format!(
                        "verify({a},{b},{},{})",
                        a.saturating_add(b).min(max),
                        a.saturating_sub(b)
                    ));
                }
            }
            let prefix = if signed { "i" } else { "u" };
            let source = format!("{RESOURCE_HEADER}mod math {{\n{}\n}}
                typealias Number=.{prefix}{width}'builtin;impl Duplicate for Number {{}}
                fn verify(a:Number,b:Number,added:Number,subtracted:Number)->bool {{math.saturating_add(a,b)==added and math.saturating_sub(a,b)==subtracted}}
                fn run()->bool {{{}}}",include!("support/math_source.rs"),cases.join(" and "));
            let w = Work::new(&source);
            for level in ["0", "3"] {
                let mut command = w.command();
                command.args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--fuel",
                    "1000000",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ]);
                for operation in nir::intrinsics::Operation::ALL {
                    command.args([
                        "--host-intrinsic",
                        &format!("{0}=input.math.target.Scalar.{0}", operation.name()),
                    ]);
                }
                success(&command.output().unwrap());
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, b"(bool true)\n", "{prefix}{width} O{level}");
            }
        }
    }
}

#[test]
fn native_saturating_contracts_prove_clamped_and_unsaturated_ranges() {
    for width in [8, 16, 32, 64, 128] {
        for signed in [false, true] {
            let (prefix, min, max, near_min, near_max) = if signed {
                let max = if width == 128 {
                    i128::MAX
                } else {
                    (1i128 << (width - 1)) - 1
                };
                let min = -max - 1;
                (
                    "i",
                    min.to_string(),
                    max.to_string(),
                    (min + 1).to_string(),
                    (max - 1).to_string(),
                )
            } else {
                let max = if width == 128 {
                    u128::MAX
                } else {
                    (1u128 << width) - 1
                };
                (
                    "u",
                    "0".into(),
                    max.to_string(),
                    "1".into(),
                    (max - 1).to_string(),
                )
            };
            let source=format!("{RESOURCE_HEADER}typealias Number=.{prefix}{width}'builtin;impl Duplicate for Number {{}}
                ^intrinsic_enum enum Ops {{add,sub}}
                fn upper(a:Number,b:Number)->r:Number where requires a >= {near_max}, requires b >= 1, ensures r == {max} {{unsafe {{Ops.add'intrinsic(a,b).as(Number)}}}}
                fn lower(a:Number,b:Number)->r:Number where requires a <= {near_min}, requires b >= 2, ensures r == {min} {{unsafe {{Ops.sub'intrinsic(a,b).as(Number)}}}}
                fn middle(a:Number,b:Number)->r:Number where requires a >= 10, requires a <= 20, requires b >= 1, requires b <= 3, ensures r >= 7, ensures r <= 19 {{unsafe {{Ops.sub'intrinsic(a,b).as(Number)}}}}
                fn run()->bool {{upper({near_max},1)=={max} and lower({near_min},2)=={min} and middle(20,1)==19}}");
            let w = Work::new(&source);
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--fuel",
                            "1000000",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--host-intrinsic",
                            "add_sat=input.Ops.add",
                            "--host-intrinsic",
                            "sub_sat=input.Ops.sub",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let result = w.execute();
                success(&result);
                assert_eq!(result.stdout, b"(bool true)\n");
            }
        }
    }
}

#[test]
fn saturating_contracts_preserve_argument_snapshots_and_reject_false_bounds() {
    let header = format!(
        "{RESOURCE_HEADER}typealias Byte=.u8'builtin;impl Duplicate for Byte {{}}^intrinsic_enum enum Ops {{add,sub}}"
    );
    let valid = "fn ordered()->r:Byte where ensures r==255 {var a:Byte=250;let ten:Byte=10;unsafe {Ops.add'intrinsic(a,do {a=1;ten}).as(Byte)}}fn run()->bool {ordered()==255}";
    let w = Work::new(&format!("{header}{valid}"));
    let args = [
        "--build",
        "run",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
        "--host-intrinsic",
        "add_sat=input.Ops.add",
        "--host-intrinsic",
        "sub_sat=input.Ops.sub",
        "--output",
        "native program",
    ];
    for level in ["0", "3"] {
        success(
            &w.command()
                .args(args)
                .args(["--opt-level", level])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
    for body in [
        "fn bad(a:Byte,b:Byte)->r:Byte where ensures r==0 {unsafe {Ops.add'intrinsic(a,b).as(Byte)}}",
        "fn bad(a:Byte,b:Byte)->r:Byte where ensures r>=a {unsafe {Ops.sub'intrinsic(a,b).as(Byte)}}",
        "fn bad(a:Byte,b:Byte)->r:Byte where requires a>=10,requires a<=20,requires b>=1,requires b<=3,ensures r>=7 {var value=a;value=0;unsafe {Ops.sub'intrinsic(value,b).as(Byte)}}",
    ] {
        let w = Work::new(&format!("{header}{body}fn run()->bool {{true}}"));
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let output = w.command().args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("postcondition"),
            "{output:?}"
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_integer_minmax_math_covers_all_widths_and_signs() {
    for width in [8, 16, 32, 64, 128] {
        for signed in [false, true] {
            let mut cases = Vec::new();
            if signed {
                let max = if width == 128 {
                    i128::MAX
                } else {
                    (1i128 << (width - 1)) - 1
                };
                let min = -max - 1;
                for (a, b) in [
                    (0, 0),
                    (max, 1),
                    (min, -1),
                    (max, min),
                    (min, max),
                    (max, -1),
                    (min, 1),
                    (max, max),
                    (min, min),
                ] {
                    cases.push(format!("verify({a},{b},{},{})", a.min(b), a.max(b)));
                }
            } else {
                let max = if width == 128 {
                    u128::MAX
                } else {
                    (1u128 << width) - 1
                };
                for (a, b) in [
                    (0, 0),
                    (max, 1),
                    (max, max),
                    (0, max),
                    (1, max),
                    (max, 0),
                    (max - 1, 1),
                    (2, 1),
                ] {
                    cases.push(format!("verify({a},{b},{},{})", a.min(b), a.max(b)));
                }
            }
            let prefix = if signed { "i" } else { "u" };
            let source = format!("{RESOURCE_HEADER}mod math {{\n{}\n}}
                typealias Number=.{prefix}{width}'builtin;impl Duplicate for Number {{}}
                fn verify(a:Number,b:Number,smaller:Number,larger:Number)->bool {{math.min(a,b)==smaller and math.max(a,b)==larger}}
                fn run()->bool {{{}}}",include!("support/math_source.rs"),cases.join(" and "));
            let w = Work::new(&source);
            for level in ["0", "3"] {
                let mut command = w.command();
                command.args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--fuel",
                    "1000000",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ]);
                for operation in nir::intrinsics::Operation::ALL {
                    command.args([
                        "--host-intrinsic",
                        &format!("{0}=input.math.target.Scalar.{0}", operation.name()),
                    ]);
                }
                success(&command.output().unwrap());
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, b"(bool true)\n", "{prefix}{width} O{level}");
            }
        }
    }
}

#[test]
fn native_integer_minmax_contracts_and_argument_snapshots() {
    for prefix in ["u", "i"] {
        for width in [8, 16, 32, 64, 128] {
            let source = format!("{RESOURCE_HEADER}
                typealias Number=.{prefix}{width}'builtin; impl Duplicate for Number {{}}
                ^intrinsic_enum enum Ops {{min,max}}
                fn lower(a:Number,b:Number)->r:Number
                where requires a>=10, requires a<=20, requires b>=15, requires b<=25,
                    ensures r>=10, ensures r<=20 {{unsafe {{Ops.min'intrinsic(a,b).as(Number)}}}}
                fn upper(a:Number,b:Number)->r:Number
                where requires a>=10, requires a<=20, requires b>=15, requires b<=25,
                    ensures r>=15, ensures r<=25 {{unsafe {{Ops.max'intrinsic(a,b).as(Number)}}}}
                fn ordered()->r:Number where ensures r==10 {{
                    var value:Number=20; var visits:Number=0;
                    let result=unsafe {{Ops.min'intrinsic(do {{visits = visits + 1;value}},do {{visits = visits + 1;value=1;let ten:Number=10;ten}}).as(Number)}};
                    assumes visits==2; result
                }}
                fn run()->bool {{lower(20,15)==15 and upper(20,25)==25 and ordered()==10}}");
            let w = Work::new(&source);
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--fuel",
                            "1000000",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--host-intrinsic",
                            "min=input.Ops.min",
                            "--host-intrinsic",
                            "max=input.Ops.max",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, b"(bool true)\n");
            }
        }
    }
    for (operation, bound) in [("min", "r>=15"), ("max", "r<=20")] {
        let w = Work::new(&format!(
            "{RESOURCE_HEADER}
            typealias Number=.i32'builtin; impl Duplicate for Number {{}}
            ^intrinsic_enum enum Ops {{op}}
            fn wrong(a:Number,b:Number)->r:Number
            where requires a>=10, requires a<=20, requires b>=15, requires b<=25,
                ensures {bound} {{unsafe {{Ops.op'intrinsic(a,b).as(Number)}}}}
            fn run()->bool {{true}}"
        ));
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let output = w
            .command()
            .args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--host-intrinsic",
                &format!("{operation}=input.Ops.op"),
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("postcondition"),
            "{output:?}"
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_integer_minmax_relations_survive_calls_and_source_writes() {
    for ty in ["i8", "u8", "i128", "u128"] {
        let header = format!(
            "{RESOURCE_HEADER}typealias Number=.{ty}'builtin;impl Duplicate for Number {{}}^intrinsic_enum enum Ops {{min,max}}"
        );
        let source = format!("{header}
            fn lower(a:Number,b:Number)->r:Number where ensures r<=a'old, ensures r<=b'old {{unsafe {{Ops.min'intrinsic(a,b).as(Number)}}}}
            fn upper(a:Number,b:Number)->r:Number where ensures r>=a'old, ensures r>=b'old {{unsafe {{Ops.max'intrinsic(a,b).as(Number)}}}}
            fn copied(a:Number,b:Number)->r:Number where ensures r<=a'old, ensures r<=b'old {{
                var source=a;
                let result=lower(source,b);
                source=0;
                result
            }}
            fn caller(a:Number,b:Number)->bool {{
                let low=copied(a,b); let high=upper(a,b);
                assumes low<=a; assumes low<=b; assumes high>=a; assumes high>=b;
                low<=high
            }}
            fn run()->bool {{caller(10,20) and caller(20,10) and caller(10,10)}}");
        let args = [
            "--build",
            "run",
            "--fuel",
            "1000000",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--host-intrinsic",
            "min=input.Ops.min",
            "--host-intrinsic",
            "max=input.Ops.max",
            "--output",
            "native program",
        ];
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args(args)
                    .args(["--opt-level", level])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
        for body in [
            "fn wrong(a:Number,b:Number)->r:Number where ensures r>=a'old {unsafe {Ops.min'intrinsic(a,b).as(Number)}}",
            "fn wrong(a:Number,b:Number)->r:Number where ensures r<=a'old {unsafe {Ops.max'intrinsic(a,b).as(Number)}}",
            "fn wrong(a:Number,b:Number)->bool {var source=a;let low=unsafe {Ops.min'intrinsic(source,b).as(Number)};source=0;assumes low<=source;true}",
            "fn wrong(a:Number,b:Number)->bool {var source=a;let low=unsafe {Ops.min'intrinsic(source,do {source=0;b}).as(Number)};assumes low<=source;true}",
        ] {
            let run = if body.contains("ensures") {
                "true"
            } else {
                "wrong(10,20)"
            };
            let w = Work::new(&format!("{header}{body}fn run()->bool {{{run}}}"));
            for level in ["0", "3"] {
                fs::write(w.0.join("native program"), b"previous").unwrap();
                let output = w
                    .command()
                    .args(args)
                    .args(["--opt-level", level])
                    .output()
                    .unwrap();
                // assumes is a runtime check, so use postconditions for static
                // rejection cases only; valid dynamic checks must remain admitted.
                if body.contains("ensures") {
                    assert!(!output.status.success());
                    assert!(
                        String::from_utf8_lossy(&output.stderr).contains("postcondition"),
                        "{output:?}"
                    );
                    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
                } else {
                    success(&output);
                    let output = w.execute();
                    assert!(
                        !output.status.success(),
                        "stale min relation removed the runtime check: {output:?}"
                    );
                    assert!(output.stdout.is_empty());
                    assert_eq!(
                        String::from_utf8_lossy(&output.stderr)
                            .matches("runtime-check-failure")
                            .count(),
                        1,
                        "{output:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn native_integer_minmax_selects_an_input_across_calls() {
    for ty in ["i8", "u8", "i128", "u128"] {
        for operation in ["min", "max"] {
            let header = format!(
                "{RESOURCE_HEADER}typealias Number=.{ty}'builtin;impl Duplicate for Number {{}}^intrinsic_enum enum Ops {{select}}"
            );
            let source=format!("{header}
                fn select(a:Number,b:Number)->r:Number
                where ensures r==a'old or r==b'old {{unsafe {{Ops.select'intrinsic(a,b).as(Number)}}}}
                fn forward(a:Number,b:Number)->r:Number
                where ensures r==a'old or r==b'old {{select(a,b)}}
                fn second(a:Number,b:Number)->r:Number where ensures r==b'old {{let selected=select(a,b);if selected==a {{b}} else {{selected}}}}
                fn run()->bool {{let a:Number=10;let b:Number=20;let r=forward(a,b);(r==a or r==b) and second(a,b)==b}}");
            let args = [
                "--build",
                "run",
                "--fuel",
                "2000000",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--host-intrinsic",
            ];
            let w = Work::new(&source);
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args(args)
                        .arg(format!("{operation}=input.Ops.select"))
                        .args(["--opt-level", level, "--output", "native program"])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, b"(bool true)\n");
            }
        }
    }
}

#[test]
fn integer_minmax_choice_rejects_false_contracts_and_stale_inputs() {
    for operation in ["min", "max"] {
        let header = format!(
            "{RESOURCE_HEADER}typealias Number=.i32'builtin;impl Duplicate for Number {{}}^intrinsic_enum enum Ops {{select}}"
        );
        for (body, entry) in [
            (
                "fn wrong(a:Number,b:Number)->r:Number where ensures r==a'old and r==b'old {unsafe {Ops.select'intrinsic(a,b).as(Number)}}",
                "true",
            ),
            (
                "fn wrong(a:Number,b:Number)->r:Number where ensures r==a'old or r==0 {unsafe {Ops.select'intrinsic(a,b).as(Number)}}",
                "true",
            ),
            (
                "fn wrong(a:Number,b:Number)->bool {var x=a;var y=b;let r=unsafe {Ops.select'intrinsic(x,y).as(Number)};x=0;y=0;assumes r==x or r==y;true}",
                "wrong(10,20)",
            ),
        ] {
            let w = Work::new(&format!("{header}{body}fn run()->bool {{{entry}}}"));
            for level in ["0", "3"] {
                fs::write(w.0.join("native program"), b"previous").unwrap();
                let output = w
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--fuel",
                        "1000000",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--host-intrinsic",
                        &format!("{operation}=input.Ops.select"),
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap();
                if entry == "true" {
                    assert!(!output.status.success());
                    assert!(
                        String::from_utf8_lossy(&output.stderr).contains("postcondition"),
                        "{output:?}"
                    );
                    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
                } else {
                    success(&output);
                    let output = w.execute();
                    assert!(!output.status.success());
                    assert!(output.stdout.is_empty());
                    assert_eq!(
                        String::from_utf8_lossy(&output.stderr)
                            .matches("runtime-check-failure")
                            .count(),
                        1,
                        "{output:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn shipped_recursive_data_uses_finite_pointer_layouts() {
    let source = include_str!("../../../development/examples/host-recursive-data.fl");
    for source in [
        source.to_owned(),
        source.replace(
            "pointer.flag.* = true;",
            "let copied=pointer.*; assumes not copied.flag; pointer.flag.* = true;",
        ),
    ] {
        let w = Work::new(&source);
        for level in ["0", "3"] {
            let output = w
                .command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap();
            success(&output);
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn recursive_data_cleanup_uses_logical_subject_layouts() {
    let w = Work::new(
        r#"
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {}
        enum Link {none, some(*Node)}
        impl Duplicate for Link {}
        struct Node {next:Link, done:*bool}
        impl Retire for Node {fn drop(*self) {self.done.*.* = true;}}
        fn run()->bool {
            var tail_done=false; var head_done=false;
            do {
                let tail=Node {next:Link.none,done:tail_done.ref};
                let head=Node {next:Link.some(tail.ref),done:head_done.ref};
            };
            tail_done and head_done
        }
    "#,
    );
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn opaque_host_pointers_do_not_merge_logical_pointee_types() {
    let w = Work::new(&format!(
        "{RESOURCE_HEADER}
        struct A {{flag:bool}} struct B {{flag:bool}}
        impl Duplicate for A {{}} impl Duplicate for B {{}}
        fn set(pointer:*A) {{pointer.flag.* = true;}}
        fn run()->bool {{var b=B {{flag:false}};set(b.ref);b.flag}}"
    ));
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let output = resource_build(&w);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("elaboration failed"),
        "{output:?}"
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn shipped_linked_list_reverses_empty_single_and_multiple_nodes() {
    let source = include_str!("../../../development/examples/host-linked-list.fl");
    for (length, start) in [
        (0u32, "Link.none"),
        (1, "Link.some(third.ref)"),
        (2, "Link.some(second.ref)"),
        (3, "Link.some(first.ref)"),
    ] {
        let source = source
            .replace(
                "var current = Link.some(first.ref);",
                &format!("var current = {start};"),
            )
            .replace(
                "var expected: U = 3;",
                &format!("var expected: U = {length};"),
            )
            .replace("value: 2", &format!("value: {}", length.saturating_sub(1)))
            .replace("value: 3", &format!("value: {length}"));
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n", "length {length}");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout, "length {length}, O{level}");
        }
    }
}

#[test]
fn linked_list_checks_order_and_does_not_hide_a_missing_link() {
    let source = include_str!("../../../development/examples/host-linked-list.fl");
    for (source, contract_failure) in [
        (source.replace("value: 3", "value: 4"), true),
        (
            source.replace("node.next.* = previous;", "node.next.* = Link.none;"),
            false,
        ),
    ] {
        let w = Work::new(&source);
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            if contract_failure {
                assert!(!output.status.success());
                assert!(output.stdout.is_empty());
                assert_eq!(
                    String::from_utf8_lossy(&output.stderr)
                        .matches("runtime-check-failure")
                        .count(),
                    1
                );
            } else {
                success(&output);
                assert_eq!(output.stdout, b"(bool false)\n");
            }
        }
    }
}

#[test]
fn native_loop_closures_reset_capture_state_and_retire_each_environment_once() {
    let source = r#"
        trait Duplicate {} trait Retire {fn drop(*self);}
        typealias U=.u32'builtin;
        impl Duplicate for bool {} impl Duplicate for U {}
        struct Watch {done:*U}
        impl Retire for Watch {fn drop(*self) {
            let n=self.done.*.*;
            assumes n < 100;
            self.done.*.* = n + 1;
        }}
        fn run()->bool {
            var count:U=0;
            var remaining:U=10;
            while remaining != 0 {
                var state=false;
                let watch=Watch {done:count.ref};
                let next=|| {watch;state=not state;state};
                if not next() {return false;}
                remaining = remaining - 1;
            };
            count == 10
        }
    "#;
    let w = Work::new(source);
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_block_local_declarations_and_modifiers() {
    for body in [
        "fn selected(value:bool)->bool {not value} selected(false)",
        "inline fn selected(value:bool)->bool {not value} selected(false)",
        "pure fn selected(value:bool)->bool {not value} selected(false)",
        "quote fn selected(value:bool)->bool {not value} selected(false)",
        "typealias Flag=bool; fn selected(value:Flag)->Flag {not value} selected(false)",
        "let first=do {inline fn selected(value:bool)->bool {not value} selected(false)}; let second=do {pure fn selected(value:bool)->bool {value} selected(true)}; first and second",
        "if true {inline fn selected(value:bool)->bool {not value} selected(false)} else {fn selected(value:bool)->bool {value} selected(false)}",
        "fn selected(value:bool)->bool {if value {true} else {selected(true)}} selected(false)",
        "let selected=false; do {fn selected(value:bool)->bool {not value} selected(false)}",
        "let selected=false; fn selected(value:bool)->bool {not value} not selected",
        "inline fn invoke(selected:bool)->bool {do {fn selected(value:bool)->bool {not value} selected(false)}} invoke(false)",
    ] {
        let source = format!("fn selected(value:bool)->bool {{value}} fn run()->bool {{{body}}}");
        let work = Work::new(&source);
        let interpreted = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        success(&interpreted);
        assert_eq!(interpreted.stdout, b"(bool true)\n", "{source}");
        for level in ["0", "3"] {
            let built = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            success(&built);
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n", "O{level}: {source}");
        }
    }
}

#[test]
fn native_block_local_generic_and_staged_functions() {
    for source in [
        "fn run()->bool {fn identity(value:T)->T where T {value} identity<bool>(true) and not identity<bool>(false)}",
        "fn run()->bool {pure comptime fn invert(value:bool)->bool {not value} const answer=invert(false);answer}",
        "fn outer(value:T)->T where T {fn identity(inner:T)->T {inner} identity(value)} fn run()->bool {outer<bool>(true) and not outer<bool>(false)}",
        "typealias U=.u32'builtin; fn outer(value:T)->T where T {fn identity(inner:T)->T {inner} identity(value)} fn run()->bool {outer<bool>(true) and outer<U>(42)==42}",
        "fn outer(value:T)->T where T {fn pass(inner:T)->T {fn identity(last:T)->T {last} identity(inner)} pass(value)} fn run()->bool {outer<bool>(true)}",
        "typealias U=.u32'builtin; fn outer(value:T)->T where T {fn local(ignored:V,inner:T)->T where V {inner} local<U>(42,value)} fn run()->bool {outer<bool>(true)}",
        "typealias U=.u32'builtin; fn maker()->fn(T)->T where T {fn local(value:T)->T {value} local} fn run()->bool {maker<bool>()(true) and not maker<bool>()(false) and maker<U>()(42)==42}",
        "fn outer(value:T)->bool where T, requires T==bool {fn local(inner:T)->bool {inner} local(value)} fn run()->bool {outer<bool>(true)}",
        "fn outer(value:T)->T where T {fn local(comptime tag:bool,inner:T)->T {inner} local(true,value)} fn run()->bool {outer<bool>(true)}",
        "fn outer(value:T)->T where T {quote fn local(inner:T)->T {inner} local(value)} fn run()->bool {outer<bool>(true)}",
        "trait Mark {} impl Mark for bool {} fn check(value:T)->bool where T:-Mark {true} fn outer(value:T)->bool where T:-Mark {fn local(inner:T)->bool {check<T>(inner)} local(value)} fn run()->bool {outer<bool>(true)}",
        "typealias Type=.Type'builtin; fn outer(value:T)->T where T {fn local(inner:V)->V where .V:Type=T {inner} local(value)} fn run()->bool {outer<bool>(true)}",
        "fn outer(value:T)->T where T {pure comptime fn invert(flag:bool)->bool {not flag} const yes=invert(false);if yes {value} else {value}} fn run()->bool {outer<bool>(true)}",
    ] {
        let work = Work::new(source);
        let output = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn lexical_function_inputs_cannot_be_reinferred_or_overridden() {
    for body in [
        "fn outer()->bool where T {fn local(inner:T)->bool {true} local(true)}",
        "fn outer()->bool where T {fn local(comptime tag:bool,inner:T)->bool {true} local(true,true)}",
        "fn outer()->bool where T {fn local(inner:T)->bool {true} local<bool>(true)}",
        "fn outer()->bool where T {fn local(inner:T)->bool {true} let callback:fn(bool)->bool=local;callback(true)}",
    ] {
        let source = format!("typealias U=.u32'builtin; {body} fn run()->bool {{outer<U>()}}");
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success(), "accepted: {source}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            !diagnostic.contains("not implemented")
                && !diagnostic.contains("unresolved declaration"),
            "{source}: {diagnostic}"
        );
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn native_block_local_captures_staged_parameter_inputs() {
    for source in [
        include_str!("../../../development/repros/block-local-staged-type-input.fl"),
        "typealias Type=.Type'builtin; typealias U=.u32'builtin; fn outer(comptime T:Type,value:T)->T {fn local(inner:T)->T {inner} local(value)} fn run()->bool {outer(bool,true) and outer(U,42)==42}",
        "fn outer(comptime flag:bool)->bool {fn local()->bool {flag} local()} fn run()->bool {outer(true) and not outer(false)}",
        "typealias Type=.Type'builtin; fn outer(comptime T:Type,comptime value:T)->T {fn local()->T {value} local()} fn run()->bool {outer(bool,true) and not outer(bool,false)}",
        "typealias Type=.Type'builtin; typealias U=.u32'builtin; fn factory(comptime T:Type)->fn(T)->T {fn local(value:T)->T {value} local} fn run()->bool {factory(bool)(true) and factory(U)(42)==42}",
        "typealias Type=.Type'builtin; fn outer(comptime T:Type,value:T)->T {fn middle(inner:T)->T {fn local(last:T)->T {last} local(inner)} middle(value)} fn run()->bool {outer(bool,true)}",
        "typealias Type=.Type'builtin; typealias U=.u32'builtin; fn outer(comptime T:Type)->bool {fn local(comptime T:Type,value:T)->T {value} local(U,42)==42} fn run()->bool {outer(bool)}",
        "typealias Type=.Type'builtin; fn outer(comptime T:Type,value:T)->bool where requires T==bool {fn local(inner:T)->bool {inner} local(value)} fn run()->bool {outer(bool,true)}",
        "typealias Type=.Type'builtin; trait Mark {} impl Mark for bool {} fn check(value:T)->bool where T:-Mark {true} fn outer(comptime T:Type,value:T)->bool where requires T:-Mark {fn local(inner:T)->bool {check<T>(inner)} local(value)} fn run()->bool {outer(bool,true)}",
        "fn factory(comptime flag:bool)->fn()->bool {fn local()->bool {flag} local} fn run()->bool {factory(true)() and not factory(false)()}",
        "fn outer(comptime flag:bool)->bool {comptime fn local()->bool {flag} const value=local();value} fn run()->bool {outer(true) and not outer(false)}",
        "typealias Type=.Type'builtin; trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {} struct Receiver {} impl Retire for Receiver {fn drop(*self) {}} impl Receiver {fn echo(self,comptime T:Type,value:T)->T {fn local(inner:T)->T {inner} local(value)}} fn run()->bool {Receiver {}.echo(bool,true)}",
    ] {
        let work = Work::new(source);
        let result = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let mut command = work.command();
            if source.contains("struct Receiver") {
                command.args(["--copy-protocol", "Duplicate", "--drop-protocol", "Retire"]);
            }
            let result = command
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let result = work.execute();
            success(&result);
            assert_eq!(result.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn block_local_staging_keeps_phase_and_fixed_input_boundaries() {
    for source in [
        "fn outer(flag:bool)->bool {fn local()->bool {flag} local()} fn run()->bool {outer(true)}",
        "typealias Type=.Type'builtin;typealias U=.u32'builtin; fn outer(comptime T:Type)->bool {fn local(inner:T)->bool {true} local(true)} fn run()->bool {outer(U)}",
        "typealias Type=.Type'builtin;typealias U=.u32'builtin; fn outer(comptime T:Type)->bool {fn local(inner:T)->bool {true} local<bool>(true)} fn run()->bool {outer(U)}",
        "fn outer(comptime flag:bool)->bool {fn local(value:flag)->bool {true} local(true)} fn run()->bool {outer(true)}",
    ] {
        let work = Work::new(source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success(), "accepted: {source}");
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
}

#[test]
fn native_lexical_staged_environment_survives_recursive_calls() {
    for source in [
        include_str!("../../../development/repros/block-local-staged-recursion.fl"),
        "typealias Type=.Type'builtin;typealias U=.u32'builtin;fn outer(comptime T:Type,value:T)->T {fn local(recur:bool,inner:T)->T {if recur {local(false,inner)} else {inner}} local(true,value)} fn run()->bool {outer(bool,true) and outer(U,42)==42}",
        "fn outer(comptime flag:bool)->bool {fn left(recur:bool)->bool {if recur {right(false)} else {flag}} fn right(recur:bool)->bool {if recur {left(false)} else {flag}} left(true) and right(true)} fn run()->bool {outer(true) and not outer(false)}",
        "typealias Type=.Type'builtin;typealias U=.u32'builtin;fn factory(comptime T:Type,comptime value:T)->fn(bool)->T {fn local(recur:bool)->T {if recur {local(false)} else {value}} local} fn run()->bool {factory(bool,true)(true) and factory(U,42)(true)==42}",
        "fn outer(comptime flag:bool)->bool {fn middle(comptime other:bool)->bool {fn local(recur:bool)->bool {if recur {local(false)} else {flag and other}} local(true)} middle(true)} fn run()->bool {outer(true) and not outer(false)}",
        "typealias Type=.Type'builtin;typealias U=.u32'builtin;fn outer(comptime T:Type,value:T)->T {fn left(inner:T)->T {let callback:fn(T)->T=right;callback(inner)} fn right(inner:T)->T {inner} left(value)} fn run()->bool {outer(bool,true) and outer(U,42)==42}",
        "typealias Type=.Type'builtin;typealias U=.u32'builtin;fn outer(comptime T:Type,value:T)->T {fn local(recur:bool,inner:T,extra:V)->T where V {if recur {local<V>(false,inner,extra)} else {inner}} local<bool>(true,value,true)} fn run()->bool {outer(bool,true) and outer(U,42)==42}",
        "fn outer(comptime flag:bool)->bool {comptime fn left(recur:bool)->bool {if recur {right(false)} else {flag}} comptime fn right(recur:bool)->bool {if recur {left(false)} else {flag}} const result=left(true);result} fn run()->bool {outer(true) and not outer(false)}",
    ] {
        let work = Work::new(source);
        let output = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn recursive_lexical_inputs_cannot_be_reinferred() {
    for body in [
        "fn local(recur:bool,inner:T)->T {if recur {local(false,true)} else {inner}} local(true,value)",
        "fn left(inner:T)->T {right(true)} fn right(inner:T)->T {inner} left(value)",
        "fn left(inner:T)->T {let callback:fn(bool)->bool=right;callback(true)} fn right(inner:T)->T {inner} left(value)",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;typealias U=.u32'builtin;fn outer(comptime T:Type,value:T)->T {{{body}}} fn run()->bool {{outer(U,42)==42}}"
        );
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success(), "accepted {source}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            !diagnostic.contains("panicked")
                && !diagnostic.contains("invalid binder")
                && !diagnostic.contains("unavailable at this use"),
            "{source}: {diagnostic}"
        );
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

// This is a temporary implementation-gap regression, not a language rejection.
// Replace the diagnostic assertions with execution checks once invocation
// environments for named local declarations are represented.
#[test]
fn ordinary_comptime_lexical_capture_reports_missing_invocation_environment() {
    for source in [
        include_str!("../../../development/repros/comptime-local-ordinary-input.fl"),
        "comptime fn outer(flag:bool)->bool {comptime fn middle()->bool {comptime fn local()->bool {flag} local()} middle()} fn run()->bool {const value=outer(true);value}",
        "comptime fn outer(flag:bool)->bool {comptime fn unused()->bool {flag} true} fn run()->bool {const value=outer(true);value}",
    ] {
        let work = Work::new(source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success(), "accepted {source}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("requires an invocation environment, which is not yet implemented"),
            "{source}: {diagnostic}"
        );
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn ordinary_comptime_inputs_remain_available_to_lambda_environments() {
    for source in [
        "comptime fn outer(flag:bool)->bool {let local=||flag;local()} fn run()->bool {const yes=outer(true);const no=outer(false);yes and not no}",
        "comptime fn outer(flag:bool)->bool {comptime fn local(flag:bool)->bool {flag} local(flag)} fn run()->bool {const yes=outer(true);const no=outer(false);yes and not no}",
    ] {
        let work = Work::new(source);
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn native_conditional_local_constants_select_success_and_failure() {
    for source in [
        "fn run()->bool {const true as found=true else {return false;};found}",
        "fn run()->bool {const true as found=false else {return true;};found}",
        "fn run()->bool {const found if found=true else {return false;};found}",
        "fn run()->bool {const found if not found=true else {return true;};found}",
        "comptime fn yes()->bool {true} fn run()->bool {const true as found=yes() else {return false;};found}",
        "fn run()->bool {let found=true;const true as found=false else {return found;};found}",
        "fn run()->bool {var active=true;while active {active=false;const true=false else {continue;};return false;} not active}",
        "fn run()->bool {var active=false;while true {const true=false else {active=true;break;};} active}",
        "comptime fn accept(flag:bool)->bool {flag} fn run()->bool {const found if accept(found)=true else {return false;};found}",
        "fn configured(comptime flag:bool)->bool {const true as found=flag else {return false;};found} fn run()->bool {configured(true) and not configured(false)}",
        "comptime fn accept(flag:bool)->bool {flag} fn configured(comptime flag:bool)->bool {const found if accept(found)=accept(flag) else {return false;};found} fn run()->bool {configured(true) and not configured(false)}",
        "fn run()->bool {const action if action()=do {var flag=false;|| {flag=not flag;flag}} else {return false;};const after=action();not after}",
        "comptime fn fail()->bool {fail()} fn run()->bool {const true if fail()=false else {return true;};false}",
        "fn run()->bool {const T if true=bool else {return false;};let flag:T=true;flag}",
    ] {
        let work = Work::new(source);
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn conditional_local_constants_preserve_phase_and_failure_scope() {
    for source in [
        "fn check(flag:bool)->bool {const value if flag=true else {return false;};value} fn run()->bool {check(true)}",
        "fn runtime(flag:bool)->bool {flag} fn run()->bool {const value if runtime(value)=true else {return false;};value}",
        "fn run()->bool {const value if value=true;value}",
        "fn run()->bool {const true=false else {true};true}",
        "fn run()->bool {const true as found=false else {return found;};found}",
        "fn run()->bool {const value if do {value=false;true}=true else {return false;};value}",
        "fn run()->bool {const true=true else {return 42;};true}",
    ] {
        let work = Work::new(source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success(), "accepted {source}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            !diagnostic.contains("panicked") && !diagnostic.contains("not yet implemented"),
            "{source}: {diagnostic}"
        );
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn conditional_local_constants_failure_preserves_runtime_cleanup() {
    let source = format!(
        "{RESOURCE_HEADER} struct Watch {{target:*bool}} impl Retire for Watch {{fn drop(*self) {{self.target.*.* = true;}}}} fn run()->bool {{var seen=false;while true {{let watch=Watch {{target:seen.ref}};const true=false else {{break;}};}};seen}}"
    );
    let work = Work::new(&source);
    for level in ["0", "3"] {
        let output = work
            .command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--opt-level",
                level,
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&output);
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn conditional_constants_keep_tuple_and_type_evidence() {
    for body in [
        "fn run()->bool {const (true as first,second)=(true,false) else {return false;};first and not second}",
        "fn run()->bool {const (true as first,second)=(false,true) else {return true;};first and second}",
        "fn run()->bool {const (first,second) if first and not second=(true,false) else {return false;};first and not second}",
        "fn run()->bool {const T if T==bool=bool else {return false;};let value:T=true;value}",
        "fn configured(comptime flag:bool)->bool {const (first,second) if first and not second=(flag,false) else {return false;};first} fn run()->bool {configured(true) and not configured(false)}",
        "fn configured(comptime flag:bool)->bool {const T if T==bool=bool else {return false;};let value:T=flag;value} fn run()->bool {configured(true) and not configured(false)}",
        "fn run()->bool {const (T,value) if T==bool and value=(bool,true) else {return false;};let checked:T=value;checked}",
        "fn configured(comptime flag:bool)->bool {const (T,value) if value=(bool,flag) else {return false;};let checked:T=value;checked} fn run()->bool {configured(true) and not configured(false)}",
        "pure comptime fn Identity<T> -> .Type'builtin {T} fn configured(comptime flag:bool)->bool {const T if T == Identity<bool> = Identity<bool> else {return false;};flag} fn run()->bool {configured(true)}",
    ] {
        let work = Work::new(&format!("{TUPLE_HEADER}\n{body}"));
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--meta-list-type",
                    "input.TupleElements",
                    "--tuple-type",
                    "input.Tuple",
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{body}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn constant_type_evidence_preserves_normal_form_identity() {
    for body in [
        "const T if true=Identity<bool> else {return false;};let value:T=true;flag",
        "const T if true=bool else {return false;};let number:.u32'builtin=1;let value:T=number;flag",
        "const T if T==bool=Identity<bool> else {return false;};flag",
    ] {
        let source = format!(
            "pure comptime fn Identity<T> -> .Type'builtin {{T}} fn configured(comptime flag:bool)->bool {{{body}}} fn run()->bool {{configured(true)}}"
        );
        let work = Work::new(&source);
        let output = work.build();
        assert!(!output.status.success(), "accepted {source}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            !diagnostic.contains("panicked")
                && !diagnostic
                    .contains("type expression requires comptime elaboration not yet implemented"),
            "{source}: {diagnostic}"
        );
        if body.contains("T==bool") {
            assert!(
                diagnostic.contains("TypeContractUnresolved"),
                "{diagnostic}"
            );
        }
    }
}

#[test]
fn conditional_constants_destructure_normal_form_inputs() {
    let prefix = "typealias Type=.Type'builtin; pure comptime fn Box<T,value:T>->Type {T} pure comptime fn Wrap<T>->Type {T}";
    for body in [
        "fn run()->bool {const Box<T,value> =Box<bool,true> else {return false;};let checked:T=value;checked}",
        "fn run()->bool {const Box<T,value> if value=Box<bool,true> else {return false;};let checked:T=value;checked}",
        "fn configured(comptime flag:bool)->bool {const Box<T,value> =Box<bool,true> else {return false;};let checked:T=value;checked and flag} fn run()->bool {configured(true) and not configured(false)}",
        "fn configured(comptime flag:bool)->bool {const Wrap<Box<T,value>> =Wrap<Box<bool,true>> else {return false;};let checked:T=value;checked and flag} fn run()->bool {configured(true) and not configured(false)}",
        "fn configured(comptime flag:bool)->bool {const Wrap<(Box<T,value>,_)> = Wrap<Tuple<Box<bool,true>,bool>> else {return false;};let checked:T=value;checked and flag} fn run()->bool {configured(true) and not configured(false)}",
    ] {
        let work = Work::new(&format!("{TUPLE_HEADER} {prefix} {body}"));
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--meta-list-type",
                    "input.TupleElements",
                    "--tuple-type",
                    "input.Tuple",
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{body}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn constant_normal_form_patterns_require_matching_heads_and_domains() {
    let prefix = "typealias Type=.Type'builtin; pure comptime fn Box<T,value:T>->Type {T} pure comptime fn Other<T,value:T>->Type {T} pure comptime fn Identity<T>->Type {T}";
    for body in [
        "const Box<T,value> = Other<bool,true> else {return false;};value",
        "const Box<T,value> = Identity<Box<bool,true>> else {return false;};value",
        "const Box<T,value> = Box<.u32'builtin,7> else {return false;};let wrong:T=true;flag",
    ] {
        let work = Work::new(&format!(
            "{prefix} fn configured(comptime flag:bool)->bool {{{body}}} fn run()->bool {{configured(true)}}"
        ));
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success(), "accepted {body}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn staged_constant_type_aliases_preserve_nominal_member_identity() {
    for body in [
        "fn configured(comptime flag:bool)->bool {const T=bool;let value:T=flag;value}",
        "struct Holder where T {pub typealias Payload=T;} fn configured(comptime flag:bool)->bool {const Container=Holder<bool>;let value:Container.Payload=flag;value}",
        "enum Holder where T {empty,pub typealias Payload=T;} fn configured(comptime flag:bool)->bool {const Container if true=Holder<bool> else {return false;};let value:Container.Payload=flag;value}",
        "struct Holder where T {pub typealias Payload=T;} fn configured(comptime flag:bool)->bool {const Container=Holder<bool>;const T=Container.Payload;let value:T=flag;value}",
        "newtype Wrapped<T> = T; fn configured(comptime flag:bool)->bool {const Container=Wrapped<bool>;const same=Container==Wrapped<bool>;same and flag}",
        "comptime fn schema(...properties:T)->T where T {properties} fn configured(comptime flag:bool)->bool {const record=schema {Payload:flag};record.Payload}",
        "struct Holder where flag:bool {pub const FLAG=flag;} fn configured(comptime flag:bool)->bool {const Container=Holder<flag>;const enabled=Container.FLAG;enabled}",
    ] {
        let work = Work::new(&format!(
            "{body} fn run()->bool {{configured(true) and not configured(false)}}"
        ));
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{body}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn constant_alias_of_staged_type_keeps_each_instance() {
    let work = Work::new(
        "typealias Type=.Type'builtin;typealias U=.u32'builtin;fn configured(comptime T:Type,value:T)->T {const Alias=T;let result:Alias=value;result} fn run()->bool {configured(bool,true) and configured(U,42)==42}",
    );
    for level in ["0", "3"] {
        let output = work
            .command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--opt-level",
                level,
            ])
            .output()
            .unwrap();
        success(&output);
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn staged_const_aliases_do_not_erase_nominal_or_normal_form_heads() {
    for declaration in [
        "newtype Wrapped<T> = T;",
        "pure comptime fn Wrapped<T> -> .Type'builtin {T}",
    ] {
        let work = Work::new(&format!(
            "{declaration} fn configured(comptime flag:bool)->bool {{const Alias=Wrapped<bool>;let wrong:Alias=true;flag}} fn run()->bool {{configured(true)}}"
        ));
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success(), "accepted {declaration}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            !diagnostic.contains("panicked")
                && !diagnostic
                    .contains("type expression requires comptime elaboration not yet implemented"),
            "{diagnostic}"
        );
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn local_type_aliases_preserve_member_visibility() {
    let work = Work::new(
        "mod hidden {pub struct Holder where T {private typealias Payload=T;}} fn configured(comptime flag:bool)->bool {const Container=hidden.Holder<bool>;let value:Container.Payload=flag;value} fn run()->bool {configured(true)}",
    );
    fs::write(work.0.join("native program"), b"previous output").unwrap();
    let output = work.build();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    assert_eq!(
        fs::read(work.0.join("native program")).unwrap(),
        b"previous output"
    );
}

#[test]
fn open_constant_formation_cannot_promote_runtime_inputs() {
    for initializer in ["Holder<flag>", "Holder<copy>"] {
        let source = format!(
            "struct Holder where flag:bool {{pub const FLAG=flag;}} fn configured(comptime mode:bool,flag:bool)->bool {{let copy=flag;const Container={initializer};const enabled=Container.FLAG;enabled}} fn run()->bool {{configured(true,true)}}"
        );
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success(), "accepted {source}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!diagnostic.contains("panicked"), "{diagnostic}");
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn local_type_aliases_select_nested_member_types() {
    let source = "struct Outer where T {pub struct Inner {pub typealias Payload=T;}} fn configured(comptime flag:bool)->bool {const Container=Outer<bool>;let value:Container.Inner.Payload=flag;value} fn run()->bool {configured(true) and not configured(false)}";
    let work = Work::new(source);
    for level in ["0", "3"] {
        let output = work
            .command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--opt-level",
                level,
            ])
            .output()
            .unwrap();
        success(&output);
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn nested_local_type_members_keep_parameters_and_value_fields() {
    for body in [
        "struct Outer where T {pub struct Inner where U {pub typealias Payload=U;}} fn configured(comptime flag:bool)->bool {const Container=Outer<.u32'builtin>;let value:Container.Inner<bool>.Payload=flag;value}",
        "struct Outer where T {pub struct Inner {pub typealias Payload=T;}} fn configured(comptime flag:bool)->bool {const Container=Outer<bool>;const Alias=Container.Inner.Payload;let value:Alias=flag;value}",
        "comptime fn record(...properties:T)->T where T {properties} fn configured(comptime flag:bool)->bool {const value=record {Inner:record {Payload:flag}};value.Inner.Payload}",
    ] {
        let work = Work::new(&format!(
            "{body} fn run()->bool {{configured(true) and not configured(false)}}"
        ));
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{body}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn nested_local_type_members_do_not_bypass_private_intermediates() {
    for member in [
        "private struct Inner {pub typealias Payload=T;}",
        "pub struct Inner {private typealias Payload=T;}",
    ] {
        let work = Work::new(&format!(
            "mod hidden {{pub struct Outer where T {{{member}}}}} fn configured(comptime flag:bool)->bool {{const Container=hidden.Outer<bool>;let value:Container.Inner.Payload=flag;value}} fn run()->bool {{configured(true)}}"
        ));
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn deep_local_type_member_chain_fits_default_budget() {
    let depth = 20;
    let mut source = String::from("struct Outer where T {");
    let mut path = String::from("Container");
    for index in 0..depth {
        source.push_str(&format!("pub struct Layer{index} {{"));
        path.push_str(&format!(".Layer{index}"));
    }
    source.push_str("pub typealias Payload=T;");
    source.push_str(&"}".repeat(depth + 1));
    for binding in [
        format!("let value:{path}.Payload=flag;"),
        format!("const Alias={path}.Payload;let value:Alias=flag;"),
    ] {
        let work = Work::new(&format!(
            "{source} fn configured(comptime flag:bool)->bool {{const Container=Outer<bool>;{binding}value}} fn run()->bool {{configured(true) and not configured(false)}}"
        ));
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            success(&output);
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn local_type_members_preserve_value_parameter_domains() {
    for body in [
        "struct Holder where flag:bool {pub const FLAG=flag;} fn configured(comptime flag:bool)->bool {const Container=Holder<true>;const result=Container.FLAG;result and flag}",
        "struct Outer where flag:bool {pub struct Inner {pub const FLAG=flag;}} fn configured(comptime flag:bool)->bool {const Container=Outer<flag>;const result=Container.Inner.FLAG;result}",
        "struct Outer where flag:bool {pub struct Inner where invert:bool {pub const FLAG=flag != invert;}} fn configured(comptime flag:bool)->bool {const Container=Outer<flag>;const result=Container.Inner<false>.FLAG;result}",
        "struct Holder where flag:bool {pub const FLAG=flag;} comptime fn Make(flag:bool)->.Type'builtin {Holder<flag>} fn configured(flag:bool)->bool {const Container=Make(true);const result=Container.FLAG;result and flag}",
    ] {
        let work = Work::new(&format!(
            "{body} fn run()->bool {{configured(true) and not configured(false)}}"
        ));
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{body}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn staged_type_factories_defer_member_selection_to_each_instance() {
    let source = include_str!("../../../development/repros/staged-type-factory-member.fl");
    let work = Work::new(source);
    for level in ["0", "3"] {
        let output = work
            .command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--opt-level",
                level,
            ])
            .output()
            .unwrap();
        success(&output);
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn staged_type_factory_members_keep_selected_identity_and_chains() {
    for source in [
        "struct Enabled {pub const FLAG=true;} struct Disabled {pub const FLAG=false;} comptime fn Make(flag:bool)->.Type'builtin {if flag {Enabled} else {Disabled}} fn configured(comptime flag:bool)->bool {const Container=Make(flag);const result=Container.FLAG;result} fn run()->bool {configured(true) and not configured(false)}",
        "struct Outer where flag:bool {pub struct Inner {pub const FLAG=flag;}} comptime fn Make(flag:bool)->.Type'builtin {Outer<flag>} fn configured(comptime flag:bool)->bool {const Container=Make(flag);const result=Container.Inner.FLAG;result} fn run()->bool {configured(true) and not configured(false)}",
        "struct Enabled {pub const FLAG=true;} struct Disabled {pub const FLAG=false;} fn configured(comptime Container:.Type'builtin)->bool {const result=Container.FLAG;result} fn run()->bool {configured(Enabled) and not configured(Disabled)}",
    ] {
        let work = Work::new(source);
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn staged_type_factory_members_recheck_access_domains_and_phase() {
    for source in [
        "mod hidden {pub struct Holder {private const FLAG=true;}} comptime fn Make()->.Type'builtin {hidden.Holder} fn configured(comptime flag:bool)->bool {const Container=Make();const result=Container.FLAG;result} fn run()->bool {configured(true)}",
        "struct Holder {} comptime fn Make()->.Type'builtin {Holder} fn configured(comptime flag:bool)->bool {const Container=Make();const result=Container.FLAG;result} fn run()->bool {configured(true)}",
        "struct Good {pub const FLAG=true;} struct Bad {pub const FLAG=3;} comptime fn Make(flag:bool)->.Type'builtin {if flag {Good} else {Bad}} fn configured(comptime flag:bool)->bool {const Container=Make(flag);const result=Container.FLAG;result} fn run()->bool {configured(true) and configured(false)}",
        "struct Holder where flag:bool {pub const FLAG=flag;} comptime fn Make(flag:bool)->.Type'builtin {Holder<flag>} fn configured(comptime mode:bool,flag:bool)->bool {const Container=Make(flag);const result=Container.FLAG;result} fn run()->bool {configured(true,true)}",
    ] {
        let work = Work::new(source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success(), "{source}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn staged_type_factory_members_supply_local_annotations() {
    let source = include_str!("../../../development/repros/staged-type-factory-annotation.fl");
    let work = Work::new(source);
    for level in ["0", "3"] {
        let output = work
            .command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--opt-level",
                level,
            ])
            .output()
            .unwrap();
        success(&output);
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn staged_member_annotations_keep_nested_and_parameter_receivers() {
    for source in [
        "struct Holder where T {pub struct Inner {pub typealias Payload=T;}} comptime fn Make(T:.Type'builtin)->.Type'builtin {Holder<T>} fn configured(comptime T:.Type'builtin,value:defer T)->defer T {const Container=Make(T);let result:Container.Inner.Payload=value;result} fn run()->bool {configured(bool,true) and configured(.u32'builtin,7)==7}",
        "struct Holder where T {pub typealias Payload=T;} fn configured(comptime Container:.Type'builtin,comptime T:.Type'builtin,value:defer T)->defer T {let result:Container.Payload=value;result} fn run()->bool {configured(Holder<bool>,bool,true) and configured(Holder<.u32'builtin>,.u32'builtin,7)==7}",
    ] {
        let work = Work::new(source);
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn staged_member_annotations_recheck_type_identity_and_access() {
    for member in [
        "pub typealias Payload=T;",
        "pub typealias Payload=.u32'builtin;",
        "private typealias Payload=T;",
        "pub const Payload=true;",
        "pub typealias Payload=Identity<T>;",
    ] {
        let work = Work::new(&format!(
            "pure comptime fn Identity<T> ->.Type'builtin {{T}} mod hidden {{pub struct Holder where T {{{member}}}}} comptime fn Make(T:.Type'builtin)->.Type'builtin {{hidden.Holder<T>}} fn configured(comptime T:.Type'builtin,value:defer T)->defer T {{const Container=Make(T);let result:Container.Payload=value;result}} fn run()->bool {{configured(bool,true)}}"
        ));
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        if member == "pub typealias Payload=T;" {
            success(&output);
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
            continue;
        }
        assert!(!output.status.success(), "{member}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn staged_type_factory_aliases_supply_local_annotations() {
    let base = include_str!("../../../development/repros/staged-type-factory-alias.fl");
    for source in [
        base.to_owned(),
        base.replace(
            "const Alias = Container.Payload;",
            "const First = Container.Payload; const Alias = First;",
        ),
        base.replace(
            "const Alias = Container.Payload;",
            "const Alias: .Type'builtin = Container.Payload;",
        ),
    ] {
        let work = Work::new(&source);
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn staged_type_factory_aliases_reject_non_types_and_wrong_identities() {
    let base = include_str!("../../../development/repros/staged-type-factory-alias.fl");
    for member in [
        "pub typealias Payload = .u32'builtin;",
        "pub const Payload = true;",
        "pub typealias Payload = Identity<T>;",
    ] {
        for typed in [false, true] {
            let mut source = format!(
                "pure comptime fn Identity<T> -> .Type'builtin {{T}} {}",
                base.replace("pub typealias Payload = T;", member)
            );
            if typed {
                source = source.replace("const Alias =", "const Alias: .Type'builtin =");
            }
            let work = Work::new(&source);
            fs::write(work.0.join("native program"), b"previous output").unwrap();
            let output = work.build();
            assert!(!output.status.success(), "{source}");
            assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
            assert_eq!(
                fs::read(work.0.join("native program")).unwrap(),
                b"previous output"
            );
        }
    }
}

#[test]
fn staged_type_members_compose_in_local_type_constructors() {
    let base = include_str!("../../../development/repros/staged-type-factory-alias.fl");
    let pointer = include_str!("../../../development/repros/staged-type-factory-pointer.fl");
    for source in [
        format!("{TUPLE_HEADER} {pointer}"),
        format!(
            "{TUPLE_HEADER} {}",
            base.replace(
                "let result: Alias = value;",
                "let result: Tuple<Alias,bool> = (value,true);"
            )
            .replace("    result\n", "    let (answer, _) = result; answer\n")
        ),
        format!(
            "{TUPLE_HEADER} {}",
            base.replace(
                "let result: Alias = value;",
                "let result: Tuple<Container.Payload,bool> = (value,true);"
            )
            .replace("    result\n", "    let (answer, _) = result; answer\n")
        ),
    ] {
        let work = Work::new(&source);
        for level in ["0", "3"] {
            let mut command = work.command();
            if source.contains("trait Duplicate") {
                command.args(["--copy-protocol", "Duplicate", "--drop-protocol", "Retire"]);
            }
            let output = command
                .args([
                    "--meta-list-type",
                    "input.TupleElements",
                    "--tuple-type",
                    "input.Tuple",
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn staged_pointer_annotations_recheck_copy_and_domains() {
    let base = include_str!("../../../development/repros/staged-type-factory-pointer.fl");
    let end = base.find("fn run()").unwrap();
    let body = base[..end].replace(" where requires T:-Duplicate", "");
    for copy in [true, false] {
        let implementation = if copy {
            "impl Duplicate for Owner {}"
        } else {
            ""
        };
        let source = format!(
            "struct Owner {{flag:bool}} {implementation} {body} fn run()->bool {{let value=configured(Owner,Owner {{flag:true}});let {{flag:answer}}=value;answer}}"
        );
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args([
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--build",
                "run",
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        if copy {
            success(&output);
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        } else {
            assert!(!output.status.success());
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(!diagnostic.contains("panicked"), "{diagnostic}");
            assert!(
                diagnostic.to_ascii_lowercase().contains("copy"),
                "{diagnostic}"
            );
            assert_eq!(
                fs::read(work.0.join("native program")).unwrap(),
                b"previous output"
            );
        }
    }
    let work = Work::new(&base.replace(
        "pub typealias Payload = T;",
        "pub typealias Payload = .u32'builtin;",
    ));
    let output = work
        .command()
        .args([
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--build",
            "run",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
}

#[test]
fn staged_type_parameter_members_trigger_composite_body_checking() {
    for source in [
        include_str!("../../../development/repros/staged-parameter-composite-annotation.fl").to_owned(),
        "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {} struct Holder {pub typealias Payload=bool;} fn flip(value:bool)->bool {not value} fn configured(comptime Container:.Type'builtin,callback:fn(bool)->bool,value:bool)->bool {let action:fn(Container.Payload)->Container.Payload=callback;action(value)} fn run()->bool {configured(Holder,flip,false) and not configured(Holder,flip,true)}".to_owned(),
        "trait Splice {assoc RuntimeTarget:.Type'builtin;fn splice(comptime data:Self)->RuntimeTarget;} impl Splice for bool {assoc RuntimeTarget:.Type'builtin=bool;fn splice(comptime data:bool)->bool {data}} trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {} struct Enabled {pub const FLAG=true;} struct Disabled {pub const FLAG=false;} fn configured(comptime Container:.Type'builtin)->bool {Container.FLAG} fn run()->bool {configured(Enabled) and not configured(Disabled)}".to_owned(),
    ] {
        let work = Work::new(&source);
        for level in ["0", "3"] {
            let mut command = work.command();
            if source.contains("trait Splice") {
                command.args(["--splice-protocol", "Splice"]);
            }
            let output = command.args(["--copy-protocol", "Duplicate", "--drop-protocol", "Retire", "--build", "run", "--output", "native program", "--opt-level", level]).output().unwrap();
            assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn staged_parameter_member_instances_recheck_callbacks_and_stage_transfers() {
    for body in [
        "struct Good {pub typealias Payload=bool;} struct Bad {pub typealias Payload=.u32'builtin;} fn flip(value:bool)->bool {not value} fn configured(comptime Container:.Type'builtin,callback:fn(bool)->bool,value:bool)->bool {let action:fn(Container.Payload)->Container.Payload=callback;action(value)} fn run()->bool {configured(Good,flip,false) and configured(Bad,flip,false)}",
        "mod hidden {pub struct Holder {private typealias Payload=bool;}} fn configured(comptime Container:.Type'builtin,value:*bool)->bool {let pointer:*Container.Payload=value;pointer.*} fn run()->bool {let value=true;configured(hidden.Holder,value.ref)}",
        "struct Holder {pub const FLAG=true;} fn configured(comptime Container:.Type'builtin)->bool {Container.FLAG} fn run()->bool {configured(Holder)}",
    ] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}} impl Duplicate for bool {{}} {body}"
        );
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args([
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--build",
                "run",
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{body}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!diagnostic.contains("panicked"), "{diagnostic}");
        if body.contains("const FLAG") {
            assert!(diagnostic.contains("Splice"), "{diagnostic}");
        }
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn staged_generic_type_members_resume_parameters_and_projections() {
    let base = include_str!("../../../development/repros/staged-generic-type-member.fl");
    for source in [
        base.to_owned(),
        base.replace("Inner where T", "Inner where T, enabled:bool").replace("Inner<bool>", "Inner<bool,flag>"),
        base.replace("Inner where T", "Inner where T, .U: .Type'builtin = T").replace("Payload = T", "Payload = U").replace("Inner<bool>", "Inner<.char'builtin,.U=bool>"),
        base.replace("let value: Container.Inner<bool>.Payload = flag;", "const Selected: .Type'builtin = Container.Inner<bool>; let value:Selected.Payload=flag;"),
    ] {
        let work = Work::new(&source);
        for level in ["0", "3"] {
            let output = work.command().args(["--build", "run", "--output", "native program", "--opt-level", level]).output().unwrap();
            assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn staged_generic_type_members_recheck_routing_formation_and_visibility() {
    let base = include_str!("../../../development/repros/staged-generic-type-member.fl");
    let private = base
        .replace("struct Outer {", "mod hidden {pub struct Outer {")
        .replace("pub struct Inner", "private struct Inner")
        .replace("comptime fn Make()", "} comptime fn Make()")
        .replace("{Outer}", "{hidden.Outer}");
    for source in [
        base.replace("Inner<bool>", "Inner<bool,bool>"),
        base.replace("Inner<bool>", "Inner<.Missing=bool>"),
        format!(
            "trait Mark {{}} {}",
            base.replace("Inner where T", "Inner where T:-Mark")
        ),
        private,
        base.replace("Inner where T", "Inner where T, enabled:bool")
            .replace("Inner<bool>", "Inner<bool,flag>")
            .replace(
                "configured(comptime flag: bool)",
                "configured(comptime mode:bool, flag: bool)",
            )
            .replace("configured(true)", "configured(true,true)")
            .replace("configured(false)", "configured(true,false)"),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success(), "{source}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn inferred_generic_member_aliases_keep_pending_provenance() {
    let base = include_str!("../../../development/repros/staged-generic-member-inferred-alias.fl");
    for source in [
        base.to_owned(),
        base.replace("const Selected = Container.Inner<bool>;", "const First = Container.Inner<bool>; const Selected=First;"),
        base.replace("const Selected = Container.Inner<bool>;", "const First = Container.Inner<bool>; const Selected=First; do {const Selected=false; Selected;};"),
        base.replace("Inner where T {pub typealias Payload = T;}", "Inner where T {pub struct Next where U {pub typealias Payload=U;}}")
            .replace("Selected.Payload", "Selected.Next<bool>.Payload"),
    ] {
        let work = Work::new(&source);
        for level in ["0", "3"] {
            let output = work.command().args(["--build", "run", "--output", "native program", "--opt-level", level]).output().unwrap();
            assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn inferred_member_provenance_does_not_grant_types_or_access() {
    let base = include_str!("../../../development/repros/staged-generic-member-inferred-alias.fl");
    let private = base
        .replace("struct Outer {", "mod hidden {pub struct Outer {")
        .replace("pub struct Inner", "private struct Inner")
        .replace("comptime fn Make()", "} comptime fn Make()")
        .replace("{Outer}", "{hidden.Outer}");
    for source in [
        base.replace("Payload = T", "Payload = .u32'builtin"),
        private,
        base.replace("pub struct Inner where T {pub typealias Payload = T;}", "pub fn inner()->bool where T {true}")
            .replace("Container.Inner<bool>", "Container.inner<bool>"),
        base.replace("let value: Selected.Payload = flag;", "do {const Selected=true;let bad:Selected.Payload=flag;};let value:Selected.Payload=flag;"),
        base.replace("const Selected = Container.Inner<bool>;", "const Selected = true;"),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success(), "{source}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
        assert_eq!(fs::read(work.0.join("native program")).unwrap(), b"previous output");
    }
}

#[test]
fn deeply_nested_open_unary_operators_compile_and_run_natively() {
    let source = format!(
        "fn choose(comptime flag:bool,value:bool)->bool {{{}(do {{inline if flag {{value}} else {{false}}}})}} fn run()->bool {{choose(true,true) and not choose(false,true)}}",
        "not ".repeat(64)
    );
    let work = Work::new(&source);
    for level in ["0", "3"] {
        let output = work
            .command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--opt-level",
                level,
            ])
            .output()
            .unwrap();
        success(&output);
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn conditional_member_aliases_resume_every_selected_branch() {
    let base = include_str!("../../../development/repros/staged-generic-member-inferred-alias.fl")
        .replace("pub struct Inner where T {pub typealias Payload = T;}",
            "pub struct Inner where T {pub typealias Payload=T;} pub struct Other where T {pub typealias Payload=T;}");
    // Shared alias branches form a DAG, not an exponentially expanded tree.
    let mut shared = String::from("do {const A0=Container.Inner<bool>;");
    for index in 1..=24 {
        shared.push_str(&format!(
            "const A{index}=if flag {{A{previous}}} else {{A{previous}}};",
            previous = index - 1
        ));
    }
    shared.push_str("A24}");
    for selection in [
        shared.as_str(),
        "if flag {Container.Inner<bool>} else {Container.Other<bool>}",
        "if flag {const Local=Container.Inner<bool>;Local} else {const Local=Container.Other<bool>;Local}",
        "if flag {const A=Container.Inner<bool>;const B=A;B} else {do {const A=Container.Other<bool>;A}}",
        "when {flag=>Container.Inner<bool>,else=>Container.Other<bool>}",
        "flag match {true=>Container.Inner<bool>,false=>Container.Other<bool>}",
        "do {if flag {do {Container.Inner<bool>}} else {Container.Other<bool>}}",
        "if flag {Container.Inner<bool>} else {if not flag {Container.Other<bool>} else {Container.Inner<bool>}}",
    ] {
        let source = base.replace(
            "const Selected = Container.Inner<bool>;",
            &format!("const Selected = {selection};"),
        );
        let work = Work::new(&source);
        let interpreted = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        assert!(
            interpreted.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&interpreted.stderr)
        );
        assert_eq!(interpreted.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn conditional_member_aliases_do_not_invent_branch_evidence() {
    let base = include_str!("../../../development/repros/staged-generic-member-inferred-alias.fl");
    for selection in [
        "if flag {Container.Inner<bool>} else {Container.Inner<.u32'builtin>}",
        "if flag {Container.Inner<bool>} else {true}",
        "when {flag=>Container.Inner<bool>,else=>true}",
        "flag match {true=>Container.Inner<bool>,false=>true}",
        "if flag {Container.Inner<bool>}",
        "flag match {true=>Container.Inner<bool>}",
        "do {const Container=true;Container.Inner<bool>}",
        "if flag {const Local=Container.Inner<bool>;Local} else {const Local=true;Local}",
        "if flag {const Local=Container.Inner<bool>;Local} else {Local}",
    ] {
        let source = base.replace(
            "const Selected = Container.Inner<bool>;",
            &format!("const Selected = {selection};"),
        );
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        assert!(!output.status.success(), "{source}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn staged_tuple_aliases_keep_member_sources_across_branch_scopes() {
    let base = include_str!("../../../development/repros/staged-member-branch-tuple-alias.fl");
    let binding = "const (Local, unused) = (Container, flag);";
    for replacement in [
        binding,
        "const ((Local, unused), last) = ((Container, flag), false);",
        "const packet=(Container,flag);const saved=packet;const (Local,unused)=saved;",
        "const ((Local, unused) as packet, last) = ((Container,flag),false);",
    ] {
        let source = base.replace(binding, replacement);
        let work = Work::new(&source);
        let arguments = [
            "--meta-list-type",
            "input.Elements",
            "--tuple-type",
            "input.Tuple",
        ];
        let output = work
            .command()
            .args(arguments)
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work
                .command()
                .args(arguments)
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn staged_tuple_aliases_preserve_shape_domains_and_constructor_admission() {
    let base = include_str!("../../../development/repros/staged-member-branch-tuple-alias.fl");
    for source in [
        base.replace("(Container, flag); Local", "(flag, Container); Local"),
        base.replace("(Local, unused)", "(Local, unused, extra)"),
        base.replace("(Container, flag); Local", "(true, false); Local"),
        base.replace(
            "const (Local, unused) = (Container, flag);",
            "do {const (Local, unused)=(Container, flag);};",
        ),
        base.replace("Payload = T", "Payload = .u32'builtin"),
        base.replace(
            "<...Es:Elements<.Type'builtin>>",
            "<Es:Elements<.Type'builtin>>",
        ),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args([
                "--meta-list-type",
                "input.Elements",
                "--tuple-type",
                "input.Tuple",
                "--build",
                "run",
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{source}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn guarded_tuple_member_aliases_keep_successful_sources() {
    let base = include_str!("../../../development/repros/staged-member-guarded-tuple-alias.fl");
    for source in [
        base.to_owned(),
        base.replace("(Local, unused) if unused", "(Local, true as unused)"),
        base.replace("const (Local, unused) if unused", "const action=do {var ready=false;|| {ready=not ready;ready}};const (Local, unused) if action()")
            .replace("Local.Inner<bool>", "if action() {Container.Inner<.u32'builtin>} else {Local.Inner<bool>}"),

    ] {
        let work = Work::new(&source);
        let protocols = ["--meta-list-type", "input.Elements", "--tuple-type", "input.Tuple"];
        let output = work.command().args(protocols).args(["--interpret", "run"]).output().unwrap();
        assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output=work.command().args(protocols).args(["--build", "run", "--output", "native program", "--opt-level",level]).output().unwrap();
            assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
            let output=work.execute();
            success(&output);
            assert_eq!(output.stdout,b"(bool true)\n");
        }
    }
}

#[test]
fn guarded_tuple_member_aliases_keep_guard_and_failure_checks() {
    let base = include_str!("../../../development/repros/staged-member-guarded-tuple-alias.fl");
    for (source, exhausted) in [
        (base.replace("if unused =", "if not unused ="), true),
        (base.replace("else {while true {}}", "else {false}"), false),
        (base.replace("if unused =", "if Container ="), false),
        (
            base.replace(
                "else {while true {}}",
                "else {let forbidden=Local;while true {}}",
            ),
            false,
        ),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args([
                "--meta-list-type",
                "input.Elements",
                "--tuple-type",
                "input.Tuple",
                "--build",
                "run",
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{source}");
        assert!(!diagnostic.contains("panicked"), "{diagnostic}");
        if exhausted {
            assert!(
                diagnostic.to_ascii_lowercase().contains("exhaust"),
                "{diagnostic}"
            );
        }
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn required_comptime_return_boundaries_are_reported_without_publishing_output() {
    let nested = include_str!("../../../development/repros/staged-member-guarded-return.fl");
    for source in [
        nested.to_owned(),
        "fn run()->bool {const value=do {return false;true};value}".into(),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let mut command = work.command();
        if source == nested {
            command.args([
                "--meta-list-type",
                "input.Elements",
                "--tuple-type",
                "input.Tuple",
            ]);
        }
        let output = command
            .args(["--build", "run", "--output", "native program"])
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success());
        // Pending cross-stage control contract: test the diagnostic and output
        // protection, not a claim that lexical returns are fully implemented.
        assert!(
            diagnostic.contains("return across a required comptime boundary is not supported"),
            "{diagnostic}"
        );
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn comptime_owned_returns_and_runtime_const_else_keep_their_targets() {
    for source in [
        "comptime fn invert(value:bool)->bool {return not value;} fn run()->bool {const value=invert(false);value}",
        "fn run()->bool {const value=do {const action=|| {return true;};action()};value}",
        "fn configured(comptime flag:bool)->bool {const true=flag else {return false;};true} fn run()->bool {configured(true) and not configured(false)}",
    ] {
        let work = Work::new(source);
        let output = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn partial_comptime_tuple_evidence_preserves_known_fields_and_invocation_values() {
    let base = include_str!("../../../development/repros/comptime-partial-type-packet.fl");
    for source in [
        base.to_owned(),
        base.replace("(Outer, flag)", "(bool, flag)").replace("Selected.Inner<bool>.Payload", "Selected"),
        base.replace("(Selected, value) = (Outer, flag)", "((Selected, value) as packet, last) = ((Outer, flag), flag)"),
        base.replace("(Selected, value) =", "(Selected, value) if value =").replace("(Outer, flag);", "(Outer, flag) else {return false;};"),
        base.replace("(Selected, value) =", "(Selected, true as value) or (Selected, false as value) ="),
        base.replace("const (Selected, value) = (Outer, flag);", "const action=do {var ready=false;|| {ready=not ready;ready}};const (Selected, value) = (Outer, action());").replace("    checked\n", "    checked and not action() and flag\n"),
    ] {
        let work = Work::new(&source);
        let protocols = ["--meta-list-type", "input.Elements", "--tuple-type", "input.Tuple"];
        let output = work.command().args(protocols).args(["--interpret", "run"]).output().unwrap();
        assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, b"(bool true)\n", "{source}");
        for level in ["0", "3"] {
            let output = work.command().args(protocols).args(["--build", "run", "--output", "native program", "--opt-level", level]).output().unwrap();
            assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n", "{source}");
        }
    }
}

#[test]
fn partial_comptime_tuple_evidence_does_not_invent_types_or_skip_checks() {
    let base = include_str!("../../../development/repros/comptime-partial-type-packet.fl");
    for source in [
        base.replace("(Outer, flag)", "(flag, Outer)"),
        base.replace("(Outer, flag)", "(Outer, flag, flag)"),
        base.replace("Payload = T", "Payload = .u32'builtin"),
        base.replace(
            "comptime fn configured",
            "pure comptime fn Identity<T> -> .Type'builtin {T} comptime fn configured",
        )
        .replace("(Outer, flag)", "(Identity<bool>, flag)")
        .replace("Selected.Inner<bool>.Payload", "Selected"),
        base.replace("(Selected, value) =", "(Selected, value) if Selected ="),
        base.replace("(Selected, value) =", "(Selected, value) if value =")
            .replace("(Outer, flag);", "(Outer, flag) else {false};"),
        base.replace(
            "(Selected, value) =",
            "(Selected, _, true as value) or (_, Selected, false as value) =",
        )
        .replace("(Outer, flag)", "(Outer, bool, flag)"),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args([
                "--meta-list-type",
                "input.Elements",
                "--tuple-type",
                "input.Tuple",
                "--build",
                "run",
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{source}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("elaboration failed"),
            "{source}: {diagnostic}"
        );
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn partial_comptime_record_and_variant_fields_keep_type_identity() {
    let record = include_str!("../../../development/repros/comptime-partial-record-packet.fl");
    let variant = include_str!("../../../development/repros/comptime-partial-variant-packet.fl");
    for source in [
        record.to_owned(),
        record.replace("struct Packet {selected: .Type'builtin, value: bool}", "struct Packet where T {selected: .Type'builtin = T, value: bool}")
            .replace("Packet {selected: Outer, value: flag}", "Packet<Outer> {value: flag}"),
        record.replace("{selected: Selected, value} =", "{value, selected: Selected} ="),
        record.replace("Packet {selected: Outer, value: flag}", "Packet {value: flag, selected: Outer}"),
        variant.to_owned(),
        variant.replace("empty, pair", "other(bool, .Type'builtin), pair")
            .replace("const .pair(Selected, value)", "const .other(value, Selected) or .pair(Selected, value)"),
        variant.replace("comptime fn configured", "struct Envelope {body: Packet} comptime fn configured")
            .replace("const .pair(Selected, value) = Packet.pair(Outer, flag)", "const {body: .pair(Selected, value)} = Envelope {body: Packet.pair(Outer, flag)}"),
        record.replace("struct Packet {selected: .Type'builtin, value: bool}", "struct Packet {first: bool, selected: .Type'builtin, last: bool}")
            .replace("const {selected: Selected, value} = Packet {selected: Outer, value: flag};", "const action=do {var ready=false;|| {ready=not ready;ready}};const {selected: Selected, first, last} = Packet {last: action(), selected: Outer, first: action()};")
            .replace("= value;", "= first and not last and flag;"),
        record.replace("struct Packet {selected: .Type'builtin, value: bool}", "struct Packet {tag: bool, selected: .Type'builtin, other: .Type'builtin, value: bool}")
            .replace("const {selected: Selected, value}", "const {tag: false, other: Selected, value} or {tag: true, selected: Selected, value}")
            .replace("Packet {selected: Outer, value: flag}", "Packet {tag: true, selected: Outer, other: bool, value: flag}"),
        variant.replace("const .pair(Selected, value)", "const .pair(Selected, value) if value"),
    ] {
        let work = Work::new(&source);
        let output = work.command().args(["--interpret", "run"]).output().unwrap();
        assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work.command().args(["--build", "run", "--output", "native program", "--opt-level", level]).output().unwrap();
            assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn partial_comptime_records_preserve_candidate_and_field_checks() {
    let record = include_str!("../../../development/repros/comptime-partial-record-packet.fl");
    let variant = include_str!("../../../development/repros/comptime-partial-variant-packet.fl");
    for source in [
        record.replace("Payload = T", "Payload = .u32'builtin"),
        record.replace("selected: Outer, value: flag", "selected: flag, value: flag"),
        record.replace("{selected: Selected, value} =", "{missing: Selected, value} ="),
        record.replace("struct Packet {selected: .Type'builtin, value: bool}", "struct Packet {selected: .Type'builtin, other: .Type'builtin, value: bool}")
            .replace("const {selected: Selected, value}", "const {selected: Selected, value: true as value} or {other: Selected, value: false as value}")
            .replace("Packet {selected: Outer, value: flag}", "Packet {selected: Outer, other: bool, value: flag}"),
        variant.replace("const .pair(Selected, value)", "const .pair(Selected, value) if Selected"),
        variant.replace("else {return false;}", "else {false}"),
        variant.replace("else {return false;}", "else {let forbidden=Selected;return false;}"),
        variant.replace("comptime fn configured", "enum Other {pair(.Type'builtin, bool)} comptime fn configured")
            .replace("const .pair(Selected, value)", "const Other.pair(Selected, value)"),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.build();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{source}");
        if source.contains("let forbidden=Selected") {
            assert!(diagnostic.contains("Unresolved identifier `Selected`"), "{diagnostic}");
        } else {
            assert!(diagnostic.contains("elaboration failed"), "{source}: {diagnostic}");
        }
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(fs::read(work.0.join("native program")).unwrap(), b"previous output");
    }
}

#[test]
fn partial_comptime_packet_aliases_preserve_binding_snapshots() {
    let tuple = include_str!("../../../development/repros/comptime-partial-packet-alias.fl");
    let record = include_str!("../../../development/repros/comptime-partial-record-packet.fl");
    let variant = include_str!("../../../development/repros/comptime-partial-variant-packet.fl");
    for source in [
        tuple.to_owned(),
        tuple.replace("const packet = ((Outer, flag), flag);", "var matched=true;inline for index in 0..2 {const T=inline if index==0 {bool} else {.str'builtin};const packet=(T,flag);const saved=packet;const (Selected,unused)=saved;matched=matched and Selected==T;};const packet = ((Outer, flag), flag);")
            .replace("    checked\n", "    checked and matched\n"),
        tuple.replace("const packet = ((Outer, flag), flag);", "var changed=flag;const packet=((Outer,changed),flag);changed=not changed;")
            .replace("    checked\n", "    checked and (changed == not flag)\n"),
        tuple.replace("const packet = ((Outer, flag), flag);", "var changed=flag;let pointer=changed.ref;const packet=((Outer,pointer.*),flag);pointer.* = not flag;")
            .replace("    checked\n", "    checked and (pointer.* == not flag)\n"),
        tuple.replace("const packet = ((Outer, flag), flag);\n    const saved = packet;\n    const (inner, unused) = saved;", "const (inner, _) or (_, inner) = ((Outer, flag), (Outer, not flag));"),
        tuple.replace("const saved = packet;", "do {const packet=(bool, not flag);const (Selected, value)=packet;let checked:Selected=value;checked;};const saved = packet;"),
        tuple.replace("const saved = packet;", "const ((_, _) as saved_inner, _) as saved = packet;").replace("const (Selected, value) = inner;", "const (Selected, value) = saved_inner;"),
        record.replace("const {selected: Selected, value} = Packet {selected: Outer, value: flag};", "const packet=Packet {selected: Outer, value: flag};const saved=packet;const {selected: Selected, value}=saved;"),
        variant.replace("const .pair(Selected, value) = Packet.pair(Outer, flag)", "const packet=Packet.pair(Outer, flag);const saved=packet;const .pair(Selected, value) = saved"),
        tuple.replace("const packet = ((Outer, flag), flag);", "const action=do {var ready=false;|| {ready=not ready;ready}};const packet = ((Outer, action()), flag);")
            .replace("    checked\n", "    checked and not action() and flag\n"),
    ] {
        let work = Work::new(&source);
        let protocols = if source.contains("fn Tuple") {vec!["--meta-list-type", "input.Elements", "--tuple-type", "input.Tuple"]} else {vec![]};
        let output = work.command().args(&protocols).args(["--interpret", "run"]).output().unwrap();
        assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, b"(bool true)\n", "{source}");
        for level in ["0", "3"] {
            let output = work.command().args(&protocols).args(["--build", "run", "--output", "native program", "--opt-level", level]).output().unwrap();
            assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n", "{source}");
        }
    }
}

#[test]
fn partial_packet_aliases_keep_nominal_identity_and_scope_checks() {
    let base = include_str!("../../../development/repros/comptime-partial-packet-alias.fl");
    for source in [
        base.replace("Payload = T", "Payload = .u32'builtin"),
        base.replace(
            "comptime fn configured",
            "pure comptime fn Identity<T> -> .Type'builtin {T} comptime fn configured",
        )
        .replace("((Outer, flag), flag)", "((Identity<bool>, flag), flag)")
        .replace("Selected.Inner<bool>.Payload", "Selected"),
        base.replace(
            "const saved = packet;",
            "const saved = ((flag, Outer), flag);",
        ),
        base.replace("const saved = packet;", "do {const saved=packet;};"),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args([
                "--meta-list-type",
                "input.Elements",
                "--tuple-type",
                "input.Tuple",
                "--build",
                "run",
                "--output",
                "native program",
            ])
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{source}");
        if source.contains("do {const saved") {
            assert!(
                diagnostic.contains("Unresolved identifier `saved`"),
                "{diagnostic}"
            );
        } else {
            assert!(diagnostic.contains("elaboration failed"), "{diagnostic}");
        }
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn tuple_enumeration_preserves_observed_type_fields() {
    let base = include_str!("../../../development/repros/comptime-type-packet-enumeration.fl");
    for source in [
        base.to_owned(),
        base.replace("let checked:Selected=value", "let checked:T=seed"),
        base.replace("inline for (index, (T, seed)) in ((bool,true),(.str'builtin,\"value\"))'tuple_enumerate", "const input=((bool,flag),(.str'builtin,\"value\"));const saved_input=input;inline for (index,(T,seed)) in saved_input'tuple_enumerate"),
        base.replace("inline for (index, (T, seed)) in ((bool,true),(.str'builtin,\"value\"))'tuple_enumerate", "const action=do {var ready=false;|| {ready=not ready;ready}};const input=((bool,action()),(.str'builtin,\"value\"));const saved_input=input;inline for (index,(T,seed)) in saved_input'tuple_enumerate")
            .replace("    checked\n", "    checked and not action()\n"),
    ] {
        let work = Work::new(&source);
        let options = ["--meta-list-type", "input.Elements", "--tuple-type", "input.Tuple"];
        let output = work.command().args(options).args(["--interpret", "run"]).output().unwrap();
        assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work.command().args(options).args(["--build", "run", "--output", "native program", "--opt-level", level]).output().unwrap();
            assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn tuple_type_observations_preserve_storage_and_expire_on_escape() {
    let base = include_str!("../../../development/repros/comptime-type-packet-enumeration.fl");
    let header = format!(
        "{RESOURCE_HEADER} impl Duplicate for .Type'builtin {{}} impl Duplicate for .str'builtin {{}}"
    );
    let source = format!("{header}{}", base
        .replace("comptime fn configured(flag: bool) -> bool {", "comptime fn configured(flag: bool) -> bool {var intact=true;")
        .replace("let checked:Selected=value;checked;", "let checked:Selected=value;let pointer=T.ref;let old=pointer.*;pointer.* = .u32'builtin;intact=intact and old==Selected and T==.u32'builtin and old!=T;checked;")
        .replace("    checked\n", "    checked and intact\n"));
    let options = [
        "--meta-list-type",
        "input.Elements",
        "--tuple-type",
        "input.Tuple",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
    ];
    for source in [
        &source,
        &source
            .replace(
                "let pointer=T.ref;let old=pointer.*;",
                "let read=||T;let pointer=T.ref;let old=read();",
            )
            .replace("and old!=T", "and old!=T and read()==old"),
    ] {
        let work = Work::new(&source);
        let output = work
            .command()
            .args(options)
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work
                .command()
                .args(options)
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
    // A write precedes the const packet. Reusing the original Type snapshot
    // would wrongly accept bool/str payloads as values of the new u32 type.
    let bad = format!(
        "{header}{}",
        base.replace(
            "{const packet=((T,seed),flag);",
            "{let pointer=T.ref;pointer.* = .u32'builtin;const packet=((T,seed),flag);"
        )
    );
    assert_ne!(bad, format!("{header}{base}"));
    let work = Work::new(&bad);
    fs::write(work.0.join("native program"), b"previous output").unwrap();
    let output = work
        .command()
        .args(options)
        .args(["--build", "run", "--output", "native program"])
        .output()
        .unwrap();
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "accepted stale Type: {bad}");
    assert!(diagnostic.contains("elaboration failed"), "{diagnostic}");
    assert!(!diagnostic.contains("panicked"));
    assert_eq!(
        fs::read(work.0.join("native program")).unwrap(),
        b"previous output"
    );
    let missing_copy = source.replace("impl Duplicate for .Type'builtin {}", "");
    assert_ne!(missing_copy, source);
    let work = Work::new(&missing_copy);
    let output = work
        .command()
        .args(options)
        .args(["--build", "run", "--output", "native program"])
        .output()
        .unwrap();
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(diagnostic.contains("Copy"), "{diagnostic}");
}

#[test]
#[cfg(target_os = "linux")]
fn native_fixture_launch_retries_busy_writer_with_a_bounded_deadline() {
    let work = Work::new("fn run()->bool {true}");
    success(&work.build());
    let hold_writer = || {
        fs::OpenOptions::new()
            .write(true)
            .open(work.0.join("native program"))
            .unwrap()
    };
    let writer = hold_writer();
    let error = Command::new(work.0.join("native program"))
        .output()
        .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(26));
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(20));
        drop(writer);
    });
    let output = work.execute();
    release.join().unwrap();
    success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");

    let writer = hold_writer();
    let started = std::time::Instant::now();
    assert!(std::panic::catch_unwind(|| work.execute()).is_err());
    assert!(started.elapsed() >= std::time::Duration::from_secs(1));
    drop(writer);
    success(&work.execute());
}

#[test]
fn partial_field_projections_keep_known_types_and_runtime_siblings() {
    let base = include_str!("../../../development/repros/comptime-partial-field-projection.fl");
    for source in [
        base.to_owned(),
        base.replace(
            "selected: Outer, value: flag",
            "selected: Outer, value: true",
        )
        .replace("    checked\n", "    checked and flag\n"),
        base.replace(
            "Selected.Inner<bool>.Payload",
            "saved.packet.selected.Inner<bool>.Payload",
        ),
        base.replace("selected: Outer", "selected: bool")
            .replace("Selected.Inner<bool>.Payload", "saved.packet.selected"),
        base.replace(
            "const Selected = saved.packet.selected;",
            "const inner=saved.packet;const alias=inner;const Selected=alias.selected;",
        ),
        base.replace(
            "packet: Packet {selected: Outer, value: flag}, untouched: flag",
            "untouched: flag, packet: Packet {value: flag, selected: Outer}",
        ),
        base.replace(
            "struct Packet {selected: .Type'builtin, value: bool}",
            "struct Packet where T {selected: .Type'builtin = T, value: bool}",
        )
        .replace("packet: Packet,", "packet: Packet<Outer>,")
        .replace(
            "Packet {selected: Outer, value: flag}",
            "Packet<Outer> {value: flag}",
        ),
        base.replace(
            "const packet = Envelope",
            "const action=do {var ready=false;|| {ready=not ready;ready}};const packet = Envelope",
        )
        .replace(
            "selected: Outer, value: flag",
            "selected: Outer, value: action()",
        )
        .replace("    checked\n", "    checked and not action() and flag\n"),
    ] {
        let work = Work::new(&source);
        let output = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn partial_field_observations_preserve_storage_and_reject_stale_types() {
    let base = include_str!("../../../development/repros/comptime-partial-field-projection.fl");
    let header = format!("{RESOURCE_HEADER} impl Duplicate for .Type'builtin {{}}");
    let source = format!("{header}{}", base
        .replace("const Selected = saved.packet.selected;", "const Selected=saved.packet.selected;let pointer=saved.packet.selected.ref;pointer.* = bool;")
        .replace("    checked\n", "    checked and saved.packet.selected==bool and Selected!=saved.packet.selected and packet.packet.selected==Outer\n"));
    let work = Work::new(&source);
    let options = ["--copy-protocol", "Duplicate", "--drop-protocol", "Retire"];
    let output = work
        .command()
        .args(options)
        .args(["--interpret", "run"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{source}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let output = work
            .command()
            .args(options)
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--opt-level",
                level,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
    for bad in [
        base.replace("const Selected = saved.packet.selected;", "let pointer=saved.packet.selected.ref;pointer.* = bool;const Selected=saved.packet.selected;"),
        base.replace("Selected.Inner<bool>.Payload", "saved.packet.selected.Inner<bool>.Payload")
            .replace("const Selected = saved.packet.selected;", "let pointer=saved.packet.selected.ref;pointer.* = bool;"),
        base.replace("Payload = T", "Payload = .u32'builtin"),
        base.replace("const Selected = saved.packet.selected;", "const Selected=saved.packet.value;"),
        base.replace("const Selected = saved.packet.selected;", "const Selected=saved.packet.missing;"),
        base.replace("comptime fn configured", "pure comptime fn Identity<T> -> .Type'builtin {T} comptime fn configured")
            .replace("selected: Outer", "selected: Identity<bool>")
            .replace("Selected.Inner<bool>.Payload", "saved.packet.selected"),
    ] {
        let work = Work::new(&format!("{header}{bad}"));
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work.command().args(options).args(["--build", "run", "--output", "native program"]).output().unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{bad}");
        assert!(diagnostic.contains("elaboration failed"), "{bad}: {diagnostic}");
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(fs::read(work.0.join("native program")).unwrap(), b"previous output");
    }
}

#[test]
fn record_field_names_do_not_claim_metatype_namespaces() {
    let base = include_str!("../../../development/repros/comptime-record-meta-field-names.fl");
    for source in [
        base.to_owned(),
        base.replace("comptime fn configured", "struct Wrapper where T {item: T} comptime fn configured")
            .replace("let checked: packet.vars.Closure = packet.vars.value;", "const wrapped=Wrapper<Packet> {item:packet};let checked:wrapped.item.vars.Closure=wrapped.item.vars.value;"),
        base.replace("let checked: packet.vars.Closure = packet.vars.value;", "const alias=packet;let checked:alias.vars.Closure=alias.vars.value;"),
        base.replace("struct Fields", "struct MetaPacket {Meta: .Type'builtin, flag:bool} struct Fields")
            .replace("let checked: packet.vars.Closure = packet.vars.value;", "const meta=MetaPacket {Meta:.Type'builtin,flag:flag};let callback=|context:meta.Meta.vars.Closure|true;let checked:packet.vars.Closure=packet.vars.value;"),
        "typealias Type=.Type'builtin;fn invoke(f:%c fn()->bool)->bool where c:Type.vars.Closure {f()} fn run()->bool {let enabled=true;invoke(||enabled)}".into(),
        "const Meta=.Type'builtin;fn invoke(f:%c fn()->bool)->bool where c:Meta.vars.Closure {f()} fn run()->bool {let enabled=true;invoke(||enabled)}".into(),
    ] {
        let work = Work::new(&source);
        let output = work.command().args(["--interpret", "run"]).output().unwrap();
        assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work.command().args(["--build", "run", "--output", "native program", "--opt-level", level]).output().unwrap();
            assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn record_metatype_field_names_do_not_bypass_type_checks() {
    let base = include_str!("../../../development/repros/comptime-record-meta-field-names.fl");
    for source in [
        base.replace("Closure: bool", "Closure: .u32'builtin"),
        base.replace("Closure: .Type'builtin", "Closure: bool")
            .replace("Closure: bool, value: flag", "Closure: true, value: flag"),
        base.replace("packet.vars.Closure =", "packet.vars.missing ="),
        base.replace(
            "comptime fn configured",
            "pure comptime fn Identity<T> -> .Type'builtin {T} comptime fn configured",
        )
        .replace("Closure: bool", "Closure: Identity<bool>"),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args(["--build", "run", "--output", "native program"])
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{source}");
        assert!(
            diagnostic.contains("elaboration failed"),
            "{source}: {diagnostic}"
        );
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn returned_record_type_fields_complete_required_computation() {
    let base = include_str!("../../../development/repros/comptime-returned-type-field.fl");
    for source in [
        base.to_owned(),
        base.replace("packet().selected", "do {packet()}.selected"),
        base.replace("packet().selected", "(if true {packet()} else {packet()}).selected"),
        base.replace("struct Packet", "struct Wrapper where T {item:T} struct Packet")
            .replace("packet().selected", "do {Wrapper<Packet> {item:packet()}}.item.selected"),
        base.replace("comptime fn packet()", "comptime fn packet(comptime flag:bool)")
            .replace("selected: bool, value: true", "selected: if flag {bool} else {.u32'builtin}, value: flag")
            .replace("packet().selected", "packet(true).selected"),
        base.replace("packet().selected", "do {var flag=false;let make=||{flag=not flag;Packet {selected:bool,value:flag}};let first=make();let second=make();Packet {selected:if first.value and not second.value {bool} else {.u32'builtin},value:true}}.selected"),
    ] {
        let work = Work::new(&source);
        let output = work.command().args(["--interpret", "run"]).output().unwrap();
        assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work.command().args(["--build", "run", "--output", "native program", "--opt-level", level]).output().unwrap();
            assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn returned_type_fields_enforce_stage_and_identity() {
    let base = include_str!("../../../development/repros/comptime-returned-type-field.fl");
    for source in [
        base.replace("selected: bool", "selected: .u32'builtin"),
        base.replace("packet().selected", "packet().value"),
        base.replace("packet().selected", "packet().missing"),
        base.replace("comptime fn packet", "fn packet"),
        base.replace("fn run()", "fn run(value:bool)").replace(
            "packet().selected",
            "do {Packet {selected:if value {bool} else {.u32'builtin},value:value}}.selected",
        ),
        base.replace(
            "comptime fn packet",
            "pure comptime fn Identity<T> -> .Type'builtin {T} comptime fn packet",
        )
        .replace("selected: bool", "selected: Identity<bool>"),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args(["--build", "run", "--output", "native program"])
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{source}");
        assert!(
            diagnostic.contains("elaboration failed"),
            "{source}: {diagnostic}"
        );
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn computed_receivers_distinguish_data_fields_from_metatype_namespaces() {
    let base = include_str!("../../../development/repros/comptime-returned-meta-field-names.fl");
    for source in [
        base.to_owned(),
        base.replace("packet().vars.Closure", "do {packet()}.vars.Closure"),
        base.replace("struct Fields", "struct Wrapper where T {item:T} struct Fields")
            .replace("packet().vars.Closure", "do {Wrapper<Packet> {item:packet()}}.item.vars.Closure"),
        base.replace("packet().vars.Closure", "do {var flag=false;let make=||{flag=not flag;Packet {vars:Fields {Closure:bool,value:flag}}};let first=make();let second=make();Packet {vars:Fields {Closure:if first.vars.value and not second.vars.value {bool} else {.u32'builtin},value:true}}}.vars.Closure"),
        base.replace("packet().vars.Closure", "(if true {packet()} else {packet()}).vars.Closure"),
        "comptime fn Meta()->.Type'builtin {.Type'builtin} fn invoke(f:%c fn()->bool)->bool where c:Meta().vars.Closure {f()} fn run()->bool {let flag=true;invoke(||flag)}".into(),
        "fn invoke(f:%c fn()->bool)->bool where c:do {.Type'builtin}.vars.Closure {f()} fn run()->bool {let flag=true;invoke(||flag)}".into(),
    ] {
        let work=Work::new(&source);
        let output=work.command().args(["--interpret","run"]).output().unwrap();
        assert!(output.status.success(),"{source}: {}",String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout,b"(bool true)\n");
        for level in ["0","3"] {
            let output=work.command().args(["--build","run","--output","native program","--opt-level",level]).output().unwrap();
            assert!(output.status.success(),"{source}: {}",String::from_utf8_lossy(&output.stderr));
            let output=work.execute();success(&output);assert_eq!(output.stdout,b"(bool true)\n");
        }
    }
}

#[test]
fn computed_metatype_paths_preserve_stage_and_type_identity_checks() {
    let base = include_str!("../../../development/repros/comptime-returned-meta-field-names.fl");
    for source in [
        base.replace("Closure: bool", "Closure: .u32'builtin"),
        base.replace("Closure: .Type'builtin", "Closure: bool").replace("Closure: bool, value: true", "Closure: true, value: true"),
        base.replace("comptime fn packet", "fn packet"),
        "comptime fn Meta()->.Type'builtin {bool} fn invoke(f:%c fn()->bool)->bool where c:Meta().vars.Closure {f()} fn run()->bool {invoke(||true)}".into(),
        "fn Meta()->.Type'builtin {.Type'builtin} fn invoke(f:%c fn()->bool)->bool where c:Meta().vars.Closure {f()} fn run()->bool {invoke(||true)}".into(),
        "pure comptime fn Identity<T> -> .Type'builtin {T} comptime fn Meta()->.Type'builtin {Identity<.Type'builtin>} fn invoke(f:%c fn()->bool)->bool where c:Meta().vars.Closure {f()} fn run()->bool {invoke(||true)}".into(),
    ] {
        let work=Work::new(&source);
        fs::write(work.0.join("native program"),b"previous output").unwrap();
        let output=work.command().args(["--build","run","--output","native program"]).output().unwrap();
        let diagnostic=String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(),"{source}");
        assert!(diagnostic.contains("elaboration failed"),"{source}: {diagnostic}");
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(fs::read(work.0.join("native program")).unwrap(),b"previous output");
    }
}

#[test]
fn returned_type_fields_select_nested_generic_members() {
    let source = include_str!("../../../development/repros/comptime-returned-type-member.fl");
    for source in [
        source.to_owned(),
        source
            .replace(
                "struct Packet",
                "struct Envelope {packet:Packet} struct Packet",
            )
            .replace(
                "comptime fn packet() -> Packet {Packet {selected: Outer}}",
                "comptime fn packet() -> Envelope {Envelope {packet:Packet {selected:Outer}}}",
            )
            .replace("packet().selected", "packet().packet.selected"),
        source
            .replace(
                "struct Packet {selected: .Type'builtin}",
                "struct Packet where T {selected: T}",
            )
            .replace(
                "-> Packet {Packet",
                "-> Packet<.Type'builtin> {Packet<.Type'builtin>",
            ),
        source
            .replace("let checked:", "const make=packet;let checked:")
            .replace("packet().selected", "make().selected"),
    ] {
        let work = Work::new(&source);
        let output = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn returned_type_member_selection_preserves_domains_and_visibility() {
    let base = include_str!("../../../development/repros/comptime-returned-type-member.fl");
    for source in [
        base.replace("struct Outer", "mod hidden {pub struct Outer")
            .replace("struct Packet", "} struct Packet")
            .replace("pub struct Inner", "private struct Inner")
            .replace("selected: Outer", "selected: hidden.Outer"),
        base.replace("struct Outer", "mod hidden {pub struct Outer")
            .replace("struct Packet", "} struct Packet")
            .replace("pub typealias Payload", "private typealias Payload")
            .replace("selected: Outer", "selected: hidden.Outer"),
        base.replace("Payload=T", "Payload=.u32'builtin"),
        base.replace("selected: Outer", "selected: bool"),
        base.replace("comptime fn packet", "fn packet"),
        base.replace(".Inner<bool>", ".Missing<bool>"),
        base.replace(".Inner<bool>", ".Inner<true>"),
        base.replace(
            "comptime fn packet",
            "pure comptime fn Identity<T> -> .Type'builtin {T} comptime fn packet",
        )
        .replace("selected: Outer", "selected: Identity<Outer>"),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args(["--build", "run", "--output", "native program"])
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{source}");
        assert!(
            diagnostic.contains("elaboration failed"),
            "{source}: {diagnostic}"
        );
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn generic_call_results_instantiate_type_field_domains() {
    let source =
        include_str!("../../../development/repros/comptime-generic-returned-type-member.fl");
    for source in [
        source.to_owned(),
        source.replace("packet(Outer)", "packet<.Type'builtin>(Outer)"),
        source.replace("packet(Outer)", "packet {Outer}"),
        source.replace("packet(value:T)", "packet(seed:T,.value:T=seed)"),
        source.replace("packet(value:T)", "packet(seed:T,.value:T=seed)").replace("packet(Outer)", "packet(Outer,.value=Outer)"),
        source.replace("let checked:", "const make=packet<.Type'builtin>;let checked:").replace("packet(Outer)", "make(Outer)"),
        source.replace("struct Packet", "struct Other {pub struct Inner where T {pub typealias Payload=.str'builtin;}} struct Packet")
            .replace("    checked\n", "    let alternate:packet(Other).selected.Inner<bool>.Payload=\"other\";checked and alternate==\"other\"\n"),
    ] {
    let work=Work::new(&source);
    let output=work.command().args(["--interpret","run"]).output().unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout,b"(bool true)\n");
    for level in ["0","3"] {
        let output=work.command().args(["--build","run","--output","native program","--opt-level",level]).output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
        let output=work.execute();success(&output);assert_eq!(output.stdout,b"(bool true)\n");
    }
}
}

#[test]
fn generic_returned_type_fields_reject_invalid_call_inputs_and_identities() {
    let base = include_str!("../../../development/repros/comptime-generic-returned-type-member.fl");
    for source in [
        base.replace("packet(Outer)", "packet(true)"),
        base.replace("packet(Outer)", "packet<bool>(Outer)"),
        base.replace("packet(Outer)", "packet {missing:Outer}"),
        base.replace("Payload=T", "Payload=.u32'builtin"),
        base.replace("comptime fn packet", "fn packet"),
        base.replace("fn run()", "fn run(input:.Type'builtin)")
            .replace("packet(Outer)", "packet(input)"),
        base.replace(
            "comptime fn packet",
            "pure comptime fn Identity<T> -> .Type'builtin {T} comptime fn packet",
        )
        .replace("packet(Outer)", "packet(Identity<Outer>)"),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args(["--build", "run", "--output", "native program"])
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{source}");
        assert!(
            diagnostic.contains("elaboration failed"),
            "{source}: {diagnostic}"
        );
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn composite_receivers_preserve_type_field_member_selection() {
    let base = include_str!("../../../development/repros/comptime-block-type-member.fl");
    for source in [
        base.to_owned(),
        base.replace("do {packet()}", "do {const saved=packet();saved}"),
        base.replace("do {packet()}", "(if true {packet()} else {packet()})"),
        base.replace("do {packet()}", "when {true=>packet(),else=>packet()}"),
        base.replace("do {packet()}", "(true match {true=>packet(),false=>packet()})"),
        "struct Data {value:bool} fn get(flag:bool)->Data {Data {value:flag}} fn run()->bool {var flag=false;let first=do {flag=not flag;get(flag)}.value;let second=do {flag=not flag;get(flag)}.value;first and not second and not flag}".into(),
    ] {
        let work=Work::new(&format!("{RESOURCE_HEADER}{source}"));
        let output=work.command().args(["--copy-protocol","Duplicate","--drop-protocol","Retire"]).args(["--interpret","run"]).output().unwrap();
        assert!(output.status.success(),"{source}: {}",String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout,b"(bool true)\n");
        for level in ["0","3"] {
            let output=work.command().args(["--copy-protocol","Duplicate","--drop-protocol","Retire"]).args(["--build","run","--output","native program","--opt-level",level]).output().unwrap();
            assert!(output.status.success(),"{source}: {}",String::from_utf8_lossy(&output.stderr));
            let output=work.execute();success(&output);assert_eq!(output.stdout,b"(bool true)\n");
        }
    }
}

#[test]
fn composite_type_field_probes_do_not_relax_stage_or_member_checks() {
    let base = include_str!("../../../development/repros/comptime-block-type-member.fl");
    for source in [
        base.replace("Payload=T", "Payload=.u32'builtin"),
        base.replace("selected: Outer", "selected: bool"),
        base.replace("comptime fn packet", "fn packet"),
        base.replace(".Inner<bool>", ".Inner<true>"),
        base.replace("fn run()", "fn run(flag:bool)")
            .replace("do {packet()}", "(if flag {packet()} else {packet()})"),
        base.replace("struct Outer", "mod hidden {pub struct Outer")
            .replace("struct Packet", "} struct Packet")
            .replace("pub struct Inner", "private struct Inner")
            .replace("selected: Outer", "selected: hidden.Outer"),
        base.replace(
            "comptime fn packet",
            "pure comptime fn Identity<T> -> .Type'builtin {T} comptime fn packet",
        )
        .replace("selected: Outer", "selected: Identity<Outer>"),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args(["--build", "run", "--output", "native program"])
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{source}");
        assert!(
            diagnostic.contains("elaboration failed"),
            "{source}: {diagnostic}"
        );
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn composite_type_values_select_nested_generic_members() {
    let base = include_str!("../../../development/repros/comptime-composite-type-member.fl");
    for source in [
        base.to_owned(),
        base.replace("pub struct Inner", "pub const FLAG=true;pub struct Inner")
            .replace("    checked\n", "    checked and do {Outer}.FLAG\n"),
        base.replace("pub struct Inner", "pub const FLAG=true;pub struct Inner")
            .replace("struct Outer", "struct Other {pub const FLAG=false;} struct Outer")
            .replace("    checked\n", "    checked and do {var flag=false;flag=not flag;if flag {Outer} else {Other}}.FLAG\n"),
        base.replace("do {Outer}", "do {const saved=Outer;saved}"),
        base.replace("do {Outer}", "(if true {Outer} else {Outer})"),
        base.replace("do {Outer}", "when {false=>Outer,else=>Outer}"),
        base.replace("do {Outer}", "(true match {true=>Outer,false=>Outer})"),
        base.replace(
            "struct Outer",
            "comptime fn Make()->.Type'builtin {Outer} struct Outer",
        )
        .replace("do {Outer}", "do {Make()}"),
    ] {
        let work = Work::new(&format!("{RESOURCE_HEADER} trait Splice {{assoc RuntimeTarget:.Type'builtin;fn splice(comptime data:Self)->RuntimeTarget;}} impl Splice for bool {{assoc RuntimeTarget:.Type'builtin=bool;fn splice(comptime data:bool)->bool {{data}}}} {source}"));
        let output = work
            .command().args(["--copy-protocol","Duplicate","--drop-protocol","Retire","--splice-protocol","Splice"])
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work
                .command().args(["--copy-protocol","Duplicate","--drop-protocol","Retire","--splice-protocol","Splice"])
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn composite_type_receivers_preserve_stage_visibility_and_normal_form_heads() {
    let base = include_str!("../../../development/repros/comptime-composite-type-member.fl");
    for source in [
        base.replace("pub struct Inner", "pub const FLAG=true;pub struct Inner")
            .replace("fn run() -> bool {", "fn run() -> bool {var flag=false;")
            .replace(
                "    checked\n",
                "    let member=do {flag=not flag;Outer}.FLAG;checked and member and flag\n",
            ),
        base.replace("Payload=T", "Payload=.u32'builtin"),
        base.replace("do {Outer}", "do {true}"),
        base.replace(".Inner<bool>", ".Inner<true>"),
        base.replace("fn run()", "fn run(flag:bool)")
            .replace("do {Outer}", "(if flag {Outer} else {Outer})"),
        base.replace(
            "struct Outer",
            "fn Make()->.Type'builtin {Outer} struct Outer",
        )
        .replace("do {Outer}", "do {Make()}"),
        base.replace("struct Outer", "mod hidden {pub struct Outer")
            .replace("fn run()", "} fn run()")
            .replace("pub struct Inner", "private struct Inner")
            .replace("do {Outer}", "do {hidden.Outer}"),
        base.replace(
            "struct Outer",
            "pure comptime fn Identity<T> -> .Type'builtin {T} struct Outer",
        )
        .replace("do {Outer}", "do {Identity<Outer>}"),
    ] {
        let work = Work::new(&format!(
            "{RESOURCE_HEADER} trait Splice {{assoc RuntimeTarget:.Type'builtin;fn splice(comptime data:Self)->RuntimeTarget;}} impl Splice for bool {{assoc RuntimeTarget:.Type'builtin=bool;fn splice(comptime data:bool)->bool {{data}}}} {source}"
        ));
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args([
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--splice-protocol",
                "Splice",
            ])
            .args(["--build", "run", "--output", "native program"])
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{source}");
        assert!(
            diagnostic.contains("elaboration failed"),
            "{source}: {diagnostic}"
        );
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn computed_type_receivers_call_static_functions() {
    let base = include_str!("../../../development/repros/comptime-computed-static-method.fl");
    for source in [
        base.to_owned(),
        base.replace("pub fn invert(value:bool)->bool {not value}", "pub fn identity(value:T)->T where T {value}").replace(".invert(false)", ".identity(true)"),
        base.replace("pub fn invert(value:bool)->bool {not value}", "pub fn identity(value:T)->T where T {value}").replace(".invert(false)", ".identity<bool>(true)"),
        base.replace("fn run()->bool {do {Ops}.invert(false)}", "fn run()->bool {let action=do {Ops}.invert;action(false)}"),
        base.replace("fn run()->bool {do {Ops}.invert(false)}", "fn run()->bool {var flag=false;let answer=do {Ops}.invert(do {flag=not flag;not flag});answer and flag}"),
        base.replace("pub fn invert", "pub comptime fn invert").replace("fn run()->bool {do {Ops}.invert(false)}", "fn run()->bool {const answer=do {Ops}.invert(false);answer}"),
        base.replace("struct Ops", "struct Other {pub fn invert(value:bool)->bool {value}} comptime fn Make(comptime flag:bool)->.Type'builtin {if flag {Ops} else {Other}} struct Ops")
            .replace("do {Ops}.invert(false)", "Make(true).invert(false) and not Make(false).invert(false)"),
        base.replace("do {Ops}", "(if true {Ops} else {Ops})"),
        base.replace("do {Ops}", "(when {false=>Ops,else=>Ops})"),
        base.replace("do {Ops}", "(true match {true=>Ops,false=>Ops})"),
        base.replace("struct Ops", "comptime fn Make()->.Type'builtin {Ops} struct Ops").replace("do {Ops}", "Make()"),
        base.replace("struct Ops", "struct Packet {selected:.Type'builtin} comptime fn packet()->Packet {Packet {selected:Ops}} struct Ops").replace("do {Ops}", "packet().selected"),
    ] {
        let work=Work::new(&source);
        let output=work.command().args(["--interpret","run"]).output().unwrap();
        assert!(output.status.success(),"{source}: {}",String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout,b"(bool true)\n");
        for level in ["0","3"] {
            let output=work.command().args(["--build","run","--output","native program","--opt-level",level]).output().unwrap();
            assert!(output.status.success(),"{source}: {}",String::from_utf8_lossy(&output.stderr));
            let output=work.execute();success(&output);assert_eq!(output.stdout,b"(bool true)\n");
        }
    }
}

#[test]
fn computed_static_method_selection_checks_stage_access_and_arguments() {
    let base = include_str!("../../../development/repros/comptime-computed-static-method.fl");
    for source in [
        base.replace(".invert(false)", ".invert(\"wrong\")"),
        base.replace("do {Ops}", "do {true}"),
        base.replace(".invert(false)", ".missing(false)"),
        base.replace("fn run()", "fn run(flag:bool)")
            .replace("do {Ops}", "(if flag {Ops} else {Ops})"),
        base.replace("struct Ops", "fn Make()->.Type'builtin {Ops} struct Ops")
            .replace("do {Ops}", "Make()"),
        base.replace("struct Ops", "mod hidden {pub struct Ops")
            .replace("pub fn invert", "private fn invert")
            .replace("fn run()", "} fn run()")
            .replace("do {Ops}", "do {hidden.Ops}"),
        base.replace(
            "struct Ops",
            "pure comptime fn Identity<T> -> .Type'builtin {T} struct Ops",
        )
        .replace("do {Ops}", "do {Identity<Ops>}"),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args(["--build", "run", "--output", "native program"])
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{source}");
        assert!(
            diagnostic.contains("elaboration failed"),
            "{source}: {diagnostic}"
        );
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn computed_type_receivers_invoke_contextual_code_methods() {
    let base = include_str!("../../../development/repros/comptime-computed-code-method.fl");
    for source in [
        base.to_owned(),
        base.replace("do {Ops}", "(if true {Ops} else {Ops})"),
        base.replace("do {Ops}", "(when {false=>Ops,else=>Ops})"),
        base.replace("do {Ops}", "(true match {true=>Ops,false=>Ops})"),
        base.replace("struct Ops", "comptime fn Make()->.Type'builtin {Ops} struct Ops").replace("do {Ops}", "Make()"),
        base.replace("struct Ops", "struct Packet {selected:.Type'builtin} comptime fn packet()->Packet {Packet {selected:Ops}} struct Ops").replace("do {Ops}", "packet().selected"),
        "trait Builder {comptime fn build()-><|-bool>;} struct Ops {} impl Builder for Ops {comptime fn build()-><|-bool> {'(true)}} fn run()->bool {~<do {Ops}.as(Builder).build()>}".into(),
        "struct Ops {pub comptime fn pattern()-><bool -| value:bool> {'<value>}} fn run()->bool {const pat:<bool -| value:bool> = do {Ops}.pattern();let ~<pat> = true;value}".into(),
        include_str!("../../../development/repros/comptime-computed-quote-method.fl").to_owned(),
        include_str!("../../../development/repros/comptime-computed-quote-method.fl").replace("fn run()->bool {do {Ops}.invert(false)}", "fn run()->bool {var flag=false;let answer=do {Ops}.invert(do {flag=not flag;not flag});answer and flag}"),
    ] {
        let work=Work::new(&source);
        let output=work.command().args(["--interpret","run"]).output().unwrap();
        assert!(output.status.success(),"{source}: {}",String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout,b"(bool true)\n");
        for level in ["0","3"] {
            let output=work.command().args(["--build","run","--output","native program","--opt-level",level]).output().unwrap();
            assert!(output.status.success(),"{source}: {}",String::from_utf8_lossy(&output.stderr));
            let output=work.execute();success(&output);assert_eq!(output.stdout,b"(bool true)\n");
        }
    }
}

#[test]
fn computed_code_methods_preserve_access_stage_and_code_contracts() {
    let base = include_str!("../../../development/repros/comptime-computed-code-method.fl");
    for source in [
        base.replace("'(true)", "'(\"wrong\")"),
        base.replace("do {Ops}", "do {true}"),
        base.replace(".build()", ".missing()"),
        base.replace("fn run()", "fn run(flag:bool)").replace("do {Ops}", "(if flag {Ops} else {Ops})"),
        base.replace("struct Ops", "fn Make()->.Type'builtin {Ops} struct Ops").replace("do {Ops}", "Make()"),
        base.replace("struct Ops", "mod hidden {pub struct Ops").replace("pub comptime fn build", "private comptime fn build").replace("fn run()", "} fn run()").replace("do {Ops}", "do {hidden.Ops}"),
        base.replace("struct Ops", "pure comptime fn Identity<T> -> .Type'builtin {T} struct Ops").replace("do {Ops}", "do {Identity<Ops>}"),
        "trait Builder {comptime fn build()-><|-bool>;} struct Ops {} fn run()->bool {~<do {Ops}.as(Builder).build()>}".into(),
        "struct Ops {pub comptime fn pattern()-><bool -| value:bool> {'<value>}} fn run()->bool {const pat:<bool -| value:.str'builtin> = do {Ops}.pattern();let ~<pat> = true;true}".into(),
    ] {
        let work=Work::new(&source);
        fs::write(work.0.join("native program"),b"previous output").unwrap();
        let output=work.command().args(["--build","run","--output","native program"]).output().unwrap();
        let diagnostic=String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(),"{source}");
        assert!(diagnostic.contains("elaboration failed"),"{source}: {diagnostic}");
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(fs::read(work.0.join("native program")).unwrap(),b"previous output");
    }
}

#[test]
fn computed_pattern_methods_export_unannotated_output_names() {
    let base = include_str!("../../../development/repros/comptime-computed-pattern-binding.fl");
    for source in [
        base.to_owned(),
        base.replace("do {Ops}", "(if true {Ops} else {Ops})"),
        base.replace("do {Ops}", "(when {false=>Ops,else=>Ops})"),
        base.replace("do {Ops}", "(true match {true=>Ops,false=>Ops})"),
        base.replace("struct Ops {", "struct Ops where T {")
            .replace("<bool -| value:bool>", "<T -| value:T>")
            .replace("do {Ops}", "do {Ops<bool>}"),
        base.replace(
            "struct Ops",
            "struct Other {pub comptime fn pattern()-><bool -| value:bool> {'<value>}} struct Ops",
        )
        .replace("do {Ops}", "(if true {Ops} else {Other})"),
    ] {
        let work = Work::new(&source);
        let output = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            let output = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn computed_pattern_output_names_do_not_grant_unrelated_contracts() {
    let base = include_str!("../../../development/repros/comptime-computed-pattern-binding.fl");
    for source in [
        base.replace("struct Ops", "struct Other {pub comptime fn pattern()-><.str'builtin -| value:.str'builtin> {'<value>}} struct Ops").replace("do {Ops}", "(if false {Ops} else {Other})"),
        base.replace("pub comptime fn pattern", "private comptime fn pattern")
            .replace("struct Ops", "mod hidden {pub struct Ops")
            .replace("fn run()", "} fn run()")
            .replace("do {Ops}", "do {hidden.Ops}"),
        base.replace(
            "struct Ops",
            "struct Other {pub comptime fn pattern()-><bool -| other:bool> {'<other>}} struct Ops",
        )
        .replace("do {Ops}", "(if false {Ops} else {Other})"),
        base.replace("struct Ops", "struct Other {} struct Ops")
            .replace("do {Ops}", "do {Other}"),
        base.replace(
            "<bool -| value:bool>",
            "<.str'builtin -| value:.str'builtin>",
        ),
        base.replace("fn run()", "fn run(flag:bool)")
            .replace("do {Ops}", "(if flag {Ops} else {Ops})"),
        base.replace(
            "struct Ops",
            "pure comptime fn Identity<T> -> .Type'builtin {T} struct Ops",
        )
        .replace("do {Ops}", "do {Identity<Ops>}"),
    ] {
        let work = Work::new(&source);
        fs::write(work.0.join("native program"), b"previous output").unwrap();
        let output = work
            .command()
            .args(["--build", "run", "--output", "native program"])
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{source}");
        assert!(
            diagnostic.contains("lowering failed") || diagnostic.contains("elaboration failed"),
            "{source}: {diagnostic}"
        );
        assert!(!diagnostic.contains("panicked"));
        assert_eq!(
            fs::read(work.0.join("native program")).unwrap(),
            b"previous output"
        );
    }
}

#[test]
fn native_wrapping_absolute_value_all_widths_and_argument_once() {
    for width in [8, 16, 32, 64, 128] {
        let max = if width == 128 {
            i128::MAX
        } else {
            (1i128 << (width - 1)) - 1
        };
        let min = -max - 1;
        let inputs: Vec<i128> = if width == 8 {
            (min..=max).collect()
        } else {
            vec![min, min + 1, -42, -1, 0, 1, 42, max - 1, max]
        };
        let cases = inputs
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                let expected = if value == min { min } else { value.abs() };
                format!("let checked_{index}=verify({value},{expected});assumes checked_{index};")
            })
            .collect::<Vec<_>>()
            .join("");
        let work = Work::new(&format!(
            "{RESOURCE_HEADER} mod math {{\n{}\n}}
            typealias Number=.i{width}'builtin;impl Duplicate for Number {{}}
            fn verify(value:Number,expected:Number)->bool {{
                var seen=false;
                let actual=math.wrapping_abs(do {{assumes not seen;seen=true;value}});
                seen and actual==expected
            }}
            fn run()->bool {{{cases}true}}",
            include!("support/math_source.rs")
        ));
        for level in ["0", "3"] {
            let mut command = work.command();
            command.args([
                "--build",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
                "--fuel",
                "10000000",
                "--opt-level",
                level,
                "--output",
                "native program",
            ]);
            for operation in nir::intrinsics::Operation::ALL {
                command.args([
                    "--host-intrinsic",
                    &format!("{0}=input.math.target.Scalar.{0}", operation.name()),
                ]);
            }
            success(&command.output().unwrap());
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n", "i{width} O{level}");
        }
    }
}

#[test]
fn wrapping_absolute_llvm_disables_minimum_poison() {
    let work = Work::new(
        "^intrinsic_enum enum Ops {absolute} fn run()->.i8'builtin {let value:.i8'builtin=-128;unsafe {Ops.absolute'intrinsic(value).as(.i8'builtin)}}",
    );
    let output = work
        .command()
        .args([
            "--emit-llvm",
            "run",
            "--host-intrinsic",
            "abs=input.Ops.absolute",
        ])
        .output()
        .unwrap();
    success(&output);
    let ir = String::from_utf8(output.stdout).unwrap();
    assert!(
        ir.contains("declare i8 @llvm.abs.i8(i8, i1 immarg)"),
        "{ir}"
    );
    assert!(
        ir.lines()
            .any(|line| line.contains("call i8 @llvm.abs.i8(") && line.ends_with(", i1 false)")),
        "{ir}"
    );
}

#[test]
fn wrapping_absolute_contracts_use_input_intervals_and_keep_minimum_exception() {
    for width in [8, 16, 32, 64, 128] {
        let minimum = if width == 128 {
            i128::MIN
        } else {
            -(1i128 << (width - 1))
        };
        let header = format!("typealias I=.i{width}'builtin;^intrinsic_enum enum Ops {{absolute}}");
        let source = format!("{header}
            fn bounded(n:I)->r:I where requires n>=-40 and n<=30, ensures r>=0 and r<=40 {{unsafe {{Ops.absolute'intrinsic(n).as(I)}}}}
            fn negative(n:I)->r:I where requires n>=-40 and n<=-10, ensures r>=10 and r<=40 {{unsafe {{Ops.absolute'intrinsic(n).as(I)}}}}
            fn positive(n:I)->r:I where requires n>=10 and n<=40, ensures r>=10 and r<=40 {{unsafe {{Ops.absolute'intrinsic(n).as(I)}}}}
            fn minimum(n:I)->r:I where requires n=={minimum}, ensures r=={minimum} {{unsafe {{Ops.absolute'intrinsic(n).as(I)}}}}
            fn ordinary(n:I)->r:I where requires n > {minimum}, ensures r>=0 {{unsafe {{Ops.absolute'intrinsic(n).as(I)}}}}
            fn run()->bool {{bounded(-40)==40 and bounded(30)==30 and bounded(0)==0 and negative(-10)==10 and positive(40)==40 and minimum({minimum})=={minimum} and ordinary(-1)==1}}"
        );
        let work = Work::new(&source);
        for level in ["0", "3"] {
            let result = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                    "--host-intrinsic",
                    "abs=input.Ops.absolute",
                ])
                .output()
                .unwrap();
            success(&result);
            let result = work.execute();
            success(&result);
            assert_eq!(result.stdout, b"(bool true)\n");
        }
        for contract in ["r>=0".to_owned(), format!("r<={minimum}")] {
            let work = Work::new(&format!(
                "{header} fn wrong(n:I)->r:I where ensures {contract} {{unsafe {{Ops.absolute'intrinsic(n).as(I)}}}} fn run()->bool {{true}}"
            ));
            fs::write(work.0.join("native program"), b"previous").unwrap();
            let result = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--host-intrinsic",
                    "abs=input.Ops.absolute",
                ])
                .output()
                .unwrap();
            assert!(!result.status.success());
            assert!(
                String::from_utf8_lossy(&result.stderr).contains("postcondition"),
                "{result:?}"
            );
            assert_eq!(
                fs::read(work.0.join("native program")).unwrap(),
                b"previous"
            );
        }
    }
}

#[test]
fn wrapping_absolute_relations_survive_contracts_and_input_writes() {
    for width in [8, 16, 32, 64, 128] {
        let source = format!("typealias I=.i{width}'builtin;^intrinsic_enum enum Ops {{absolute}}
            fn positive(n:I)->r:I where requires n>=0, ensures r==n'old {{unsafe {{Ops.absolute'intrinsic(n).as(I)}}}}
            fn negative(n:I)->r:I where requires n>=-40 and n<=-10, ensures r + n'old == 0 {{unsafe {{Ops.absolute'intrinsic(n).as(I)}}}}
            fn choice(n:I)->r:I where requires n>=-40 and n<=30, ensures r==n'old or r + n'old == 0 {{unsafe {{Ops.absolute'intrinsic(n).as(I)}}}}
            fn saved(n:I)->r:I where requires n>=10 and n<=40, ensures r==n'old {{var input=n;let result=unsafe {{Ops.absolute'intrinsic(input).as(I)}};input=0;result}}
            fn run()->bool {{positive(42)==42 and negative(-40)==40 and choice(-30)==30 and choice(30)==30 and choice(0)==0 and saved(31)==31}}"
        );
        let work = Work::new(&source);
        for level in ["0", "3"] {
            let result = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                    "--host-intrinsic",
                    "abs=input.Ops.absolute",
                ])
                .output()
                .unwrap();
            success(&result);
            let result = work.execute();
            success(&result);
            assert_eq!(result.stdout, b"(bool true)\n");
        }
    }
}

#[test]
fn wrapping_absolute_relations_do_not_authorize_wrong_or_stale_results() {
    let header = "typealias I=.i32'builtin;^intrinsic_enum enum Ops {absolute}";
    for contract in ["r==n'old", "r==0"] {
        let work = Work::new(&format!(
            "{header} fn wrong(n:I)->r:I where ensures {contract} {{unsafe {{Ops.absolute'intrinsic(n).as(I)}}}} fn run()->bool {{true}}"
        ));
        let result = work
            .command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--host-intrinsic",
                "abs=input.Ops.absolute",
            ])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("postcondition"),
            "{result:?}"
        );
        assert!(!work.0.join("native program").exists());
    }
    let work = Work::new(&format!(
        "{header} fn stale(n:I)->bool where requires n>=10 and n<=40 {{var input=n;let result=unsafe {{Ops.absolute'intrinsic(input).as(I)}};input=0;assumes result==input;true}} fn run()->bool {{stale(31)}}"
    ));
    for level in ["0", "3"] {
        let result = work
            .command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--opt-level",
                level,
                "--host-intrinsic",
                "abs=input.Ops.absolute",
            ])
            .output()
            .unwrap();
        success(&result);
        let result = work.execute();
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
    }
}

#[test]
fn generic_scalar_equality_uses_proved_primitive_families() {
    let source = "typealias F32=.f32'builtin; pure fn same(a:T,b:T)->bool where T, requires T == .i8'builtin or T == .u16'builtin or T == .f32'builtin or T == .f64'builtin or T == bool or T == .char'builtin or T == .str'builtin or T == .Symbol'builtin {a == b}
        fn different(a:T,b:T)->bool where T, requires T == .i8'builtin or T == bool {a != b}
        fn run()->bool {
            same<.i8'builtin>(-4,-4) and not same<.i8'builtin>(-4,4)
            and same<.u16'builtin>(65535,65535)
            and same<.f32'builtin>(-0.0,0.0) and same<.f64'builtin>(1.5,1.5)
            and not same<F32>(F32.NaN,F32.NaN)
            and same<.char'builtin>('x','x') and not same<.char'builtin>('x','y')
            and same<bool>(true,true) and not same<bool>(true,false)
            and same<.str'builtin>(\"hello\",\"hello\") and not same<.str'builtin>(\"hello\",\"other\")
            and same<.Symbol'builtin>(.ready,.ready) and not same<.Symbol'builtin>(.ready,.done)
            and different<.i8'builtin>(-1,1) and different<bool>(false,true)
        }";
    let work = Work::new(source);
    let reference = work
        .command()
        .args(["--interpret", "run"])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap(),
        );
        let result = work.execute();
        success(&result);
        assert_eq!(result.stdout, reference.stdout);
    }
}

#[test]
fn generic_integer_contracts_check_every_allowed_width() {
    let source = "^intrinsic_enum enum Ops {absolute}
        pure fn magnitude(value:T)->result:T where T,
            requires T == .i8'builtin or T == .i16'builtin or T == .i32'builtin or T == .i64'builtin or T == .i128'builtin,
            ensures result == value'old or result == -value'old {
            unsafe {Ops.absolute'intrinsic(value).as(T)}
        }
        fn run()->bool {
            magnitude<.i8'builtin>(-128) == -128 and magnitude<.i8'builtin>(-5) == 5
            and magnitude<.i16'builtin>(-32768) == -32768 and magnitude<.i16'builtin>(-5) == 5
            and magnitude<.i32'builtin>(-2147483648) == -2147483648
            and magnitude<.i64'builtin>(-9223372036854775808) == -9223372036854775808
            and magnitude<.i128'builtin>(-170141183460469231731687303715884105728) == -170141183460469231731687303715884105728
        }";
    let work = Work::new(source);
    for level in ["0", "3"] {
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                    "--host-intrinsic",
                    "abs=input.Ops.absolute",
                ])
                .output()
                .unwrap(),
        );
        let result = work.execute();
        success(&result);
        assert_eq!(result.stdout, b"(bool true)\n");
    }
    // Invalid generic definitions must fail even when never instantiated by
    // the entry point; checking only observed calls is not a universal proof.
    for body in [
        "pure fn bad(value:T)->T where T, requires T == .i8'builtin or T == .i16'builtin { -value }",
        "pure fn bad(value:T)->result:T where T, requires T == .i8'builtin or T == .i16'builtin, ensures result == -value'old {unsafe {Ops.absolute'intrinsic(value).as(T)}}",
    ] {
        let work = Work::new(&format!(
            "^intrinsic_enum enum Ops {{absolute}} {body} fn run()->bool {{true}}"
        ));
        let result = work
            .command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--host-intrinsic",
                "abs=input.Ops.absolute",
            ])
            .output()
            .unwrap();
        assert!(!result.status.success(), "accepted {body}");
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .contains("machine arithmetic range is not proven"),
            "{result:?}"
        );
        assert!(!work.0.join("native program").exists());
    }
}

#[test]
fn generic_integer_arithmetic_and_ordering_preserve_domains_and_evaluation() {
    let integers = [
        "i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128",
    ];
    let domain = integers
        .iter()
        .map(|p| format!("T == .{p}'builtin"))
        .collect::<Vec<_>>()
        .join(" or ");
    let unsigned = integers[5..]
        .iter()
        .map(|p| format!("T == .{p}'builtin"))
        .collect::<Vec<_>>()
        .join(" or ");
    let header = format!("
        pure fn zero(value:T)->result:T where T, requires {domain}, ensures result == 0 {{value - value}}
        pure fn literal_zero()->result:T where T, requires {domain}, ensures result == 0 {{0 + 0}}
        pure fn early(value:T)->T where T, requires {domain} {{value + do {{return value;}}}}
        pure fn add_zero(a:T,b:T)->result:T where T, requires {domain}, requires a == 0, ensures result == b'old {{a + b}}
        pure fn multiply_zero(a:T,b:T)->result:T where T, requires {domain}, requires a == 0, ensures result == 0 {{a * b}}
        pure fn distance(a:T,b:T)->T where T, requires {unsigned} {{if a >= b {{a - b}} else {{b - a}}}}
        pure fn quotient(a:T,b:T)->T where T, requires {unsigned}, requires b != 0 {{a / b}}
        pure fn remainder(a:T,b:T)->T where T, requires {unsigned}, requires b != 0 {{a % b}}
        pure fn ordered(a:T,b:T)->bool where T, requires {domain} {{a < b and a <= b and b > a and b >= a}}
    ");
    let mut checks = Vec::new();
    for primitive in integers {
        checks.push(format!("zero<.{primitive}'builtin>(17) == 0 and literal_zero<.{primitive}'builtin>() == 0 and early<.{primitive}'builtin>(17) == 17 and add_zero<.{primitive}'builtin>(0,17) == 17 and multiply_zero<.{primitive}'builtin>(0,17) == 0 and ordered<.{primitive}'builtin>(3,17)"));
    }
    for primitive in &integers[5..] {
        checks.push(format!("distance<.{primitive}'builtin>(3,17) == 14 and distance<.{primitive}'builtin>(17,3) == 14 and quotient<.{primitive}'builtin>(17,3) == 5 and remainder<.{primitive}'builtin>(17,3) == 2"));
    }
    checks.push("zero<.i128'builtin>(-170141183460469231731687303715884105728) == 0 and ordered<.i128'builtin>(-170141183460469231731687303715884105728,0) and add_zero<.u128'builtin>(0,340282366920938463463374607431768211455) == 340282366920938463463374607431768211455 and not ordered<.u128'builtin>(340282366920938463463374607431768211455,0)".into());
    for check in checks {
        let work = Work::new(&format!(
            "{header} fn run()->bool {{var seen=false;let result=add_zero<.u8'builtin>(do {{assumes not seen;seen=true;0}},do {{assumes seen;9}});seen and result == 9 and {}}}",
            check
        ));
        let reference = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &work
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn generic_integer_arithmetic_rejects_unproved_bounds_domains_and_rounding() {
    for (definition, diagnostic) in [
        (
            "fn bad(a:T,b:T)->T where T, requires T == .i8'builtin or T == .i16'builtin {a + b}",
            "machine arithmetic range is not proven",
        ),
        (
            "fn bad(a:T,b:T)->T where T, requires T == .u8'builtin or T == .u16'builtin {a - b}",
            "machine arithmetic range is not proven",
        ),
        (
            "fn bad(a:T,b:T)->T where T, requires T == .u8'builtin or T == .u16'builtin {a * b}",
            "machine arithmetic range is not proven",
        ),
        (
            "fn bad(a:T,b:T)->T where T, requires T == .u8'builtin or T == .u16'builtin {a / b}",
            "divisor is not proven nonzero",
        ),
        (
            "fn bad(a:T,b:T)->T where T, requires T == .i8'builtin or T == .i16'builtin {a / b}",
            "rounding protocol",
        ),
        (
            "fn bad(a:T,b:T)->T where T, requires T == .i8'builtin or T == .i16'builtin {a % b}",
            "remainder rounding",
        ),
        ("fn bad(a:T,b:T)->T where T {a + b}", "type mismatch"),
        (
            "fn bad(a:T,b:T)->T where T, requires T == .i8'builtin or T == bool {a + b}",
            "type mismatch",
        ),
        (
            "fn bad(a:T,b:T)->bool where T, requires T == .i8'builtin or T == bool {a < b}",
            "ordering protocol",
        ),
        (
            "fn bad()->result:T where T, requires T == .i8'builtin or T == .i16'builtin, ensures true {128}",
            "integer-formation",
        ),
        (
            "fn bad(value:T)->result:T where T, requires T == .i8'builtin or T == .i16'builtin, ensures result == 128 {value}",
            "integer-formation",
        ),
        (
            "fn bad(.value:T=128)->result:T where T, requires T == .i8'builtin or T == .i16'builtin, ensures true {value}",
            "integer-formation",
        ),
    ] {
        let work = Work::new(&format!("{definition} fn run()->bool {{true}}"));
        let result = work.build();
        assert!(!result.status.success(), "accepted {definition}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(diagnostic),
            "{definition}: {result:?}"
        );
        assert!(!work.0.join("native program").exists());
    }
}

#[test]
fn correlated_generic_contracts_do_not_expand_the_full_cartesian_domain() {
    let inputs = (0..8)
        .map(|i| format!("value{i}:T{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let parameters = (0..8)
        .map(|i| format!("T{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let domains = (0..8)
        .map(|i| format!("requires T{i} == .i8'builtin or T{i} == .i16'builtin"))
        .collect::<Vec<_>>()
        .join(",");
    let correlations = (1..8)
        .map(|i| format!("requires T{i} == T0"))
        .collect::<Vec<_>>()
        .join(",");
    let definition = format!(
        "pure fn checked({inputs})->result:bool where {parameters}, {domains}, {correlations}, ensures result {{value0 == value0}}"
    );
    for width in ["i8", "i16"] {
        let arguments = vec![format!(".{width}'builtin"); 8].join(",");
        let work = Work::new(&format!(
            "{definition} fn run()->bool {{checked<{arguments}>(1,2,3,4,5,6,7,8)}}"
        ));
        for level in ["0", "3"] {
            success(
                &work
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, b"(bool true)\n");
        }
    }
    let work = Work::new(&format!(
        "{} fn run()->bool {{true}}",
        definition.replace("ensures result", "ensures not result")
    ));
    let result = work.build();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("postcondition is not proven"),
        "{result:?}"
    );
    assert!(!work.0.join("native program").exists());
}

#[test]
fn generic_scalar_equality_rejects_unproved_or_nonprimitive_families() {
    for source in [
        "fn same(a:T,b:T)->bool where T {a == b} fn run()->bool {same(true,true)}",
        "struct Other {} fn same(a:T,b:T)->bool where T, requires T == bool or T == Other {a == b} fn run()->bool {same(true,true)}",
        "pure comptime fn Identity<T> -> .Type'builtin {T} fn same(a:T,b:T)->bool where T, requires T == bool or T == Identity<bool> {a == b} fn run()->bool {same(true,true)}",
        "fn same(a:T,b:T)->bool where T, requires T == bool or T == .Type'builtin {a == b} fn run()->bool {same(true,true)}",
    ] {
        let work = Work::new(source);
        let result = work.build();
        assert!(!result.status.success(), "accepted {source}");
        let diagnostic = String::from_utf8_lossy(&result.stderr);
        assert!(
            diagnostic.contains("equality protocol for this type is not implemented")
                || (source.contains("Identity")
                    && diagnostic.contains("structural type view remains unresolved")),
            "{result:?}"
        );
        assert!(!work.0.join("native program").exists());
    }
}

#[test]
fn generic_native_negation_survives_protocol_binding() {
    let domain = ["i8", "i16", "i32", "i64", "i128"]
        .map(|p| format!("T == .{p}'builtin"))
        .join(" or ");
    let header = format!(
        "{RESOURCE_HEADER}\n{}\n{}\n
        impl Duplicate for T where T, requires {domain} {{}}
        pure fn negate(value:T)->result:T where T, requires {domain},
            requires value >= -127, ensures result == -value'old {{ -value }}
        struct Flag {{enabled:bool}}
        impl Duplicate for Flag {{}}
        impl Neg for Flag {{assoc Output:Type=bool;fn neg(self)->bool {{self.enabled}}}}
        impl Not for Flag {{assoc Output:Type=bool;fn invert(self)->bool {{self.enabled}}}}
        -- These implementations must not replace native scalar operations.
        impl Neg for i8 {{assoc Output:Type=bool;fn neg(self)->bool {{false}}}}
        impl Not for bool {{assoc Output:Type=bool;fn invert(self)->bool {{self}}}}
        typealias Bits=.BitVec'builtin<4>;
        impl Duplicate for Bits {{}}
        impl Not for Bits {{assoc Output:Type=bool;fn invert(self)->bool {{false}}}}
        fn bits(value:Bits)->Bits {{not value}}
        ",
        include_str!("../../../library/builtin/primitives.fl"),
        include!("support/operators_source.rs")
    );
    for primitive in ["i8", "i16", "i32", "i64", "i128"] {
        let work = Work::new(&format!(
            "{header}
            fn run()->bool {{var seen=false;
                let value=negate<{primitive}>(do {{assumes not seen;seen=true;-127}});
                seen and value == 127 and negate<{primitive}>(17) == -17
                    and negate<{primitive}>(0) == 0 and not false
                    and bits(3) == 12 and (-Flag {{enabled:true}})
                    and (not Flag {{enabled:true}})
            }}"
        ));
        let bindings = [
            "--neg-protocol",
            "input.Neg",
            "--not-protocol",
            "input.Not",
            "--copy-protocol",
            "input.Duplicate",
            "--drop-protocol",
            "input.Retire",
            "--fuel",
            "1000000",
        ];
        let reference = work
            .command()
            .args(bindings)
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &work
                    .command()
                    .args(bindings)
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let result = work.execute();
            success(&result);
            assert_eq!(result.stdout, reference.stdout);
        }
    }
}

#[test]
fn bound_negation_keeps_generic_range_obligations() {
    let work = Work::new(
        "trait Neg {assoc Output:.Type'builtin;fn neg(self)->Output;}
        fn bad(value:T)->T where T, requires T == .i8'builtin or T == .i16'builtin { -value }
        fn run()->bool {true}",
    );
    let output = work
        .command()
        .args([
            "--neg-protocol",
            "input.Neg",
            "--build",
            "run",
            "--output",
            "native program",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("machine arithmetic range is not proven"),
        "{output:?}"
    );
    assert!(!work.0.join("native program").exists());
}

#[test]
fn generic_float_operators_preserve_ieee_values_and_evaluation() {
    let header = format!(
        "{}\n{}\n
        pure fn add(a:T,b:T)->T where T, requires T == f32 or T == f64 {{a + b}}
        pure fn sub(a:T,b:T)->T where T, requires T == f32 or T == f64 {{a - b}}
        pure fn mul(a:T,b:T)->T where T, requires T == f32 or T == f64 {{a * b}}
        pure fn div(a:T,b:T)->T where T, requires T == f32 or T == f64 {{a / b}}
        pure fn neg(a:T)->T where T, requires T == f32 or T == f64 {{ -a }}
        pure fn literal()->T where T, requires T == f32 or T == f64 {{0.25 + 0.5}}
        pure fn multiply_add(a:T,b:T,c:T)->T where T, requires T == f32 or T == f64 {{a * b + c}}
        pure fn less(a:T,b:T)->bool where T, requires T == f32 or T == f64 {{a < b}}
        pure fn less_equal(a:T,b:T)->bool where T, requires T == f32 or T == f64 {{a <= b}}
        pure fn greater(a:T,b:T)->bool where T, requires T == f32 or T == f64 {{a > b}}
        pure fn greater_equal(a:T,b:T)->bool where T, requires T == f32 or T == f64 {{a >= b}}
        pure fn early(a:T)->T where T, requires T == f32 or T == f64 {{a * do {{return a;}}}}
        ",
        include_str!("../../../library/builtin/primitives.fl"),
        include!("support/operators_source.rs")
    );
    let bindings = [
        "--add-protocol",
        "input.Add",
        "--sub-protocol",
        "input.Sub",
        "--mul-protocol",
        "input.Mul",
        "--div-protocol",
        "input.Div",
        "--neg-protocol",
        "input.Neg",
    ];
    for primitive in ["f32", "f64"] {
        let rounding = if primitive == "f32" {
            "multiply_add<F>(4097.0,4097.0,-16785408.0) == 0.0"
        } else {
            "multiply_add<F>(134217729.0,134217729.0,-18014398777917440.0) == 0.0"
        };
        for body in [
            rounding,
            "var seen=false;let total=add<F>(do {assumes not seen;seen=true;1.5},do {assumes seen;2.25}); seen and total == 3.75 and sub<F>(4.0,1.5) == 2.5 and mul<F>(1.5,2.0) == 3.0 and div<F>(3.0,2.0) == 1.5 and neg<F>(2.5) == -2.5 and literal<F>() == 0.75 and early<F>(4.0) == 4.0",
            "let nan=div<F>(0.0,0.0);let inf=div<F>(1.0,0.0);nan != nan and not less<F>(nan,inf) and not less_equal<F>(nan,inf) and not greater<F>(nan,inf) and not greater_equal<F>(nan,inf) and greater<F>(inf,1.0) and less<F>(neg<F>(inf),-1.0)",
            "let negative=neg<F>(0.0);less<F>(div<F>(1.0,negative),0.0) and greater<F>(div<F>(1.0,neg<F>(negative)),0.0) and less<F>(1.0,2.0) and less_equal<F>(2.0,2.0) and greater<F>(2.0,1.0) and greater_equal<F>(2.0,2.0)",
        ] {
            for options in [&[][..], &bindings[..]] {
                let source = format!("{header} typealias F={primitive};fn run()->bool {{{body}}}");
                let work = Work::new(&source);
                let reference = work
                    .command()
                    .args(options)
                    .args(["--interpret", "run"])
                    .output()
                    .unwrap();
                success(&reference);
                assert_eq!(reference.stdout, b"(bool true)\n");
                for level in ["0", "3"] {
                    success(
                        &work
                            .command()
                            .args(options)
                            .args([
                                "--build",
                                "run",
                                "--output",
                                "native program",
                                "--opt-level",
                                level,
                            ])
                            .output()
                            .unwrap(),
                    );
                    let output = work.execute();
                    success(&output);
                    assert_eq!(output.stdout, reference.stdout);
                }
            }
        }
    }
}

#[test]
fn generic_float_operators_reject_unproved_and_mixed_domains() {
    for body in [
        "fn bad(a:T,b:T)->T where T {a + b}",
        "fn bad(a:T,b:U)->T where T,U, requires T == .f32'builtin or T == .f64'builtin, requires U == .f32'builtin or U == .f64'builtin {a + b}",
        "pure fn bad(a:T,p:*bool)->T where T, requires T == .f32'builtin or T == .f64'builtin {a * do {p.* = true;return a;}}",
        "fn bad(a:T,b:T)->T where T, requires T == .f32'builtin or T == .i32'builtin {a + b}",
        "fn bad(a:T)->T where T, requires T == .f64'builtin or T == bool { -a }",
        "fn bad(a:T,b:T)->bool where T, requires T == .f64'builtin or T == bool {a < b}",
        "fn bad(a:T,b:T)->T where T, requires T == .f32'builtin or T == .f64'builtin {a % b}",
        "pure comptime fn Identity<T> -> .Type'builtin {T} fn bad(a:T,b:T)->T where T, requires T == .f32'builtin or T == Identity<.f32'builtin> {a + b}",
    ] {
        let work = Work::new(&format!("{body} fn run()->bool {{true}}"));
        let output = work.build();
        assert!(!output.status.success(), "accepted {body}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("type mismatch")
                || diagnostic.contains("ordering protocol")
                || diagnostic.contains("remainder rounding")
                || diagnostic.contains("structural type view remains unresolved")
                || diagnostic.contains("lacks P4 evidence"),
            "{diagnostic}"
        );
        assert!(!work.0.join("native program").exists());
    }
}

#[test]
fn unary_nonreturning_operands_preserve_control_and_effects() {
    let header = "trait Neg {assoc Output:.Type'builtin;fn neg(self)->Output;}
        trait Not {assoc Output:.Type'builtin;fn invert(self)->Output;}
        fn negate_exit(value:T)->T where T { -do {return value;} }
        fn invert_exit(value:T)->T where T {not do {return value;}}
        pure fn pure_exit(value:T)->T where T,
            requires T == .f32'builtin or T == .f64'builtin { -do {return value;} }
        pure fn pure_bool(value:bool)->bool {not do {return value;}}
        fn run()->bool {var seen=false;
            let result=negate_exit<bool>(do {assumes not seen;seen=true;true});
            seen and result and negate_exit<bool>(true) and invert_exit<bool>(true)
                and negate_exit<.i8'builtin>(-128) == -128
                and invert_exit<.i8'builtin>(-128) == -128
                and pure_exit<.f32'builtin>(1.5) == 1.5
                and pure_exit<.f64'builtin>(-2.5) == -2.5
                and pure_bool(true)
        }";
    let bindings = ["--neg-protocol", "input.Neg", "--not-protocol", "input.Not"];
    for options in [&[][..], &bindings[..]] {
        let work = Work::new(header);
        let reference = work
            .command()
            .args(options)
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &work
                    .command()
                    .args(options)
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
    let staged = header
        .replace("fn negate_exit", "comptime fn negate_exit")
        .replace("fn invert_exit", "comptime fn invert_exit")
        .replace("pure fn", "pure comptime fn")
        .replace("fn run", "comptime fn run");
    for options in [&[][..], &bindings[..]] {
        let work = Work::new(&staged);
        let output = work
            .command()
            .args(options)
            .args(["--invoke-comptime", "run"])
            .output()
            .unwrap();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn unary_nonreturning_operands_still_check_operand_bodies() {
    for (body, diagnostic) in [
        (
            "pure fn bad(p:*bool)->bool { -do {p.* = true;return true;} }",
            "lacks P4 evidence",
        ),
        (
            "pure fn bad(p:*bool)->bool {not do {p.* = true;return true;}}",
            "lacks P4 evidence",
        ),
        (
            "fn bad()->bool { -do {return true;missing;} }",
            "Unresolved identifier",
        ),
        (
            "fn bad()->bool {not do {return 1.5;}}",
            "float literal requires",
        ),
    ] {
        let work = Work::new(&format!("{body} fn run()->bool {{true}}"));
        let output = work.build();
        assert!(!output.status.success(), "accepted {body}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "{output:?}"
        );
        assert!(!work.0.join("native program").exists());
    }
}

#[test]
fn unary_nonreturning_operands_run_cleanup_on_each_exit() {
    for operator in ["-", "not "] {
        for control in ["return true", "break", "continue"] {
            let source = format!(
                "{RESOURCE_HEADER}
                struct Watch {{target:*bool}}
                impl Retire for Watch {{fn drop(*self) {{
                    assumes not self.target.*.*;
                    self.target.*.* = true;
                }}}}
                fn leave(target:*bool)->bool {{
                    while not target.* {{
                        let watch=Watch {{target:target}};
                        {operator}do {{{control};}};
                    }}
                    true
                }}
                fn run()->bool {{var seen=false;let result=leave(seen.ref);seen and result}} "
            );
            let work = Work::new(&source);
            let bindings = ["--copy-protocol", "Duplicate", "--drop-protocol", "Retire"];
            let reference = work
                .command()
                .args(bindings)
                .args(["--interpret", "run"])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &work
                        .command()
                        .args(bindings)
                        .args([
                            "--build",
                            "run",
                            "--output",
                            "native program",
                            "--opt-level",
                            level,
                        ])
                        .output()
                        .unwrap(),
                );
                let output = work.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn machine_integer_checks_traverse_float_operations() {
    for source in [
        include_str!("../../../development/examples/host-generic-iteration.fl"),
        "typealias F=.f64'builtin;typealias U=.u8'builtin;
        fn run()->bool {var steps:U=0;var seen=false;
            let value:F=(do {assumes not seen;seen=true;1.5}) + (do {assumes seen;steps += 1;2.0});
            let negative= -do {steps += 1;value};
            steps == 2 and value == 3.5 and negative == -3.5}",
        "typealias F=.f32'builtin;typealias U=.u8'builtin;
        fn run()->bool {var steps:U=0;
            let value:F=(do {steps += 1;3.0}) / (do {steps += 1;0.0});
            steps == 2 and value > 0.0}",
    ] {
        let work = Work::new(source);
        let reference = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &work
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn float_operations_do_not_hide_integer_obligations_or_prove_exact_identities() {
    for (body, diagnostic) in [
        (
            "fn bad(value:.f64'builtin,n:.u8'builtin)->.f64'builtin {var divisor=n;if divisor != 0 {let ignored=value + do {divisor=0;value};let next=n / divisor;} value}",
            "divisor is not proven nonzero",
        ),
        (
            "fn bad(value:.f64'builtin,n:.u8'builtin)->.f64'builtin {value + do {let next=n + 1;value}}",
            "machine arithmetic range is not proven",
        ),
        (
            "fn bad(value:.f64'builtin,n:.u8'builtin)->.f64'builtin { -do {let next=n + 1;value} }",
            "machine arithmetic range is not proven",
        ),
        (
            "fn bad(value:.f32'builtin,n:.u8'builtin)->.f32'builtin {value * do {let next=n / 0;value}}",
            "divisor is not proven nonzero",
        ),
        (
            "pure fn bad(value:.f64'builtin)->result:.f64'builtin where ensures result == 0.0 {value - value}",
            "postcondition is not proven",
        ),
        (
            "pure fn bad(value:.f32'builtin)->result:.f32'builtin where ensures result == value'old {value + 0.0}",
            "postcondition is not proven",
        ),
    ] {
        let work = Work::new(&format!("{body} fn run()->bool {{true}}"));
        let output = work.build();
        assert!(!output.status.success(), "accepted {body}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "{output:?}"
        );
        assert!(!work.0.join("native program").exists());
    }
}

#[test]
fn mixed_scalar_ordering_uses_each_concrete_domains_comparison() {
    let domains = [
        "i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128", "f32", "f64", "char",
    ];
    let constraint = domains
        .iter()
        .map(|p| format!("T == .{p}'builtin"))
        .collect::<Vec<_>>()
        .join(" or ");
    let header=format!("
        pure fn ordered(a:T,b:T)->bool where T, requires {constraint} {{a < b and a <= b and b > a and b >= a}}
        pure fn choose(a:T,b:T)->T where T, requires {constraint} {{if a < b {{a}} else {{b}}}}
        pure fn unordered(a:T,b:T)->bool where T, requires {constraint} {{not (a <= b) and not (a >= b)}}
    ");
    for domain in domains {
        let (low, high) = match domain {
            "f32" | "f64" => ("-1.5", "2.25"),
            "char" => ("'a'", "'界'"),
            "i8" => ("-128", "127"),
            "i16" => ("-32768", "32767"),
            "i32" => ("-2147483648", "2147483647"),
            "i64" => ("-9223372036854775808", "9223372036854775807"),
            "i128" => (
                "-170141183460469231731687303715884105728",
                "170141183460469231731687303715884105727",
            ),
            "u128" => ("0", "340282366920938463463374607431768211455"),
            _ => ("0", "127"),
        };
        let extra = if matches!(domain, "f32" | "f64") {
            "let nan:F=0.0 / 0.0;let specials=unordered<F>(nan,low) and not ordered<F>(nan,high) and choose<F>(nan,high) == high;".to_owned()
        } else {
            "let specials=true;".to_owned()
        };
        let source=format!("{header} typealias F=.{domain}'builtin;
            fn run()->bool {{let low:F={low};let high:F={high};{extra}
                var seen=false;let result=choose<F>(do {{assumes not seen;seen=true;high}},do {{assumes seen;low}});
                specials and seen and result == low and ordered<F>(low,high) and not ordered<F>(high,low)
                    and not unordered<F>(low,high) and choose<F>(low,high) == low
            }}");
        let work = Work::new(&source);
        let reference = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &work
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let result = work.execute();
            success(&result);
            assert_eq!(result.stdout, reference.stdout);
        }
    }
}

#[test]
fn mixed_scalar_ordering_rejects_nonordered_domains() {
    for domain in [
        "T == .char'builtin or T == .str'builtin",
        "T == .u8'builtin or T == bool",
        "T == .f64'builtin or T == .Symbol'builtin",
        "T == .u32'builtin or T == .Type'builtin",
        "T == .f32'builtin or T == Identity<.f32'builtin>",
    ] {
        let work = Work::new(&format!(
            "pure comptime fn Identity<T> -> .Type'builtin {{T}}
            fn bad(a:T,b:T)->bool where T, requires {domain} {{a < b}}
            fn run()->bool {{true}}"
        ));
        let output = work.build();
        assert!(!output.status.success(), "accepted {domain}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("ordering protocol")
                || diagnostic.contains("structural type view remains unresolved"),
            "{diagnostic}"
        );
        assert!(!work.0.join("native program").exists());
    }
}

#[test]
fn mixed_scalar_ordering_checks_bounds_without_totalizing_nan() {
    let example = include_str!("../../../development/examples/host-generic-order.fl");
    let definitions = example.split("fn run()").next().unwrap();
    for body in [
        "bound_if_ordered<.i32'builtin>(0,10,0);true",
        "bound_if_ordered<.char'builtin>('m','z','a');true",
        "let nan:.f32'builtin=0.0 / 0.0;bound_if_ordered<.f32'builtin>(0.5,0.0,nan);true",
    ] {
        let work = Work::new(&format!("{definitions} fn run()->bool {{{body}}}"));
        let reference = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        assert!(!reference.status.success(), "accepted {body}");
        for level in ["0", "3"] {
            success(
                &work
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            assert!(
                !work.execute().status.success(),
                "accepted {body} at O{level}"
            );
        }
    }
    for level in ["0", "3"] {
        let work = Work::new(example);
        success(
            &work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap(),
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn comparison_nonreturning_left_does_not_invent_an_operand_domain() {
    for operator in ["==", "!=", "<", "<=", ">", ">="] {
        let source=format!("fn leave(value:T)->bool where T {{do {{return true;}} {operator} value}}
            pure fn pure_leave(value:T)->bool where T,
                requires T == .f32'builtin or T == .f64'builtin {{do {{return true;}} {operator} value}}
            fn run()->bool {{leave<bool>(false) and leave<.i8'builtin>(-128)
                and pure_leave<.f32'builtin>(0.0) and pure_leave<.f64'builtin>(-0.0)}}");
        let work = Work::new(&source);
        let reference = work
            .command()
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &work
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
        let staged = source
            .replace("pure fn", "pure comptime fn")
            .replace("fn leave", "comptime fn leave")
            .replace("fn run", "comptime fn run");
        let work = Work::new(&staged);
        let output = work
            .command()
            .args(["--invoke-comptime", "run"])
            .output()
            .unwrap();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn comparison_nonreturning_left_skips_rhs_and_keeps_cleanup() {
    for operator in ["==", "!=", "<", "<=", ">", ">="] {
        for control in ["return true", "break", "continue"] {
            let source=format!("{RESOURCE_HEADER}
                struct Watch {{target:*bool}}
                impl Retire for Watch {{fn drop(*self) {{assumes not self.target.*.*;self.target.*.* = true;}}}}
                fn leave(retired:*bool,touched:*bool)->bool {{
                    while not retired.* {{
                        (do {{let watch=Watch {{target:retired}};{control};}})
                            {operator} (do {{touched.* = true;false}});
                    }}
                    true
                }}
                fn run()->bool {{var retired=false;var touched=false;
                    let result=leave(retired.ref,touched.ref);result and retired and not touched}}");
            let work = Work::new(&source);
            let bindings = ["--copy-protocol", "Duplicate", "--drop-protocol", "Retire"];
            let reference = work
                .command()
                .args(bindings)
                .args(["--interpret", "run"])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &work
                        .command()
                        .args(bindings)
                        .args([
                            "--build",
                            "run",
                            "--output",
                            "native program",
                            "--opt-level",
                            level,
                        ])
                        .output()
                        .unwrap(),
                );
                let output = work.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn comparison_nonreturning_left_still_checks_both_operand_bodies() {
    for operator in ["==", "!=", "<", "<=", ">", ">="] {
        for (source, diagnostic) in [
            (
                format!("fn run()->bool {{do {{return true;}} {operator} missing}}"),
                "Unresolved identifier",
            ),
            (
                format!(
                    "fn take(value:bool)->bool {{value}} fn run()->bool {{do {{return true;}} {operator} take(1.5)}}"
                ),
                "float literal requires",
            ),
            (
                format!(
                    "pure fn bad(p:*bool)->bool {{do {{p.* = true;return true;}} {operator} false}} fn run()->bool {{true}}"
                ),
                "lacks P4 evidence",
            ),
        ] {
            let work = Work::new(&source);
            let output = work.build();
            assert!(!output.status.success(), "accepted {source}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(diagnostic),
                "{output:?}"
            );
            assert!(!work.0.join("native program").exists());
        }
    }
}

#[test]
fn comparison_nonreturning_right_keeps_left_evaluation() {
    for operator in ["==", "!=", "<", "<=", ">", ">="] {
        let source=format!("{RESOURCE_HEADER} typealias F=.f32'builtin;impl Duplicate for F {{}}
            pure fn pure_leave(value:F)->bool {{value {operator} do {{return true;}}}}
            fn leave(seen:*bool)->bool {{
                (do {{seen.* = true;let value:F=1.5;value}}) {operator} do {{return true;}}
            }}
            fn run()->bool {{var seen=false;let result=leave(seen.ref);result and seen and pure_leave(0.0)}}");
        let work = Work::new(&source);
        let bindings = ["--copy-protocol", "Duplicate", "--drop-protocol", "Retire"];
        let reference = work
            .command()
            .args(bindings)
            .args(["--interpret", "run"])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &work
                    .command()
                    .args(bindings)
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                    ])
                    .output()
                    .unwrap(),
            );
            let output = work.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_bitwise_library_operations_preserve_width_sign_and_operand_order() {
    let library = include!("support/math_source.rs");
    for width in [8, 16, 32, 64, 128] {
        for signed in [false, true] {
            let primitive = format!("{}{}", if signed { 'i' } else { 'u' }, width);
            let high = 1u128 << (width - 1);
            let maximum = if signed { high - 1 } else { high | (high - 1) };
            let minimum = if signed {
                format!("-{high}")
            } else {
                "0".into()
            };
            let all = if signed {
                "-1".into()
            } else {
                maximum.to_string()
            };
            let checks = if signed {
                format!(
                    "math.bit_and<Word>({minimum},-1)=={minimum}
                    and math.bit_or<Word>({minimum},-1)==-1
                    and math.bit_xor<Word>({minimum},-1)=={maximum}"
                )
            } else {
                format!(
                    "math.bit_and<Word>({maximum},{high})=={high}
                    and math.bit_or<Word>({maximum},{high})=={maximum}
                    and math.bit_xor<Word>({maximum},{high})=={}",
                    high - 1
                )
            };
            let source=format!("mod math {{
{library}
}} typealias Word=.{primitive}'builtin;
                fn run()->bool {{
                    var phase:.u8'builtin=0;
                    let zero=math.bit_and<Word>(do {{assumes phase==0;phase=1;0}},do {{assumes phase==1;phase=2;{all}}});
                    let ones=math.bit_or<Word>(do {{assumes phase==2;phase=3;{all}}},do {{assumes phase==3;phase=4;0}});
                    let cancelled=math.bit_xor<Word>(do {{assumes phase==4;phase=5;{all}}},do {{assumes phase==5;phase=6;{all}}});
                    phase==6 and zero==0 and ones=={all} and cancelled==0
                    and math.bit_and<Word>(5,3)==1 and math.bit_or<Word>(5,3)==7
                    and math.bit_xor<Word>(5,3)==6 and {checks}
                }}");
            let work = Work::new(&source);
            for level in ["0", "3"] {
                let mut command = work.command();
                command.args([
                    "--build",
                    "run",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ]);
                for operation in nir::intrinsics::Operation::ALL {
                    command.args([
                        "--host-intrinsic",
                        &format!(
                            "{}=input.math.target.Scalar.{}",
                            operation.name(),
                            operation.name()
                        ),
                    ]);
                }
                success(&command.output().unwrap());
                let result = work.execute();
                success(&result);
                assert_eq!(result.stdout, b"(bool true)\n", "{primitive} O{level}");
            }
        }
    }
}

#[test]
fn native_bitwise_intrinsics_reject_invalid_domains_interfaces_and_contracts() {
    for operation in ["bit_and", "bit_or", "bit_xor"] {
        for body in [
            "fn run()->bool {unsafe {Ops.bits'intrinsic(true,false).as(bool)}}",
            "fn run()->.f32'builtin {unsafe {Ops.bits'intrinsic(1.0,2.0).as(.f32'builtin)}}",
            "fn run()->.u8'builtin {Ops.bits'intrinsic(1,2).as(.u8'builtin)}",
            "fn run()->.u8'builtin {unsafe {Ops.bits'intrinsic(1).as(.u8'builtin)}}",
            "fn run()->.u8'builtin {let a:.u16'builtin=1;let b:.u8'builtin=2;unsafe {Ops.bits'intrinsic(a,b).as(.u8'builtin)}}",
            "fn bad(a:T,b:T)->T where T {unsafe {Ops.bits'intrinsic(a,b).as(T)}} fn run()->bool {true}",
            "fn bad(a:.u8'builtin,b:.u8'builtin)->r:.u8'builtin where ensures r==a'old {unsafe {Ops.bits'intrinsic(a,b).as(.u8'builtin)}} fn run()->bool {true}",
        ] {
            let work = Work::new(&format!("^intrinsic_enum enum Ops {{bits}} {body}"));
            let result = work
                .command()
                .args([
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--host-intrinsic",
                    &format!("{operation}=input.Ops.bits"),
                ])
                .output()
                .unwrap();
            assert!(!result.status.success(), "accepted {operation}: {body}");
            assert!(!work.0.join("native program").exists());
        }
    }
}

#[test]
fn native_bitwise_contracts_preserve_signed_bounds_and_evaluated_values() {
    for width in [8, 16, 32, 64, 128] {
        for signed in [false, true] {
            let primitive = format!("{}{}", if signed { 'i' } else { 'u' }, width);
            let mask = if signed {
                "-1".to_owned()
            } else {
                (u128::MAX >> (128 - width)).to_string()
            };
            let edge = if signed {
                format!("-{}", 1u128 << (width - 1))
            } else {
                mask.clone()
            };
            let maximum = if signed {
                ((1u128 << (width - 1)) - 1).to_string()
            } else {
                mask.clone()
            };
            let minimum = if signed { edge.clone() } else { "0".into() };
            let bound = if signed { "-16" } else { "15" };
            let extra = if signed {
                "fn filled(a:Word)->r:Word where ensures r>=-16 and r<=-1 {unsafe {let mask:Word=-16;Ops.bor'intrinsic(a,mask).as(Word)}}"
            } else {
                "fn filled(a:Word)->r:Word where ensures r>=15 {unsafe {let mask:Word=15;Ops.bor'intrinsic(a,mask).as(Word)}}"
            };
            let source=format!("^intrinsic_enum enum Ops {{band,bor,bxor}} typealias Word=.{primitive}'builtin;
                fn masked(a:Word)->r:Word where ensures r>=0 and r<=15 {{unsafe {{let mask:Word=15;Ops.band'intrinsic(a,mask).as(Word)}}}}
                fn annihilate(a:Word)->r:Word where ensures r==0 {{unsafe {{let mask:Word=0;Ops.band'intrinsic(a,mask).as(Word)}}}}
                fn identity(a:Word)->r:Word where requires a>=3 and a<=5, ensures r>=3 and r<=5 {{unsafe {{let mask:Word=0;Ops.bxor'intrinsic(a,mask).as(Word)}}}}
                fn ones(a:Word)->r:Word where ensures r=={mask} {{unsafe {{let mask:Word={mask};Ops.bor'intrinsic(a,mask).as(Word)}}}}
                fn exact(a:Word,b:Word)->r:Word where requires a==5, requires b==3, ensures r==6 {{unsafe {{Ops.bxor'intrinsic(a,b).as(Word)}}}}
                fn captured()->r:Word where ensures r==1 {{var a:Word=5;unsafe {{Ops.band'intrinsic(a,do {{a=3;a}}).as(Word)}}}}
                fn inverted()->r:Word where ensures r=={maximum} {{unsafe {{let lo:Word={minimum};let hi:Word={mask};Ops.bxor'intrinsic(lo,hi).as(Word)}}}}
                {extra}
                fn run()->bool {{masked({edge})<=15 and annihilate({edge})==0 and identity(4)==4 and ones(0)=={mask} and exact(5,3)==6 and captured()==1 and inverted()=={maximum} and filled({edge})>={bound}}}");
            let work = Work::new(&source);
            for level in ["0", "3"] {
                success(
                    &work
                        .command()
                        .args([
                            "--build",
                            "run",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                            "--host-intrinsic",
                            "bit_and=input.Ops.band",
                            "--host-intrinsic",
                            "bit_or=input.Ops.bor",
                            "--host-intrinsic",
                            "bit_xor=input.Ops.bxor",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = work.execute();
                success(&output);
                assert_eq!(output.stdout, b"(bool true)\n");
            }
        }
    }
}

#[test]
fn native_bitwise_contracts_reject_sign_confusion_and_stale_operands() {
    for (primitive, operation, body) in [
        (
            "i8",
            "bit_and",
            "fn bad(a:Word)->r:Word where ensures r>=0 {unsafe {let mask:Word=-1;Ops.bits'intrinsic(a,mask).as(Word)}}",
        ),
        (
            "i8",
            "bit_or",
            "fn bad(a:Word)->r:Word where ensures r>=15 {unsafe {let mask:Word=15;Ops.bits'intrinsic(a,mask).as(Word)}}",
        ),
        (
            "u8",
            "bit_and",
            "fn bad(a:Word)->r:Word where ensures r>=15 {unsafe {let mask:Word=15;Ops.bits'intrinsic(a,mask).as(Word)}}",
        ),
        (
            "u8",
            "bit_or",
            "fn bad(a:Word)->r:Word where ensures r<=15 {unsafe {let mask:Word=15;Ops.bits'intrinsic(a,mask).as(Word)}}",
        ),
        (
            "u8",
            "bit_and",
            "fn bad()->r:Word where ensures r==3 {var a:Word=5;unsafe {Ops.bits'intrinsic(a,do {a=3;a}).as(Word)}}",
        ),
        (
            "u8",
            "bit_xor",
            "fn bad()->r:Word where ensures r==0 {var a:Word=5;unsafe {Ops.bits'intrinsic(a,do {a=3;a}).as(Word)}}",
        ),
    ] {
        let source = format!(
            "^intrinsic_enum enum Ops {{bits}} typealias Word=.{primitive}'builtin; {body} fn run()->bool {{true}}"
        );
        let work = Work::new(&source);
        let output = work
            .command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--host-intrinsic",
                &format!("{operation}=input.Ops.bits"),
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted {body}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("postcondition is not proven"),
            "{output:?}"
        );
        assert!(!work.0.join("native program").exists());
    }
}

#[test]
fn computed_runtime_receivers_keep_phase_and_evaluate_once() {
    for field in ["value", "vars"] {
        for receiver in [
            "do {seen=not seen;Cell {value:flag,vars:flag}}",
            "if flag {seen=not seen;Cell {value:true,vars:true}} else {seen=not seen;Cell {value:false,vars:false}}",
            "when {flag=>do {seen=not seen;Cell {value:true,vars:true}},else=>do {seen=not seen;Cell {value:false,vars:false}}}",
            "flag match {true=>do {seen=not seen;Cell {value:true,vars:true}},false=>do {seen=not seen;Cell {value:false,vars:false}}}",
        ] {
            let source = format!(
                "{RESOURCE_HEADER}
                struct Cell {{value:bool,vars:bool}}
                fn trial(flag:bool)->bool {{
                    var seen=false;
                    let selected=({receiver}).{field};
                    selected == flag and seen
                }}
                fn run()->bool {{trial(true) and trial(false)}}"
            );
            let work = Work::new(&source);
            let reference = work
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            assert!(
                reference.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&reference.stderr)
            );
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                let output = work
                    .command()
                    .args([
                        "--build",
                        "run",
                        "--output",
                        "native program",
                        "--opt-level",
                        level,
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                    ])
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{source}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let output = work.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn computed_runtime_pointer_receivers_keep_selected_write_target() {
    let source = format!(
        "{RESOURCE_HEADER}
        struct Cell {{value:bool}}
        fn write(p:*Cell) {{(do {{let saved=p;saved}}).value.* = true;}}
        fn trial(flag:bool)->bool {{
            let first=Cell {{value:false}};let second=Cell {{value:false}};
            (if flag {{first.ref}} else {{second.ref}}).value.* = true;
            let selected=first.value == flag and second.value == not flag;
            write(first.ref);write(second.ref);
            selected and first.value and second.value
        }}
        fn run()->bool {{trial(true) and trial(false)}}"
    );
    let work = Work::new(&source);
    let reference = work
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success(),
        "{}",
        String::from_utf8_lossy(&reference.stderr)
    );
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        let output = work
            .command()
            .args([
                "--build",
                "run",
                "--output",
                "native program",
                "--opt-level",
                level,
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = work.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_handler_parameter_control_paths_preserve_writes_and_cleanup() {
    let source = include_str!("../../../development/examples/host-handler-parameter-control.fl");
    for direct in [false, true] {
        let source = if direct {
            source.replace(
                "let completed = subject(pointer, retired)#",
                "let completed = inspect(pointer, retired)#",
            )
        } else {
            source.to_owned()
        };
        for (failure, early, resume) in [
            (None, false, false),
            (Some("already retired"), false, false),
            (Some("already retired"), false, true),
            (Some("already retired"), true, false),
            (Some("write omitted"), false, false),
            (Some("write omitted"), false, true),
            (Some("write omitted"), true, false),
        ] {
            let source = if failure.is_some() {
                let prefix = source.rsplit_once("fn run()").unwrap().0;
                format!("{prefix}fn run()->bool {{trial(false,{early},{resume})}}")
            } else {
                source.clone()
            };
            let source = match failure {
                Some("already retired") => {
                    source.replace("var retired = false;", "var retired = true;")
                }
                Some("write omitted") => {
                    source.replace("value.* = not previous;", "value.* = previous;")
                }
                _ => source.clone(),
            };
            let w = Work::new(&source);
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            if failure.is_none() {
                success(&reference);
                assert_eq!(reference.stdout, b"(bool true)\n");
            } else {
                assert!(!reference.status.success(), "{failure:?}");
            }
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                if failure.is_none() {
                    success(&output);
                    assert_eq!(output.stdout, reference.stdout);
                } else {
                    assert!(
                        !output.status.success(),
                        "{failure:?}, direct={direct}, O{level}"
                    );
                    assert!(output.stdout.is_empty());
                }
            }
        }
    }
}

#[test]
fn native_handler_parameter_access_preserves_lexical_code_results() {
    let header = "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        fn outcome()->bool {true}
        fn other()->bool {false}
        fn read(pointer:*bool)->bool {pointer.*}
        fn ignore(pointer:*bool)->bool {false}
        effect inspect(pointer:*bool)->bool;
        fn subject(pointer:*bool)->#inspect bool {inspect(pointer)#}";
    for request in ["subject(pointer)#", "inspect(pointer)#"] {
        for (declaration, ty, returned, fallback, use_result) in [
            ("", "fn()->bool", "outcome", "other", "selected()"),
            (
                "",
                "fn(*bool)->bool",
                "read",
                "ignore",
                "selected(seen.ref)",
            ),
            (
                "",
                "!bool fn()->bool",
                "outcome",
                "error false",
                "selected match {call! => call(),error _=>false}",
            ),
            (
                "struct Callback {call:fn()->bool}impl Retire for Callback {fn drop(*self){}}",
                "Callback",
                "Callback {call:outcome}",
                "Callback {call:other}",
                "selected.call()",
            ),
            (
                "enum Callback {present(fn()->bool),missing}impl Retire for Callback {fn drop(*self){}}",
                "Callback",
                "Callback.present(outcome)",
                "Callback.present(other)",
                "selected match {.present(call)=>call(),.missing=>false}",
            ),
        ] {
            let w = Work::new(&format!(
                "{header}{declaration}
                 fn update(pointer:*bool)->{ty} {{
                     {request} {{inspect(value)=>do {{value.* = true;return {returned};}},}};
                     {fallback}
                 }}
                 fn run()->bool {{var seen=false;let selected=update(seen.ref);let good={use_result};good and seen}}"
            ));
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn native_handler_parameter_access_preserves_closure_return_scope() {
    let source = include_str!("../../../development/repros/host-handler-closure-return.fl");
    for direct in [false, true] {
        for captured in [false, true] {
            let mut source = source.to_owned();
            if direct {
                source = source.replace("subject(pointer)# {", "inspect(pointer)# {");
            }
            if !captured {
                source = source.replace("%{previous: bool} fn()", "fn()");
                source = source.replace("return previous;", "return true;");
                source = source.replace("|| previous", "|| false");
            }
            // The returned closure owns a snapshot; subsequent caller writes
            // must not change its result or absorb its explicit return.
            source = source.replace(
                "selected() and seen",
                "seen = false; selected() and not seen",
            );
            let w = Work::new(&source);
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn native_handler_local_pointer_closures_keep_invocation_and_escape_distinct() {
    let source = include_str!("../../../development/repros/host-handler-closure-return.fl");
    for request in ["subject(pointer)# {", "inspect(pointer)# {"] {
        for update in [
            "let change=|| {value.* = true;}; change();",
            "let borrow=|| {return value;}; let local=borrow(); local.* = true;",
            "let unused=|| {value.* = false;}; value.* = true;",
        ] {
            let source = source
                .replace("subject(pointer)# {", request)
                .replace("value.* = true;", update);
            let w = Work::new(&source);
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn checked_handler_pointer_closure_return_still_requires_exit_provenance() {
    let source = include_str!("../../../development/repros/host-handler-pointer-closure-return.fl");
    for request in ["subject(pointer)# {", "inspect(pointer)# {"] {
        let w = Work::new(&source.replace("subject(pointer)# {", request));
        let reference = w.command().args(["--interpret", "run"]).output().unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let checked = resource_build(&w);
        assert!(!checked.status.success());
        let diagnostic = String::from_utf8_lossy(&checked.stderr);
        assert!(
            diagnostic.contains("memory contract") || diagnostic.contains("provenance"),
            "{diagnostic}"
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_nested_handler_parameter_access_keeps_resume_and_return_scopes() {
    for outer in ["inspect(pointer)#", "subject(pointer)#"] {
        for inner in ["change(value)#", "modify(value)#"] {
            for (exit, expected) in [
                ("false", "not seen"),
                ("resume false;", "not seen"),
                ("return true;", "seen"),
            ] {
                let w = Work::new(&format!(
                    "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                     impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
                     effect inspect(pointer:*bool)->bool; effect change(pointer:*bool)->bool;
                     fn subject(pointer:*bool)->#inspect bool {{inspect(pointer)#}}
                     fn modify(pointer:*bool)->#change bool {{change(pointer)#}}
                     fn update(pointer:*bool)->bool {{
                        {outer} {{inspect(value)=>do {{
                            {inner} {{change(target)=>do {{target.* = true; {exit}}},}};
                            value.* = false;
                            return not value.*;
                        }},}};
                        false
                     }}
                     fn run()->bool {{var seen=false; let good=update(seen.ref); good and {expected}}}"
                ));
                let reference = w
                    .command()
                    .args([
                        "--interpret",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                    ])
                    .output()
                    .unwrap();
                success(&reference);
                assert_eq!(reference.stdout, b"(bool true)\n");
                for level in ["0", "3"] {
                    success(
                        &w.command()
                            .args([
                                "--build",
                                "run",
                                "--copy-protocol",
                                "Duplicate",
                                "--drop-protocol",
                                "Retire",
                                "--opt-level",
                                level,
                                "--output",
                                "native program",
                            ])
                            .output()
                            .unwrap(),
                    );
                    let output = w.execute();
                    success(&output);
                    assert_eq!(output.stdout, reference.stdout);
                }
            }
        }
    }
}

#[test]
fn native_handler_copy_callback_captures_preserve_request_time_calls() {
    for callback in [
        "if flag {set} else {clear}",
        "if flag {|target:*bool| {target.* = true;}} else {|target:*bool| {target.* = false;}}",
    ] {
        for request in ["inspect(pointer)#", "subject(pointer)#"] {
            for nested in [false, true] {
                let invoke = if nested {
                    "relay(value)# {relay(target)=>do {callback(target);false},};"
                } else {
                    "callback(value);"
                };
                let w=Work::new(&format!(
                    "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                     impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
                     impl Duplicate for fn(*bool) {{}}
                     effect inspect(pointer:*bool)->bool; effect relay(pointer:*bool)->bool;
                     fn subject(pointer:*bool)->#inspect bool {{inspect(pointer)#;inspect(pointer)#}}
                     fn set(value:*bool) {{value.* = true;}}
                     fn clear(value:*bool) {{value.* = false;}}
                     fn update(pointer:*bool,flag:bool)->bool {{
                        let callback={callback};
                        {request} {{inspect(value)=>do {{{invoke}value.*}},}}
                     }}
                     fn trial(flag:bool)->bool {{var seen=not flag;let good=update(seen.ref,flag);good==flag and seen==flag}}
                     fn run()->bool {{trial(true) and trial(false)}}"
                ));
                let reference = w
                    .command()
                    .args([
                        "--interpret",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                    ])
                    .output()
                    .unwrap();
                success(&reference);
                assert_eq!(reference.stdout, b"(bool true)\n");
                for level in ["0", "3"] {
                    success(
                        &w.command()
                            .args([
                                "--build",
                                "run",
                                "--copy-protocol",
                                "Duplicate",
                                "--drop-protocol",
                                "Retire",
                                "--opt-level",
                                level,
                                "--output",
                                "native program",
                            ])
                            .output()
                            .unwrap(),
                    );
                    let output = w.execute();
                    success(&output);
                    assert_eq!(output.stdout, reference.stdout);
                }
            }
        }
    }
}

#[test]
fn checked_handler_callback_capture_requires_stable_storage() {
    let source = include_str!("../../../development/repros/host-handler-captured-code.fl");
    for (proof, mutation) in [(false, true), (true, true)] {
        let source = if proof {
            source.replace(
                "impl Duplicate for bool {}",
                "impl Duplicate for bool {} impl Duplicate for fn(*bool) {}",
            )
        } else {
            source.to_owned()
        };
        let source = if mutation {
            source
                .replace("let callback = change;", "var callback = change;")
                .replace("callback(value);", "callback=change; callback(value);")
        } else {
            source
        };
        let w = Work::new(&source);
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let output = resource_build(&w);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("rebinding"), "{error}");
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_handler_copy_callback_calls_each_request_once() {
    let w = Work::new(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        impl Duplicate for fn(*bool) {}
        effect inspect(pointer:*bool)->bool;
        fn subject(pointer:*bool)->#inspect bool {inspect(pointer)#;inspect(pointer)#}
        fn toggle(value:*bool) {value.* = not value.*;}
        fn run()->bool {var seen=false;let callback=toggle;
            let result=subject(seen.ref)# {inspect(value)=>do {callback(value);value.*},};
            not result and not seen
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_handler_borrows_noncopy_callbacks_and_preserves_live_environment() {
    for callback in ["toggle", "|target:*bool| {bit=not bit;target.* = bit;}"] {
        for (request, before) in [("inspect(seen.ref)#", true), ("subject(seen.ref)#", false)] {
            for resume in [false, true] {
                for nested in [false, true] {
                    let invoke = if nested {
                        "relay(value)# {relay(target)=>do {callback(target);false},};"
                    } else {
                        "callback(value);"
                    };
                    let finish = if resume { "resume value.*;" } else { "value.*" };
                    let w=Work::new(&format!(
                        "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                         impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
                         effect inspect(pointer:*bool)->bool;effect relay(pointer:*bool)->bool;
                         fn subject(pointer:*bool)->#inspect bool {{inspect(pointer)#;inspect(pointer)#}}
                         fn toggle(value:*bool) {{value.* = not value.*;}}
                         fn run()->bool {{
                            var seen=false; var bit=false; let callback={callback};
                            let result={request} {{inspect(value)=>do {{{invoke}{finish}}},}};
                            let previous=seen;
                            callback(seen.ref);
                            result=={before} and previous=={before} and seen==not {before}
                         }}"
                    ));
                    let reference = w
                        .command()
                        .args([
                            "--interpret",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                        ])
                        .output()
                        .unwrap();
                    success(&reference);
                    assert_eq!(reference.stdout, b"(bool true)\n");
                    for level in ["0", "3"] {
                        success(
                            &w.command()
                                .args([
                                    "--build",
                                    "run",
                                    "--copy-protocol",
                                    "Duplicate",
                                    "--drop-protocol",
                                    "Retire",
                                    "--opt-level",
                                    level,
                                    "--output",
                                    "native program",
                                ])
                                .output()
                                .unwrap(),
                        );
                        let output = w.execute();
                        success(&output);
                        assert_eq!(output.stdout, reference.stdout);
                    }
                }
            }
        }
    }
}

#[test]
fn checked_handler_borrowed_callback_cannot_be_consumed_on_any_exit() {
    for finish in [
        "false",
        "resume false;",
        "return false;",
        "assumes false; false",
        "if early {consume(callback);return false;} false",
        "if early {consume(callback);assumes false;} false",
    ] {
        let consume = if finish.starts_with("if early") {
            ""
        } else {
            "consume(callback);"
        };
        let w = Work::new(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
             impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
             effect inspect(pointer:*bool)->bool;
             fn toggle(value:*bool) {{value.* = not value.*;}}
             fn consume(callback:fn(*bool)) {{}}
             fn bad(pointer:*bool,early:bool)->bool {{
                let callback=toggle;
                inspect(pointer)# {{inspect(value)=>do {{{consume}{finish}}},}}
             }}
             fn run()->bool {{var seen=false;bad(seen.ref,true)}}"
        ));
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let output = resource_build(&w);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("borrowed handler callable environment must remain complete"),
            "{error}"
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_noncopy_named_callee_observation_covers_result_shapes() {
    for (ty, body, result) in [
        ("bool", "pointer.*", "first and second"),
        ("*bool", "pointer", "first.* and second.*"),
        (
            "Pick",
            "Pick {pointer:pointer}",
            "first.pointer.* and second.pointer.*",
        ),
    ] {
        for callee in ["read", "if choose {read} else {other}"] {
            let w = Work::new(&format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                 impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
                 struct Pick {{pointer:*bool}}impl Retire for Pick {{fn drop(*self){{}}}}
                 fn read(pointer:*bool)->{ty} {{{body}}}
                 fn other(pointer:*bool)->{ty} {{{body}}}
                 fn trial(choose:bool)->bool {{var seen=true;let callback={callee};
                    let first=callback(seen.ref);let second=callback(seen.ref);{result}
                 }}
                 fn run()->bool {{trial(true) and trial(false)}}"
            ));
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn checked_named_callee_observation_does_not_restore_moved_value() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        fn read()->bool {true}
        fn run()->bool {let callback=read;let moved=callback;callback()}",
    );
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let output = resource_build(&w);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("NotPresent") || error.contains("MissingPlace"),
        "{error}"
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn native_handler_borrowed_owned_environment_keeps_state_and_final_cleanup() {
    let source = include_str!("../../../development/repros/host-handler-owned-environment.fl");
    for completion in ["resume value.*;", "value.*", "return value.*;"] {
        for failure in [None, Some("dead"), Some("unfinished")] {
            if completion == "return value.*;" && failure == Some("unfinished") {
                continue;
            }
            let source = source.replace("resume value.*;", completion);
            let source = match failure {
                Some("dead") => source.replace("live:true", "live:false"),
                Some("unfinished") => source.replace("    callback(seen.ref);", ""),
                _ => source,
            };
            let w = Work::new(&source);
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            if failure.is_none() {
                success(&reference);
                assert_eq!(reference.stdout, b"(bool true)\n");
            } else {
                assert!(!reference.status.success(), "{failure:?}");
            }
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                if failure.is_none() {
                    success(&output);
                    assert_eq!(output.stdout, reference.stdout);
                } else {
                    assert!(!output.status.success(), "{failure:?}");
                    assert!(output.stdout.is_empty());
                }
            }
        }
    }
}

#[test]
fn checked_handler_borrowed_owned_environment_cannot_deliver_its_field() {
    let source = include_str!("../../../development/repros/host-handler-owned-environment.fl")
        .replace(
            "fn run() -> bool {",
            "fn consume(state:State) {}\nfn run() -> bool {",
        )
        .replace(
            "state.bit = not state.bit;\n        target.* = state.bit;",
            "state.bit = true; target.* = true; consume(state);",
        );
    for completion in ["resume value.*;", "value.*", "return value.*;"] {
        let w = Work::new(&source.replace("resume value.*;", completion));
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let output = resource_build(&w);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("borrowed handler callable environment must remain complete"),
            "{error}"
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_noncopy_field_callee_observation_preserves_owners_and_result_origins() {
    for (ty, body, result) in [
        ("bool", "pointer.*", "first and second"),
        ("*bool", "pointer", "first.* and second.*"),
        (
            "Pick",
            "Pick {pointer:pointer}",
            "first.pointer.* and second.pointer.*",
        ),
    ] {
        for chosen in ["read", "if choose {read} else {other}"] {
            for (init, field) in [
                (format!("Actions {{call:{chosen}}}"), "actions.call"),
                (
                    format!("Outer {{inner:Actions {{call:{chosen}}}}}"),
                    "actions.inner.call",
                ),
                (format!("pack {{call:{chosen}}}"), "actions.call"),
            ] {
                let w=Work::new(&format!(
                    "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                     impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
                     struct Pick {{pointer:*bool}}impl Retire for Pick {{fn drop(*self){{}}}}
                     struct Actions {{call:fn(*bool)->{ty}}}impl Retire for Actions {{fn drop(*self){{}}}}
                     struct Outer {{inner:Actions}}impl Retire for Outer {{fn drop(*self){{}}}}
                     fn pack(...fields:T)->T where T {{fields}}
                     fn read(pointer:*bool)->{ty} {{{body}}}
                     fn other(pointer:*bool)->{ty} {{{body}}}
                     fn trial(choose:bool)->bool {{var seen=true;let actions={init};
                        let first={field}(seen.ref);let second={field}(seen.ref);{result}
                     }}
                     fn run()->bool {{trial(true) and trial(false)}}"
                ));
                let reference = w
                    .command()
                    .args([
                        "--interpret",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                    ])
                    .output()
                    .unwrap();
                success(&reference);
                assert_eq!(reference.stdout, b"(bool true)\n");
                for level in ["0", "3"] {
                    success(
                        &w.command()
                            .args([
                                "--build",
                                "run",
                                "--copy-protocol",
                                "Duplicate",
                                "--drop-protocol",
                                "Retire",
                                "--opt-level",
                                level,
                                "--output",
                                "native program",
                            ])
                            .output()
                            .unwrap(),
                    );
                    let output = w.execute();
                    success(&output);
                    assert_eq!(output.stdout, reference.stdout);
                }
            }
        }
    }
}

#[test]
fn native_noncopy_field_callee_is_selected_before_argument_rebinding() {
    let w = Work::new(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        struct Actions {call:fn(*bool,*bool)->*bool}impl Retire for Actions {fn drop(*self){}}
        fn first(left:*bool,right:*bool)->*bool {left}
        fn second(left:*bool,right:*bool)->*bool {right}
        fn run()->bool {var left=false;var right=false;var actions=Actions {call:first};
            let selected=actions.call(do {actions.call=second;left.ref},right.ref);
            selected.* = true;
            let before=left and not right;
            let later=actions.call(left.ref,right.ref);later.* = true;
            before and left and right
        }",
    );
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_noncopy_field_callee_observes_only_its_owned_position() {
    let w=Work::new("trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}
        struct Token {live:bool}impl Retire for Token {fn drop(*self){assumes self.live.*;self.live.* = false;}}
        fn pack(...fields:T)->T where T {fields}
        fn ready()->bool {true}
        fn consume(token:Token) {}
        fn run()->bool {let actions=pack {call:ready,payload:Token {live:true}};
            consume(actions.payload);actions.call() and actions.call()
        }");
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn checked_noncopy_field_callee_does_not_restore_a_moved_field() {
    let source = include_str!("../../../development/repros/host-noncopy-field-callee.fl").replace(
        "    actions.call() and actions.call()",
        "    let moved=actions.call; actions.call()",
    );
    let w = Work::new(&source);
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let output = resource_build(&w);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("unavailable")
            || error.contains("NotPresent")
            || error.contains("MissingPlace"),
        "{error}"
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn native_field_closure_keeps_state_and_owned_cleanup() {
    let source = include_str!("../../../development/repros/host-field-closure-state.fl");
    let owned = source.replace("fn pack", "struct State {bit:bool,live:bool}\nimpl Retire for State {fn drop(*self){assumes self.live.*;assumes not self.bit.*;self.live.* = false;}}\nfn pack")
        .replace("var bit=false;", "var state=State {bit:false,live:true};")
        .replace("bit=not bit;bit", "state.bit=not state.bit;state.bit");
    for source in [source.to_owned(), owned] {
        for nested in [false, true] {
            let source = if nested {
                source
                    .replace("pack {call:||", "pack {inner:pack {call:||")
                    .replace("}};", "}}};")
                    .replace("actions.call()", "actions.inner.call()")
            } else {
                source.clone()
            };
            let w = Work::new(&source);
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn native_field_closure_pointer_writeback_preserves_siblings() {
    let w=Work::new("trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        fn pack(...fields:T)->T where T {fields}
        fn run()->bool {var left=false;var right=false;var sibling=false;
            var current=left.ref;
            let actions=pack {saved:sibling.ref,inner:pack {call:|next:*bool| {let old=current;current=next;old}}};
            let first=actions.inner.call(right.ref);first.* = true;
            let second=actions.inner.call(left.ref);second.* = true;
            actions.saved.* = true;
            left and right and sibling
        }");
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn checked_field_closure_does_not_revive_consumed_captures() {
    let w=Work::new("trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}
        struct Token {live:bool}impl Retire for Token {fn drop(*self){assumes self.live.*;self.live.* = false;}}
        fn pack(...fields:T)->T where T {fields}
        fn consume(token:Token) {}
        fn run()->bool {let token=Token {live:true};let actions=pack {call:|| {consume(token);true}};
            actions.call() and actions.call()
        }");
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let output = resource_build(&w);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("unavailable")
            || error.contains("NotPresent")
            || error.contains("MissingPlace"),
        "{error}"
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn native_field_closure_failure_writes_back_consumed_environment() {
    for consumption in ["", "captured;"] {
        let source=format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            typealias Count=.u32'builtin;
            struct Token {{target:*Count}}
            impl Retire for Token {{fn drop(*self){{let n=self.target.*.*;if n < 100 {{self.target.*.* = n + 1;}}}}}}
            fn pack(...fields:T)->T where T {{fields}}
            fn trial(target:*Count,again:bool) {{let outer=Token {{target:target}};let captured=Token {{target:target}};
                let actions=pack {{inner:pack {{call:|| {{{consumption} assumes not again;captured;}}}}}};
                actions.inner.call();target.* = 99;
            }}
            fn run()->bool {{let count:Count=0;trial(count.ref,true);false}}");
        // The unreachable final reference keeps the capture present in both
        // variants; it must not consume it twice on the successful path.
        let source = if consumption.is_empty() {
            source
        } else {
            source.replace("assumes not again;captured;", "assumes not again;")
        };
        assert_native_failure_cleans_two_frames(&source);
    }
}

#[test]
fn native_field_closure_enum_pointer_guards_follow_receiver_path() {
    let w=Work::new("trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        enum Choice {none,left(*bool),right(*bool)}impl Duplicate for Choice {}
        fn pack(...fields:T)->T where T {fields}
        fn write(value:Choice)->bool {value match {.none=>false,.left(p)=>do {p.* = true;true},.right(p)=>do {p.* = true;true},}}
        fn trial(flag:bool)->bool {var left=false;var right=false;var sibling=false;
            var selected=if flag {Choice.left(left.ref)} else {Choice.right(right.ref)};
            let actions=pack {saved:sibling.ref,inner:pack {call:|next:Choice| {let previous=selected;selected=next;previous}}};
            let first=actions.inner.call(if flag {Choice.right(right.ref)} else {Choice.left(left.ref)});
            let second=actions.inner.call(Choice.none);
            actions.saved.* = true;
            write(first) and write(second) and left and right and sibling
        }
        fn run()->bool {trial(true) and trial(false)}");
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_field_closure_conditional_consumption_cleans_remaining_environment() {
    let source=include_str!("../../../development/examples/host-closure-consume.fl")
        .replace("fn trial", "fn pack(...fields:T)->T where T {fields}\nfn trial")
        .replace("        callback(flag);", "        let actions=pack {inner:pack {call:callback}};\n        actions.inner.call(flag);");
    for deliver_after in [false, true] {
        let source = if deliver_after {
            source.replace(
                "actions.inner.call(flag);",
                "actions.inner.call(flag);finish(actions.inner.call);",
            )
        } else {
            source.clone()
        };
        let w = Work::new(&source);
        if deliver_after {
            let output = resource_build(&w);
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("unavailable"));
            continue;
        }
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_owned_field_callback_crosses_generic_return_and_default_frames() {
    let original = include_str!("../../../development/examples/host-owned-field-callback.fl");
    for route in ["tail", "return", "explicit"] {
        let source = match route {
            "return" => original.replace("    callback\n}", "    return callback;\n}"),
            "explicit" => original.replace(
                "warm(actions.call, seen.ref)",
                "warm(actions.call, seen.ref, .twice = true)",
            ),
            _ => original.to_owned(),
        };
        for failure in [
            None,
            Some("too_few"),
            Some("too_many"),
            Some("already_retired"),
        ] {
            let source = match failure {
                Some("too_few") => source.replace("        packet.inner.call(seen.ref);", ""),
                Some("too_many") => source.replace(
                    "        packet.inner.call(seen.ref);",
                    "        packet.inner.call(seen.ref); packet.inner.call(seen.ref);",
                ),
                Some("already_retired") => source.replace("var done = false;", "var done = true;"),
                _ => source.clone(),
            };
            let w = Work::new(&source);
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            assert_eq!(
                reference.status.success(),
                failure.is_none(),
                "{route} {failure:?}: {}",
                String::from_utf8_lossy(&reference.stderr)
            );
            if failure.is_none() {
                assert_eq!(reference.stdout, b"(bool true)\n");
            }
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                assert_eq!(
                    output.status.success(),
                    failure.is_none(),
                    "{route} {failure:?}: {output:?}"
                );
                if failure.is_none() {
                    assert_eq!(output.stdout, reference.stdout);
                } else {
                    assert!(output.stdout.is_empty());
                }
            }
        }
    }
}

#[test]
fn checked_owned_field_callback_cannot_reuse_transferred_record_field() {
    let original = include_str!("../../../development/examples/host-owned-field-callback.fl");
    let source = original.replace(
        "        let packet =",
        "        actions.call(seen.ref);\n        let packet =",
    );
    let w = Work::new(&source);
    fs::write(w.0.join("native program"), b"previous").unwrap();
    let output = resource_build(&w);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("unavailable")
            || error.contains("NotPresent")
            || error.contains("MissingPlace"),
        "{error}"
    );
    assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
}

#[test]
fn native_handler_borrows_record_callback_fields() {
    let original = include_str!("../../../development/repros/host-handler-record-callback.fl");
    for (nominal, nested) in [(false, false), (true, false), (false, true)] {
        let source = if nominal {
            original.replace("fn pack", "struct Actions {call:%{state:State} fn(*bool)}\nstruct Outer {inner:Actions}\nimpl Retire for Actions {fn drop(*self){}}\nimpl Retire for Outer {fn drop(*self){}}\nfn pack")
            .replace("pack {inner:pack {call:callback}}", "Outer {inner:Actions {call:callback}}")
        } else {
            original.to_owned()
        };
        let source = if nested {
            source.replace("actions.inner.call(value);", "subject(value)# {inspect(inner)=>do {actions.inner.call(inner);resume inner.*;},};")
        } else {
            source
        };
        for completion in ["resume value.*;", "value.*", "return value.*;"] {
            let source = source.replace("resume value.*;", completion);
            let source = if nested && completion == "return value.*;" {
                source.replace("subject(value)#", "inspect(value)#")
            } else {
                source
            };
            let w = Work::new(&source);
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn checked_handler_record_callback_cannot_be_delivered_on_any_exit() {
    let original = include_str!("../../../development/repros/host-handler-record-callback.fl")
        .replace("fn run()", "fn discard(value:T) where T {}\nfn run()")
        .replace("fn pack", "struct Token {live:bool}impl Retire for Token {fn drop(*self){assumes self.live.*;self.live.* = false;}}\nfn pack")
        .replace("pack {inner:pack {call:callback}}", "pack {inner:pack {call:callback},token:Token {live:true}}");
    for delivered in ["actions.inner.call", "actions.token"] {
        for completion in [
            "resume value.*;",
            "value.*",
            "return value.*;",
            "assumes false; value.*",
        ] {
            let source = original.replace(
                "resume value.*;",
                &format!("discard({delivered});{completion}"),
            );
            let w = Work::new(&source);
            fs::write(w.0.join("native program"), b"previous").unwrap();
            let output = resource_build(&w);
            assert!(!output.status.success());
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains("borrowed handler callable environment must remain complete"),
                "{error}"
            );
            assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
        }
    }
}

#[test]
fn native_handler_record_named_candidates_preserve_field_identity_and_order() {
    let w=Work::new("trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        struct Actions {first:fn(*bool),second:fn(*bool)}impl Retire for Actions {fn drop(*self){}}
        fn set_true(target:*bool){assumes not target.*;target.* = true;}
        fn also_true(target:*bool){assumes not target.*;target.* = true;}
        fn set_false(target:*bool){assumes target.*;target.* = false;}
        effect step(target:*bool)->bool;
        fn subject(target:*bool)->#step bool {step(target)#;step(target)#}
        fn trial(choose:bool)->bool {var seen=false;
            let actions=Actions {first:if choose {set_true}else{also_true},second:set_false};
            let result=subject(seen.ref)# {step(value)=>do {actions.first(value);actions.second(value);resume not value.*;},};
            actions.first(seen.ref);result and seen
        }
        fn run()->bool {trial(true) and trial(false)}");
    let reference = w
        .command()
        .args([
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ])
        .output()
        .unwrap();
    success(&reference);
    assert_eq!(reference.stdout, b"(bool true)\n");
    for level in ["0", "3"] {
        success(
            &w.command()
                .args([
                    "--build",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                    "--opt-level",
                    level,
                    "--output",
                    "native program",
                ])
                .output()
                .unwrap(),
        );
        let output = w.execute();
        success(&output);
        assert_eq!(output.stdout, reference.stdout);
    }
}

#[test]
fn native_handler_record_data_writes_preserve_disjoint_code_fields() {
    let original = include_str!("../../../development/repros/host-handler-record-state.fl");
    for nested in [false, true] {
        let source = if nested {
            original.replace("fn identity", "struct Outer {inner:Actions}\nimpl Retire for Outer {fn drop(*self){}}\nfn identity")
                .replace("var actions = Actions {call: identity, flag: false};", "var actions = Outer {inner:Actions {call: identity, flag: false}};")
                .replace("actions.flag", "actions.inner.flag").replace("actions.call", "actions.inner.call")
        } else {
            original.to_owned()
        };
        for completion in ["resume ", "", "return "] {
            let source = source.replace("resume ", completion);
            // A plain arm result must not be discarded by a trailing semicolon.
            let source = if completion.is_empty() {
                source
                    .replace("actions.call(actions.flag);", "actions.call(actions.flag)")
                    .replace(
                        "actions.inner.call(actions.inner.flag);",
                        "actions.inner.call(actions.inner.flag)",
                    )
            } else {
                source
            };
            let source = source.replace(
                "impl Retire for Actions { fn drop(*self) {} }",
                if completion == "return " {
                    "impl Retire for Actions { fn drop(*self) {assumes self.flag.*;} }"
                } else {
                    "impl Retire for Actions { fn drop(*self) {assumes not self.flag.*;} }"
                },
            );
            let w = Work::new(&source);
            let reference = w
                .command()
                .args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "Retire",
                ])
                .output()
                .unwrap();
            success(&reference);
            assert_eq!(reference.stdout, b"(bool true)\n");
            for level in ["0", "3"] {
                success(
                    &w.command()
                        .args([
                            "--build",
                            "run",
                            "--copy-protocol",
                            "Duplicate",
                            "--drop-protocol",
                            "Retire",
                            "--opt-level",
                            level,
                            "--output",
                            "native program",
                        ])
                        .output()
                        .unwrap(),
                );
                let output = w.execute();
                success(&output);
                assert_eq!(output.stdout, reference.stdout);
            }
        }
    }
}

#[test]
fn checked_handler_record_code_overlap_and_address_exposure_still_reject() {
    let original = include_str!("../../../development/repros/host-handler-record-state.fl");
    for mutation in [
        "actions.call = identity;",
        "actions = Actions {call:identity,flag:false};",
        "let alias=actions.ref;",
        "let alias=actions.call.ref;",
        "let alias=actions.flag.ref;",
    ] {
        for before_install in [false, true] {
            let source = if before_install {
                original.replace(
                    "    let result =",
                    &format!("    {mutation}\n    let result ="),
                )
            } else {
                original.replace(
                    "            actions.flag =",
                    &format!("            {mutation}\n            actions.flag ="),
                )
            };
            let w = Work::new(&source);
            if before_install && !mutation.contains(".ref") {
                for level in ["0", "3"] {
                    success(
                        &w.command()
                            .args([
                                "--build",
                                "run",
                                "--copy-protocol",
                                "Duplicate",
                                "--drop-protocol",
                                "Retire",
                                "--opt-level",
                                level,
                                "--output",
                            ])
                            .arg(w.0.join("native program"))
                            .output()
                            .unwrap(),
                    );
                    let output = Command::new(w.0.join("native program")).output().unwrap();
                    success(&output);
                    assert_eq!(output.stdout, b"(bool true)\n");
                }
                continue;
            }
            fs::write(w.0.join("native program"), b"previous").unwrap();
            let output = resource_build(&w);
            assert!(!output.status.success());
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains("writeback contract")
                    || error.contains("environment memory contract"),
                "{mutation}: {error}"
            );
            assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
        }
    }
}

#[test]
fn native_handler_borrows_owned_data_and_preserves_final_cleanup() {
    let original = include_str!("../../../development/repros/host-handler-owned-data.fl");
    for nested in [false, true] {
        let source = if nested {
            original.replace("effect step", "struct Outer {inner:Actions}\nimpl Retire for Outer {fn drop(*self){}}\neffect step")
            .replace("var actions = Actions {flag: false, live: true};", "var actions = Outer {inner:Actions {flag: false, live: true}};")
            .replace("actions.flag", "actions.inner.flag").replace("actions.live", "actions.inner.live")
        } else {
            original.to_owned()
        };
        for completion in ["resume ", "", "return "] {
            let source = source.replace("resume ", completion);
            let source = if completion.is_empty() {
                source
                    .replace("            actions.flag;", "            actions.flag")
                    .replace(
                        "            actions.inner.flag;",
                        "            actions.inner.flag",
                    )
            } else {
                source
            };
            let source = if completion == "return " {
                source.replace("assumes not self.flag.*;", "assumes self.flag.*;")
            } else {
                source
            };
            for valid in [false, true] {
                let source = if valid {
                    source.clone()
                } else {
                    source.replace("live: true", "live: false")
                };
                let w = Work::new(&source);
                let reference = w
                    .command()
                    .args([
                        "--interpret",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                    ])
                    .output()
                    .unwrap();
                assert_eq!(
                    reference.status.success(),
                    valid,
                    "{}",
                    String::from_utf8_lossy(&reference.stderr)
                );
                if valid {
                    assert_eq!(reference.stdout, b"(bool true)\n");
                }
                for level in ["0", "3"] {
                    success(
                        &w.command()
                            .args([
                                "--build",
                                "run",
                                "--copy-protocol",
                                "Duplicate",
                                "--drop-protocol",
                                "Retire",
                                "--opt-level",
                                level,
                                "--output",
                                "native program",
                            ])
                            .output()
                            .unwrap(),
                    );
                    let output = w.execute();
                    assert_eq!(output.status.success(), valid, "{output:?}");
                    if valid {
                        assert_eq!(output.stdout, reference.stdout);
                    } else {
                        assert!(output.stdout.is_empty());
                    }
                }
            }
        }
    }
}

#[test]
fn checked_handler_owned_data_cannot_be_consumed_on_any_exit() {
    let original = include_str!("../../../development/repros/host-handler-owned-data.fl")
        .replace("fn run()", "fn discard(value:T) where T {}\nfn run()");
    for completion in [
        "resume false;",
        "false",
        "return false;",
        "assumes false;false",
    ] {
        let source = original.replace(
            "resume actions.flag;",
            &format!("discard(actions);{completion}"),
        );
        let w = Work::new(&source);
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let output = resource_build(&w);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("borrowed handler data capture must remain complete"),
            "{error}"
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_handler_restored_owner_preserves_cleanup_across_exits() {
    let original = include_str!("../../../development/repros/host-handler-restored-owner.fl");
    for (completion, failure) in [
        ("resume state.flag;", false),
        ("state.flag", false),
        ("return state.flag;", false),
        ("assumes false;resume state.flag;", true),
    ] {
        let w = Work::new(&original.replace("resume state.flag;", completion));
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        assert_eq!(
            reference.status.success(),
            !failure,
            "{}",
            String::from_utf8_lossy(&reference.stderr)
        );
        if !failure {
            assert_eq!(reference.stdout, b"(bool true)\n");
        }
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            assert_eq!(output.status.success(), !failure, "{output:?}");
            if !failure {
                assert_eq!(output.stdout, reference.stdout);
            } else {
                assert!(output.stdout.is_empty());
            }
        }
    }
}

#[test]
fn checked_handler_restored_owner_rejects_exits_before_restoration() {
    let original = include_str!("../../../development/repros/host-handler-restored-owner.fl");
    for exit in ["assumes false;", "return false;", "resume false;"] {
        let source = original.replace(
            "            state = State",
            &format!("            {exit}\n            state = State"),
        );
        let w = Work::new(&source);
        fs::write(w.0.join("native program"), b"previous").unwrap();
        let output = resource_build(&w);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("borrowed handler data capture must remain complete"),
            "{error}"
        );
        assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
    }
}

#[test]
fn native_handler_enum_callbacks_preserve_tags_and_mutable_data() {
    let original = include_str!("../../../development/repros/host-handler-enum-callback.fl");
    for known in [
        None,
        Some("Choice.none"),
        Some("Choice.first(identity)"),
        Some("Choice.second(other)"),
    ] {
        let source = if let Some(value) = known {
            original.replace("if empty {Choice.none} else if tag {Choice.first(identity)} else {Choice.second(other)}",value)
        } else {
            original.to_owned()
        };
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn checked_handler_enum_code_cannot_rebind_a_previously_absent_variant() {
    let original=include_str!("../../../development/repros/host-handler-enum-callback.fl")
        .replace("if empty {Choice.none} else if tag {Choice.first(identity)} else {Choice.second(other)}", "Choice.none");
    for mutation in [
        "actions.selected = Choice.first(identity);",
        "let exposed=actions.selected.ref;",
    ] {
        for before in [true, false] {
            let source = if before {
                original.replace(
                    "    let result =",
                    &format!("    {mutation}\n    let result ="),
                )
            } else {
                original.replace(
                    "            actions.flag =",
                    &format!("            {mutation}\n            actions.flag ="),
                )
            };
            let w = Work::new(&source);
            if before && !mutation.contains(".ref") {
                for level in ["0", "3"] {
                    success(
                        &w.command()
                            .args([
                                "--build",
                                "run",
                                "--copy-protocol",
                                "Duplicate",
                                "--drop-protocol",
                                "Retire",
                                "--opt-level",
                                level,
                                "--output",
                            ])
                            .arg(w.0.join("native program"))
                            .output()
                            .unwrap(),
                    );
                    let output = Command::new(w.0.join("native program")).output().unwrap();
                    success(&output);
                    assert_eq!(output.stdout, b"(bool true)\n");
                }
                continue;
            }
            fs::write(w.0.join("native program"), b"previous").unwrap();
            let output = resource_build(&w);
            assert!(!output.status.success());
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains("writeback contract")
                    || error.contains("environment memory contract"),
                "{error}"
            );
            assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
        }
    }
}

#[test]
fn native_handler_nested_enum_callbacks_keep_conditional_paths() {
    let original = include_str!("../../../development/repros/host-handler-enum-callback.fl");
    let selection = "if empty {Envelope.empty} else {Envelope.wrapped(if tag {Choice.first(identity)} else {Choice.second(other)})}";
    let source=original.replace("struct Actions", "enum Envelope {empty,wrapped(Choice)}\nimpl Duplicate for Envelope {}\nstruct Actions")
        .replace("selected: Choice", "selected: Envelope")
        .replace("if empty {Choice.none} else if tag {Choice.first(identity)} else {Choice.second(other)}",selection)
        .replace("actions.selected match {", "actions.selected match {.empty=>actions.flag,.wrapped(choice)=>choice match {")
        .replace("            }\n        },", "            },}\n        },");
    for known_absent in [false, true] {
        let source = if known_absent {
            source.replace(selection, "Envelope.wrapped(Choice.none)")
        } else {
            source.clone()
        };
        let w = Work::new(&source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn native_handler_callback_snapshot_is_per_installation() {
    for source in [
        include_str!("../../../development/repros/host-handler-callback-installation.fl"),
        include_str!("../../../development/repros/host-handler-nested-callback-installation.fl"),
    ] {
        let w = Work::new(source);
        let reference = w
            .command()
            .args([
                "--interpret",
                "run",
                "--copy-protocol",
                "Duplicate",
                "--drop-protocol",
                "Retire",
            ])
            .output()
            .unwrap();
        success(&reference);
        assert_eq!(reference.stdout, b"(bool true)\n");
        for level in ["0", "3"] {
            success(
                &w.command()
                    .args([
                        "--build",
                        "run",
                        "--copy-protocol",
                        "Duplicate",
                        "--drop-protocol",
                        "Retire",
                        "--opt-level",
                        level,
                        "--output",
                        "native program",
                    ])
                    .output()
                    .unwrap(),
            );
            let output = w.execute();
            success(&output);
            assert_eq!(output.stdout, reference.stdout);
        }
    }
}

#[test]
fn checked_handler_callback_snapshot_covers_nested_installations() {
    // The inner installation can finish before an outer request mutates the
    // same slot. The still-active outer snapshot must not inherit stale code.
    for mutation in ["callback = identity;", "let exposed = callback.ref;"] {
        for site in ["computation", "outer", "inner"] {
            let computation = if site == "computation" { mutation } else { "" };
            let outer = if site == "outer" { mutation } else { "" };
            let inner = if site == "inner" { mutation } else { "" };
            let source = format!(
                "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                impl Duplicate for bool {{}}
                fn identity(value:bool)->bool {{value}}
                fn invert(value:bool)->bool {{not value}}
                effect step()->bool; effect nested()->bool;
                fn subject()->#step bool {{step()#;step()#}}
                fn run()->bool {{
                    var callback=invert;
                    (do {{{computation} subject()}})# {{step()=>do {{
                        let result=nested()# {{nested()=>do {{{inner} callback(false)}},}};
                        {outer}
                        resume result;
                    }},}}
                }}"
            );
            let w = Work::new(&source);
            fs::write(w.0.join("native program"), b"previous").unwrap();
            let output = resource_build(&w);
            assert!(!output.status.success(), "{site}: {mutation}");
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains("writeback contract")
                    || error.contains("environment memory contract"),
                "{site}: {mutation}: {error}"
            );
            assert_eq!(fs::read(w.0.join("native program")).unwrap(), b"previous");
        }
    }
}

#[test]
fn quoted_tuple_contract_requires_explicit_copy_and_preserves_caller() {
    let source = include_str!("../../../development/examples/host-quote-contract-patterns.fl");
    for has_copy in [true, false] {
        let source = if has_copy {
            source.to_owned()
        } else {
            source.replace("impl Duplicate for Tuple<bool, bool> {}", "")
        };
        let w = Work::new(&source);
        fs::write(w.0.join("tuple.fl"), include!("support/tuple_source.rs")).unwrap();
        fs::write(w.0.join("meta.fl"), include!("support/meta_source.rs")).unwrap();
        fs::write(
            w.0.join("resources.fl"),
            include!("support/resources_source.rs"),
        )
        .unwrap();
        for level in ["0", "3"] {
            let output = Command::new(env!("CARGO_BIN_EXE_luna"))
                .current_dir(&w.0)
                .args([
                    "--packages-v1",
                    "--builtin-impl-package",
                    "3",
                    "--package",
                    "1",
                    "101",
                    "app",
                    "source.fl",
                    "--package",
                    "2",
                    "202",
                    "tuples",
                    "tuple.fl",
                    "--package",
                    "3",
                    "303",
                    "meta",
                    "meta.fl",
                    "--package",
                    "4",
                    "404",
                    "resources",
                    "resources.fl",
                    "--dependency",
                    "1",
                    "tuples",
                    "2",
                    "--dependency",
                    "1",
                    "meta",
                    "3",
                    "--dependency",
                    "1",
                    "resources",
                    "4",
                    "--dependency",
                    "2",
                    "meta",
                    "3",
                    "--primary",
                    "1",
                    "--tuple-type",
                    "tuples.Tuple",
                    "--meta-list-type",
                    "meta.List",
                    "--copy-protocol",
                    "Duplicate",
                    "--drop-protocol",
                    "resources.Drop",
                    "--fuel",
                    "1000000",
                    "--build",
                    "run",
                    "--output",
                    "native program",
                    "--opt-level",
                    level,
                ])
                .output()
                .unwrap();
            if has_copy {
                success(&output);
                let result = w.execute();
                success(&result);
                assert_eq!(result.stdout, b"(bool true)\n");
            } else {
                assert!(!output.status.success());
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("guard expression changes caller resources"),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(!w.0.join("native program").exists());
            }
        }
    }
}
