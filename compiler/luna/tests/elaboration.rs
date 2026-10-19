use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn extended_post_do_rejects_duplicate_inputs_and_ambiguous_markers() {
    let schema = "comptime fn schema(...properties:T)->T where T {properties} const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;";
    rejected(
        &format!(
            "{schema} fn apply(lambda action:fn()->bool,...properties:Options) {{}} fn run() {{apply {{flag:true,flag:false}} do {{true}};}}"
        ),
        "property supplied more than once",
    );
    rejected(
        &format!(
            "{schema} fn apply(lambda first:fn()->bool,lambda second:fn()->bool,...properties:Options) {{}} fn run() {{apply {{flag:true}} do {{true}};}}"
        ),
        "ambiguous post do lambda parameter",
    );
    rejected(
        &format!(
            "{schema} fn apply(lambda action:fn()->bool,...properties:Options) {{}} fn run() {{apply {{|| true,flag:true}} do {{true}};}}"
        ),
        "positional inputs cannot append",
    );
}

#[test]
fn tactic_lambda_rejects_missing_boundaries_out_of_range_and_unknown_arity() {
    rejected(
        "fn apply(action:fn(bool)->bool)->bool {action(true)} fn run()->bool {apply(_0)}",
        "lambda-marked argument",
    );
    rejected(
        "fn apply(lambda action:fn(bool)->bool)->bool {action(true)} fn run()->bool {apply(_1)}",
        "exceeds target arity",
    );
    rejected(
        "fn apply(lambda action:fn(bool)->bool)->bool {action(true)} fn run()->bool {apply((|| _0)())}",
        "lambda-marked argument",
    );
    rejected("fn run()->bool {_0}", "lambda-marked argument");
    rejected(
        "fn apply(lambda action:T)->T where T {action} fn run() {let f=apply(_0);}",
        "context",
    );
    rejected(
        "fn invalid(.value:bool=_0)->bool {value} fn apply(lambda action:fn(bool)->bool)->bool {action(true)} fn run()->bool {apply(invalid())}",
        "lambda-marked argument",
    );
    rejected(
        "fn apply(action:fn()->bool)->bool {action()} fn run()->bool {apply() do {_0}}",
        "lambda-marked argument",
    );
    rejected(
        "fn outer(lambda action:fn(bool)->bool)->bool {action(true)} fn inner(lambda action:fn(bool)->bool)->bool {action(false)} fn run()->bool {outer(inner(_0))}",
        "type",
    );
}

#[test]
fn post_do_rejects_missing_ambiguous_and_duplicate_targets() {
    rejected(
        "fn apply() {} fn run() {apply() do {true};}",
        "no anchored parameter target",
    );
    rejected(
        "fn apply(lambda first:fn()->bool,lambda second:fn()->bool) {} fn run() {apply() do {true};}",
        "ambiguous post do lambda parameter",
    );
    rejected(
        "fn apply(action:fn()->bool) {} fn run() {apply(|| true) do {false};}",
        "argument count mismatch",
    );
    rejected(
        "fn apply(value:bool) {} fn run() {apply() do {true};}",
        "lambda target is not callable",
    );
    rejected(
        "fn apply(lambda action:fn()->bool) {} fn run() {let callback=apply;}",
        "lambda-marked function values require parameter protocol metadata",
    );
}

#[test]
fn implicit_explicit_supply_rejects_missing_duplicate_and_erased_protocols() {
    rejected(
        "fn take(implicit value:bool)->bool {value} fn run()->bool {take()}",
        "automatic supply protocol is not yet implemented",
    );
    rejected(
        "fn take(implicit value:bool)->bool {value} fn run()->bool {take(true)}",
        "too many positional inputs",
    );
    rejected(
        "fn take(implicit value:bool)->bool {value} fn run()->bool {take(.value=true,.value=false)}",
        "parameter supplied more than once",
    );
    rejected(
        "fn take(implicit value:bool)->bool {value} fn run()->bool {take(.value='x')}",
        "type",
    );
    rejected(
        "fn take(implicit value:bool)->bool {value} fn run()->bool {let callback=take;callback(true)}",
        "implicit function values require parameter protocol metadata",
    );
    rejected(
        "trait Pick {fn pick(implicit value:bool)->bool;} impl Pick for bool {fn pick(implicit value:bool)->bool {value}} fn run()->bool {let callback=bool.as(Pick).pick;callback(true)}",
        "implicit function values require parameter protocol metadata",
    );
}

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(source: &str) -> Self {
        loop {
            let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("luna-elaboration-{}-{serial}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    fs::write(path.join("input.fl"), source).unwrap();
                    return Self(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create {}: {error}", path.display()),
            }
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_luna"));
        command
            .current_dir(&self.0)
            .env_remove("FLURRY_SYSROOT")
            .env("NO_COLOR", "1")
            .env("RUST_BACKTRACE", "0")
            .arg("input.fl")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        // Drain concurrently so error recovery cannot fill a pipe and hang a test.
        fn drain(mut pipe: impl Read) -> Vec<u8> {
            const LIMIT: usize = 1024 * 1024;
            let mut result = Vec::new();
            let mut buffer = [0; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => return result,
                    Ok(count) => {
                        let keep = count.min(LIMIT.saturating_sub(result.len()));
                        result.extend_from_slice(&buffer[..keep]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => panic!("cannot read CLI output: {error}"),
                }
            }
        }
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let stdout_reader = thread::spawn(move || drain(stdout));
        let stderr_reader = thread::spawn(move || drain(stderr));
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                timed_out = true;
                let _ = child.kill();
                break child.wait().unwrap();
            }
            thread::sleep(Duration::from_millis(10));
        };
        let output = Output {
            status,
            stdout: stdout_reader.join().unwrap(),
            stderr: stderr_reader.join().unwrap(),
        };
        assert!(!timed_out, "CLI timeout: {}", details(&output));
        output
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn details(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn accepted(source: &str) -> String {
    let output = Sandbox::new(source).run(&["--emit", "nir"]);
    assert!(output.status.success(), "{}", details(&output));
    assert!(output.stderr.is_empty(), "{}", details(&output));
    let dump = String::from_utf8(output.stdout).unwrap();
    assert!(dump.starts_with("(nir-package"));
    dump
}
fn rejected(source: &str, message: &str) {
    let sandbox = Sandbox::new(source);
    fs::write(sandbox.0.join("result"), "previous output").unwrap();
    let output = sandbox.run(&["--emit", "nir", "--output", "result"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "source: {source}\n{}",
        details(&output)
    );
    assert!(output.stdout.is_empty(), "{}", details(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(message),
        "{}",
        details(&output)
    );
    assert_eq!(
        fs::read_to_string(sandbox.0.join("result")).unwrap(),
        "previous output"
    );
}

#[test]
fn symbol_indexed_trait_declarations_preserve_domains_and_select_complete_instances() {
    let source = "typealias Symbol=.Symbol'builtin;
        trait Indexed where suffix:Symbol,R {}
        struct S {} impl Indexed<.px,bool> for S {}
        fn need(value:T) where T :- Indexed<.px,bool> {}
        fn run()->bool {need(S {});true}";
    let dump = accepted(source);
    assert!(
        dump.contains("(parameter-types (primitive Symbol) (primitive Type))"),
        "{dump}"
    );
    let executed = Sandbox::new(source).run(&["--interpret", "run"]);
    assert!(executed.status.success(), "{}", details(&executed));
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout).trim(),
        "(bool true)"
    );
    rejected(
        &source.replace("impl Indexed<.px,bool>", "impl Indexed<.em,bool>"),
        "trait bound",
    );
    rejected(
        &source.replace("Indexed<.px,bool> for S", "Indexed<\"px\",bool> for S"),
        "mismatch",
    );
}

#[test]
fn symbol_trait_defaults_use_prior_inputs_and_do_not_become_types() {
    let inherited = "typealias Symbol=.Symbol'builtin;
        trait Parent where suffix:Symbol {}
        trait Child :- Parent<suffix> where suffix:Symbol {}
        struct S {} impl Parent<.px> for S {} impl Child<.px> for S {}
        fn need(value:T) where T :- Parent<.px> {}
        fn forward(value:T) where T :- Child<.px> {need(value);}
        fn run()->bool {forward(S {});true}";
    accepted(inherited);
    let executed = Sandbox::new(inherited).run(&["--interpret", "run"]);
    assert!(executed.status.success(), "{}", details(&executed));
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout).trim(),
        "(bool true)"
    );
    rejected(
        &inherited.replace("impl Parent<.px>", "impl Parent<.em>"),
        "parent",
    );
    for suffix in [".px", ".em"] {
        accepted(&format!(
            "typealias Symbol=.Symbol'builtin;
            trait Indexed where suffix:Symbol,.again:Symbol=suffix,R {{}}
            struct S {{}} impl Indexed<{suffix},bool> for S {{}}
            fn need(value:T) where T :- Indexed<{suffix},bool,.again={suffix}> {{}}
            fn run() {{need(S {{}});}}"
        ));
    }
    rejected(
        "typealias Symbol=.Symbol'builtin; trait Bad where suffix:Symbol { fn wrong()->suffix; }",
        "cannot be used as a Type",
    );
    rejected(
        "typealias Symbol=.Symbol'builtin; trait Bad where .suffix:Symbol=later,later:Symbol {}",
        "later parameter",
    );
    rejected(
        "typealias Symbol=.Symbol'builtin; trait Bad where .suffix:Symbol=true {}",
        "mismatch",
    );
}

#[test]
fn calls_branches_locals_and_not_reach_typed_nir() {
    let dump = accepted(
        "typealias Flag = true'type; fn invert(value: Flag) -> Flag { not value } fn choose(flag: Flag, left: Flag, right: Flag) -> Flag { let selected = if flag { left } else { right } invert(selected) } fn run() -> Flag { choose(true, false, true) }",
    );
    for tag in [
        "(call ",
        "(if ",
        "(let ",
        "(not ",
        "(type (primitive bool))",
    ] {
        assert!(dump.contains(tag), "{dump}");
    }
}
#[test]
fn forward_functions_and_recursion_use_declared_signatures() {
    let dump = accepted(
        "typealias Flag = true'type; fn first(value: Flag) -> Flag { second(value) } fn second(value: Flag) -> Flag { if value { first(false) } else { true } }",
    );
    assert_eq!(dump.matches("(call ").count(), 2);
}
#[test]
fn parameter_identities_remain_distinct() {
    let dump = accepted(
        "typealias Flag = true'type; fn choose(flag: Flag, left: Flag, right: Flag) -> Flag { if flag { left } else { right } }",
    );
    for id in ["(local 2 1)", "(local 2 3)", "(local 2 5)"] {
        assert_eq!(dump.matches(id).count(), 2, "{dump}");
    }
}
#[test]
fn tuples_and_explicit_returns_are_checked() {
    accepted(
        "typealias Flag = true'type; fn pair(x: Flag, y: Flag) -> (Flag, Flag) { (x, y) } fn early() -> Flag { return true; }",
    );
    rejected(
        "typealias Flag = true'type; fn bad() -> Flag { return 'x'; }",
        "type mismatch",
    );
}
#[test]
fn statement_semicolon_discards_but_tail_returns() {
    accepted("fn okay() -> true'type { true } fn sink() { true; }");
    rejected("fn bad() -> true'type { true; }", "type mismatch");
}
#[test]
fn call_arity_and_parameter_type_fail_without_output() {
    rejected(
        "typealias Flag = true'type; fn id(x: Flag) -> Flag { x } fn bad() -> Flag { id() }",
        "argument count mismatch",
    );
    rejected(
        "typealias Flag = true'type; fn id(x: Flag) -> Flag { x } fn bad() -> Flag { id('x') }",
        "type mismatch",
    );
    rejected("fn bad() { true(); }", "not a function");
}
#[test]
fn condition_and_branch_types_are_checked() {
    rejected(
        "fn bad() -> true'type { if 'x' { true } else { false } }",
        "type mismatch",
    );
    rejected(
        "fn bad() -> true'type { if true { 'x' } else { false } }",
        "type mismatch",
    );
    rejected(
        "fn bad() -> true'type { if true { true } }",
        "type mismatch",
    );
}
#[test]
fn branch_bindings_and_parameters_do_not_escape() {
    rejected(
        "fn first(value: true'type) { } fn second() { value; }",
        "Unresolved identifier",
    );
    rejected(
        "fn bad() { if true { let hidden = true; } hidden; }",
        "Unresolved identifier",
    );
}
#[test]
fn shadowing_initializer_reads_outer_binding() {
    accepted("fn shadow(value: true'type) -> true'type { let value = not value; value }");
    rejected("fn bad() { let value = value; }", "Unresolved identifier");
}
#[test]
fn type_alias_chains_work_and_cycles_fail() {
    accepted(
        "typealias First = Second; typealias Second = true'type; fn id(x: First) -> Second { x }",
    );
    rejected(
        "typealias First = Second; typealias Second = First; fn id(x: First) { }",
        "cyclic type alias",
    );
}
#[test]
fn logical_operators_keep_short_circuit_structure() {
    let dump = accepted("fn logic(x: true'type,y: true'type) -> true'type { x and y or not x }");
    assert!(dump.contains("(and "));
    assert!(dump.contains("(or "));
}
#[test]
fn unbound_numeric_suffix_does_not_become_checked_nir() {
    rejected(
        "fn unsupported() { 1.25f32; }",
        "bound LiteralInto protocol",
    );
    rejected("fn unsupported() { 123u32; }", "bound LiteralInto protocol");
}

#[test]
fn unprocessed_stage_and_attribute_information_blocks_nir() {
    let pure = accepted("pure fn checked() { }");
    assert!(pure.contains("(phase runtime) (inline false) (pure true)"));
    rejected(
        "^unknown fn pending() { }",
        "definition attribute or modifier",
    );
    rejected("fn pending() { comptime { true; } }", "ComptimeBlock");
}

#[test]
fn higher_order_function_values_and_returns_keep_signatures() {
    let dump = accepted(
        "typealias Flag = true'type; typealias Unary = fn(Flag) -> Flag; fn invert(value: Flag) -> Flag { not value } fn select() -> Unary { invert } fn apply(f: Unary, value: Flag) -> Flag { f(value) } fn run() -> Flag { apply(select(), true) }",
    );
    assert_eq!(dump.matches("(call ").count(), 3);
    rejected(
        "typealias Flag = true'type; typealias Unary = fn(Flag) -> Flag; fn apply(f: Unary) -> Flag { f(true) } fn run() -> Flag { apply(true) }",
        "type mismatch",
    );
}

#[test]
fn void_unit_and_guarded_return_preserve_control_flow() {
    accepted("fn unit() { () } fn early() { return (); }");
    let dump =
        accepted("fn guarded(flag: true'type) -> true'type { return false while flag; true }");
    assert!(dump.contains("(return "));
    assert!(dump.contains("(if "));
}

#[test]
fn strings_are_escaped_and_nir_dump_is_deterministic() {
    let sandbox = Sandbox::new("fn text() -> \"\"'type { \"a\\\"b\\n\\x{0}\" }");
    let first = sandbox.run(&["--emit", "nir"]);
    let second = sandbox.run(&["--emit", "nir"]);
    assert!(first.status.success(), "{}", details(&first));
    assert!(second.status.success(), "{}", details(&second));
    assert_eq!(first.stdout, second.stdout);
    let dump = String::from_utf8(first.stdout).unwrap();
    assert!(dump.contains("a\\\"b\\n\\u0000"), "{dump}");
}

#[test]
fn lambda_parameters_use_callable_expectations() {
    let dump = accepted(
        "typealias Flag = true'type; fn apply(f: fn(Flag) -> Flag, x: Flag) -> Flag { f(x) } fn run() -> Flag { apply(|value| not value, true) }",
    );
    assert!(dump.contains("(captures)"), "{dump}");
    assert!(dump.contains("(not "), "{dump}");
    rejected(
        "fn missing() { let f = |value| value; }",
        "needs an annotation",
    );
    rejected(
        "fn bad(f: fn(true'type) -> true'type) { } fn run() { bad(|| true); }",
        "lambda parameter count",
    );
}
#[test]
fn typed_lambdas_infer_results_and_support_local_calls() {
    accepted("fn run() -> true'type { let invert = |x: true'type| not x; invert(true) }");
    accepted(
        "fn run() -> true'type { let invert = |x: true'type| { return not x; } invert(true) }",
    );
    rejected(
        "fn bad() { let f = |x: true'type| { if x { return true; } 'x' } }",
        "type mismatch",
    );
}
#[test]
fn captures_are_explicit_and_cannot_erase_into_plain_functions() {
    let dump = accepted(
        "fn run(flag: true'type) -> true'type { let captured = || flag and flag; captured() }",
    );
    assert_eq!(dump.matches("(capture (binding").count(), 1, "{dump}");
    assert!(
        dump.contains("(closure (environment (primitive bool))"),
        "{dump}"
    );
    rejected(
        "fn bad(flag: true'type) -> fn() -> true'type { || flag }",
        "type mismatch",
    );
}
#[test]
fn nested_capture_and_shadowing_preserve_binding_identity() {
    let dump = accepted(
        "fn run(flag: true'type) -> true'type { let outer = || { let inner = || flag; inner() } outer() }",
    );
    assert_eq!(dump.matches("(capture (binding").count(), 2, "{dump}");
    let dump = accepted(
        "fn run(flag: true'type) -> true'type { let local = |flag: true'type| flag; local(false) }",
    );
    assert_eq!(dump.matches("(capture (binding").count(), 0, "{dump}");
    rejected(
        "fn bad() { let f = |hidden: true'type| hidden; hidden; }",
        "Unresolved identifier",
    );
}
#[test]
fn lambda_return_does_not_return_from_enclosing_function() {
    accepted("fn run() -> 'x''type { let f = || { return true; } f(); 'x' }");
    rejected(
        "fn run() -> true'type { let f = || { return 'x'; } f() }",
        "type mismatch",
    );
}

#[test]
fn parameter_type_queries_do_not_create_runtime_captures() {
    let dump = accepted(
        "fn run(sample: true'type) -> true'type { let f: fn(true'type) -> true'type = |value: sample'type| value; f(false) }",
    );
    assert_eq!(dump.matches("(capture (binding").count(), 0, "{dump}");
}

#[test]
fn captures_follow_first_use_order_not_parameter_order() {
    let dump = accepted(
        "typealias Flag = true'type; fn run(first: Flag, second: Flag) -> Flag { let f = || second and first and second; f() }",
    );
    let second = dump.find("(capture (binding (local 2 3))").unwrap();
    let first = dump.find("(capture (binding (local 2 1))").unwrap();
    assert!(second < first, "{dump}");
    assert_eq!(dump.matches("(capture (binding").count(), 2, "{dump}");
}

#[test]
fn lambda_annotations_and_nested_returns_are_independently_checked() {
    accepted(
        "typealias Flag = true'type; fn run() -> Flag { let f = |x: Flag| -> Flag { if x { return false; } true } f(true) }",
    );
    rejected(
        "typealias Flag = true'type; fn run() { let f = |x: Flag| -> Flag { 'x' } }",
        "type mismatch",
    );
    rejected(
        "typealias Flag = true'type; fn apply(f: fn(Flag) -> Flag) { } fn run() { apply(|x: 'a''type| true); }",
        "type mismatch",
    );
    accepted(
        "typealias Flag = true'type; fn run(x: Flag) -> Flag { let f = || { let g = || { return 'x'; } g(); return x; } f() }",
    );
}

#[test]
fn optional_defaults_resolve_earlier_parameters_in_definition_scope() {
    let dump = accepted(
        "typealias Flag = true'type; fn select(value: Flag, .fallback: Flag = not value) -> Flag { fallback } fn run() -> Flag { let value = false; select(true) }",
    );
    assert!(dump.contains("(default true)"), "{dump}");
    assert!(dump.contains("(parameter-bindings"), "{dump}");
    rejected(
        "typealias Flag = true'type; fn bad(.first: Flag = second, .second: Flag = true) { }",
        "Unresolved identifier",
    );
}
#[test]
fn explicit_named_values_disable_their_default_and_keep_source_order() {
    let dump = accepted(
        "typealias Flag = true'type; fn select(value: Flag, .first: Flag = false, .second: Flag = first) -> Flag { second } fn run() -> Flag { select(.second = false, true, .first = true) }",
    );
    let call = &dump[dump.rfind("(call ").unwrap()..];
    assert!(!call.contains("(default true)"), "{call}");
    let second = call.find("(argument (parameter 2)").unwrap();
    let value = call.find("(argument (parameter 0)").unwrap();
    let first = call.find("(argument (parameter 1)").unwrap();
    assert!(second < value && value < first, "{call}");
}
#[test]
fn default_chains_follow_declaration_order_after_explicit_arguments() {
    let dump = accepted(
        "typealias Flag = true'type; fn chain(value: Flag, .first: Flag = value, .second: Flag = not first) -> Flag { second } fn run() -> Flag { chain(false) }",
    );
    let call = &dump[dump.rfind("(call ").unwrap()..];
    assert!(
        call.find("(argument (parameter 0)").unwrap()
            < call.find("(argument (parameter 1)").unwrap()
    );
    assert!(
        call.find("(argument (parameter 1)").unwrap()
            < call.find("(argument (parameter 2)").unwrap()
    );
    assert_eq!(call.matches("(default true)").count(), 2);
}
#[test]
fn optional_routing_rejects_unknown_duplicate_and_anchored_names() {
    let prefix = "typealias Flag = true'type; fn select(value: Flag, .fallback: Flag = true) -> Flag { fallback } ";
    for (call, message) in [
        (
            "select(true, .fallback = false, .fallback = true)",
            "more than once",
        ),
        ("select(.value = true)", "no named parameter entry"),
        ("select(true, .unknown = false)", "no named parameter entry"),
        ("select(true, false)", "too many positional"),
        ("select(.fallback = false)", "missing anchored"),
    ] {
        rejected(&format!("{prefix} fn run() -> Flag {{ {call} }}"), message);
    }
}
#[test]
fn default_purity_and_parameter_types_are_checked_at_definition() {
    rejected(
        "typealias Flag = true'type; fn bad(.fallback: Flag = 'x') { }",
        "type mismatch",
    );
    accepted(
        "typealias Flag = true'type; fn source() -> Flag { true } fn good(.fallback: Flag = source()) { }",
    );
    rejected(
        "typealias Flag = true'type; fn source() -> Flag { source() } fn bad(.fallback: Flag = source()) { }",
        "P3 stability/termination",
    );
    accepted(
        "typealias Flag = true'type; fn finite(flag: Flag, .fallback: Flag = if flag { false } else { true }) -> Flag { fallback } fn run() -> Flag { finite(true) }",
    );
}

#[test]
fn optional_entries_do_not_consume_positional_arguments() {
    let dump = accepted(
        "typealias Flag = true'type; fn select(.first: Flag = false, value: Flag, .last: Flag = value) -> Flag { last } fn run() -> Flag { select(true) }",
    );
    let call = &dump[dump.rfind("(call ").unwrap()..];
    let supplied = call
        .find("(argument (parameter 1) (default false)")
        .unwrap();
    let first = call.find("(argument (parameter 0) (default true)").unwrap();
    let last = call.find("(argument (parameter 2) (default true)").unwrap();
    assert!(supplied < first && first < last, "{call}");
}
#[test]
fn defaults_preserve_declaration_bindings_and_reject_duplicate_parameters() {
    let dump = accepted(
        "typealias Flag = true'type; fn select(value: Flag, .fallback: Flag = value) -> Flag { fallback } fn run(value: Flag) -> Flag { select(false) }",
    );
    let call = &dump[dump.rfind("(call ").unwrap()..];
    let default = &call[call.find("(argument (parameter 1)").unwrap()..];
    assert!(default.contains("(local 2 1)"), "{default}");
    assert!(!default.contains("(local 3 1)"), "{default}");
    rejected(
        "typealias Flag = true'type; fn bad(x: Flag, .x: Flag = false) { }",
        "duplicate function parameter",
    );
    rejected(
        "typealias Flag = true'type; fn optional(.flag: Flag = true) -> Flag { flag } fn run() { let f = optional; }",
        "parameter protocol metadata",
    );
}

#[test]
fn assignments_check_place_type_and_capture_effect_boundary() {
    rejected("fn bad() { var flag = true; flag = 'x'; }", "type mismatch");
    rejected("fn bad() { true = false; }", "assignment place");
    accepted("fn pending(flag: true'type) { let write = || { flag = false; } }");
}
#[test]
fn loop_transfers_cannot_escape_functions_or_lambdas() {
    rejected("fn bad() { break; }", "outside a loop");
    rejected("fn bad() { continue; }", "outside a loop");
    rejected(
        "fn bad() { while true { let f = || { break; } break; } }",
        "outside a loop",
    );
    accepted("fn labeled() { while:label true { break label; } }");
    rejected(
        "fn bad() { while:label true { let label = true; break label; } }",
        "not a loop label",
    );
    rejected(
        "fn bad() { while:label true { let f = || { break label; } break; } }",
        "outside a loop",
    );
}

#[test]
fn matches_check_exhaustiveness_guards_and_arm_result_types() {
    rejected(
        "fn bad() { true match { true => (), } }",
        "non-exhaustive match",
    );
    rejected(
        "fn bad() { true match { value if value => (), } }",
        "non-exhaustive match",
    );
    rejected(
        "fn bad() { true match { _ if 'x' => (), _ => (), } }",
        "type mismatch",
    );
    rejected(
        "fn bad() { true match { true => false, false => 'x', } }",
        "type mismatch",
    );
    rejected(
        "fn bad() { true match { 'x' => (), _ => (), } }",
        "type mismatch",
    );
    accepted("fn good() { true match { true or false => (), } }");
}

#[test]
fn unsupported_pattern_protocols_are_not_silently_erased() {
    rejected(
        "fn bad() { (true,false) match { [a,b] => (), _ => (), } }",
        "pattern view/protocol",
    );
    rejected(
        "fn bad() { true match { a or b => (), } }",
        "same binding names",
    );
}

#[test]
fn pattern_bindings_do_not_leak_to_other_arms_or_after_matches() {
    rejected(
        "fn bad() { true match { x as x => (), } }",
        "duplicate pattern binding",
    );
    rejected(
        "fn bad() { true match { x if x => (), _ => { x; }, } }",
        "Unresolved identifier",
    );
    rejected(
        "fn bad() { true match { x => (), }; x; }",
        "Unresolved identifier",
    );
    rejected(
        "fn bad() { if true is x do { } else { x; } }",
        "Unresolved identifier",
    );
}

#[test]
fn grouped_patterns_preserve_precedence_without_tuple_destructuring() {
    accepted("fn good() { true match { (true or false) as flag if flag => (), _ => (), } }");
    rejected(
        "fn bad() { true match { (true,) => (), _ => (), } }",
        "tuple pattern requires a tuple type",
    );
}

#[test]
fn let_else_requires_a_diverging_failure_and_checks_pattern_contracts() {
    rejected("fn bad() { let true = false; }", "refutable let pattern");
    rejected(
        "fn bad() { let true = false else { false }; }",
        "must diverge",
    );
    rejected(
        "fn bad() { let true = 'x' else { return; } }",
        "type mismatch",
    );
    rejected(
        "fn bad() { let value if 'x' = true else { return; } }",
        "type mismatch",
    );
    rejected(
        "fn bad() { let true as value = false else { value; return; } }",
        "Unresolved identifier",
    );
    let dump = accepted("fn good() -> true'type { let (a,b) = (true,false); a and not b }");
    assert!(dump.contains("(tuple-pattern"));
}

#[test]
fn or_binding_correspondence_rejects_missing_extra_and_duplicate_names() {
    rejected(
        "fn bad() { true match { (true as x) or false => (), } }",
        "same binding names",
    );
    rejected(
        "fn bad() { true match { (true as x) or ((false as x) as y) => (), } }",
        "same binding names",
    );
    rejected(
        "fn bad() { true match { (true as x) or ((false as x) as x) => (), } }",
        "duplicate pattern binding",
    );
    rejected(
        "fn bad() { let (true as x) or false = true else { return; } }",
        "same binding names",
    );
}

#[test]
fn typed_let_and_const_stage_boundaries_are_preserved() {
    accepted(
        "typealias Flag = true'type; fn good() { let x: Flag = true; let true as y: Flag = x else { return; } }",
    );
    accepted("fn good() { const x = true; let y = x; }");
    rejected("fn bad(x: true'type) { const y = x; }", "runtime parameter");
}

#[test]
fn when_rejects_invalid_conditions_branches_and_fallback_order() {
    rejected(
        "fn run() { when { true => true, false => 'x', } }",
        "type mismatch",
    );
    rejected(
        "typealias Flag = true'type; fn run() -> Flag { when { 'x' => true, else => false, } }",
        "type mismatch",
    );
    rejected(
        "typealias Flag = true'type; fn run() -> Flag { when { true => true, else => 'x', } }",
        "type mismatch",
    );
    rejected(
        "typealias Flag = true'type; fn run() { when { else => (), true => (), } }",
        "when fallback must be the final arm",
    );
    rejected(
        "typealias Flag = true'type; fn run() { when { else => (), else => (), } }",
        "when fallback must be the final arm",
    );
    rejected(
        "typealias Flag = true'type; fn run() -> Flag { when { true => true, } }",
        "type mismatch",
    );
}

#[test]
fn when_without_expected_type_unifies_all_returning_arms() {
    let dump = accepted(
        "typealias Flag = true'type; fn run() -> Flag { let selected = when { false => { return false; }, true => true, else => false, }; selected }",
    );
    assert!(dump.contains("(if"));
    rejected(
        "typealias Flag = true'type; fn run() { let value = when { false => true, else => 'x', }; }",
        "type mismatch",
    );
}

#[test]
fn when_arm_declarations_cannot_escape_or_feed_another_arm() {
    rejected(
        "fn run() { when { true => let hidden = true, else => hidden, } }",
        "Unresolved identifier",
    );
    rejected(
        "fn run() { when { true => let hidden = true, }; hidden; }",
        "Unresolved identifier",
    );
    rejected(
        "fn run() { when { true => var hidden = true, hidden => (), } }",
        "Unresolved identifier",
    );
}

#[test]
fn struct_construction_checks_completeness_identity_and_field_types() {
    let prefix = "typealias Flag = true'type; struct S { flag: Flag, } ";
    rejected(
        &format!("{prefix} fn run() {{ S {{}} }}"),
        "missing required struct field",
    );
    rejected(
        &format!("{prefix} fn run() {{ S {{ flag: true, flag: false }} }}"),
        "struct field supplied more than once",
    );
    rejected(
        &format!("{prefix} fn run() {{ S {{ other: true }} }}"),
        "unknown struct field",
    );
    rejected(
        &format!("{prefix} fn run() {{ S {{ true }} }}"),
        "struct fields must be fully named",
    );
    rejected(
        &format!("{prefix} fn run() {{ S {{ flag: 'x' }} }}"),
        "type mismatch",
    );
    rejected(
        &format!(
            "{prefix} struct Other {{ flag: Flag, }} fn run() -> S {{ Other {{ flag: true }} }}"
        ),
        "type mismatch",
    );
    rejected(
        &format!("{prefix} fn run(value: S) {{ value.missing }}"),
        "unknown struct field",
    );
}

#[test]
fn struct_definition_checks_defaults_duplicate_fields_and_recursive_storage() {
    rejected(
        "typealias Flag = true'type; struct S { a: Flag, a: Flag, }",
        "duplicate struct field",
    );
    rejected(
        "typealias Flag = true'type; struct S { a: Flag = 'x', }",
        "type mismatch",
    );
    rejected(
        "struct A { b: B, } struct B { a: A, }",
        "recursive by-value struct representation",
    );
    accepted("struct Node { next: *Node, }");
}

#[test]
fn let_reassignment_is_rejected_but_var_and_field_writes_are_distinct() {
    rejected(
        "fn run() { let value = true; value = false; }",
        "cannot reassign a let binding",
    );
    rejected(
        "fn run() { var value = true; do { let value = false; value = true; }; }",
        "cannot reassign a let binding",
    );
    rejected(
        "fn run() { let true as value = true else { return; }; value = false; }",
        "cannot reassign a let binding",
    );
    accepted("fn run() { var value = true; value = false; }");
    let prefix = "typealias Flag = true'type; struct S { value: Flag, } ";
    rejected(
        &format!("{prefix} fn run() {{ let object = S {{ value: true }}; object.value = 'x'; }}"),
        "type mismatch",
    );
    rejected(
        &format!(
            "{prefix} fn run() {{ let object = S {{ value: true }}; object.missing = false; }}"
        ),
        "unknown struct field",
    );
    accepted(&format!(
        "{prefix} fn run() {{ let object = S {{ value: true }}; let writer = || {{ object.value = false; }}; }}"
    ));
    rejected(
        &format!(
            "{prefix} fn make() -> S {{ S {{ value: true }} }} fn run() {{ make().value = false; }}"
        ),
        "assignment place elaboration",
    );
}

#[test]
fn enum_domains_are_nominal_closed_and_guards_do_not_prove_coverage() {
    rejected(
        "enum E { a, b, } fn run(value: E) { value match { .a => (), } }",
        "non-exhaustive match",
    );
    rejected(
        "enum E { a, b, } fn run(value: E) { value match { .a => (), .b if true => (), } }",
        "non-exhaustive match",
    );
    rejected(
        "enum E { a, } fn run() -> E { E.missing }",
        "unknown enum variant",
    );
    rejected(
        "enum E { a, } enum F { a, } fn run() -> E { F.a }",
        "type mismatch",
    );
    rejected("enum E { a, a, }", "duplicate enum member");
    rejected("enum E { payload = true, }", "enum representation");
    accepted("enum Empty {} fn absurd(value: Empty) { value match {} }");
}

#[test]
fn payload_patterns_require_complete_unguarded_coverage() {
    let prefix = "typealias Flag = true'type; enum E { pair(Flag, Flag), } ";
    for arms in [
        ".pair(true, _) => ()",
        ".pair(true, _) => (), .pair(false, _) if true => ()",
        ".pair(true, true) => (), .pair(false, false) => ()",
    ] {
        rejected(
            &format!("{prefix} fn run(value: E) {{ value match {{ {arms}, }} }}"),
            "non-exhaustive match",
        );
    }
    accepted(&format!(
        "{prefix} fn run(value: E) {{ value match {{ .pair(true, _) => (), .pair(false, _) => (), }} }}"
    ));
    rejected(
        "enum E { text('x''type), } fn run(value: E) { value match { .text('x') => (), } }",
        "non-exhaustive match",
    );
    accepted("enum Empty {} enum E { impossible(Empty), } fn absurd(value: E) { value match {} }");
}

#[test]
fn enum_payload_constructors_check_shape_types_and_pattern_binding_sets() {
    let prefix = "typealias Flag = true'type; enum E { unit, empty(), pair(Flag, Flag), } ";
    for (body, expected) in [
        ("E.unit()", "unit enum variant does not take arguments"),
        ("E.empty", "requires arguments"),
        ("E.pair(true)", "argument count mismatch"),
        ("E.pair(true, 'x')", "type mismatch"),
        (
            "E.pair(.left = true, .right = false)",
            "requires positional arguments",
        ),
    ] {
        rejected(&format!("{prefix} fn run() -> E {{ {body} }}"), expected);
    }
    rejected(
        &format!("{prefix} fn run(value: E) {{ value match {{ .pair(x) => (), _ => (), }} }}"),
        "pattern arity mismatch",
    );
    rejected(
        &format!(
            "{prefix} fn run(value: E) {{ value match {{ .pair(x, _) or .pair(_, y) => (), _ => (), }} }}"
        ),
        "same binding names",
    );
}

#[test]
fn enum_payload_representation_cycles_include_struct_edges() {
    rejected("enum E { recursive(E), }", "recursive by-value");
    rejected(
        "struct S { e: E, } enum E { recursive(S), }",
        "recursive by-value",
    );
    accepted("enum E { end, next(*E), }");
}

#[test]
fn enum_coverage_budget_does_not_turn_partial_products_into_proofs() {
    let types = vec!["true'type"; 14].join(", ");
    let patterns = vec!["true"; 14].join(", ");
    rejected(
        &format!(
            "enum E {{ product({types}), }} fn run(value: E) {{ value match {{ .product({patterns}) => (), }} }}"
        ),
        "non-exhaustive match",
    );
    let wildcards = vec!["_"; 14].join(", ");
    accepted(&format!(
        "enum E {{ product({types}), }} fn run(value: E) {{ value match {{ .product({wildcards}) => (), }} }}"
    ));
}

#[test]
fn payload_constant_expression_patterns_cannot_discard_the_payload() {
    rejected(
        "enum E { flag(true'type), } fn run(value: E) { value match { < E.flag(true) > => (), } }",
        "equality protocol for this type is not implemented",
    );
    rejected(
        "enum E { pair(true'type, true'type), } fn run(value: E) { value match { .pair(x, x) => (), } }",
        "duplicate pattern binding",
    );
}

#[test]
fn named_enum_payloads_reject_bad_fields_shapes_and_incomplete_coverage() {
    let prefix = "enum E { data { flag: true'type }, } ";
    for (body, diagnostic) in [
        ("E.data {}", "missing required"),
        ("E.data { flag: true, flag: false }", "more than once"),
        ("E.data { missing: true }", "unknown enum payload field"),
        ("E.data { flag: 'x' }", "type mismatch"),
        ("E.data(true)", "does not take arguments"),
    ] {
        rejected(&format!("{prefix} fn run() -> E {{ {body} }}"), diagnostic);
    }
    rejected(
        &format!("{prefix} fn run(value: E) {{ value match {{ .data {{ missing: _ }} => (), }} }}"),
        "unknown enum payload field",
    );
    rejected(
        &format!("{prefix} fn run(value: E) {{ value match {{ .data {{ flag: true }} => (), }} }}"),
        "non-exhaustive match",
    );
    rejected(
        "enum E { data { flag: true'type = 'x' }, }",
        "type mismatch",
    );
    rejected(
        "enum E { data { flag: true'type, flag: true'type }, }",
        "duplicate enum payload field",
    );
    accepted("enum E { empty {}, } fn run() -> E { E.empty {} }");
}

#[test]
fn wide_enum_payload_coverage_is_symbolic_and_preserves_correlations() {
    let types = vec!["true'type"; 64].join(", ");
    let total = vec!["(true or false)"; 64].join(", ");
    accepted(&format!(
        "enum E {{ wide({types}), }} fn run(value: E) {{ value match {{ .wide({total}) => (), }} }}"
    ));
    let mut left = vec!["_"; 64];
    left[0] = "true";
    let mut right = vec!["_"; 64];
    right[0] = "false";
    accepted(&format!(
        "enum E {{ wide({types}), }} fn run(value: E) {{ value match {{ .wide({}) => (), .wide({}) => (), }} }}",
        left.join(", "),
        right.join(", ")
    ));
    left[63] = "true";
    right[63] = "false";
    rejected(
        &format!(
            "enum E {{ wide({types}), }} fn run(value: E) {{ value match {{ .wide({}) => (), .wide({}) => (), }} }}",
            left.join(", "),
            right.join(", ")
        ),
        "non-exhaustive match",
    );
}

#[test]
fn qualified_enum_patterns_check_identity_and_constructor_shape() {
    rejected(
        "enum E { same, } enum F { same, } fn run(value: E) { value match { F.same => (), } }",
        "type mismatch",
    );
    rejected(
        "enum E { data(true'type), } enum F { data(true'type), } fn run(value: E) { value match { F.data(_) => (), } }",
        "type mismatch",
    );
    rejected(
        "enum E { data { x: true'type }, } enum F { data { x: true'type }, } fn run(value: E) { value match { F.data { x } => (), } }",
        "type mismatch",
    );
    rejected(
        "enum E { data(true'type), } fn run(value: E) { value match { E.data => (), } }",
        "requires arguments",
    );
    rejected(
        "enum E { unit, } fn run(value: E) { value match { E.missing => (), } }",
        "unknown enum variant",
    );
}

#[test]
fn generic_bodies_are_checked_abstractly_and_calls_must_infer_consistent_types() {
    accepted("fn keep(value: T) -> T where T { value }");
    rejected(
        "fn invalid(value: T) -> T where T { true }",
        "type mismatch",
    );
    rejected(
        "fn invalid(value: T) -> true'type where T { not value }",
        "type mismatch",
    );
    rejected(
        "fn select(a: T, b: T) -> T where T { a } fn run() { select(true, 'x'); }",
        "generic call inference failed",
    );
    rejected(
        "fn keep(value: T) -> T where T { value } fn run() -> 'x''type { keep(true) }",
        "type mismatch",
    );
    rejected(
        "fn unused() where T {} fn run() { unused(); }",
        "generic call inference failed",
    );
    rejected(
        "fn keep(value: T) -> T where T { value } fn run() { let callback = keep; }",
        "generic function value requires instantiation",
    );
}

#[test]
fn explicit_generic_arguments_and_function_contexts_are_checked() {
    let prefix = "fn keep(value: T) -> T where T { value } ";
    rejected(
        &format!("{prefix} fn run() {{ keep<true'type, 'x''type>(true); }}"),
        "invalid function type arguments",
    );
    rejected(
        &format!("{prefix} fn run() {{ keep<'x''type>(true); }}"),
        "type mismatch",
    );
    rejected(
        &format!("{prefix} fn run() {{ let callback: fn(true'type) -> 'x''type = keep; }}"),
        "generic function context inference failed",
    );
    rejected(
        "fn plain(value: true'type) -> true'type { value } fn run() { plain<true'type>(true); }",
        "explicit instantiation requires a generic function",
    );
    rejected(
        &format!("{prefix} fn run() {{ let callback = keep<true>; }}"),
        "type expression",
    );
}

#[test]
fn generic_argument_deferral_requires_evidence_and_preserves_type_errors() {
    rejected(
        "fn keep(value: T) -> T where T { value } fn run() { keep(|x| x); }",
        "generic argument context is unresolved",
    );
    rejected(
        "fn apply(value: T, f: fn(T) -> T) -> T where T { f(value) } fn run() { apply(true, |x| 'x'); }",
        "type mismatch",
    );
    rejected(
        "fn same(a: T, b: T, f: fn(T) -> T) -> T where T { f(a) } fn run() { same(true, 'x', |x| x); }",
        "generic call inference failed",
    );
}

#[test]
fn generic_optional_routing_and_function_value_metadata_are_checked() {
    let prefix =
        "typealias Flag = true'type; fn keep(value: T, .flag: Flag = true) -> T where T { value } ";
    for (body, diagnostic) in [
        ("keep(true, false)", "too many positional"),
        ("keep(.flag = true)", "missing anchored"),
        ("keep(true, .value = false)", "no named parameter entry"),
        ("keep(true, .flag = true, .flag = false)", "more than once"),
        ("keep<Flag>(true, .flag = 'x')", "type mismatch"),
        (
            "keep<Flag>",
            "optional function values require parameter protocol metadata",
        ),
    ] {
        rejected(&format!("{prefix} fn run() {{ {body}; }}"), diagnostic);
    }
    rejected(
        "fn bad(value: T, .other: T = true) -> T where T { value }",
        "type mismatch",
    );
}

#[test]
fn calls_cannot_assume_the_requirements_they_need_to_prove() {
    let prefix = "typealias Flag = true'type; fn require_flag(value: T) -> T where T, requires T == Flag { value } ";
    rejected(
        &format!("{prefix} fn run() {{ require_flag('x'); }}"),
        "type requirement is not satisfied",
    );
    rejected(
        &format!("{prefix} fn run() {{ let value = require_flag<'x''type>; }}"),
        "type requirement is not satisfied",
    );
    rejected(
        &format!("{prefix} fn unproved(value: U) -> U where U {{ require_flag(value) }}"),
        "type requirement is not satisfied",
    );
    rejected(
        "fn impossible() where requires false {} fn run() { impossible(); }",
        "type requirement is not satisfied",
    );
    rejected(
        "typealias Flag = true'type; fn first(value: T) -> Flag where T, requires T == Flag { value } fn unrelated(value: U) -> Flag where U { value }",
        "type mismatch",
    );
}

#[test]
fn runtime_specifications_are_not_sent_to_ptc() {
    for condition in ["flag", "flag == true"] {
        let dump = accepted(&format!(
            "fn run(flag: true'type) where requires {condition} {{}}"
        ));
        assert!(
            dump.contains("(requires) (program-requires (expr"),
            "{dump}"
        );
    }
    rejected(
        "fn run(flag: true'type) where ensures flag {}",
        "program postcondition is not proven",
    );
}

#[test]
fn definition_type_assumptions_do_not_leak_into_struct_defaults() {
    rejected(
        "fn unreachable() where requires false {} struct Box { flag: true'type = 'x', }",
        "type mismatch",
    );
}

#[test]
fn delayed_nested_calls_do_not_suppress_errors_or_invent_types() {
    let prefix = "fn keep(value: T) -> T where T { value } fn apply(callback: fn(T) -> T, value: T) -> T where T { callback(value) } ";
    rejected(
        &format!("{prefix} fn run() {{ apply(keep(|x| 'x'), true); }}"),
        "type mismatch",
    );
    rejected(
        &format!("{prefix} fn run() {{ keep(keep(|x| x)); }}"),
        "generic argument context is unresolved",
    );
    rejected(
        &format!(
            "{prefix} fn run() {{ apply(do {{ let invalid: true'type = 'x'; keep(|x| x) }}, true); }}"
        ),
        "type mismatch",
    );
    rejected(
        &format!(
            "{prefix} fn only_char(value: T) -> T where T, requires T == 'x''type {{ value }} fn run() {{ apply(keep(only_char), true); }}"
        ),
        "type requirement is not satisfied",
    );
}

#[test]
fn partial_callback_context_keeps_full_signature_and_requirement_checks() {
    let prefix = "fn map(value: T, callback: fn(T) -> U) -> U where T, U { callback(value) } ";
    rejected(
        &format!("{prefix} fn run() {{ map(true, |x: 'x''type| 'y'); }}"),
        "generic call inference failed",
    );
    rejected(
        &format!("{prefix} fn run() {{ map(true, |x, y| 'y'); }}"),
        "lambda parameter count",
    );
    rejected(
        &format!("{prefix} fn run() {{ map(true, |x| {{ if x {{ return true; }} 'y' }}); }}"),
        "type mismatch",
    );
    rejected(
        &format!("{prefix} fn run() {{ let captured = 'y'; map(true, |x| captured); }}"),
        "generic call inference failed",
    );
    rejected(
        "fn map(value: T, callback: fn(T) -> U) -> U where T, U, requires U == true'type { callback(value) } fn run() { map(true, |x| 'y'); }",
        "type requirement is not satisfied",
    );
}

#[test]
fn partial_callback_branches_keep_type_scope_and_exhaustiveness_checks() {
    let prefix = "fn map(value: T, callback: fn(T) -> U) -> U where T, U { callback(value) } ";
    for callback in [
        "if true { |x| 'y' } else { |x| x }",
        "true match { true => |x| 'y', false => |x| x, }",
        "when { true => |x| 'y', else => |x| x, }",
    ] {
        rejected(
            &format!("{prefix} fn run() {{ map(true, {callback}); }}"),
            "type mismatch",
        );
    }
    rejected(
        &format!("{prefix} fn run() {{ map(true, true match {{ true => |x| x, }}); }}"),
        "non-exhaustive match",
    );
    rejected(
        &format!("{prefix} fn run() {{ map(true, do {{ let hidden = true; |x| x }}); hidden; }}"),
        "Unresolved identifier `hidden`",
    );
}

#[test]
fn nested_partial_calls_keep_constraints_and_reject_unresolved_or_bad_callbacks() {
    let prefix = "fn keep(value: T) -> T where T { value } fn map(value: T, callback: fn(T) -> U) -> U where T, U { callback(value) } ";
    rejected(
        &format!(
            "{prefix} fn factory() -> fn(T) -> T where T, requires T == 'x''type {{ keep<T> }} fn run() {{ map(true, keep(factory())); }}"
        ),
        "type requirement is not satisfied",
    );
    rejected(
        &format!("{prefix} fn run() {{ map(true, keep(|x, y| x)); }}"),
        "lambda parameter count",
    );
    rejected(
        &format!("{prefix} fn run() {{ keep(keep(|x| x)); }}"),
        "generic argument context is unresolved",
    );
}

#[test]
fn generic_structs_check_definition_fields_and_explicit_argument_contracts() {
    let prefix = "struct Box where T { value: T, } ";
    rejected(
        &format!("{prefix} fn run() {{ Box<true'type> {{ value: 'x' }}; }}"),
        "type mismatch",
    );
    rejected(
        &format!("{prefix} fn run(value: Box) {{}}"),
        "explicit type arguments",
    );
    rejected(
        &format!("{prefix} fn run(value: Box<true'type, 'x''type>) {{}}"),
        "argument count mismatch",
    );
    rejected(
        &format!(
            "{prefix} fn run() {{ let value = Box<true'type> {{ value: true }}; value.value = 'x'; }}"
        ),
        "type mismatch",
    );
    rejected("struct Bad where T { value: T = true, }", "type mismatch");
    rejected(
        "struct Tag where T {} fn run() -> Tag<true'type> { Tag<'x''type> {} }",
        "type mismatch",
    );
}

#[test]
fn generic_representation_checks_distinguish_finite_nesting_and_cycles() {
    accepted(
        "struct Box where T { value: T, } struct Middle { value: Box<true'type>, } struct Outer { value: Box<Middle>, }",
    );
    rejected(
        "struct Box where T { value: T, } struct Cycle { value: Box<Cycle>, }",
        "recursive by-value",
    );
    rejected(
        "struct Grow where T { value: Grow<(T, T)>, }",
        "representation expansion budget exhausted",
    );
    accepted("struct Node where T { value: T, next: *Node<T>, }");
}

#[test]
fn generic_struct_type_values_are_not_mistaken_for_runtime_functions() {
    for (expression, diagnostic) in [
        ("Box(true)", "declaration is not a supported callable"),
        (
            "Box<true'type>",
            "comptime-only value cannot inhabit runtime storage",
        ),
        ("Box", "generic type requires explicit type arguments"),
    ] {
        rejected(
            &format!("struct Box where T {{ value: T, }} fn run() {{ {expression}; }}"),
            diagnostic,
        );
    }
}

#[test]
fn generic_enums_check_specialized_fields_patterns_and_defaults() {
    let prefix = "enum Choice where T { none, value(T), } ";
    rejected(
        &format!("{prefix} fn run() -> Choice<true'type> {{ .value('x') }}"),
        "type mismatch",
    );
    rejected(
        &format!(
            "{prefix} fn run(value: Choice<true'type>) -> true'type {{ value match {{ .none => false, .value(true) => true, }} }}"
        ),
        "non-exhaustive match",
    );
    rejected(
        &format!(
            "{prefix} fn run(value: Choice<'x''type>) -> true'type {{ value match {{ .none => false, .value(true) => true, .value(false) => false, }} }}"
        ),
        "type mismatch",
    );
    rejected(
        &format!("{prefix} fn run(value: Choice) {{}}"),
        "explicit type arguments",
    );
    rejected(
        "enum Bad where T { value { item: T = true }, }",
        "type mismatch",
    );
    rejected(
        "enum Choice where T { none, value(T), } struct Cycle { value: Choice<Cycle>, }",
        "recursive by-value",
    );
    rejected(
        "enum Grow where T { value(Grow<(T, T)>), }",
        "representation expansion budget exhausted",
    );
}

#[test]
fn qualified_generic_patterns_keep_their_explicit_instance_identity() {
    rejected(
        "typealias Flag = true'type; typealias Character = 'x''type; enum Choice where T { none, value(T), } fn run(value: Choice<Flag>) -> Flag { value match { Choice<Character>.none => false, Choice<Flag>.value(flag) => flag, } }",
        "type mismatch",
    );
}

#[test]
fn qualified_pattern_inline_type_arguments_are_checked_as_expressions() {
    rejected(
        "enum Choice where T { none, value(T), } fn run(value: Choice<true'type>) -> true'type { value match { Choice<'x''type>.none => false, Choice<true'type>.value(flag) => flag, } }",
        "type mismatch",
    );
    rejected(
        "enum Choice where T { none, value(T), } fn run(value: Choice<true'type>) -> true'type { value match { Choice<Missing>.none => false, Choice<true'type>.value(flag) => flag, } }",
        "Unresolved identifier `Missing`",
    );
}

#[test]
fn nominal_argument_inference_still_checks_whole_types_and_requirements() {
    rejected(
        "struct Box where T { value: T, } fn choose(a: Box<T>, b: Box<T>) -> T where T { a.value } fn run() { choose(Box<true'type> { value: true }, Box<'x''type> { value: 'x' }); }",
        "generic call inference failed",
    );
    rejected(
        "struct Tag where T {} fn checked(value: Tag<T>) where T, requires T == true'type {} fn run() { checked(Tag<'x''type> {}); }",
        "type requirement is not satisfied",
    );
}

#[test]
fn nominal_contracts_reject_invalid_types_at_every_formation_site() {
    let prefix = "struct Checked where T, requires T == true'type { value: T, } ";
    for body in [
        "typealias Bad = Checked<'x''type>;",
        "fn bad(value: Checked<'x''type>) {}",
        "fn bad() -> Checked<'x''type> {}",
        "struct Outer { value: Checked<'x''type>, }",
        "enum Outer { value(Checked<'x''type>), }",
        "fn run() { Checked<'x''type> { value: 'x' }; }",
        "fn run() { let x: Checked<'x''type> = true; }",
    ] {
        rejected(
            &format!("{prefix}{body}"),
            "type requirement is not satisfied",
        );
    }
    rejected(
        "enum Checked where T, requires T == true'type { none, } fn run() { Checked<'x''type>.none; }",
        "type requirement is not satisfied",
    );
}

#[test]
fn nominal_contracts_cannot_be_assumed_by_unconstrained_users() {
    let prefix = "struct Checked where T, requires T == true'type { value: T, } ";
    for body in [
        "fn bad(value: Checked<U>) where U {}",
        "struct Outer where U { value: Checked<U>, }",
        "enum Outer where U { value(Checked<U>), }",
        "fn bad(value: U) where U { Checked<U> { value: value }; }",
    ] {
        rejected(&format!("{prefix}{body}"), "type requirement");
    }
    rejected(
        "struct Disabled where requires false {} typealias Bad = Disabled;",
        "type requirement is not satisfied",
    );
}

#[test]
fn nominal_defaults_use_only_their_own_declaration_contracts() {
    rejected(
        "struct Bad where T { value: T = true, } fn unrelated() where requires false {}",
        "type mismatch",
    );
    rejected(
        "struct Bad where T, requires T == 'x''type { value: T = true, }",
        "type mismatch",
    );
    rejected(
        "enum Bad where T, requires T == 'x''type { named { value: T = true }, }",
        "type mismatch",
    );
}

#[test]
fn nominal_contract_operands_do_not_bootstrap_their_own_validity() {
    rejected(
        "struct Checked where T, requires T == true'type {} fn bad() where U, requires Checked<U> == Checked<U> {}",
        "type requirement",
    );
    rejected(
        "struct Bad where requires true, ensures true {}",
        "program specification elaboration is not implemented",
    );
    rejected(
        "struct Bad where requires 'x' == 'x' {}",
        "program specifications are not handled by PTC",
    );
    rejected(
        "enum Checked where T, requires T == true'type { value(T), } fn bad() { Checked<'x''type>.value('x'); }",
        "type requirement is not satisfied",
    );
    rejected(
        "enum Checked where T, requires T == true'type { named { value: T }, } fn bad() { Checked<'x''type>.named { value: 'x' }; }",
        "type requirement is not satisfied",
    );
}

#[test]
fn constrained_coverage_does_not_hide_missing_or_guarded_cases() {
    rejected(
        "fn bad(value: T) -> true'type where T, requires T == true'type { value match { true => true, } }",
        "non-exhaustive match",
    );
    rejected(
        "enum Pair where T { pair(T, T), } fn bad(value: Pair<T>) -> true'type where T, requires T == true'type { value match { .pair(true, _) => true, .pair(_, true) => true, } }",
        "non-exhaustive match",
    );
    rejected(
        "fn bad(value: T) -> true'type where T, requires T == true'type { value match { true if true => true, false => false, } }",
        "non-exhaustive match",
    );
    rejected(
        "fn good(value: T) -> true'type where T, requires T == true'type { value match { true => true, false => false, } } fn bad(value: U) -> true'type where U { value match { true => true, false => false, } }",
        "type mismatch",
    );
}

#[test]
fn tuple_patterns_check_shape_field_types_and_complete_coverage() {
    rejected(
        "fn bad() { let (a, b) = true; }",
        "tuple pattern requires a tuple type",
    );
    rejected(
        "fn bad() { let (a,) = (true, false); }",
        "tuple pattern arity mismatch",
    );
    rejected(
        "fn bad() { let (true, x) = ('x', false); }",
        "type mismatch",
    );
    rejected(
        "fn bad() { let (true, x) = (true, false); }",
        "refutable let pattern",
    );
    rejected(
        "fn bad(value: (true'type, true'type)) -> true'type { value match { (true, _) => true, (_, true) => true, } }",
        "non-exhaustive match",
    );
    rejected(
        "fn bad(value: (true'type, true'type)) -> true'type { value match { (true, _) if true => true, (false, _) => false, } }",
        "non-exhaustive match",
    );
}

#[test]
fn tuple_binding_scopes_enforce_mutability_and_or_compatibility() {
    rejected(
        "fn bad() { let (a, b) = (true, false); a = false; }",
        "cannot reassign a let binding",
    );
    rejected(
        "fn bad(value: (true'type, true'type)) -> true'type { value match { (x, true) or (false, y) => true, _ => false, } }",
        "binding",
    );
    rejected(
        "fn bad(value: (true'type, 'x''type)) -> true'type { value match { (x, _) or (_, x) => true, } }",
        "type mismatch",
    );
}

#[test]
fn record_patterns_check_names_types_and_exhaustiveness() {
    let prefix = "struct Pair { left: true'type, right: true'type, } ";
    for (body, message) in [
        (
            "fn bad(value: Pair) { let { missing: x } = value; }",
            "unknown struct pattern field",
        ),
        (
            "fn bad(value: Pair) { let { left: x, left: y } = value; }",
            "duplicate struct pattern field",
        ),
        (
            "fn bad(value: Pair) { let { left: x, right: x } = value; }",
            "duplicate pattern binding",
        ),
        (
            "fn bad(value: Pair) { let { left: 'x' } = value; }",
            "type mismatch",
        ),
        (
            "fn bad(value: Pair) { let { left: true } = value; }",
            "refutable let pattern",
        ),
        (
            "fn bad(value: Pair) -> true'type { value match { { left: true } => true, { right: true } => true, } }",
            "non-exhaustive match",
        ),
        (
            "fn bad(value: Pair) { let { left } = value; left = false; }",
            "cannot reassign a let binding",
        ),
    ] {
        rejected(&format!("{prefix}{body}"), message);
    }
    rejected(
        "fn bad() { let { value } = true; }",
        "record pattern requires a struct type",
    );
}

#[test]
fn structural_views_require_proof_and_preserve_complete_pattern_checks() {
    rejected(
        "struct Box { value: true'type, } fn bad(value: T) -> true'type where T { value.value }",
        "field projection requires",
    );
    rejected(
        "struct A { value: true'type, } struct B { value: true'type, } fn bad(value: T) -> true'type where T, requires T == A or T == B { value.value }",
        "field projection requires",
    );
    rejected(
        "fn bad(value: T) -> true'type where T, requires T == (true'type, true'type) { value match { (true, _) => true, (_, true) => true, } }",
        "non-exhaustive match",
    );
    rejected(
        "enum E where T { some(T), none, } fn bad(value: T) -> true'type where T, requires T == E<true'type> { value match { .some(true) => true, .none => false, } }",
        "non-exhaustive match",
    );
    rejected(
        "struct Box { value: true'type, } fn bad(value: T) where T, requires T == Box { value.value = 'x'; }",
        "type mismatch",
    );
}

#[test]
fn constrained_callable_views_preserve_arity_types_and_capture_boundaries() {
    rejected(
        "fn bad(callback: F) where F, requires F == fn(true'type) -> true'type { callback(); }",
        "argument count mismatch",
    );
    rejected(
        "fn bad(callback: F) where F, requires F == fn(true'type) -> true'type { callback('x'); }",
        "type mismatch",
    );
    rejected(
        "fn bad() -> F where F, requires F == fn(true'type) -> true'type { |x| 'x' }",
        "type mismatch",
    );
    rejected(
        "fn bad(flag: true'type) -> F where F, requires F == fn(true'type) -> true'type { |x| flag }",
        "type mismatch",
    );
    rejected(
        "fn bad(callback: F) where F, requires F == fn(true'type) -> true'type or F == fn('x''type) -> 'x''type { callback(true); }",
        "called expression is not a function",
    );
}

#[test]
fn chained_patterns_check_binding_order_types_and_refutability() {
    rejected(
        "fn bad() { let x and next is next = true; }",
        "Unresolved identifier",
    );
    rejected(
        "fn bad() { let x and x is x = true; }",
        "duplicate pattern binding",
    );
    rejected(
        "fn bad() { let x and 'x' is true = true; }",
        "type mismatch",
    );
    rejected(
        "fn bad() { let x and x is true = true; }",
        "refutable let pattern",
    );
    rejected(
        "fn bad() { let x and x is true = false else { x; return; }; }",
        "Unresolved identifier",
    );
    rejected(
        "fn bad(value: true'type) -> true'type { value match { x and x is true => true, } }",
        "non-exhaustive match",
    );
    rejected(
        "fn bad(value: true'type) -> true'type { value match { (x and x is y) or x => x, } }",
        "same binding names",
    );
}

#[test]
fn chained_pattern_computations_cannot_bypass_default_stability_checks() {
    rejected(
        "fn recurse() -> true'type { recurse() } fn bad(.value: true'type = true match { x and recurse() is _ => x, }) {}",
        "stability/termination evidence",
    );
    rejected(
        "fn recurse() -> true'type { recurse() } fn bad(.value: true'type = do { let x and recurse() is y = true; y }) {}",
        "stability/termination evidence",
    );
}

#[test]
fn let_pattern_bindings_are_immutable_during_sequential_checks() {
    for source in [
        "fn bad() { let x and (do { x = false; x }) is y = true; }",
        "fn bad() { let (x if (do { x = false; true })) = true else { return; }; }",
        "fn bad() { let ((x and false is true) or (x and (do { x = false; true }) is true)) = true else { return; }; }",
        "fn bad() { let ((x and (do { x = false; x }) is y), z) = (true, false); }",
        "struct Box { value: true'type, } fn bad(value: Box) { let { value: (x and (do { x = false; x }) is y) } = value; }",
    ] {
        rejected(source, "cannot reassign a let binding");
    }
}

#[test]
fn trait_implementations_check_required_members_and_signatures() {
    rejected(
        "trait Flag { fn read() -> true'type; } struct S {} impl Flag for S {}",
        "missing required trait method",
    );
    rejected(
        "trait Flag { fn read() -> true'type; } struct S {} impl Flag for S { fn read() -> 'x''type { 'x' } }",
        "does not match its trait signature",
    );
    rejected(
        "trait Flag { fn read(value: true'type) -> true'type; } struct S {} impl Flag for S { fn read() -> true'type { true } }",
        "does not match its trait signature",
    );
    rejected(
        "fn missing() -> true'type;",
        "bodyless function requires a trait contract",
    );
}

#[test]
fn trait_implementation_preconditions_cannot_be_strengthened() {
    rejected(
        "trait Flag { fn read() -> true'type; } struct S {} impl Flag for S { fn read() -> true'type where requires false { true } }",
        "strengthens trait preconditions",
    );
    accepted(
        "trait Flag { fn read() -> true'type where requires false; } struct S {} impl Flag for S { fn read() -> true'type { true } }",
    );
    rejected(
        "trait Flag { fn read() -> true'type where requires false; } struct S {} impl Flag for S { fn read() -> true'type { true } } fn run() -> true'type { S.as(Flag).read() }",
        "trait method precondition is not established",
    );
}

#[test]
fn implementation_world_rejects_concrete_overlap_including_aliases() {
    rejected(
        "trait Flag {} struct S {} typealias Alias = S; impl Flag for S {} impl Flag for Alias {}",
        "implementation overlap",
    );
    rejected(
        "struct S {} impl S { fn read() {} } impl S { fn read() {} }",
        "implementation overlap",
    );
    rejected(
        "impl true'type { fn read() {} }",
        "inherent implementation admission failed",
    );
}

#[test]
fn trait_dispatch_requires_an_implementation_and_does_not_expose_default_bodies() {
    rejected(
        "trait Flag { fn read() -> true'type { true } } fn run() -> true'type { Flag.read() }",
        "trait method requires a selected implementation",
    );
    rejected(
        "trait Flag { fn read() -> true'type { true } } fn run() { let f = Flag.read; }",
        "trait method requires a selected implementation",
    );
    rejected(
        "trait Flag { fn read() -> true'type { true } } struct S {} fn run() -> true'type { S.as(Flag).read() }",
        "method implementation is not uniquely established",
    );
}

#[test]
fn self_is_lexical_and_trait_defaults_remain_abstract() {
    rejected(
        "fn bad(value: Self) {}",
        "Self requires an enclosing nominal type, trait or implementation",
    );
    rejected(
        "struct A {} impl A { fn keep(value: Self) -> Self { value } } fn bad(value: Self) {}",
        "Self requires an enclosing nominal type, trait or implementation",
    );
    rejected(
        "trait Read { fn read(value: Self) -> true'type { value.flag } } struct A { flag: true'type, } impl Read for A {}",
        "field projection requires a supported struct",
    );
    rejected(
        "trait Keep { fn keep(value: Self) -> Self; } struct A {} struct B {} impl Keep for A { fn keep(value: B) -> B { value } }",
        "does not match its trait signature",
    );
}

#[test]
fn self_in_method_requirements_is_substituted_at_interface_calls() {
    accepted(
        "struct A {} trait Only { fn keep(value: Self) -> Self where requires Self == A; } impl Only for A { fn keep(value: Self) -> Self { value } } fn run() -> A { A.as(Only).keep(A {}) }",
    );
    rejected(
        "struct A {} struct B {} trait Only { fn keep(value: Self) -> Self where requires Self == A; } impl Only for B { fn keep(value: Self) -> Self { value } } fn run() -> B { B.as(Only).keep(B {}) }",
        "trait method precondition is not established",
    );
}

#[test]
fn receiver_contract_is_distinct_from_an_ordinary_self_typed_argument() {
    rejected(
        "trait Read { fn read(self) -> true'type; } struct S {} impl Read for S { fn read(value: Self) -> true'type { true } }",
        "method receiver does not match",
    );
    rejected(
        "trait Read { fn read(value: Self) -> true'type; } struct S {} impl Read for S { fn read(self) -> true'type { true } }",
        "method receiver does not match",
    );
    rejected(
        "struct S {} impl S { fn read() -> true'type { true } } fn run() -> true'type { S {}.read() }",
        "value method call requires a receiver",
    );
    rejected(
        "struct S {} impl S { fn read(flag: true'type, self) {} }",
        "self must be the first parameter",
    );
    rejected("fn run() { self; }", "self requires a method receiver");
}

#[test]
fn selected_receiver_method_does_not_fall_back_after_argument_errors() {
    rejected(
        "trait Read { fn read(self, flag: true'type) -> true'type; } struct S {} impl Read for S { fn read(self, flag: true'type) -> true'type { flag } } fn run() -> true'type { S {}.as(Read).read() }",
        "method argument count mismatch",
    );
    rejected(
        "struct S {} impl S { fn read(self, flag: true'type) -> true'type { flag } } fn run() -> true'type { S {}.read('x') }",
        "type mismatch",
    );
}

#[test]
fn default_member_calls_must_prove_the_called_member_contract() {
    rejected(
        "trait Read { fn read(self) -> true'type where requires false; fn use_read(self) -> true'type { self.as(Read).read() } }",
        "trait method precondition is not established",
    );
    accepted(
        "trait Read { fn read(self) -> true'type where requires false; fn use_read(self) -> true'type where requires false { self.as(Read).read() } }",
    );
    rejected(
        "trait Other { fn read(self) -> true'type; } trait Read { fn use_read(self) -> true'type { self.as(Other).read() } }",
        "method implementation is not uniquely established",
    );
}

#[test]
fn parent_interfaces_require_independent_implementations_for_the_same_subject() {
    rejected(
        "trait Parent { fn read() -> true'type; } trait Child :- Parent {} struct S {} impl Child for S { fn read() -> true'type { true } }",
        "required parent interface implementation is not established",
    );
    rejected(
        "trait Parent {} trait Child :- Parent {} struct S {} struct Other {} impl Parent for Other {} impl Child for S {}",
        "required parent interface implementation is not established",
    );
    rejected(
        "struct Parent {} trait Child :- Parent {}",
        "parent interface must be a trait",
    );
    rejected(
        "trait A :- B {} trait B :- A {}",
        "cyclic parent interface contracts are not yet implemented",
    );
}

#[test]
fn parent_capabilities_do_not_discharge_method_specific_preconditions() {
    rejected(
        "trait Parent { fn read() -> true'type where requires false; } trait Child :- Parent { fn run() -> true'type { Self.as(Parent).read() } }",
        "trait method precondition is not established",
    );
    accepted(
        "trait Parent { fn read() -> true'type where requires false; } trait Child :- Parent { fn run() -> true'type where requires false { Self.as(Parent).read() } }",
    );
}

#[test]
fn inherited_member_projection_requires_the_requested_child_view() {
    rejected(
        "trait Parent { fn read(self) -> true'type { true } } trait Child :- Parent {} struct S {} impl Parent for S {} fn run() -> true'type { S {}.as(Child).read() }",
        "requested interface view is not established",
    );
    rejected(
        "trait Parent { fn read() -> true'type { true } } trait Child :- Parent {} struct S {} impl Parent for S {} fn run() -> true'type { S.as(Child).read() }",
        "requested interface view is not established",
    );
}

#[test]
fn inherited_member_ambiguity_is_not_resolved_by_parent_order() {
    for parents in ["First + Second", "Second + First"] {
        rejected(
            &format!(
                "trait First {{ fn read() -> true'type {{ true }} }} trait Second {{ fn read() -> true'type {{ false }} }} trait Child :- {parents} {{ fn inspect() -> true'type {{ Self.read() }} }}"
            ),
            "ambiguous inherited member",
        );
    }
    rejected(
        "trait First { fn read(self) -> true'type { true } } trait Second { fn read(self) -> true'type { false } } trait Child :- First + Second { fn inspect(self) -> true'type { self.read() } }",
        "ambiguous inherited member",
    );
}

#[test]
fn generic_trait_bounds_are_obligations_not_assumptions_at_call_sites() {
    rejected(
        "trait Read {} struct S {} fn consume(value: T) where T :- Read {} fn run() { consume(S {}); }",
        "trait bound is not established",
    );
    rejected(
        "trait Read {} struct S {} fn consume(value: T) where T :- Read {} fn forward(value: U) where U { consume(value); }",
        "trait bound is not established",
    );
    rejected(
        "trait First {} trait Second {} struct S {} impl First for S {} fn consume(value: T) where T :- First + Second {} fn run() { consume(S {}); }",
        "trait bound is not established",
    );
    rejected(
        "trait Read {} struct S {} fn consume(value: T) where T :- Read {} fn run() { let f: fn(S) = consume; }",
        "trait bound is not established",
    );
}

#[test]
fn independent_generic_capabilities_keep_member_sources_ambiguous() {
    rejected(
        "trait First { fn read(self) -> true'type; } trait Second { fn read(self) -> true'type; } fn read(value: T) -> true'type where T :- First + Second { value.read() }",
        "ambiguous capability member",
    );
}

#[test]
fn nominal_trait_bounds_are_checked_at_all_formation_sites() {
    for suffix in [
        "typealias Bad = Box<S>;",
        "fn bad(value: Box<S>) {}",
        "struct Bad { value: Box<S>, }",
        "fn bad() { Box<S> { value: S {} }; }",
        "fn bad() { let x: Box<S> = Box<S> { value: S {} }; }",
    ] {
        rejected(
            &format!(
                "trait Marker {{}} struct S {{}} struct Box where T :- Marker {{ value: T, }} {suffix}"
            ),
            "trait bound is not established",
        );
    }
    rejected(
        "trait Marker {} struct S {} enum Maybe where T :- Marker { none, some(T), } fn bad() { Maybe<S>.none; }",
        "trait bound is not established",
    );
    rejected(
        "trait Marker {} struct Box where T :- Marker {} fn bad(value: Box<U>) where U {}",
        "trait bound is not established",
    );
}

#[test]
fn contract_operand_formation_cannot_assume_its_own_requires() {
    rejected(
        "struct Checked where T, requires T == true'type {} fn bad() where U, requires U == true'type and Checked<U> == Checked<U> {}",
        "type requirement",
    );
    accepted(
        "trait Marker {} struct Box where T :- Marker {} fn good() where U :- Marker, requires Box<U> == Box<U> {}",
    );
}

#[test]
fn nominal_field_defaults_cannot_borrow_a_callers_capabilities() {
    rejected(
        "trait Value { fn value() -> true'type; } struct Bad where T { value: true'type = read<T>(), } fn read() -> true'type where U :- Value { U.value() } fn make() -> Bad<T> where T :- Value { Bad<T> {} }",
        "trait bound is not established",
    );
}

#[test]
fn generic_implementation_members_inherit_the_head_telescope() {
    accepted(
        "trait Echo { fn echo(self) -> Self; } struct Box where T { value: T, } impl Echo for Box<T> where T { fn echo(self) -> Self { Box<T> { value: self.value } } } fn run() -> Box<true'type> { Box<true'type> { value: true }.as(Echo).echo() }",
    );
    accepted(
        "trait Read { fn read(self) -> true'type; } struct Box where T { value: T, } impl Read for Box<T> where T, requires T == true'type { fn read(self) -> true'type { self.value } } fn run() -> true'type { Box<true'type> { value: true }.as(Read).read() }",
    );
}

#[test]
fn generic_implementation_selection_uses_requires_and_precision() {
    accepted(
        "trait Read { fn read() -> true'type; } struct Box where T {} impl Read for Box<T> where T, requires T == true'type { fn read() -> true'type { true } } impl Read for Box<U> where U, requires U == 'x'\'type { fn read() -> true'type { false } } fn run() -> true'type { Box<true'type>.as(Read).read() }",
    );
    accepted(
        "trait Read { fn read() -> true'type; } struct Box where T {} impl Read for Box<T> where T { fn read() -> true'type { false } } impl Read for Box<true'type> { fn read() -> true'type { true } } fn run() -> true'type { Box<true'type>.as(Read).read() }",
    );
    rejected(
        "trait Read {} struct Box where T {} impl Read for Box<T> where T {} impl Read for Box<U> where U {}",
        "implementation overlap",
    );
    rejected(
        "trait Read { fn read() -> true'type; } struct Box where T {} impl Read for Box<T> where T, requires T == true'type { fn read() -> true'type { true } } fn run() -> true'type { Box<'x'\'type>.as(Read).read() }",
        "method implementation is not uniquely established",
    );
}

#[test]
fn generic_implementation_contracts_do_not_leak_or_strengthen() {
    rejected(
        "trait Read { fn read(self) -> true'type; } struct Box where T { value: T, } impl Read for Box<T> where T { fn read(self) -> true'type { self.value } }",
        "type",
    );
    rejected(
        "trait Read { fn read() -> true'type; } struct Box where T {} impl Read for Box<T> where T { fn read() -> true'type where requires T == true'type { true } }",
        "strengthens trait preconditions",
    );
    accepted(
        "trait Read { fn read() -> true'type; } struct Box where T {} impl Read for Box<T> where T, requires T == true'type { fn read() -> true'type where requires T == true'type { true } } impl Read for Box<U> where U, requires U == 'x'\'type { fn read() -> true'type { false } }",
    );
    accepted(
        "trait Echo { fn echo(self) -> Self { self } } struct Box where T {} impl Echo for Box<T> where T {} fn run() -> Box<true'type> { Box<true'type> {}.as(Echo).echo() }",
    );
}

#[test]
fn generic_implementation_parent_obligations_are_universal() {
    accepted(
        "trait Parent {} trait Child :- Parent {} struct Box where T {} impl Parent for Box<U> where U {} impl Child for Box<T> where T {}",
    );
    rejected(
        "trait Parent {} trait Child :- Parent {} struct Box where T {} impl Parent for Box<true'type> {} impl Child for Box<T> where T {}",
        "required parent interface implementation is not established",
    );
    accepted(
        "trait Parent {} trait Child :- Parent {} struct Box where T {} impl Parent for Box<U> where U, requires U == true'type {} impl Child for Box<T> where T, requires T == true'type {}",
    );
}

#[test]
fn generic_implementation_requires_cannot_justify_contract_formation_circularly() {
    rejected(
        "trait Marker {} struct Checked where T, requires T == true'type {} impl Marker for T where T, requires T == true'type and Checked<T> == Checked<T> {}",
        "type requirement",
    );
    rejected(
        "trait Read { fn read() -> true'type; } struct Box where T {} impl Read for Box<T> where T, requires T == true'type { fn read() -> true'type { true } } fn forward() -> true'type where U { Box<U>.as(Read).read() }",
        "method implementation is not uniquely established",
    );
    accepted(
        "trait Read { fn read() -> true'type; } struct Box where T {} impl Read for Box<T> where T, requires T == true'type { fn read() -> true'type { true } } fn forward() -> true'type where U, requires U == true'type { Box<U>.as(Read).read() }",
    );
}

#[test]
fn generic_implementation_unused_parameters_do_not_escape_as_instances() {
    rejected(
        "trait Read { fn read() -> true'type; } struct S {} impl Read for S where T { fn read() -> true'type { true } } fn run() -> true'type { S.as(Read).read() }",
        "method implementation is not uniquely established",
    );
    accepted(
        "trait Echo { fn echo(self) -> Self; } impl Echo for T where T { fn echo(self) -> Self { self } } struct S {} fn run() -> S { S {}.as(Echo).echo() }",
    );
}

#[test]
fn generic_inherent_overlap_checks_shared_member_domains() {
    rejected(
        "struct Box where T {} impl Box<T> where T { fn get() {} } impl Box<U> where U { fn get() {} }",
        "inherent implementation overlap",
    );
    accepted(
        "struct Box where T {} impl Box<T> where T { fn first() {} } impl Box<U> where U { fn second() {} } fn run() { Box<true'type>.first(); Box<true'type>.second(); }",
    );
    rejected(
        "struct Box where T {} impl Box<T> where T { fn get() {} } impl Box<U> where U, requires U == true'type { fn get() {} }",
        "inherent implementation overlap",
    );
}

#[test]
fn generic_inherent_pending_specific_candidate_blocks_blanket_selection() {
    rejected(
        "struct Box where T {} impl Box<T> where T { fn read() -> true'type { false } } impl Box<true'type> { fn read() -> true'type { true } } fn forward() -> true'type where U { Box<U>.read() }",
        "inherent method applicability is unresolved",
    );
    accepted(
        "struct Box where T {} impl Box<T> where T { fn read() -> true'type { false } } impl Box<true'type> { fn read() -> true'type { true } } fn forward() -> true'type where U, requires U == true'type { Box<U>.read() }",
    );
}

#[test]
fn generic_inherent_cannot_escape_subject_ownership_or_unproved_requires() {
    rejected(
        "impl T where T { fn read() {} }",
        "inherent implementation admission failed",
    );
    rejected(
        "struct Box where T {} impl Box<T> where T, requires T == true'type { fn read() {} } fn forward() where U { Box<U>.read(); }",
        "inherent method applicability is unresolved",
    );
    rejected(
        "struct Box where T { value: T, } impl Box<T> where T { fn read(self) -> true'type { self.value } }",
        "type",
    );
}

#[test]
fn generic_inherent_selected_member_errors_do_not_fall_back() {
    rejected(
        "struct Box where T {} impl Box<T> where T { fn call(value: true'type) {} } impl Box<true'type> { fn call(value: 'x''type) {} } fn run() { Box<true'type>.call(true); }",
        "type",
    );
}

#[test]
fn inherent_optional_routing_rejects_duplicate_unknown_and_missing_inputs() {
    let prefix = "struct S {} impl S { fn choose(self, required: true'type, .option: true'type = false) {} } ";
    rejected(
        &format!("{prefix} fn run() {{ S {{}}.choose(true, .option = true, .option = false); }}"),
        "parameter supplied more than once",
    );
    rejected(
        &format!("{prefix} fn run() {{ S {{}}.choose(true, .missing = true); }}"),
        "no named parameter entry",
    );
    rejected(
        &format!("{prefix} fn run() {{ S {{}}.choose(.option = true); }}"),
        "method argument count mismatch",
    );
    rejected(
        &format!("{prefix} fn run() {{ S {{}}.choose(true, false); }}"),
        "argument count mismatch",
    );
}

#[test]
fn inherent_method_values_do_not_recover_erased_default_entries() {
    rejected(
        "struct S {} impl S { fn choose(.option: true'type = false) {} } fn run() { let f = S.choose; f(); }",
        "argument count mismatch",
    );
    rejected(
        "struct S {} impl S { fn choose(.first: true'type = later, .later: true'type = false) {} }",
        "Unresolved identifier",
    );
}

#[test]
fn implementation_trait_bounds_are_caller_obligations() {
    rejected(
        "trait Read { fn read() -> true'type; } trait Marker {} struct S {} struct Box where T {} impl Read for Box<T> where T :- Marker { fn read() -> true'type { true } } fn run() -> true'type { Box<S>.as(Read).read() }",
        "method implementation is not uniquely established",
    );
    rejected(
        "trait Read { fn read() -> true'type; } trait Marker {} struct Box where T {} impl Read for T where T { fn read() -> true'type { false } } impl Read for Box<T> where T :- Marker { fn read() -> true'type { true } } fn forward() -> true'type where U { Box<U>.as(Read).read() }",
        "method implementation is not uniquely established",
    );
}

#[test]
fn implementation_trait_bounds_do_not_bootstrap_cycles() {
    rejected(
        "trait Read { fn read() -> true'type { true } } struct S {} impl Read for T where T :- Read {} fn run() -> true'type { S.as(Read).read() }",
        "method implementation is not uniquely established",
    );
    rejected(
        "trait A { fn read() -> true'type { true } } trait B {} struct S {} impl A for T where T :- B {} impl B for T where T :- A {} fn run() -> true'type { S.as(A).read() }",
        "method implementation is not uniquely established",
    );
}

#[test]
fn implementation_trait_bounds_do_not_make_same_rank_heads_disjoint() {
    rejected(
        "trait Read {} trait First {} trait Second {} struct Box where T {} impl Read for Box<T> where T :- First {} impl Read for Box<U> where U :- Second {}",
        "implementation overlap",
    );
}

#[test]
fn implementation_trait_bounds_can_supply_parent_obligations_abstractly() {
    accepted("trait Parent {} trait Child :- Parent {} impl Child for T where T :- Parent {}");
    rejected(
        "trait Parent {} trait Child :- Parent {} impl Child for T where T {}",
        "required parent interface implementation is not established",
    );
}

#[test]
fn parameterized_trait_source_requires_complete_matching_interface_arguments() {
    rejected(
        "trait Echo where T { fn echo(value: T) -> T { value } } struct S {} impl Echo<true'type> for S {} fn run() -> 'x''type { S.as(Echo<'x''type>).echo('x') }",
        "method implementation is not uniquely established",
    );
    rejected(
        "trait Echo where T {} struct S {} impl Echo for S {}",
        "interface type argument count mismatch",
    );
    rejected(
        "trait Echo where T {} struct S {} impl Echo<true'type, 'x''type> for S {}",
        "interface type argument count mismatch",
    );
    rejected(
        "trait Echo where T { fn echo(value: T) -> T; } struct S {} impl Echo<true'type> for S { fn echo(value: 'x''type) -> 'x''type { value } }",
        "does not match its trait signature",
    );
}

#[test]
fn parameterized_trait_source_parent_instances_and_member_sources_do_not_collapse() {
    rejected(
        "trait Parent where T {} trait Child :- Parent<T> where T {} struct S {} impl Parent<'x''type> for S {} impl Child<true'type> for S {}",
        "required parent interface implementation is not established",
    );
    rejected(
        "trait Echo where T { fn echo(self, value: T) -> T; } fn bad(value: U) -> true'type where U :- Echo<true'type> + Echo<'x''type> { value.echo(true) }",
        "ambiguous capability member",
    );
}

#[test]
fn parameterized_trait_source_bounds_cannot_borrow_another_instance() {
    rejected(
        "trait Echo where T {} struct S {} impl Echo<'x''type> for S {} fn need(value: U) where U :- Echo<true'type> {} fn run() { need(S {}); }",
        "trait bound is not established",
    );
    rejected(
        "trait Echo where T {} struct S {} impl Echo<'x''type> for S {} struct Holder where U :- Echo<true'type> { value: U, } fn run() { Holder<S> { value: S {} }; }",
        "trait bound is not established",
    );
}

#[test]
fn trait_parameter_contracts_are_checked_at_interface_formation_sites() {
    rejected(
        "trait Checked where T, requires T == true'type {} struct S {} impl Checked<'x''type> for S {}",
        "type requirement",
    );
    rejected(
        "trait Checked where T, requires T == true'type {} fn use_bound(value: U) where T, U :- Checked<T> {}",
        "type requirement",
    );
    rejected(
        "trait Marker {} struct S {} trait Checked where T :- Marker {} impl Checked<S> for S {}",
        "trait bound is not established",
    );
    rejected(
        "trait Checked where T, requires T == true'type {} trait Child :- Checked<T> where T {}",
        "type requirement",
    );
    accepted(
        "trait Checked where T, requires T == true'type {} fn use_bound(value: U) where T, U :- Checked<T>, requires T == true'type {}",
    );
}

#[test]
fn trait_parameter_requirements_cannot_justify_their_own_operand_formation() {
    rejected(
        "struct Needs where T, requires T == true'type {} trait Bad where U, requires U == true'type and Needs<U> == Needs<U> {}",
        "type requirement",
    );
    accepted("trait OnlyBool where requires Self == true'type {}");
}

#[test]
fn trait_parameter_assumptions_do_not_leak_to_other_traits_or_implementations() {
    rejected(
        "trait Checked where T, requires T == true'type { fn accept(value: T) -> true'type { value } } trait Unchecked where U { fn accept(value: U) -> true'type { value } }",
        "type",
    );
    rejected(
        "trait Checked where T, requires T == true'type { fn accept(value: T) -> true'type { value } } struct S {} impl Checked<U> for S where U {}",
        "type requirement",
    );
}

#[test]
fn trait_optional_argument_routing_rejects_invalid_supplies() {
    let prefix = "trait Choose { fn choose(value: true'type, .option: true'type = value) -> true'type; } struct Host {} impl Choose for Host { fn choose(input: true'type, answer: true'type) -> true'type { answer } }";
    for (call, message) in [
        (
            "Host.as(Choose).choose(true, .option = true, .option = false)",
            "parameter supplied more than once",
        ),
        (
            "Host.as(Choose).choose(true, .answer = true)",
            "no named parameter entry",
        ),
        (
            "Host.as(Choose).choose(.option = true)",
            "argument count mismatch",
        ),
        (
            "Host.as(Choose).choose(true, false)",
            "argument count mismatch",
        ),
        (
            "Host.as(Choose).choose(true, .option = 'x')",
            "type mismatch",
        ),
    ] {
        rejected(&format!("{prefix} fn entry() {{ {call}; }}"), message);
    }
    rejected(
        &format!("{prefix} fn entry() {{ let f = Host.as(Choose).choose; f(true); }}"),
        "argument count mismatch",
    );
}

#[test]
fn trait_required_method_defaults_are_checked_without_any_implementation() {
    rejected(
        "trait Bad { fn f(.first: true'type = later, .later: true'type = false); }",
        "Unresolved identifier",
    );
    rejected(
        "trait Bad { fn f(.flag: true'type = 'x'); }",
        "type mismatch",
    );
    rejected(
        "trait Bad where T { fn f(value: T, .copy: T = value); }",
        "default expression requires P3 stability",
    );
}

#[test]
fn memory_defaults_admit_stable_observation_but_not_pointer_writes() {
    for source in [
        "fn read(p:*bool,.value:bool=p.*)->bool {value}",
        "fn helper(p:*bool)->bool {p.*} fn read(p:*bool,.value:bool=helper(p))->bool {value}",
    ] {
        accepted(source);
    }
    rejected(
        "fn read(p:*bool,.value:bool=do {p.* = false; true})->bool {value}",
        "default expression requires P3 stability",
    );
}

#[test]
fn trait_subject_requirements_reject_invalid_implementations_and_leaked_guarantees() {
    rejected(
        "trait Same where T, requires Self == T {} impl Same<true'type> for 'x''type {}",
        "implementation does not establish trait subject requirement",
    );
    rejected(
        "trait Same where T, requires Self == T {} impl Same<U> for T where T, U {}",
        "implementation does not establish trait subject requirement",
    );
    rejected(
        "trait Same where T, requires Self == T {} fn valid(value: S) -> T where T, S :- Same<T> { value } fn invalid(value: S) -> T where T, S { value }",
        "type mismatch",
    );
    rejected(
        "trait Same where T, requires Self == T {} impl Same<T> for T where T {} fn convert(value: S) -> T where T, S :- Same<T> { value } fn invalid() -> true'type { convert<true'type, 'x''type>('x') }",
        "trait bound is not established",
    );
    rejected(
        "struct Needs where T, requires T == true'type {} trait Bad where requires Self == true'type and Needs<Self> == Needs<Self> {}",
        "type requirement",
    );
}

#[test]
fn trait_subject_guarantees_form_contract_operands_without_assuming_the_contract() {
    accepted(
        "trait OnlyBool where requires Self == true'type {} struct Needs where T, requires T == true'type {} fn valid(value: T) where T :- OnlyBool, requires Needs<T> == Needs<T> {}",
    );
    rejected(
        "trait OnlyBool where requires Self == true'type {} struct Needs where T, requires T == true'type {} fn invalid(value: T) where T, requires T == true'type and Needs<T> == Needs<T> {}",
        "type requirement",
    );
}

#[test]
fn subject_dependent_trait_capabilities_reject_missing_or_wrong_instances() {
    let prefix = "trait Read where S {} trait Adapter where T :- Read<Self> {} struct Host {} struct Other {} struct Leaf {}";
    rejected(
        &format!("{prefix} impl Adapter<Leaf> for Host {{}}"),
        "required trait contract capability implementation is not established",
    );
    rejected(
        &format!("{prefix} impl Read<Other> for Leaf {{}} impl Adapter<Leaf> for Host {{}}"),
        "required trait contract capability implementation is not established",
    );
    rejected(
        &format!("{prefix} impl Adapter<T> for Host where T {{}}"),
        "required trait contract capability implementation is not established",
    );
}

#[test]
fn integer_runtime_proxies_reject_overflow_instead_of_wrapping_or_widening() {
    for source in [
        "fn entry() -> .i8'builtin { 128 }",
        "fn entry() -> .i8'builtin { -129 }",
        "fn entry() -> .u8'builtin { 256 }",
        "fn entry() -> .u8'builtin { -1 }",
        "fn entry() -> .i128'builtin { 170141183460469231731687303715884105728 }",
        "fn entry() -> .u128'builtin { 340282366920938463463374607431768211456 }",
        "fn entry() { let value = 2147483648; }",
    ] {
        rejected(source, "integer does not fit");
    }
    rejected(
        "fn entry() -> true'type { 1 }",
        "integer runtime proxy requires",
    );
    rejected(
        "fn entry() -> .usize'builtin { 1 }",
        "target pointer-width information",
    );
    rejected(
        "fn entry() -> 1'type { 1 }",
        "comptime-only value cannot inhabit runtime storage",
    );
}

#[test]
fn integer_runtime_proxies_keep_literal_type_distinct_from_runtime_proxy() {
    let dump = accepted(
        "typealias Literal = 999999999999999999999999999999999999999999999999999999999999999999999999'type; typealias Negative = (-1)'type; fn check() where requires Literal == .Integer'builtin and Negative == .Integer'builtin {}",
    );
    assert!(dump.contains("(primitive Integer)"), "{dump}");
}

#[test]
fn float_runtime_proxies_reject_unsupported_targets_and_finite_overflow() {
    rejected(
        "fn entry() -> .f32'builtin { 1.0e39 }",
        "overflowing finite float literal conversion is not yet implemented",
    );
    rejected(
        "fn entry() -> .f64'builtin { -1.0e309 }",
        "overflowing finite float literal conversion is not yet implemented",
    );
    rejected(
        "fn entry() -> .i32'builtin { 1.0 }",
        "float literal requires a Float, f32 or f64 target",
    );
    rejected(
        "fn entry() -> 0.1'type { 0.1 }",
        "comptime-only value cannot inhabit runtime storage",
    );
    let dump = accepted(
        "typealias Positive = 1.0e9999'type; typealias Negative = (-0.0)'type; fn check() where requires Positive == .Float'builtin and Negative == .Float'builtin {}",
    );
    assert!(dump.contains("(primitive Float)"), "{dump}");
}

#[test]
fn bitvec_source_rejects_invalid_widths_and_unsupported_representations() {
    let prefix = "fn byte() -> .u8'builtin { 0 } typealias Bits = do { let value: .u8'builtin = 0; value'bitvec }'type; ";
    for body in ["fn bad() -> Bits { 256 }", "fn bad() -> Bits { -1 }"] {
        rejected(
            &format!("{prefix}{body}"),
            "integer literal does not fit the BitVec width",
        );
    }
    rejected(
        &format!("{prefix} fn bad() -> Bits {{ byte()'bitvec and true }}"),
        "type mismatch",
    );
    rejected(
        "fn bad() -> true'type { true'bitvec }",
        "bit view requires a supported fixed-width representation",
    );
}

#[test]
fn bitvec_slice_boundaries_and_conversion_widths_are_checked_before_nir() {
    let prefix = "fn word() -> .u16'builtin { 0 } ";
    for body in [
        "word()'bitvec[16]",
        "word()'bitvec[17..17]",
        "word()'bitvec[7..3]",
        "word()'bitvec[0..17]",
    ] {
        rejected(
            &format!("{prefix} fn bad() {{ {body}; }}"),
            "BitVec index or slice is out of bounds",
        );
    }
    rejected(
        &format!("{prefix} fn bad() {{ word()'bitvec.as(.u8'builtin); }}"),
        "BitVec narrowing requires an explicit slice",
    );
    rejected(
        &format!("{prefix} fn bad() {{ word()'bitvec.as(true'type); }}"),
        "other representations need unsafe construction support",
    );
    rejected(
        &format!("{prefix} fn bad(index: .u8'builtin) {{ word()'bitvec[index]; }}"),
        "BitVec index is not proven in bounds",
    );
    rejected(
        &format!("{prefix} fn bad() {{ word()'bitvec[-1]; }}"),
        "BitVec index must be nonnegative",
    );
    rejected(
        &format!("{prefix} fn bad() {{ word()'bitvec[0..=3]; }}"),
        "explicit half-open range",
    );
    rejected(
        &format!("{prefix} fn bad() {{ word()'bitvec[0..8] == word()'bitvec; }}"),
        "type mismatch",
    );
}

#[test]
fn bitvec_writeback_rejects_invalid_locations_widths_and_captured_state() {
    rejected(
        "fn bad() { let x: .u8'builtin = 0; x'bitvec[0..4] = 1; }",
        "cannot reassign a let binding",
    );
    rejected(
        "fn bad() { var x: .u8'builtin = 0; x'bitvec[0..4] = 16; }",
        "integer literal does not fit the BitVec width",
    );
    rejected(
        "fn bad() { var x: .u8'builtin = 0; x'bitvec[0..4] = x'bitvec; }",
        "type mismatch",
    );
    rejected(
        "fn bad() { var x: .u8'builtin = 0; x'bitvec[0..9] = 0; }",
        "BitVec index or slice is out of bounds",
    );
    accepted("fn run() { var x: .u8'builtin = 0; let f = || { x'bitvec[0] = 1; }; f(); }");
    rejected(
        "fn bad() { var x = true; x'bitvec[0] = 0; }",
        "bit writeback requires a supported scalar or BitVec representation",
    );
    rejected(
        "fn byte() -> .u8'builtin { 0 } fn bad() { byte()'bitvec[0] = 1; }",
        "assignment place elaboration is not implemented",
    );
}

#[test]
fn signature_observation_defers_but_does_not_discard_contract_obligations() {
    rejected(
        "typealias Bad = constrained()'type; fn constrained() -> true'type where requires true'type == 'x''type { true }",
        "type requirement is not satisfied",
    );
    rejected(
        "typealias Bad = identity('x')'type; fn identity(value: T) -> T where T, requires T == true'type { value }",
        "type requirement is not satisfied",
    );
    rejected(
        "fn first() -> second()'type { true } fn second() -> first()'type { true }",
        "cyclic function signature dependency",
    );
    rejected(
        "typealias Recursive = call()'type; fn call() -> Recursive { true }",
        "cyclic",
    );
    accepted(
        "fn first() -> second()'type { true } fn second() -> true'type where requires true'type == true'type { true }",
    );
}

#[test]
fn signature_observation_keeps_definition_contracts_out_of_their_own_operands() {
    rejected(
        r#"
        fn guarded() -> T where T, requires T == true'type { true }
        fn circular() -> T where T, requires guarded<T>()'type == true'type { true }
    "#,
        "type requirement is not satisfied",
    );
    accepted(
        r#"
        fn projected(value: T) -> guarded<T>()'type where T, requires T == true'type { value }
        fn guarded() -> T where T, requires T == true'type { true }
        fn entry() -> true'type { projected(true) }
    "#,
    );
    rejected(
        r#"
        trait Needed {}
        typealias Bad = constrained('x')'type;
        fn constrained(value: T) -> T where T :- Needed { value }
    "#,
        "trait",
    );
}

#[test]
fn signature_formation_obligations_survive_abandoned_argument_probes() {
    rejected(
        r#"
        typealias Observed = choose(do { let marker = shape; identity(0) }, target())'type;
        struct Box where T, requires T == true'type { value: T }
        fn shape() -> Box<'x''type> { true }
        fn identity(value: T) -> T where T { value }
        fn choose(a: T, b: T) -> T where T { a }
        fn target() -> .i32'builtin { 0 }
    "#,
        "type requirement is not satisfied",
    );
}

#[test]
fn signature_dependency_depth_is_bounded_and_ordinary_chains_resolve() {
    fn chain(length: usize) -> String {
        let mut source = String::new();
        for i in 0..length {
            source.push_str(&format!("fn f{i}() -> f{}()'type {{ true }} ", i + 1));
        }
        source.push_str(&format!("fn f{length}() -> true'type {{ true }}"));
        source
    }
    accepted(&chain(24));
    rejected(
        &chain(80),
        "function signature dependency depth limit exceeded",
    );
}

#[test]
fn bitvec_patterns_reject_non_bit_subjects_bad_digits_widths_and_duplicate_bindings() {
    rejected(
        "fn bad() -> true'type { 7 matches 0b111 }",
        "bit-vector patterns require a BitVec subject",
    );
    rejected(
        "fn bad() -> true'type { 7'bitvec matches 0b111 }",
        "bit-vector pattern width differs from its subject",
    );
    rejected(
        "fn bad() -> true'type { 7'bitvec matches 0b0 2 }",
        "invalid digit in bit-vector literal segment",
    );
    rejected(
        "fn bad() -> true'type { 7'bitvec matches 0x(x: .u16'builtin) (x: .u16'builtin) }",
        "duplicate pattern binding",
    );
    rejected(
        "fn bad() -> true'type { 7'bitvec matches 0x(x: true'type) }",
        "supported unrestricted scalar or BitVec representation",
    );
}

#[test]
fn bitvec_patterns_keep_phase_exhaustiveness_and_branch_binding_checks() {
    rejected(
        "fn bad(value: .BitVec'builtin<4>) -> true'type { value matches 0b < value > }",
        "required comptime",
    );
    rejected(
        "fn bad() -> true'type { 7'bitvec match { 0x00000007 => true } }",
        "exhaustive",
    );
    rejected(
        "fn bad() -> true'type { 7'bitvec match { 0x(x: .u32'builtin) or 0x(y: .u32'builtin) => true, _ => false } }",
        "binding",
    );
}

#[test]
fn bitvec_coverage_preserves_holes_guards_and_tuple_correlations() {
    rejected(
        "fn bad() -> true'type { (0'bitvec[0],true) match { (0b0,false)=>true,(0b1,true)=>false } }",
        "exhaustive",
    );
    rejected(
        "fn bad() -> true'type { 0'bitvec[0..2] match { 0b00=>true,0b01=>false,0b10=>true } }",
        "exhaustive",
    );
    rejected(
        "fn bad() -> true'type { 0'bitvec[0] match { 0b0=>true,0b1 if false=>false } }",
        "exhaustive",
    );
    accepted(
        "typealias Bit=(0'bitvec[0])'type; fn ready() -> true'type { 0'bitvec[0..2] match { 0b00=>true,0b01=>false,0b1(_: Bit)=>true } }",
    );
}

#[test]
fn explicit_bitvec_type_construction_checks_arguments_identity_and_literal_ranges() {
    for (source, message) in [
        (
            "typealias Bad = .BitVec'builtin;",
            "requires a width argument",
        ),
        (
            "typealias Bad = .BitVec'builtin<1,2>;",
            "exactly one positional comptime width",
        ),
        (
            "typealias Bad = .BitVec'builtin<-1>;",
            "nonnegative comptime integer",
        ),
        (
            "typealias Bad = .BitVec'builtin<1.0>;",
            "nonnegative comptime integer",
        ),
        (
            "typealias Bad = .BitVec'builtin<340282366920938463463374607431768211456>;",
            "implementation size limit",
        ),
        (
            "fn bad() -> .BitVec'builtin<9> { 512 }",
            "integer literal does not fit the BitVec width",
        ),
        (
            "fn bad() -> .BitVec'builtin<0> { 1 }",
            "integer literal does not fit the BitVec width",
        ),
        (
            "fn bad(width: .u32'builtin) -> .BitVec'builtin<width> { 0 }",
            "cannot read a runtime parameter",
        ),
        (
            "fn a(value: .BitVec'builtin<8>) -> .BitVec'builtin<9> { value }",
            "type mismatch",
        ),
    ] {
        rejected(source, message);
    }
    accepted(
        "struct BitVec where T { value: T } fn normal() -> BitVec<true'type> { BitVec<true'type> { value: true } } fn intrinsic() -> .BitVec'builtin<8> { 255 }",
    );
    accepted(
        "typealias A = .BitVec'builtin<0x08>; typealias B = .BitVec'builtin<8>; fn same(value: A) -> B { value }",
    );
}

#[test]
fn explicit_bitvec_width_identity_participates_in_trait_coherence() {
    rejected(
        r#"
        trait Read { fn read(self) -> true'type; }
        impl Read for .BitVec'builtin<8> { fn read(self) -> true'type { true } }
        impl Read for .BitVec'builtin<0x08> { fn read(self) -> true'type { false } }
    "#,
        "overlap",
    );
    rejected(
        r#"
        trait Read { fn read(self) -> true'type; }
        impl Read for .BitVec'builtin<8> { fn read(self) -> true'type { true } }
        fn invoke(value: T) -> true'type where T :- Read { value.read() }
        fn bad() -> true'type { let value: .BitVec'builtin<9> = 0; invoke(value) }
    "#,
        "trait bound is not established",
    );
}

#[test]
fn bitvec_scalar_reinterpretation_requires_exact_width_and_valid_representation() {
    for target in ["i8", "i32", "f32", "f64"] {
        rejected(
            &format!("fn bad(value: .u16'builtin) {{ value'bitvec.as(.{target}'builtin); }}"),
            "requires exactly the target width",
        );
    }
    rejected(
        "fn bad(value: .u8'builtin) { value'bitvec.as(true'type); }",
        "other representations need unsafe construction support",
    );
    rejected(
        "fn bad(value: .u32'builtin) { value'bitvec.as('x''type); }",
        "other representations need unsafe construction support",
    );
    // Positive signed codes plus the sign-bit mask cover all byte representations.
    let arms = (0..=127)
        .map(|v| format!("0x({v}: .i8'builtin) => true"))
        .collect::<Vec<_>>()
        .join(",");
    accepted(&format!(
        "fn all(value: .u8'builtin) -> true'type {{ value'bitvec match {{ {arms}, 0b1(_: .BitVec'builtin<7>) => false }} }}"
    ));
    let missing = (1..=127)
        .map(|v| format!("0x({v}: .i8'builtin) => true"))
        .collect::<Vec<_>>()
        .join(",");
    rejected(
        &format!(
            "fn bad(value: .u8'builtin) -> true'type {{ value'bitvec match {{ {missing}, 0b1(_: .BitVec'builtin<7>) => false }} }}"
        ),
        "exhaustive",
    );
    rejected(
        "fn bad(value: .u32'builtin) -> true'type { value'bitvec match { 0x(0.0: .f32'builtin) => true } }",
        "exhaustive",
    );
}

#[test]
fn comptime_phase_boundaries_are_checked_without_promoting_constant_runtime_calls() {
    rejected(
        "fn runtime() -> true'type { true } comptime fn bad() -> true'type { runtime() }",
        "runtime function is unavailable during comptime",
    );
    rejected(
        "comptime fn compute() -> true'type { true } fn bad() -> true'type { compute() }",
        "comptime-only value cannot inhabit runtime storage",
    );
    rejected(
        "fn runtime() -> true'type { true } comptime fn bad() -> true'type { if false { runtime() } else { true } }",
        "runtime function is unavailable during comptime",
    );
    rejected(
        "comptime fn compute() -> true'type { true } fn bad() { let escaped = compute; }",
        "comptime-only value cannot inhabit runtime storage",
    );
    accepted(
        "comptime fn compute() -> true'type { true } comptime fn keep() { let callback = compute; }",
    );
    rejected(
        "fn runtime() -> true'type { true } comptime fn bad(.flag: true'type = runtime()) -> true'type { flag }",
        "runtime function is unavailable during comptime execution",
    );
    accepted(
        "pure comptime fn compute(flag: true'type) -> true'type { not flag } fn runtime() -> true'type { true }",
    );
}

#[test]
fn comptime_higher_order_boundaries_remain_explicit() {
    rejected(
        "comptime fn bad(callback: fn() -> true'type) -> true'type { callback() }",
        "callable type is unavailable in this execution phase",
    );
    accepted("comptime fn identity(value: T) -> T where T { value }");
    accepted("comptime fn run() -> true'type { let callback = || true; callback() }");
    accepted("trait Read { comptime fn read(self) -> true'type; }");
}

#[test]
fn comptime_integer_values_cannot_escape_into_runtime_storage() {
    for source in [
        "fn bad(value: .Integer'builtin) -> .Integer'builtin { value }",
        "fn bad() { let value: .Integer'builtin = 1; }",
        "struct Box { value: .Integer'builtin } fn bad(value: Box) -> Box { value }",
        "struct Box where T { value: T } fn bad(value: Box<.Integer'builtin>) -> Box<.Integer'builtin> { value }",
        "enum Choice { empty, some(.Integer'builtin) } fn bad(value: Choice) -> Choice { value }",
        "fn bad(value: (.Integer'builtin, true'type)) -> (.Integer'builtin, true'type) { value }",
    ] {
        rejected(source, "comptime-only value cannot inhabit runtime storage");
    }
    // A phantom type argument does not create a stored comptime value.
    accepted(
        "struct Marker where T {} fn keep(value: Marker<.Integer'builtin>) -> Marker<.Integer'builtin> { value }",
    );
    accepted(
        "struct Box { value: .Integer'builtin } comptime fn keep(value: Box) -> Box { value }",
    );
    rejected(
        "comptime fn bad(value: .Integer'builtin) -> .Integer'builtin { value / 2 }",
        "Integer division operator rounding protocol is not implemented",
    );
    rejected(
        "comptime fn bad(value: .Integer'builtin) -> .Integer'builtin { value % 2 }",
        "protocol or construct not yet implemented",
    );
}

#[test]
fn comptime_type_values_keep_formation_and_runtime_stage_boundaries() {
    rejected(
        "comptime fn Bad() -> .Type'builtin { .BitVec'builtin<true> }",
        "type mismatch",
    );
    rejected(
        "comptime fn Bad() -> .Type'builtin { .BitVec'builtin<1,2> }",
        "exactly one positional comptime width",
    );
    rejected(
        "struct Box where T, requires T == true'type {} comptime fn Bad() -> .Type'builtin { Box<.u8'builtin> }",
        "type requirement is not satisfied",
    );
    rejected(
        "fn Bad() -> .Type'builtin { .u8'builtin }",
        "comptime-only value cannot inhabit runtime storage",
    );
    rejected(
        "fn bad() { let value = .u8'builtin; }",
        "comptime-only value cannot inhabit runtime storage",
    );
    accepted(
        "typealias Type = .Type'builtin; pure comptime fn Identity(value: Type) -> Type { value } comptime fn Good() -> Type { Identity(.u8'builtin) }",
    );
}

#[test]
fn required_comptime_type_formation_rejects_runtime_calls_cycles_and_execution_failures() {
    rejected(
        "fn Runtime() -> .u8'builtin { 8 } typealias Bits = .BitVec'builtin<Runtime()>;",
        "type mismatch",
    );
    rejected(
        "fn Runtime() -> .Type'builtin { .u8'builtin } typealias Alias = Runtime();",
        "cannot execute a runtime function",
    );
    rejected(
        "typealias A = Compute(); comptime fn Compute() -> A { .u8'builtin }",
        "cyclic",
    );
    rejected(
        "comptime fn Endless() -> .Type'builtin { Endless() } typealias Alias = Endless();",
        "DepthExhausted",
    );
    rejected(
        "comptime fn Bits() -> .Type'builtin { .BitVec'builtin<-1> } typealias Alias = Bits();",
        "InvalidBitWidth",
    );
    rejected(
        "comptime fn FalseContract() -> .Type'builtin where requires false { .u8'builtin } typealias Alias = FalseContract();",
        "type requirement is not satisfied",
    );
    rejected(
        "comptime fn Width() -> .Integer'builtin { -1 } typealias Alias = .BitVec'builtin<Width()>;",
        "nonnegative comptime integer",
    );
    accepted(
        "comptime fn Choose(flag: true'type) -> .Type'builtin { if flag { .u8'builtin } else { .u16'builtin } } typealias Alias = Choose(1 == 1); fn value() -> Alias { 255 }",
    );
}

#[test]
fn required_comptime_alias_completion_is_reused_across_many_consumers() {
    let mut source = String::from(
        "typealias Byte = Slow(); comptime fn Slow() -> .Type'builtin { var n: .Integer'builtin = 0; while n < 1000 { n = n + 1; } .u8'builtin }",
    );
    for i in 0..128 {
        source.push_str(&format!("fn use_{i}(value: Byte) -> Byte {{ value }} "));
    }
    accepted(&source);
}

#[test]
fn generic_comptime_preserves_definition_checks_and_phase_boundaries() {
    rejected(
        "comptime fn bad(value: T) -> T where T { true }",
        "type mismatch",
    );
    rejected(
        "comptime fn identity(value: T) -> T where T { value } fn bad() -> true'type { identity(true) }",
        "comptime-only value cannot inhabit runtime storage",
    );
    rejected(
        "fn identity(value: T) -> T where T { value } comptime fn bad() -> true'type { identity(true) }",
        "runtime function is unavailable",
    );
    rejected(
        "comptime fn Checked(value: T) -> .Type'builtin where T, requires false { T } typealias Bad = Checked(true);",
        "type requirement is not satisfied",
    );
    accepted(
        "comptime fn identity(value: T) -> T where T { value } comptime fn keep() { let callback = identity<true'type>; }",
    );
}

#[test]
fn required_comptime_contracts_do_not_prove_themselves_or_leak_between_scopes() {
    rejected(
        "typealias Bad = Checked(.u8'builtin); comptime fn Checked(value: T) -> .Type'builtin where T, requires T == true'type { T }",
        "type requirement is not satisfied",
    );
    rejected(
        "typealias Bad = Outer(); comptime fn Outer() -> .Type'builtin { Inner() } comptime fn Inner() -> .Type'builtin where requires false { .u8'builtin }",
        "type requirement is not satisfied",
    );
    rejected(
        "struct Restricted where T, requires T == true'type {} comptime fn Bad(value: T) -> .Type'builtin where T, requires Restricted<T> == Restricted<T> { T } typealias Result = Bad(.u8'builtin);",
        "type requirement is not satisfied",
    );
}

#[test]
fn comptime_defaults_reject_dependency_cycles_and_invalid_contracts() {
    rejected(
        "comptime fn A(.value: .Type'builtin = B()) -> .Type'builtin { value } comptime fn B(.value: .Type'builtin = A()) -> .Type'builtin { value } typealias Bad = A();",
        "function default dependency",
    );
    rejected(
        "comptime fn Bad(.value: .Type'builtin = Checked()) -> .Type'builtin { value } comptime fn Checked() -> .Type'builtin where requires false { .u8'builtin } typealias Result = Bad();",
        "type requirement is not satisfied",
    );
}

#[test]
fn computed_bit_segments_check_phase_width_contracts_and_coverage() {
    rejected(
        "comptime fn bit() -> .BitVec'builtin<1> { 1 } fn bad(value: .BitVec'builtin<2>) -> true'type { value matches 0b < bit() > }",
        "bit-vector pattern width differs",
    );
    rejected(
        "fn bit() -> .BitVec'builtin<1> { 1 } fn bad(value: .BitVec'builtin<1>) -> true'type { value matches 0b < bit() > }",
        "cannot execute a runtime function",
    );
    rejected(
        "comptime fn bit() -> .BitVec'builtin<1> where requires false { 1 } fn bad(value: .BitVec'builtin<1>) -> true'type { value matches 0b < bit() > }",
        "type requirement is not satisfied",
    );
    rejected(
        "fn bad(value: .BitVec'builtin<1>) -> true'type { value matches 0b < 1 > }",
        "requires a concrete BitVec value",
    );
    rejected(
        "comptime fn bit() -> .BitVec'builtin<1> { 1 } fn bad(value: .BitVec'builtin<1>) -> true'type { value match { 0b < bit() > => true, } }",
        "exhaustive",
    );
}

#[test]
fn normal_form_source_checks_arguments_body_and_opaque_identity() {
    rejected(
        "pure comptime fn Checked<T: .Type'builtin> -> .Type'builtin where requires T == true'type { T } typealias Bad = Checked<.u8'builtin>;",
        "type requirement is not satisfied",
    );
    accepted(
        "pure comptime fn Numeric<N: .Float'builtin> -> .Type'builtin { .u8'builtin } fn keep(value:Numeric<1.0000000000000000000000000000001>)->Numeric<10.000000000000000000000000000001e-1> {value}",
    );
    rejected(
        "pure comptime fn Numeric<N: .Float'builtin> -> .Type'builtin { .u8'builtin } fn bad(value:Numeric<1.0000000000000000000000000000001>)->Numeric<1.0000000000000000000000000000002> {value}",
        "unresolved type equality",
    );

    rejected(
        "pure comptime fn Identity<T: .Type'builtin> -> .Type'builtin { T } fn bad(value: Identity<.u8'builtin>) -> .u8'builtin { value }",
        "unresolved type equality",
    );
    rejected(
        "pure comptime fn A<T: .Type'builtin> -> .Type'builtin { T } pure comptime fn B<T: .Type'builtin> -> .Type'builtin { T } fn bad(value: A<.u8'builtin>) -> B<.u8'builtin> { value }",
        "unresolved type equality",
    );
    rejected(
        "pure comptime fn Identity<T: .Type'builtin> -> .Type'builtin { T } typealias Bad = Identity<.u8'builtin, .u16'builtin>;",
        "normal-form argument count mismatch",
    );
    rejected(
        "pure comptime fn Identity<T: .Type'builtin> -> .Type'builtin { T } comptime fn Bad() -> .Type'builtin { Identity(.u8'builtin) }",
        "normal-form heads cannot be invoked",
    );
    rejected(
        "pure comptime fn Bad<T: .Type'builtin> -> .Type'builtin { true }",
        "type mismatch",
    );
}

#[test]
fn normal_form_integer_arguments_reject_wrong_stage_and_preserve_opacity() {
    rejected(
        "pure comptime fn Bits<N: .Integer'builtin> -> .Type'builtin { .BitVec'builtin<N> } fn bad(value: Bits<8>) -> Bits<9> { value }",
        "unresolved type equality",
    );
    rejected(
        "pure comptime fn Bits<N: .Integer'builtin> -> .Type'builtin { .BitVec'builtin<N> } fn bad(value: Bits<8>) -> .BitVec'builtin<8> { value }",
        "unresolved type equality",
    );
    rejected(
        "pure comptime fn Bits<N: .Integer'builtin> -> .Type'builtin { .BitVec'builtin<N> } typealias Bad = Bits<true>;",
        "type mismatch",
    );
    rejected(
        "pure comptime fn Bits<N: .Integer'builtin> -> .Type'builtin { .BitVec'builtin<N> } fn bad(width: .u8'builtin) -> Bits<width> { while true {} }",
        "normal-form annotation cannot read an ordinary runtime input",
    );
    accepted(
        "pure comptime fn Bits<N: .Integer'builtin> -> .Type'builtin { .BitVec'builtin<N> } fn same(value: Bits<0x10>) -> Bits<16> { value }",
    );
}

#[test]
fn mixed_normal_form_inputs_reject_wrong_types_stages_and_ranges() {
    rejected(
        "pure comptime fn Select<B: true'type> -> .Type'builtin { .u8'builtin } fn bad(flag: true'type) -> Select<flag> { while true {} }",
        "normal-form annotation cannot read an ordinary runtime input",
    );

    rejected(
        "pure comptime fn Select<B: true'type> -> .Type'builtin { .u8'builtin } typealias Bad = Select<1>;",
        "requires a supported fixed-width integer target",
    );
    rejected(
        "pure comptime fn Select<B: true'type> -> .Type'builtin { .u8'builtin } fn bad(value: Select<true>) -> Select<false> { value }",
        "unresolved type equality",
    );
    rejected(
        "pure comptime fn Select<B: true'type> -> .Type'builtin { .u8'builtin } pure comptime fn Wrong<N: .Integer'builtin> -> .Type'builtin { Select<N> }",
        "normal-form value argument type mismatch",
    );
    rejected(
        "pure comptime fn Octet<N: .u8'builtin> -> .Type'builtin { .u8'builtin } typealias Bad = Octet<256>;",
        "does not fit the requested fixed-width representation",
    );
    rejected(
        "pure comptime fn Signed<N: .i8'builtin> -> .Type'builtin { .i8'builtin } typealias Bad = Signed<-129>;",
        "does not fit the requested fixed-width representation",
    );
}

#[test]
fn dependent_normal_form_inputs_cannot_skip_types_ranges_or_equality_proofs() {
    rejected(
        "pure comptime fn Bad<value: T, T> -> .Type'builtin { T }",
        "normal-form parameter type is not yet supported",
    );

    rejected(
        "pure comptime fn Tagged<T: .Type'builtin, value: T> -> .Type'builtin { T } typealias Bad = Tagged<.u8'builtin, 256>;",
        "does not fit",
    );
    rejected(
        "pure comptime fn Tagged<T: .Type'builtin, value: T> -> .Type'builtin { T } typealias Bad = Tagged<.u8'builtin, true>;",
        "type mismatch",
    );
    rejected(
        "pure comptime fn Tagged<T: .Type'builtin, value: T> -> .Type'builtin { T } pure comptime fn Wrong<A: .Type'builtin, value: A, B: .Type'builtin> -> .Type'builtin { Tagged<B, value> }",
        "normal-form value argument type mismatch",
    );
    rejected(
        "pure comptime fn Tagged<T: .Type'builtin, value: T> -> .Type'builtin { T } pure comptime fn Equal<A: .Type'builtin, value: A, B: .Type'builtin> -> .Type'builtin where requires A == B { Tagged<B, value> } typealias Bad = Equal<.u8'builtin, 1, .u16'builtin>;",
        "type requirement is not satisfied",
    );
}

#[test]
fn dynamic_normal_form_inputs_keep_phase_and_formation_contracts() {
    rejected(
        "pure comptime fn Bits<N: .Integer'builtin> -> .Type'builtin { .u8'builtin } fn Bad() -> .Type'builtin { Bits<8> }",
        "comptime-only value cannot inhabit runtime storage",
    );
    rejected(
        "pure comptime fn Checked<T: .Type'builtin> -> .Type'builtin where requires T == .u8'builtin { T } comptime fn Bad() -> .Type'builtin { Checked<.u16'builtin> }",
        "type requirement is not satisfied",
    );
    accepted(
        "pure comptime fn Checked<T: .Type'builtin> -> .Type'builtin where requires T == .u8'builtin { T } comptime fn Bad(ty: .Type'builtin) -> .Type'builtin { Checked<ty> }",
    );
    accepted(
        "pure comptime fn Tagged<T: .Type'builtin, value: T> -> .Type'builtin { T } comptime fn Bad(ty: .Type'builtin, value: .u8'builtin) -> .Type'builtin { Tagged<ty, value> }",
    );
    rejected(
        "pure comptime fn Select<B: true'type> -> .Type'builtin { .u8'builtin } fn flag() -> true'type { true } comptime fn Bad() -> .Type'builtin { Select<flag()> }",
        "runtime function is unavailable during comptime",
    );
}

#[test]
fn dynamic_normal_form_sparse_inputs_do_not_lose_dependency_checks() {
    rejected(
        "pure comptime fn Mixed<A: .Type'builtin, B: .Type'builtin, value: B> -> .Type'builtin { B } comptime fn Bad(first: .Type'builtin) -> .Type'builtin { Mixed<first, .u8'builtin, true> }",
        "type mismatch",
    );
    accepted(
        "pure comptime fn Mixed<A: .Type'builtin, B: .Type'builtin, value: A> -> .Type'builtin { B } comptime fn Bad(first: .Type'builtin) -> .Type'builtin { Mixed<first, .u8'builtin, 1> }",
    );
    accepted(
        "pure comptime fn Mixed<A: .Type'builtin, B: .Type'builtin, value: B> -> .Type'builtin { B } comptime fn Bad(last: .Type'builtin) -> .Type'builtin { Mixed<.u8'builtin, last, 1> }",
    );
    rejected(
        "pure comptime fn Mixed<A: .Type'builtin, B: .Type'builtin, value: B> -> .Type'builtin { B } comptime fn Bad(first: .Type'builtin) -> .Type'builtin { Mixed<first, .u8'builtin, 256> }",
        "does not fit",
    );
}

#[test]
fn nested_normal_form_constraints_do_not_unfold_or_ignore_dynamic_inputs() {
    rejected(
        "pure comptime fn Identity<T: .Type'builtin> -> .Type'builtin { T } pure comptime fn Byte<T: .Type'builtin> -> .Type'builtin where requires T == .u8'builtin { T } comptime fn Bad() -> .Type'builtin { Byte<Identity<.u8'builtin>> }",
        "type requirement",
    );
    rejected(
        "pure comptime fn Bits<N: .Integer'builtin> -> .Type'builtin { .u8'builtin } pure comptime fn Byte<T: .Type'builtin> -> .Type'builtin where requires T == Bits<8> { T } comptime fn Bad() -> .Type'builtin { Byte<Bits<9>> }",
        "type requirement",
    );
    accepted(
        "pure comptime fn Identity<T: .Type'builtin> -> .Type'builtin { T } pure comptime fn Byte<T: .Type'builtin> -> .Type'builtin where requires T == Identity<.u8'builtin> { T } comptime fn Bad(value: .Type'builtin) -> .Type'builtin { Byte<Identity<value>> }",
    );
}

#[test]
fn normal_form_value_contracts_reject_wrong_inputs_and_defer_unavailable_inputs() {
    let prefix = "pure comptime fn Bits<N: .Integer'builtin> -> .Type'builtin { .u8'builtin } pure comptime fn Checked<T: .Type'builtin, N: .Integer'builtin> -> .Type'builtin where requires T == Bits<N> { T }";
    rejected(
        &format!("{prefix} typealias Bad = Checked<Bits<8>, 9>;"),
        "type requirement",
    );
    rejected(
        &format!("{prefix} comptime fn Bad() -> .Type'builtin {{ Checked<Bits<8>, 9> }}"),
        "type requirement",
    );
    accepted(&format!(
        "{prefix} comptime fn Bad(width: .Integer'builtin) -> .Type'builtin {{ Checked<Bits<8>, width> }}"
    ));
    rejected(
        &format!(
            "{prefix} pure comptime fn Bad<N: .Integer'builtin> -> .Type'builtin {{ Checked<Bits<N>, 8> }}"
        ),
        "type requirement",
    );
    rejected(
        "pure comptime fn Independent<T: .Type'builtin, N: .Integer'builtin> -> .Type'builtin where requires T == .u8'builtin { T } comptime fn Bad(width: .Integer'builtin) -> .Type'builtin { Independent<.u16'builtin, width> }",
        "type requirement",
    );
}

#[test]
fn normal_form_constant_arithmetic_does_not_hide_false_constraints_or_run_calls() {
    let prefix = "pure comptime fn Bits<N: .Integer'builtin> -> .Type'builtin { .u8'builtin } pure comptime fn Checked<T: .Type'builtin, N: .Integer'builtin> -> .Type'builtin where requires T == Bits<N> { T }";
    rejected(
        &format!("{prefix} comptime fn Bad() -> .Type'builtin {{ Checked<Bits<2 * 4>, 3 + 6> }}"),
        "type requirement",
    );
    accepted(&format!(
        "{prefix} comptime fn width() -> .Integer'builtin {{ width() }} comptime fn Bad() -> .Type'builtin {{ Checked<Bits<8>, width()> }}"
    ));
    accepted(&format!(
        "{prefix} pure comptime fn Bad<N: .Integer'builtin> -> .Type'builtin {{ Checked<Bits<N>, N + 0> }}"
    ));
}

#[test]
fn residual_normal_form_contracts_fail_required_formation() {
    let prefix = include_str!("../../../development/examples/residual-normal-form-contracts.fl");
    rejected(
        &format!("{prefix} typealias Bad = Build(.u16'builtin);"),
        "TypeContractRejected",
    );
    rejected(
        &format!("{prefix} typealias Bad = Width(9);"),
        "TypeContract",
    );
    rejected(
        &format!("{prefix} typealias Bad = Selected(false);"),
        "TypeContractRejected",
    );
    accepted(&format!("{prefix} typealias Good = Selected(true);"));
}

#[test]
fn residual_normal_form_contracts_keep_independent_conjuncts_at_definition_time() {
    rejected(
        "pure comptime fn Bits<N: .Integer'builtin> -> .Type'builtin { .u8'builtin } pure comptime fn Checked<T: .Type'builtin, N: .Integer'builtin> -> .Type'builtin where requires (T == .u8'builtin and Bits<N> == Bits<8>) { T } comptime fn Bad(width: .Integer'builtin) -> .Type'builtin { Checked<.u16'builtin, width> }",
        "type requirement",
    );
    accepted(
        "pure comptime fn Bits<N: .Integer'builtin> -> .Type'builtin { .u8'builtin } pure comptime fn Checked<T: .Type'builtin, N: .Integer'builtin> -> .Type'builtin where requires (T == .u8'builtin and Bits<N> == Bits<8>) { T } comptime fn Build(width: .Integer'builtin) -> .Type'builtin { Checked<.u8'builtin, width> } typealias Good = Build(8);",
    );
}

#[test]
fn dependent_normal_form_inputs_reject_invalid_instantiations_and_ambiguous_expressions() {
    let prefix = include_str!("../../../development/examples/dependent-normal-form-inputs.fl");
    for input in [
        "Number(.i8'builtin)",
        "Negative(.u8'builtin)",
        "Number(true'type)",
    ] {
        rejected(
            &format!("{prefix} typealias Bad = {input};"),
            "required comptime evaluation failed",
        );
    }
    rejected(
        &format!("{prefix} typealias Bad = Pass(.u16'builtin, 8);"),
        "TypeContractRejected",
    );
    rejected(
        &format!("{prefix} typealias Bad = Checked(.u16'builtin);"),
        "TypeContractRejected",
    );
    accepted(&format!(
        "{prefix} comptime fn Conditional(ty: Type) -> Type {{ Tagged<ty, if true {{ 1 }} else {{ 2 }}> }}"
    ));
}

#[test]
fn contextual_normal_form_branches_retain_ordinary_type_and_phase_checks() {
    let prefix = include_str!("../../../development/examples/contextual-normal-form-inputs.fl");
    rejected(
        &format!("{prefix} typealias Bad = UnselectedOverflow(.u8'builtin);"),
        "required comptime evaluation failed",
    );
    rejected(
        &format!(
            "{prefix} fn runtime() -> .u8'builtin {{ 1 }} comptime fn Bad(ty: Type) -> Type {{ Tagged<ty, if true {{ 1 }} else {{ runtime() }}> }}"
        ),
        "runtime function is unavailable during comptime",
    );
    rejected(
        &format!(
            "{prefix} comptime fn Bad(ty: Type) -> Type {{ Tagged<ty, if true {{ 1 }} else {{ missing }}> }}"
        ),
        "missing",
    );
    rejected(
        &format!(
            "{prefix} comptime fn Bad(ty: Type) -> Type {{ Tagged<ty, do {{ let hidden: .u8'builtin = 1; hidden }}>; Tagged<ty, hidden> }}"
        ),
        "hidden",
    );
    accepted(&format!(
        "{prefix} typealias Good = Early(.u8'builtin, true);"
    ));
}

#[test]
fn contextual_normal_form_match_keeps_coverage_scope_and_phase_checks() {
    let prefix = include_str!("../../../development/examples/contextual-normal-form-match.fl");
    rejected(
        &format!(
            "{prefix} comptime fn Bad(ty: Type, flag: true'type) -> Type {{ Tagged<ty, flag match {{ true => 1 }}> }}"
        ),
        "non-exhaustive match",
    );
    rejected(
        &format!(
            "{prefix} comptime fn Bad(ty: Type, flag: true'type) -> Type {{ Tagged<ty, flag match {{ value if true => 1 }}> }}"
        ),
        "non-exhaustive match",
    );
    rejected(
        &format!(
            "{prefix} comptime fn Bad(ty: Type, flag: true'type) -> Type {{ Tagged<ty, flag match {{ value if false => 1, _ => value }}> }}"
        ),
        "value",
    );
    rejected(
        &format!(
            "{prefix} fn runtime(flag: true'type) -> true'type {{ flag }} comptime fn Bad(ty: Type, flag: true'type) -> Type {{ Tagged<ty, flag match {{ value and runtime(value) is true => 1, _ => 2 }}> }}"
        ),
        "runtime function is unavailable during comptime",
    );
    rejected(
        &format!("{prefix} typealias Bad = UnselectedOverflow(.u8'builtin);"),
        "required comptime evaluation failed",
    );
}

#[test]
fn contextual_normal_form_when_keeps_fallback_scope_and_phase_rules() {
    let prefix = include_str!("../../../development/examples/contextual-normal-form-when.fl");
    rejected(
        &format!(
            "{prefix} comptime fn Bad(ty: Type) -> Type {{ Tagged<ty, when {{ else => 1, true => 2 }}> }}"
        ),
        "fallback must be the final arm",
    );
    rejected(
        &format!(
            "{prefix} comptime fn Bad(ty: Type) -> Type {{ Tagged<ty, when {{ 'x' => 1, else => 2 }}> }}"
        ),
        "type mismatch",
    );
    rejected(
        &format!(
            "{prefix} comptime fn Bad(ty: Type) -> Type {{ Tagged<ty, when {{ true => do {{ let hidden: .u8'builtin = 1; hidden }}, else => hidden }}> }}"
        ),
        "hidden",
    );
    rejected(
        &format!(
            "{prefix} fn runtime() -> true'type {{ true }} comptime fn Bad(ty: Type) -> Type {{ Tagged<ty, when {{ true => 1, runtime() => 2, else => 3 }}> }}"
        ),
        "runtime function is unavailable during comptime",
    );
    rejected(
        &format!("{prefix} typealias Bad = UnselectedOverflow(.u8'builtin);"),
        "required comptime evaluation failed",
    );
    rejected(
        &format!(
            "{prefix} comptime fn Empty(ty: Type) -> Type {{ Tagged<ty, when {{}}> }} typealias Bad = Empty(.u8'builtin);"
        ),
        "TypeContractRejected",
    );
}

#[test]
fn normal_form_text_inputs_reject_wrong_kinds_and_different_names() {
    let prefix = include_str!("../../../development/examples/normal-form-text.fl");
    rejected(
        &format!("{prefix} typealias Bad = Named<'x'>;"),
        "type mismatch",
    );
    rejected(
        &format!("{prefix} typealias Bad = Symbol<\"x\">;"),
        "type mismatch",
    );
    rejected(
        &format!("{prefix} fn bad(value: Named<\"left\">) -> Named<\"right\"> {{ value }}"),
        "unresolved type equality",
    );
    rejected(
        &format!("{prefix} typealias Bad = Checked<Named<\"left\">, \"right\">;"),
        "type requirement",
    );
}

#[test]
fn normal_form_tuple_values_reject_wrong_fields_shapes_and_contracts() {
    let prefix = include_str!("../../../development/examples/normal-form-tuples.fl");
    rejected(
        &format!("{prefix} typealias Bad = Config<(true, 1)>;"),
        "type mismatch",
    );
    rejected(
        &format!("{prefix} typealias Bad = Checked<Config<(8, true)>, (9, true)>;"),
        "type requirement",
    );
    rejected(
        &format!("{prefix} typealias Bad = Typed((.u8'builtin, .u8'builtin));"),
        "TypeContractRejected",
    );
    rejected(
        &format!("{prefix} typealias Bad = Typed((.i8'builtin, Flag));"),
        "required comptime evaluation failed",
    );
    rejected(
        &format!("{prefix} typealias Bad = Partial(.i8'builtin);"),
        "required comptime evaluation failed",
    );
    rejected(
        &format!("{prefix} typealias Bad = Typed((.u8'builtin, Flag, Flag));"),
        "Arity",
    );
}

#[test]
fn normal_form_bitvec_inputs_reject_width_range_and_wrong_contracts() {
    let prefix = include_str!("../../../development/examples/normal-form-bitvec.fl");
    rejected(
        &format!("{prefix} typealias Bad = Checked<Mask<1>, 2>;"),
        "type requirement",
    );
    rejected(
        &format!("{prefix} typealias Bad = Typed(.BitVec'builtin<7>);"),
        "required comptime evaluation failed",
    );
    rejected(
        &format!("{prefix} typealias Bad = Mask<256>;"),
        "does not fit the BitVec width",
    );
}

#[test]
fn symbolic_tuple_normal_inputs_preserve_definition_binders() {
    let prefix = include_str!("../../../development/examples/normal-form-symbolic-tuples.fl");
    accepted(prefix);
    rejected(
        &format!(
            "{prefix} pure comptime fn Bad<N: Int> -> Type where requires Pair<(N, true)> == Pair<(N, true)> {{ RequirePair<Pair<(N, true)>, (N, false)> }}"
        ),
        "type requirement",
    );
    rejected(
        &format!("{prefix} fn bad(value: Pair<(1, true, false)>) {{}}"),
        "arity mismatch",
    );
}

#[test]
fn normal_form_unit_values_reject_non_unit_inputs() {
    let prefix = include_str!("../../../development/examples/normal-form-unit.fl");
    rejected(
        &format!("{prefix} typealias Bad = Token<false>;"),
        "type mismatch",
    );
    rejected(
        &format!("{prefix} typealias Bad = Typed(.u8'builtin);"),
        "TypeContractRejected",
    );
}

#[test]
fn contextual_type_tuples_reject_value_fields_and_check_unselected_arms() {
    let prefix = include_str!("../../../development/examples/contextual-type-tuples.fl");
    rejected(
        &format!(
            "{prefix} comptime fn Bad(ty: Type) -> Type {{ Tagged<ty, (.u8'builtin, true)> }} typealias Wrong = Bad(Type);"
        ),
        "TypeContractRejected",
    );
    rejected(
        &format!(
            "{prefix} comptime fn Bad(ty: Type) -> Type {{ Tagged<ty, if true {{ (.u8'builtin, ()) }} else {{ (.u8'builtin, true) }}> }} typealias Wrong = Bad(Type);"
        ),
        "TypeContractRejected",
    );
    rejected(
        &format!(
            "{prefix} comptime fn Bad(ty: Type) -> Type {{ Tagged<ty, do {{}}> }} typealias Wrong = Bad(Type);"
        ),
        "TypeContractRejected",
    );
}

#[test]
fn type_facts_propagate_through_tuple_valued_normal_inputs() {
    let prefix = include_str!("../../../development/examples/normal-form-tuple-congruence.fl");
    accepted(prefix);
    rejected(
        &format!(
            "{prefix} pure comptime fn Bad<A: Type, B: Type> -> Type where requires Pair<(A, true)> == Pair<(B, true)> {{ Same<A, B> }}"
        ),
        "type requirement",
    );
}

#[test]
fn requires_type_operands_preserve_nested_formation_diagnostics() {
    let prefix = "typealias Type = .Type'builtin; typealias Int = .Integer'builtin; pure comptime fn Pair<v: (Int, true'type)> -> Type { .u8'builtin } ";
    let invalid_field =
        format!("{prefix} fn bad() where requires Pair<(false, true)> == .u8'builtin {{}}");
    let start = invalid_field.find("false").unwrap();
    rejected(
        &invalid_field,
        &format!(
            "{start}..{}: requires type operand could not be elaborated: type mismatch",
            start + 5
        ),
    );
    for operand in ["Pair<(1, true, false)>", "Pair<(1,)>"] {
        rejected(
            &format!("{prefix} fn bad() where requires {operand} == .u8'builtin {{}}"),
            "normal-form tuple argument arity mismatch",
        );
        rejected(
            &format!("{prefix} fn bad() where requires .u8'builtin == {operand} {{}}"),
            "normal-form tuple argument arity mismatch",
        );
    }
    rejected(
        &format!("{prefix} fn bad() where requires Pair<(false, true)> == .u8'builtin {{}}"),
        "type mismatch",
    );
    rejected(
        "typealias Type = .Type'builtin; pure comptime fn Byte<T: Type> -> Type where requires T == .u8'builtin { T } fn bad() where requires Byte<.u16'builtin> == .u8'builtin {}",
        "type requirement is not satisfied",
    );
    rejected(
        "typealias Type = .Type'builtin; typealias Int = .Integer'builtin; comptime fn Loop() -> Type { Loop() } fn bad() where requires Loop() == .u8'builtin {}",
        "DepthExhausted",
    );
}

#[test]
fn required_contract_computation_distinguishes_calls_from_dependency_cycles() {
    accepted(include_str!(
        "../../../development/examples/required-contract-computation.fl"
    ));
    rejected(
        "typealias Type = .Type'builtin; comptime fn Loop() -> Type { Loop() } fn bad() where requires Loop() == .u8'builtin {}",
        "DepthExhausted",
    );
    rejected(
        "typealias Type = .Type'builtin; comptime fn A() -> Type where requires B() == .u8'builtin { .u8'builtin } comptime fn B() -> Type where requires A() == .u8'builtin { .u8'builtin }",
        "cyclic or excessively deep declaration contract dependency",
    );
    rejected(
        "typealias Type = .Type'builtin; comptime fn Restricted() -> Type where requires false { .u8'builtin } fn bad() where requires Restricted() == .u8'builtin {}",
        "type requirement is not satisfied",
    );
}

#[test]
fn constant_bit_operations_check_normal_form_contracts_before_invocation() {
    let prefix = include_str!("../../../development/examples/normal-form-bit-operations.fl");
    accepted(prefix);
    for expression in [
        "not 0xAF",
        "(0xA0 or 0x0F) and 0xF0",
        "do { let high: .BitVec'builtin<4> = 10; let low: .BitVec'builtin<4> = 15; (high ++ low)[0..8] }",
        "do { let low: .BitVec'builtin<4> = 15; low.as(Bits) }",
    ] {
        rejected(
            &format!("{prefix} comptime fn Bad() -> Type {{ Checked<Mask<{expression}>, 0> }}"),
            "type requirement",
        );
    }
    rejected(
        &format!(
            "{prefix} comptime fn Bad() -> Type {{ CheckedFlag<Flag<(do {{ let byte: .u8'builtin = 10; byte'bitvec == byte'bitvec }})>, false> }}"
        ),
        "type requirement",
    );
    rejected(
        &format!(
            "{prefix} comptime fn Bad() -> Type {{ CheckedFlag<Flag<not false and true>, false> }}"
        ),
        "type requirement",
    );
}

#[test]
fn module_constants_check_dependencies_phase_and_type_before_publication() {
    accepted(
        "typealias Int = .Integer'builtin; const FIRST: Int = SECOND + 1; const SECOND = 8; typealias Bits = .BitVec'builtin<FIRST>; comptime fn read() -> Int { FIRST }",
    );
    rejected(
        "const FIRST = SECOND; const SECOND = FIRST;",
        "constant dependency",
    );
    rejected("const FIRST = FIRST;", "constant dependency");
    rejected("const VALUE: .u8'builtin = true;", "type mismatch");
    rejected(
        "const VALUE = 8; typealias Bad = VALUE;",
        "does not contain a Type value",
    );
    rejected(
        "const VALUE = runtime(); fn runtime() -> .u8'builtin { 8 }",
        "cannot execute a runtime function",
    );
    rejected(
        "const VALUE = 8; fn runtime() -> .Integer'builtin { VALUE }",
        "cross-stage Splice",
    );
    rejected(
        "const VALUE = build(); comptime fn build() -> .u8'builtin where requires false { 8 }",
        "type requirement is not satisfied",
    );
    rejected("const VALUE = 8; const VALUE = 9;", "Duplicate definition");
}

#[test]
fn aggregate_constants_prepare_forward_descriptors_and_enforce_construction() {
    accepted(
        "const VALUE = Box { value: 8 }; struct Box { value: .Integer'builtin } comptime fn read() -> .Integer'builtin { VALUE.value } typealias Bits = .BitVec'builtin<read()>;",
    );
    accepted(
        "const VALUE: Entry<.Integer'builtin> = .value(8); enum Entry where T { empty, value(T) } comptime fn read() -> .Integer'builtin { VALUE match { .empty => 0, .value(n) => n } } typealias Bits = .BitVec'builtin<read()>;",
    );
    rejected(
        "struct Box where T, requires T == true'type { value: T } const VALUE = Box<.u8'builtin> { value: 8 };",
        "type requirement",
    );
    rejected(
        "struct Box { value: .u8'builtin } const VALUE = Box { value: true };",
        "type mismatch",
    );
    rejected(
        "const WIDTH = read(); struct Box { value: .BitVec'builtin<WIDTH> } comptime fn read() -> .Integer'builtin { let value = Box { value: 0 }; 8 }",
        "dependency",
    );
    rejected(
        "struct Box { value: .u8'builtin } const VALUE = Box { value: 8 }; fn read() -> Box { VALUE }",
        "cross-stage Splice",
    );
}

#[test]
fn function_constants_preserve_comptime_phase_types() {
    accepted(
        "comptime fn identity(value: T) -> T where T { value } const VALUE = identity<.u8'builtin>;",
    );
    accepted(
        "comptime fn identity(value: .u8'builtin) -> .u8'builtin { value } const VALUE = identity;",
    );
    rejected(
        "const VALUE = 8; comptime fn run() -> .Integer'builtin { VALUE() }",
        "called expression is not a function",
    );
}

#[test]
fn comptime_callable_types_cannot_erase_or_forge_execution_phases() {
    for source in [
        "fn run(callback: comptime fn() -> true'type) -> true'type { callback() }",
        "struct Box { callback: comptime fn() -> true'type } fn keep(value: Box) -> Box { value }",
        "comptime fn value() -> true'type { true } fn run() { let callback = value; }",
    ] {
        rejected(source, "comptime-only value cannot inhabit runtime storage");
    }
    for source in [
        "fn value() -> true'type { true } comptime fn run() { let callback: comptime fn() -> true'type = value; }",
        "comptime fn value() -> true'type { true } comptime fn run() { let callback: fn() -> true'type = value; }",
    ] {
        rejected(source, "type mismatch");
    }
    rejected(
        "comptime fn run(callback: fn() -> true'type) -> true'type { callback() }",
        "callable type is unavailable in this execution phase",
    );
    rejected(
        "fn identity(value: T) -> T where T { value } comptime fn apply(callback: comptime fn(T) -> T, value: T) -> T where T { callback(value) } comptime fn run() -> .Integer'builtin { apply(identity, 8) }",
        "type mismatch",
    );
}

#[test]
fn comptime_closure_environments_cannot_cross_phase_or_disappear() {
    accepted(
        "comptime fn run() { var value: .Integer'builtin = 0; let next = || { value = value + 1; value }; }",
    );

    for source in [
        "fn run() { let callback: comptime fn() -> true'type = || true; }",
        "comptime fn run() { let callback: fn() -> true'type = || true; }",
    ] {
        rejected(source, "lambda target requires a different execution phase");
    }
    rejected(
        "fn runtime() -> true'type { true } comptime fn run() { let callback = || runtime(); }",
        "runtime function is unavailable during comptime",
    );
    rejected(
        "comptime fn run(flag: true'type) { let callback: comptime fn() -> true'type = || flag; }",
        "type mismatch",
    );
    rejected(
        "const CALLBACK = || true; fn run() { let callback = CALLBACK; }",
        "cross-stage Splice",
    );
}

#[test]
fn explicit_closure_contexts_preserve_empty_elimination_and_ordered_type_identity() {
    accepted(
        r#"
        typealias Bool = true'type;
        typealias Char = 'a''type;
        fn proof() where requires
            %{} Bool == Bool and
            %{first: Bool} fn() == %{renamed: Bool} fn() and
            not (%{first: Bool, second: Char} fn() == %{first: Char, second: Bool} fn())
        {}
    "#,
    );
    for source in [
        "fn bad(flag: true'type) -> %{} fn() -> true'type { || flag }",
        "fn bad() -> %{flag: true'type} fn() -> true'type { || true }",
        "fn bad(a: true'type, b: 'a''type) -> %{b: 'a''type, a: true'type} fn() -> (true'type, 'a''type) { || (a, b) }",
    ] {
        rejected(source, "type mismatch");
    }
    rejected(
        "typealias Bad = %{a: true'type, a: true'type} fn();",
        "duplicate closure environment binding",
    );
    rejected(
        "typealias Bad = %{a: true'type} %{b: true'type} fn();",
        "repeated nonempty closure qualification",
    );
    rejected(
        "typealias Bad = %{a: true'type} true'type;",
        "non-callable closure-qualified bodies",
    );
    rejected(
        "fn bad(flag: true'type) -> %{flag: true'type = flag} fn() -> true'type { || flag }",
        "dependency/provenance checking",
    );
    rejected(
        "fn bad(callback: %C fn()) where C {}",
        "open closure contexts require",
    );
    rejected(
        "fn bad() -> %{hidden: true'type} fn() -> true'type { || hidden }",
        "Unresolved identifier",
    );
}

#[test]
fn generic_aliases_check_arity_scope_cycles_and_target_types() {
    for source in [
        "typealias Pair<T> = (T, T); fn bad(value: Pair) {}",
        "typealias Pair<T> = (T, T); fn bad(value: Pair<>) {}",
        "typealias Pair<T> = (T, T); fn bad(value: Pair<true'type, 'a''type>) {}",
    ] {
        rejected(source, "type alias");
    }
    rejected("typealias Bad<T, T> = T;", "duplicate clause parameter");
    rejected(
        "typealias Identity<T> = T; fn bad(value: T) {}",
        "Unresolved identifier",
    );
    rejected("typealias Bad<T> = true;", "type mismatch");
    rejected("typealias Bad<T> = Missing;", "Unresolved identifier");
    rejected(
        "typealias A<T> = B<T>; typealias B<U> = A<U>;",
        "cyclic type alias",
    );
    rejected("typealias Loop<T> = Loop<(T, T)>;", "cyclic type alias");
    rejected(
        "typealias Sized<N: .Integer'builtin> = true'type;",
        "clause kind is not implemented",
    );
    rejected(
        "typealias Type = .Type'builtin; pure comptime fn Identity<T> -> Type { T } typealias Alias<T> = Identity<T>; fn bad(value: Alias<true'type>) -> true'type { value }",
        "unresolved type equality",
    );
}

#[test]
fn generic_alias_bounds_are_checked_even_when_an_expansion_is_cached() {
    accepted(
        "trait Marker {} impl Marker for true'type {} typealias Checked<T :- Marker> = T; fn keep(value: Checked<true'type>) -> true'type { value }",
    );
    rejected(
        "trait Marker {} impl Marker for true'type {} typealias Checked<T :- Marker> = T; fn good(value: Checked<true'type>) {} fn bad(value: Checked<'a''type>) {}",
        "trait",
    );
    rejected(
        "trait Marker {} typealias Checked<T :- Marker> = T; fn generic(value: Checked<T>) -> T where T { value }",
        "trait",
    );
}

#[test]
fn generic_alias_targets_forward_the_callers_bound_evidence_without_skipping_definition_checks() {
    accepted(
        "trait Marker {} struct Restricted where T :- Marker { value: T } typealias Wrapped<T :- Marker> = Restricted<T>; fn keep(value: Wrapped<T>) -> Restricted<T> where T :- Marker { value }",
    );
    rejected(
        "trait Marker {} struct Restricted where T :- Marker { value: T } typealias Wrapped<T> = Restricted<T>;",
        "trait bound is not established",
    );
    rejected(
        "trait Marker {} struct Restricted where T :- Marker { value: T } typealias Wrapped<T :- Marker> = Restricted<T>; fn keep(value: Wrapped<T>) where T {}",
        "trait bound is not established",
    );
}

#[test]
fn generic_alias_expansion_is_bounded_and_recovers_for_a_fresh_package() {
    let mut source = String::from("typealias Level0<T> = (T, T);\n");
    for level in 1..25 {
        source.push_str(&format!(
            "typealias Level{level}<T> = Level{}<Level{}<T>>;\n",
            level - 1,
            level - 1
        ));
    }
    source.push_str("fn bad(value: Level24<true'type>) {}\n");
    rejected(&source, "type alias expansion budget exhausted");
    accepted(
        "typealias Pair<T> = (T, T); typealias Four<T> = Pair<Pair<T>>; fn keep(value: Four<true'type>) -> ((true'type, true'type), (true'type, true'type)) { value }",
    );
}

#[test]
fn residual_aliases_preserve_bounds_and_reject_computation_cycles() {
    accepted(
        "typealias Type = .Type'builtin; trait Marker {} impl Marker for true'type {} comptime fn IdentityType(value: Type) -> Type { value } typealias Checked<T :- Marker> = IdentityType(T); typealias Wrapped<T :- Marker> = Checked<T>; fn keep(value: Wrapped<true'type>) -> true'type { value }",
    );
    rejected(
        "typealias Type = .Type'builtin; trait Marker {} comptime fn IdentityType(value: Type) -> Type { value } typealias Checked<T :- Marker> = IdentityType(T); typealias Wrapped<T> = Checked<T>;",
        "trait bound is not established",
    );
    rejected(
        "typealias Type = .Type'builtin; comptime fn IdentityType(value: Type) -> Type { value } typealias A<T> = IdentityType(B<T>); typealias B<T> = A<T>;",
        "cyclic type alias",
    );
}

#[test]
fn computed_tuple_type_fields_reject_values_and_runtime_calls() {
    rejected(
        "typealias Type = .Type'builtin; comptime fn Bad(value: Type) -> Type { (value, true) }",
        "type mismatch",
    );
    rejected(
        "typealias Type = .Type'builtin; fn RuntimeType() -> Type { true'type } comptime fn PairType() -> Type { (RuntimeType(), true'type) } typealias Pair = PairType();",
        "runtime",
    );
}

#[test]
fn computed_function_type_fields_preserve_type_and_phase_rejections() {
    rejected(
        "typealias Type = .Type'builtin; comptime fn Bad(value: Type) -> Type { (fn(value) -> true) }",
        "type mismatch",
    );
    rejected(
        "typealias Type = .Type'builtin; fn RuntimeType() -> Type { true'type } comptime fn Bad() -> Type { (fn(RuntimeType()) -> true'type) } typealias Callback = Bad();",
        "runtime",
    );
    rejected(
        "typealias Type = .Type'builtin; fn RuntimeType() -> Type { true'type } comptime fn Bad() -> Type { (fn(true'type) -> RuntimeType()) } typealias Callback = Bad();",
        "runtime",
    );
    rejected(
        "typealias Type = .Type'builtin; typealias Bool = true'type; comptime fn CallbackType(value: Type) -> Type { (comptime fn(value) -> value) } typealias Callback = CallbackType(Bool); fn bad(callback: Callback) -> Bool { callback(true) }",
        "comptime",
    );
}

#[test]
fn computed_environment_types_keep_phase_and_context_restrictions() {
    for result in [
        "%{saved: RuntimeType()} fn()",
        "%{saved: true'type} RuntimeType()",
        "*RuntimeType()",
    ] {
        rejected(
            &format!(
                "typealias Type = .Type'builtin; fn RuntimeType() -> Type {{ true'type }} comptime fn Bad() -> Type {{ {result} }} typealias Invalid = Bad();"
            ),
            "runtime",
        );
    }
    rejected(
        "typealias Type = .Type'builtin; comptime fn Bad(value: Type) -> Type { %{a: value, a: value} fn() }",
        "duplicate closure environment binding",
    );
    rejected(
        "typealias Type = .Type'builtin; comptime fn Bad(value: Type) -> Type { %{a: value = true} fn() }",
        "provenance",
    );
    rejected(
        "typealias Type = .Type'builtin; comptime fn Wrap(value: Type) -> Type { %{saved: true'type} value } typealias Bad = Wrap(true'type);",
        "required comptime evaluation failed",
    );
    rejected(
        "typealias Type = .Type'builtin; comptime fn Wrap(value: Type) -> Type { %{saved: true'type} value } typealias Once = %{saved: true'type} fn(); typealias Bad = Wrap(Once);",
        "required comptime evaluation failed",
    );
}

#[test]
fn computed_nominal_arguments_do_not_escape_formation_contracts() {
    let header = "typealias Type = .Type'builtin; typealias Bool = true'type;";
    let declaration = "struct Checked where T, requires T == Bool { value: T }";
    let factory = "comptime fn CheckedType(value: Type) -> Type { Checked<value> }";
    for source in [
        format!("{header}{declaration}{factory}typealias Bad = CheckedType('z''type);"),
        format!("{header}typealias Bad = CheckedType('z''type);{factory}{declaration}"),
    ] {
        rejected(&source, "TypeContractRejected");
    }
    rejected(
        "typealias Type = .Type'builtin; struct Disabled where T, requires false {} comptime fn DisabledType(value: Type) -> Type { Disabled<value> }",
        "type requirement is not satisfied",
    );
    accepted(
        "typealias Type = .Type'builtin; trait Marker {} impl Marker for bool {} struct Checked where T :- Marker {} comptime fn CheckedType(value: Type) -> Type { Checked<value> } typealias Valid = CheckedType(bool);",
    );
    rejected(
        "typealias Type = .Type'builtin; trait Marker {} struct Checked where T :- Marker {} comptime fn CheckedType(value: Type) -> Type { Checked<value> } typealias Invalid = CheckedType(bool);",
        "TypeContractUnresolved",
    );
    rejected(
        "typealias Type = .Type'builtin; struct Box where T { value: T } fn RuntimeType() -> Type { true'type } comptime fn Bad() -> Type { Box<RuntimeType()> } typealias Invalid = Bad();",
        "runtime",
    );
}

#[test]
fn dynamic_alias_arguments_keep_bounds_and_phase_checks() {
    accepted(
        "typealias Type = .Type'builtin; trait Marker {} impl Marker for bool {} typealias Checked<T :- Marker> = T; comptime fn CheckedType(value: Type) -> Type { Checked<value> } typealias Valid = CheckedType(bool);",
    );
    rejected(
        "typealias Type = .Type'builtin; trait Marker {} typealias Checked<T :- Marker> = T; comptime fn CheckedType(value: Type) -> Type { Checked<value> } typealias Invalid = CheckedType(bool);",
        "TypeContractUnresolved",
    );
    rejected(
        "typealias Type = .Type'builtin; typealias Pair<T> = (T, T); fn RuntimeType() -> Type { true'type } comptime fn Bad() -> Type { Pair<RuntimeType()> } typealias Invalid = Bad();",
        "runtime",
    );
    rejected(
        "typealias Type = .Type'builtin; typealias Pair<T> = (T, T); comptime fn Bad() -> Type { Pair<true> }",
        "type mismatch",
    );
}

#[test]
fn dynamic_alias_expression_expansion_has_a_work_limit() {
    let mut source = String::from(
        "typealias Type = .Type'builtin; comptime fn IdentityType(value: Type) -> Type { value } typealias Level0<T> = IdentityType(T);\n",
    );
    for level in 1..18 {
        source.push_str(&format!(
            "typealias Level{level}<T> = (Level{}<T>, Level{}<T>);\n",
            level - 1,
            level - 1
        ));
    }
    source.push_str("comptime fn Explode(value: Type) -> Type { Level17<value> }");
    rejected(&source, "type alias expression expansion budget exhausted");
}

#[test]
fn type_value_equality_rejects_value_operands_and_opaque_required_decisions() {
    rejected(
        "comptime fn bad() -> true'type { true'type == true }",
        "type mismatch",
    );
    rejected(
        "typealias Type = .Type'builtin; typealias Bool = true'type; pure comptime fn Opaque<T> -> Type { T } comptime fn Choose() -> Type { if Opaque<Bool> == Bool { Bool } else { 'z''type } } typealias Result = Choose();",
        "TypeContractUnresolved",
    );
}

#[test]
fn type_reference_patterns_require_outer_bindings_type_operands_and_fallbacks() {
    rejected(
        "typealias Type = .Type'builtin; comptime fn bad(value: Type) -> true'type { value match { 'Missing => true, _ => false } }",
        "Unresolved identifier",
    );
    rejected(
        "typealias Type = .Type'builtin; typealias Bool = true'type; comptime fn bad(value: Type) -> Bool { value match { 'Bool => true } }",
        "non-exhaustive match",
    );
    rejected(
        "typealias Type = .Type'builtin; comptime fn bad(value: Type) -> true'type { value match { < true > => true, _ => false } }",
        "type mismatch",
    );
    rejected(
        "typealias Type = .Type'builtin; comptime fn bad(value: Type) -> true'type { value match { 'value => true, _ => Missing } }",
        "Unresolved identifier",
    );
}

#[test]
fn tuple_type_patterns_do_not_claim_exhaustiveness_or_export_arm_bindings() {
    rejected(
        "typealias Type = .Type'builtin; comptime fn Bad(value: Type) -> Type { value match { (A, B) => A } }",
        "non-exhaustive match",
    );
    rejected(
        "typealias Type = .Type'builtin; comptime fn Bad(value: Type) -> Type { value match { (A, B) => A, _ => B } }",
        "Unresolved identifier",
    );
    rejected(
        "typealias Type = .Type'builtin; comptime fn Bad(value: Type) -> Type { value match { (A, A) => A, _ => value } }",
        "duplicate pattern binding",
    );
}

#[test]
fn normal_form_patterns_reject_wrong_heads_arguments_and_missing_coverage() {
    let prefix = "typealias Type = .Type'builtin; typealias Int = .Integer'builtin; pure comptime fn Array<T, N: Int> -> Type { T } ";
    for (pattern, diagnostic) in [
        ("Array<T>", "argument count mismatch"),
        (
            "Array<T: A, T: B>",
            "duplicate normal-form pattern argument",
        ),
        (
            "Array<Missing: A, N: B>",
            "unknown normal-form pattern argument",
        ),
        ("Array<A, A>", "duplicate pattern binding"),
    ] {
        rejected(
            &format!(
                "{prefix}comptime fn Bad(value: Type) -> Type {{ value match {{ {pattern} => value, _ => value }} }}"
            ),
            diagnostic,
        );
    }
    rejected(
        &format!(
            "{prefix}comptime fn Bad(value: Type) -> Type {{ value match {{ Array<A, _> => A }} }}"
        ),
        "non-exhaustive match",
    );
    rejected(
        "typealias Type = .Type'builtin; comptime fn IdentityType(value: Type) -> Type { value } comptime fn Bad(value: Type) -> Type { value match { IdentityType<T> => T, _ => value } }",
        "pattern head is not a normal-form definition",
    );
    rejected(
        "typealias Type = .Type'builtin; pure comptime fn Box<T, value: T> -> Type { T } comptime fn Bad(value: Type) -> Type { value match { Box<T, item> => T, _ => value } }",
        "dependent normal-form value patterns",
    );
}

#[test]
fn normal_form_dependent_pattern_types_are_checked_per_candidate() {
    let prefix = "typealias Type = .Type'builtin; typealias Bool = true'type; typealias Int = .Integer'builtin; pure comptime fn Box<T, value: T> -> Type { T } ";
    rejected(
        &format!(
            "{prefix}comptime fn Bad(value: Type) -> Bool {{ value match {{ Box<(_ if Bool == Bool), item> => item, _ => false }} }}"
        ),
        "dependent normal-form value patterns",
    );
    rejected(
        &format!(
            "{prefix}comptime fn Bad(value: Type) -> Bool {{ value match {{ Box<'Int, item> => item, _ => false }} }}"
        ),
        "type mismatch",
    );
    rejected(
        &format!(
            "{prefix}comptime fn Bad(value: Type) -> Bool {{ value match {{ Box<'Bool, item> or Box<'Int, item> => item, _ => false }} }}"
        ),
        "type mismatch",
    );
    rejected(
        &format!(
            "{prefix}comptime fn Bad(value: Type) -> Bool {{ value match {{ Box<'Bool, true> => true, Box<T, item> => false, _ => false }} }}"
        ),
        "dependent normal-form value patterns",
    );
}

#[test]
fn computed_value_patterns_require_compatible_equality_and_coverage() {
    rejected(
        "comptime fn bad(x: true'type, expected: .Integer'builtin) -> true'type { x match { 'expected => true, _ => false } }",
        "type mismatch",
    );
    rejected(
        "comptime fn bad(x: true'type, expected: true'type) -> true'type { x match { 'expected => true } }",
        "non-exhaustive match",
    );
    rejected(
        "comptime fn bad(x: (true'type, true'type), expected: (true'type, true'type)) -> true'type { x match { 'expected => true, _ => false } }",
        "equality protocol for this type is not implemented",
    );
    rejected(
        "comptime fn bad(x: .u8'builtin, y: .u16'builtin) -> true'type { x == y }",
        "type mismatch",
    );
}

#[test]
fn range_patterns_check_endpoint_types_and_do_not_assume_coverage() {
    rejected(
        "comptime fn bad(x: .u8'builtin) -> true'type { x match { 0..=256 => true, _ => false } }",
        "integer does not fit",
    );
    rejected(
        "comptime fn bad(x: .Integer'builtin) -> true'type { x match { 0..10 => true } }",
        "non-exhaustive match",
    );
    rejected(
        "comptime fn bad(x: .Integer'builtin) -> true'type { x match { 'a'..='z' => true, _ => false } }",
        "type mismatch",
    );
    rejected(
        "comptime fn bad(x: true'type) -> true'type { x match { false..=true => true, _ => false } }",
        "range pattern ordering protocol",
    );
}

#[test]
fn interval_coverage_rejects_gaps_dynamic_bounds_and_uncorrelated_products() {
    let prefix = "typealias Byte = .u8'builtin; typealias Bool = true'type; ";
    for body in [
        "comptime fn bad(x: Byte) -> Bool { x match { 0..127 => true, 128..=255 => false } }",
        "comptime fn bad(x: Byte) -> Bool { x match { 0..128 => true, 128..255 => false } }",
        "comptime fn bad(x: Byte, high: Byte) -> Bool { x match { 0..='high => true } }",
        "comptime fn bad(x: Byte) -> Bool { x match { 0..=255 if false => true } }",
        "comptime fn bad(x: (Byte, Bool)) -> Bool { x match { (0..128, true) => true, (128..=255, false) => false } }",
    ] {
        rejected(&format!("{prefix}{body}"), "non-exhaustive match");
    }
}

#[test]
fn scalar_ordering_rejects_mixed_types_and_unsupported_protocols() {
    for source in [
        "comptime fn bad(a: .u8'builtin, b: .u16'builtin) -> true'type { a < b }",
        "comptime fn bad(a: .i8'builtin, b: .u8'builtin) -> true'type { a >= b }",
    ] {
        rejected(source, "type mismatch");
    }
    rejected(
        "comptime fn bad(a: .u8'builtin) -> true'type { 256 < a }",
        "integer does not fit",
    );
    for source in ["comptime fn bad(a: true'type, b: true'type) -> true'type { a < b }"] {
        rejected(source, "ordering protocol for this type is not implemented");
    }
}

#[test]
fn guard_coverage_does_not_invent_missing_points_or_execute_functions() {
    let prefix = "typealias Int = .Integer'builtin; typealias Bool = true'type; ";
    for body in [
        "comptime fn bad(x: Int) -> Bool { x match { n if n < 0 => true, n if n > 0 => false } }",
        "comptime fn bad(x: Int) -> Bool { x match { n if n != n => true } }",
        "comptime fn bound() -> Int { 0 } comptime fn bad(x: Int) -> Bool { x match { n if n < bound() => true, n if n >= bound() => false } }",
        "comptime fn bad(x: Int, bound: Int) -> Bool { x match { n if n < bound => true, n if n >= bound => false } }",
        "comptime fn bad(x: Int) -> Bool { x match { n if n == 0 and n != 0 => true } }",
    ] {
        rejected(&format!("{prefix}{body}"), "non-exhaustive match");
    }
}

#[test]
fn interface_methods_require_matching_execution_phases() {
    for source in [
        "trait Read { comptime fn read(self) -> true'type; } struct S {} impl Read for S { fn read(self) -> true'type { true } }",
        "trait Read { fn read(self) -> true'type; } struct S {} impl Read for S { comptime fn read(self) -> true'type { true } }",
    ] {
        rejected(source, "does not match its trait signature");
    }
    rejected(
        "trait Read { fn read(self) -> true'type; } struct S {} impl Read for S { fn read(self) -> true'type { true } } comptime fn bad(value: S) -> true'type { value.as(Read).read() }",
        "callable type is unavailable in this execution phase",
    );
    rejected(
        "trait Read { comptime fn read(self) -> true'type; } struct S {} impl Read for S { comptime fn read(self) -> true'type { true } } fn bad(value: S) -> true'type { value.as(Read).read() }",
        "callable type is unavailable in this execution phase",
    );
}

#[test]
fn required_methods_wait_for_a_complete_world_and_preserve_phase_failures() {
    accepted(
        "typealias Type = .Type'builtin; trait Shape { comptime fn Output(self) -> Type; } struct S {} impl Shape for S { comptime fn Output(self) -> Type { true'type } } comptime fn ChooseType() -> Type { S {}.as(Shape).Output() } typealias Early = ChooseType();",
    );
    rejected(
        "typealias Type = .Type'builtin; typealias Chosen = ChooseType(); trait Shape { comptime fn Output(self) -> Type; comptime fn consume(self, value: Chosen) -> true'type; } struct S {} impl Shape for S { comptime fn Output(self) -> Type { true'type } comptime fn consume(self, value: Chosen) -> true'type { true } } comptime fn ChooseType() -> Type { S {}.as(Shape).Output() }",
        "waiting for the trait implementation world",
    );
    rejected(
        "typealias Int = .Integer'builtin; trait Size { fn size(self) -> .u8'builtin; } struct S {} impl Size for S { fn size(self) -> .u8'builtin { 8 } } comptime fn ChooseType() -> .Type'builtin { let source = S {}; if source.as(Size).size() == 0 { true'type } else { 'a''type } } fn bad() { let value: ChooseType() = 'a'; }",
        "callable type is unavailable in this execution phase",
    );
}

#[test]
fn required_default_projection_preserves_cycles_and_final_body_checks() {
    let source = include_str!("../../../development/examples/overridden-comptime-default.fl");
    rejected(
        &source.replace(
            "impl Shape for Selector { comptime fn Output(self) -> Type { true'type } }",
            "impl Shape for Selector {}",
        ),
        "cyclic type alias",
    );
    rejected(
        &source.replace("-> Type { Chosen }", "-> Type { false }"),
        "type mismatch",
    );
}

#[test]
fn required_parameter_defaults_preserve_cycles_and_final_validation() {
    let source = include_str!("../../../development/examples/explicit-comptime-defaults.fl");
    rejected(
        &source.replace("Choose(.kind = true'type)", "Choose()"),
        "cyclic type alias",
    );
    rejected(
        &source.replace(".kind: Type = Chosen", ".kind: Type = false"),
        "type mismatch",
    );
}

#[test]
fn parameter_default_queries_reject_real_cycles_and_invalid_remaining_defaults() {
    rejected(
        "typealias Type = .Type'builtin; typealias Bool = true'type; typealias Cyclic = Choose(); comptime fn Choose(.first: Type = Choose(.first = Bool), .second: Type = Choose(.second = Bool)) -> Type { first }",
        "cyclic or excessively deep function default dependency",
    );
    let source = include_str!("../../../development/examples/parameter-default-queries.fl");
    rejected(
        &source.replace(".kind: Type = Chosen", ".kind: Type = false"),
        "type mismatch",
    );
}

#[test]
fn float_comparisons_require_matching_precision_and_keep_coverage_conservative() {
    for source in [
        "comptime fn bad(a: .f32'builtin, b: .f64'builtin) -> true'type { a < b }",
        "comptime fn bad(a: .f32'builtin, b: .f64'builtin) -> true'type { a == b }",
        "comptime fn bad(a: .f32'builtin, b: .u32'builtin) -> true'type { a >= b }",
    ] {
        rejected(source, "type mismatch");
    }
    rejected(
        "comptime fn bad(a: .f32'builtin, b: .f32'builtin) -> true'type { a match { 'b => true } }",
        "exhaustive",
    );
}

#[test]
fn float_arithmetic_rejects_mixed_precision_and_non_numeric_operands() {
    for source in [
        "fn bad(a: .f32'builtin, b: .f64'builtin) -> .f32'builtin { a + b }",
        "comptime fn bad(a: .f32'builtin, b: .f64'builtin) -> .f32'builtin { a / b }",
        "fn bad(a: .f32'builtin, b: true'type) -> .f32'builtin { a * b }",
        "fn bad(a: true'type) -> .f32'builtin { -a }",
    ] {
        rejected(source, "type mismatch");
    }
    rejected(
        "comptime fn bad(a: .Integer'builtin, b: .Integer'builtin) -> .Integer'builtin { a / b }",
        "Integer division operator rounding protocol",
    );
}

#[test]
fn numeric_peer_context_does_not_coerce_fixed_operand_types() {
    for source in [
        "comptime fn fixed() -> .f64'builtin { 1.0 } comptime fn bad(value: .f32'builtin) -> true'type { fixed() == value }",
        "comptime fn bad(a: .f64'builtin, b: .f32'builtin) -> .f32'builtin { (a + 1.0) + b }",
        "comptime fn bad(value: .f32'builtin) -> true'type { let fixed: .f64'builtin = 1.0; fixed < value }",
    ] {
        rejected(source, "type mismatch");
    }
}

#[test]
fn float_associated_constants_require_type_evidence_and_preserve_stage_boundaries() {
    rejected(
        "pure comptime fn Identity<T> -> .Type'builtin { T } typealias Hidden = Identity<.f32'builtin>; comptime fn bad() { Hidden.NaN; }",
        // An opaque normal-form head cannot supply the floating type evidence.
        // Keep the unresolved proof diagnostic, rather than treating Unknown as
        // proof that the subject is a concrete non-floating struct type.
        "structural type view remains unresolved",
    );
    rejected(
        "typealias F32 = .f32'builtin; fn bad() -> .f64'builtin { F32.NaN }",
        "type mismatch",
    );
    for source in [
        "typealias I = .i32'builtin; fn bad() { I.NaN; }",
        "fn bad(value: .f32'builtin) { value.Infinity; }",
        "comptime fn bad() -> T where T { T.NaN }",
        "typealias F32 = .f32'builtin; comptime fn bad() { F32.NotAConstant; }",
    ] {
        rejected(source, "field projection requires a supported struct");
    }
    rejected(
        "typealias F32 = .f32'builtin; const Stored = F32.NaN; fn bad() -> F32 { Stored }",
        "constant runtime use requires cross-stage Splice",
    );
}

#[test]
fn program_assumes_rejects_non_boolean_unchecked_calls_and_type_propositions() {
    rejected(
        "fn bad() { assumes 1; }",
        "integer runtime proxy requires a supported fixed-width integer target",
    );
    for source in [
        "fn condition() -> true'type { true } fn bad() { assumes condition(); }",
        "fn bad() { var flag = true; assumes do { flag = false; true }; }",
        "fn bad() { assumes do { assumes true; true }; }",
        "fn bad() { assumes .i32'builtin == .u32'builtin; }",
    ] {
        rejected(
            source,
            "assumes requires a supported effect-free program predicate",
        );
    }
}

#[test]
fn program_assumes_failure_aborts_required_type_invocation() {
    rejected(
        "comptime fn Select() -> .Type'builtin { assumes false; true'type } typealias Chosen = Select();",
        "ProgramContractRejected",
    );
}

#[test]
fn checked_machine_arithmetic_requires_proof_before_each_operation() {
    for source in [
        "fn bad(value: .u8'builtin) -> .u8'builtin { value + 1 }",
        "fn bad(value: .u8'builtin) -> .u8'builtin { let result = value + 1; assumes value < 255; result }",
        "fn bad(value: .u8'builtin) -> .u8'builtin { var x = value; assumes x < 255; x = 255; x + 1 }",
        "fn bad(value: .u8'builtin, flag: true'type) -> .u8'builtin { if flag { assumes value < 255; } value + 1 }",
        "fn bad(flag: true'type) { var x: .u8'builtin = 0; while flag { x = x + 1; } }",
        "comptime fn bad() -> .u8'builtin { (250 + 10) - 10 }",
        "fn bad(value: .i8'builtin) -> .i8'builtin { -value }",
        "fn bad(value: .u8'builtin) -> .u8'builtin { assumes value + 1 < 255; value }",
        "comptime fn Select() -> .Type'builtin { let x: .u8'builtin = 255 + 1; true'type } typealias Chosen = Select();",
        "fn unused(.value: .u8'builtin = 255 + 1) {}",
    ] {
        rejected(source, "machine arithmetic range is not proven");
    }
    rejected(
        "comptime fn generic(value: T) -> .u8'builtin where T, requires T == .u8'builtin { value + 1 }",
        "machine arithmetic range is not proven",
    );
}

#[test]
fn checked_machine_arithmetic_cannot_hide_in_closures_or_unanalyzed_patterns() {
    rejected(
        "fn bad() { let callback: fn(.u8'builtin) -> .u8'builtin = |value| value + 1; }",
        "machine arithmetic range is not proven",
    );
    rejected(
        "fn bad(value: .u8'builtin) -> .u8'builtin { value match { < do { let a: .u8'builtin = 255; a + 1 } > => 0, _ => value } }",
        "machine arithmetic range is not proven",
    );
}

#[test]
fn checked_machine_closures_cannot_export_body_checks_or_returns_to_the_creator() {
    for source in [
        "fn bad(value: .u8'builtin) -> .u8'builtin { let delayed = || { assumes value < 255; true }; value + 1 }",
        "fn bad() -> .u8'builtin { let delayed = || { return true; }; 255 + 1 }",
        "fn bad() -> .u8'builtin { let delayed = || { assumes false; true }; 255 + 1 }",
        "fn bad() { let value: .u8'builtin = 255; let outer = || { let inner = || value + 1; inner() }; }",
        "fn bad() { let increment = |value: .u8'builtin| value + 1; increment(1); }",
    ] {
        rejected(source, "machine arithmetic range is not proven");
    }
}

#[test]
fn checked_machine_matches_do_not_leak_failed_guards_or_forget_their_writes() {
    for source in [
        "fn bad(value: .u8'builtin) -> .u8'builtin { value match { n if n < 255 => n + 1, _ => value + 1 } }",
        "fn bad(value: .u8'builtin) -> .u8'builtin { var current: .u8'builtin = 4; value match { n if (do { current = 255; false }) => n, _ => current + 1 } }",
        "fn bad(value: .u8'builtin) -> .u8'builtin { value match { n if n == 255 => n + 1, _ => value } }",
    ] {
        rejected(source, "machine arithmetic range is not proven");
    }
}

#[test]
fn checked_machine_tuples_do_not_alias_source_variables_or_ignore_bad_branches() {
    for source in [
        "fn bad() { var x: .u8'builtin = 255; let pair: (.u8'builtin, .u8'builtin) = (x, 1); x = 0; let callback = || { x; let (a, b) = pair; a + b }; }",
        "fn bad() -> .u8'builtin { var x: .u8'builtin = 255; let pair: (.u8'builtin, .u8'builtin) = (x, 1); x = 0; let (a, b) = pair; a + b }",
        "fn bad(flag: true'type) -> .u8'builtin { let pair: (.u8'builtin, .u8'builtin) = if flag { (255, 1) } else { (0, 0) }; let (a, b) = pair; a + b }",
        "fn bad(pair: (.u8'builtin, .u8'builtin)) -> .u8'builtin { let (a, b) = pair; a + b }",
        "fn bad() { let pair: (.u8'builtin, .u8'builtin) = (255, 1); let callback = || { let (a, b) = pair; a + b }; }",
    ] {
        rejected(source, "machine arithmetic range is not proven");
    }
}

#[test]
fn checked_machine_nominals_forget_stale_fields_and_do_not_confuse_enum_payloads() {
    for source in [
        "struct Box { value: .u8'builtin } fn bad() -> .u8'builtin { var box = Box { value: 4 }; box.value = 255; box.value + 1 }",
        "struct Box { value: .u8'builtin } fn bad(box: Box) -> .u8'builtin { box.value + 1 }",
        "enum E { small(.u8'builtin), large(.u8'builtin) } fn bad(flag: true'type) -> .u8'builtin { let value = if flag { E.small(4) } else { E.large(255) }; value match { .small(n) => n + 1, .large(n) => n + 1 } }",
        "enum E { none, byte(.u8'builtin) } fn bad() -> .u8'builtin { let value = E.byte(255); value match { .none => 0, .byte(n) => n + 1 } }",
    ] {
        rejected(source, "machine arithmetic range is not proven");
    }
}

#[test]
fn checked_field_writes_cannot_reuse_old_leaf_bounds_or_invent_sibling_bounds() {
    for body in [
        "var copy = pair; if copy.left < 100 { copy.left = 100; } copy.left + 1",
        "var pair = Pair { left: 4, right: 5 }; pair.left = 255; pair.left + 1",
        "var copy = pair; copy.left = 100; copy.right + 1",
        "var pair = Pair { left: 4, right: 5 }; pair.left = do { pair.right = 255; 0 }; pair.right + 1",
    ] {
        rejected(
            &format!(
                "struct Pair {{ left: .u8'builtin, right: .u8'builtin }} fn bad(pair: Pair) -> .u8'builtin {{ {body} }}"
            ),
            "machine arithmetic range is not proven",
        );
    }
}

#[test]
fn checked_field_predicates_expire_on_overlapping_writes_and_calls() {
    for body in [
        "var copy = pair; assumes copy.left < 255; while flag { copy.left = 255; break; } copy.left + 1",
        "var copy = pair; assumes copy.left < 255; copy.left = 255; copy.left + 1",
        "var copy = pair; assumes copy.left < 255; copy = Pair { left: 255, right: 0 }; copy.left + 1",
        "var copy = pair; assumes copy.left < 255; if flag { copy.left = 255; } copy.left + 1",
        "if flag { assumes pair.left < 255; } pair.left + 1",
        "assumes pair.left < 255 or pair.right < 255; pair.left + 1",
        "var outer = Outer { inner: pair, tail: 0 }; assumes outer.inner.left < 255; outer.inner = Pair { left: 255, right: 0 }; outer.inner.left + 1",
        "assumes pair.left < 255; let other = Pair { left: pair.right, right: 0 }; other.left + 1",
    ] {
        rejected(
            &format!(
                "typealias Byte = .u8'builtin; struct Pair {{ left: Byte, right: Byte }} struct Outer {{ inner: Pair, tail: Byte }} fn unknown() {{}} fn bad(pair: Pair, flag: true'type) -> Byte {{ {body} }}"
            ),
            "machine arithmetic range is not proven",
        );
    }
}

#[test]
fn checked_field_predicates_do_not_make_calls_effect_free() {
    rejected(
        "struct Pair { left: .u8'builtin } fn make() -> Pair { Pair { left: 0 } } fn bad() { assumes make().left < 255; }",
        "assumes requires a supported effect-free program predicate",
    );
}

#[test]
fn checked_relational_arithmetic_rejects_stale_partial_and_retroactive_proofs() {
    for body in [
        "end - start",
        "assumes start + end < 255; start + end",
        "assumes start <= end; var value = start; value = 255; end - value",
        "if flag { assumes start <= end; } end - start",
        "assumes start <= end or flag; end - start",
        "var pair = Pair { start: start, end: end }; assumes pair.start <= pair.end; pair.start = 255; pair.end - pair.start",
    ] {
        rejected(
            &format!(
                "typealias Byte = .u8'builtin; struct Pair {{ start:Byte,end:Byte }} fn unknown() {{}} fn bad(start: Byte,end:Byte,flag:true'type) -> Byte {{ {body} }}"
            ),
            "machine arithmetic range is not proven",
        );
    }
}

#[test]
fn checked_transitive_arithmetic_rejects_missing_and_invalidated_links() {
    for body in [
        "assumes a <= b; c - a",
        "assumes a <= b or b <= c; c - a",
        "if flag { assumes a <= b and b <= c; } c - a",
        "var value = Triple { first:a,middle:b,last:c }; assumes value.first <= value.middle and value.middle <= value.last; value.first = 255; value.last - value.first",
    ] {
        rejected(
            &format!(
                "typealias Byte=.u8'builtin; struct Triple {{ first:Byte,middle:Byte,last:Byte }} fn unknown() {{}} fn bad(a:Byte,b:Byte,c:Byte,flag:true'type) -> Byte {{ {body} }}"
            ),
            "machine arithmetic range is not proven",
        );
    }
}

#[test]
fn checked_relational_copies_do_not_alias_new_or_unrelated_values() {
    for body in [
        "var pair = Pair { start:a,end:b }; assumes pair.start <= pair.end; var copy = pair; if a == 0 { copy.start = 255; copy.end = 0; } copy.end - copy.start",
        "assumes a <= b; let old = a; var next = b; next = 0; next - old",
        "var pair = Pair { start:a,end:b }; assumes pair.start <= pair.end; var copy = pair; copy.start = 255; copy.end - copy.start",
        "var pair = Pair { start:a,end:b }; assumes pair.start <= pair.end; let old = pair; pair.start = 255; pair.end - pair.start",
        "assumes a <= b; let callback = |x: Byte| x - a; callback(b)",
        "var x = a; assumes x <= b; x = 255; let callback = || b - x; callback()",
    ] {
        rejected(
            &format!(
                "typealias Byte=.u8'builtin; struct Pair {{ start:Byte,end:Byte }} fn bad(a:Byte,b:Byte) -> Byte {{ {body} }}"
            ),
            "machine arithmetic range is not proven",
        );
    }
}

#[test]
fn checked_aggregate_relations_reject_mixed_versions_and_replaced_payloads() {
    for body in [
        "assumes start <= end; let pair = (start, end); let callback = |other: Byte| { let (a, b) = pair; other - a }; callback(end)",
        "var b = end; assumes start <= b; let pair = (start, do { b = 0; b }); let (a, z) = pair; z - a",
        "assumes start <= end; var pair = (start, end); pair = (255, 0); let (a, b) = pair; b - a",
        "assumes start <= end; var value = Choice.pair(start, end); value = Choice.pair(255, 0); value match { .pair(a, b) => b - a, _ => 0 }",
        "var b = end; assumes start <= b; let pair = Pair { start: start, end: do { b = 0; b } }; pair.end - pair.start",
        "assumes start <= end; let pair = (end, start); let (a, b) = pair; b - a",
    ] {
        rejected(
            &format!(
                "fn unknown() {{}} typealias Byte = .u8'builtin; struct Pair {{ start:Byte,end:Byte }} enum Choice {{ empty,pair(Byte,Byte) }} fn bad(start: Byte,end: Byte) -> Byte {{ {body} }}"
            ),
            "machine arithmetic range is not proven",
        );
    }
}

#[test]
fn checked_aggregate_joins_do_not_import_one_branch_or_wrong_variant_payloads() {
    for body in [
        "let value = if a <= b { (a,b) } else { (a,b) }; let (start,end) = value; end - start",
        "let value = if flag { assumes a <= b; Pair { start:a,end:b } } else { Pair { start:255,end:0 } }; value.end - value.start",
        "let value = flag match { true => { assumes a <= b; (a,b) }, false => (255,0) }; let (start,end) = value; end - start",
        "let value = if flag { assumes a <= b; Choice.pair(a,b) } else { Choice.tuple((255,0)) }; value match { .pair(start,end) => end - start, .tuple((start,end)) => end - start, _ => 0 }",
    ] {
        rejected(
            &format!(
                "typealias Byte = .u8'builtin; struct Pair {{ start:Byte,end:Byte }} enum Choice {{ empty,pair(Byte,Byte),tuple((Byte,Byte)) }} fn bad(a:Byte,b:Byte,flag:true'type) -> Byte {{ {body} }}"
            ),
            "machine arithmetic range is not proven",
        );
    }
}

#[test]
fn checked_composed_differences_reject_missing_links_and_intermediate_overflow() {
    for body in [
        "assumes a <= 10000 and b <= 10000 and c <= 10000 and d <= 10000; assumes c <= a; (a + b) - (c + d)",
        "assumes a <= 10000 and b <= 10000 and c <= 10000 and d <= 10000; assumes c <= a or d <= b; (a + b) - (c + d)",
        "assumes c <= a and d <= b; (a + b) - (c + d)",
        "assumes a <= 10000 and c <= a and d <= a; a - (c + d)",
    ] {
        rejected(
            &format!(
                "typealias Word=.u16'builtin; fn bad(a:Word,b:Word,c:Word,d:Word) -> Word {{ {body} }}"
            ),
            "machine arithmetic range is not proven",
        );
    }
}

#[test]
fn program_preconditions_reject_unproved_calls_and_contract_erasure() {
    let definition = "typealias Byte=.u8'builtin; fn distance(a:Byte,b:Byte) -> Byte where requires a <= b { b - a }";
    for body in [
        "distance(a,b)",
        "assumes a <= b; distance(b,a)",
        "var value=b; assumes a <= value; value=0; distance(a,value)",
        "var value=b; assumes a <= value; distance(a,do { value=0; value })",
        "if a == 0 { assumes a <= b; } distance(a,b)",
    ] {
        rejected(
            &format!("{definition} fn bad(a:Byte,b:Byte) -> Byte {{ {body} }}"),
            "program precondition is not proven",
        );
    }
    rejected(
        &format!("{definition} fn bad() {{ let erased=distance; }}"),
        "erase program preconditions",
    );
    rejected(
        "typealias Byte=.u8'builtin; fn bad(x:Byte) where requires x + 1 > x {}",
        "machine arithmetic range is not proven",
    );
    rejected(
        "typealias Byte=.u8'builtin; fn bad(x:Byte,.y:Byte=x + 1) where requires x < 255 {}",
        "machine arithmetic range is not proven",
    );
    rejected(
        "fn enabled(flag:true'type) where requires flag {} fn bad(flag:true'type) { enabled(flag); }",
        "program precondition is not proven",
    );
    rejected(
        "fn enabled(flag:true'type) where requires flag {} fn bad() { enabled(false); }",
        "program precondition is not proven",
    );
    rejected(
        "comptime fn Selected(flag:true'type) -> .Type'builtin where requires flag { true'type } typealias Bad=Selected(false);",
        "program precondition is not proven",
    );
    rejected(
        "fn mixed(value:T,flag:true'type) -> T where T, requires T == true'type, requires flag { value } fn bad() { mixed('x',true); }",
        "type requirement is not satisfied",
    );
}

#[test]
fn program_postconditions_reject_bad_exits_and_result_scope_leaks() {
    for body in [
        "255",
        "if flag { return 255; } 0",
        "if flag { 0 } else { 255 }",
    ] {
        rejected(
            &format!(
                "typealias Byte=.u8'builtin; fn bad(flag:true'type) -> result:Byte where ensures result < 100 {{ {body} }}"
            ),
            "program postcondition is not proven",
        );
    }
    rejected(
        "fn bad() -> result:.u8'builtin where ensures result + 1 > result { 255 }",
        "machine arithmetic range is not proven",
    );
    rejected(
        "fn bad() -> result:.u8'builtin { result }",
        "Unresolved identifier",
    );
    rejected(
        "fn bad() -> result:.u8'builtin where requires result < 100 { 0 }",
        "Unresolved identifier",
    );
    rejected(
        "fn bad(value:.u8'builtin) -> value:.u8'builtin { value }",
        "result name duplicates",
    );
    rejected(
        "typealias Byte=.u8'builtin; fn changed(value:Byte) -> result:Byte where ensures result == value { value=255; value } fn bad(value:Byte) -> Byte { value - changed(value) }",
        "machine arithmetic range is not proven",
    );
    // A caller must not make an invalid producer's declaration trustworthy.
    rejected(
        "typealias Byte=.u8'builtin; fn caller() -> Byte { bad() + 155 } fn bad() -> result:Byte where ensures result < 100 { 255 }",
        "program postcondition is not proven",
    );
}

#[test]
fn program_entry_snapshots_reject_scope_errors_and_false_guarantees() {
    rejected(
        "fn bad(value:.u8'builtin) -> .u8'builtin { value'old }",
        "old is only available in program postconditions",
    );
    rejected(
        "fn bad(value:.u8'builtin) where requires value'old < 100 {}",
        "old is only available in program postconditions",
    );
    rejected(
        "fn bad(value:.u8'builtin) -> result:.u8'builtin where ensures result == result'old { value }",
        "old requires an entry parameter",
    );
    rejected(
        "fn bad(value:.u8'builtin) -> result:.u8'builtin where ensures result == value'old { value=0; value }",
        "program postcondition is not proven",
    );
    rejected(
        "fn bad(value:.u8'builtin) -> result:.u8'builtin where ensures result == (value + 1)'old { 0 }",
        "machine arithmetic range is not proven",
    );
    rejected(
        "typealias Byte=.u8'builtin; fn changed(value:Byte) -> result:Byte where ensures result == value { value=255; value } fn bad(value:Byte) -> result:Byte where ensures result == value'old { changed(value) }",
        "program postcondition is not proven",
    );
}

#[test]
fn program_entry_snapshots_do_not_confuse_mutated_call_arguments() {
    rejected(
        "typealias Byte=.u8'builtin; fn partial(value:Byte) -> result:Byte where requires value <= 15, ensures result <= (value * value)'old { 0 } fn bad(value:Byte) -> result:Byte where requires value <= 15, ensures result == (value * value)'old { partial(value) }",
        "program postcondition is not proven",
    );
    rejected(
        "typealias Byte=.u8'builtin; fn identity(value:Byte) -> result:Byte where ensures result == value'old { value } fn bad(value:Byte) -> result:Byte where ensures result == value'old { value=0; identity(value) }",
        "program postcondition is not proven",
    );
    rejected(
        "typealias Byte=.u8'builtin; fn first(a:Byte,b:Byte) -> result:Byte where ensures result == a'old { a } fn bad(value:Byte) -> result:Byte where ensures result == value'old { first(do { value=0; value }, value) }",
        "program postcondition is not proven",
    );
}

#[test]
fn trait_program_contracts_reject_strengthening_and_broken_guarantees() {
    let prefix = "typealias Byte=.u8'builtin; trait Step { fn next(value:Byte) -> result:Byte where requires value < 255, ensures result == value'old + 1; } struct S {}";
    rejected(
        &format!(
            "{prefix} impl Step for S {{ fn next(input:Byte) -> Byte where requires input < 100 {{ input + 1 }} }}"
        ),
        "strengthens trait program preconditions",
    );
    rejected(
        &format!("{prefix} impl Step for S {{ fn next(input:Byte) -> Byte {{ input }} }}"),
        "program postcondition is not proven",
    );
    rejected(
        &format!(
            "{prefix} impl Step for S {{ fn next(input:Byte) -> Byte {{ input + 1 }} }} fn bad(value:Byte) -> Byte {{ S.as(Step).next(value) }}"
        ),
        "program precondition is not proven",
    );
    rejected(
        &format!(
            "{prefix} impl Step for S {{ fn next(input:Byte) -> Byte {{ input + 1 }} }} fn bad() {{ let erased=S.as(Step).next; }}"
        ),
        "trait method value would erase program contracts",
    );
    rejected(
        "typealias Byte=.u8'builtin; trait Read { fn read(value:Byte) -> Byte; } struct S {} impl Read for S { fn read(value:Byte) -> Byte where requires value < 100 { value } }",
        "strengthens trait program preconditions",
    );
    rejected(
        "typealias Byte=.u8'builtin; trait Bad { fn next(value:Byte) -> result:Byte where requires value < 255, ensures result == value'old + 1 { value } }",
        "program postcondition is not proven",
    );
    rejected(
        &format!(
            "{prefix} impl Step for S {{ fn next(input:Byte) -> Byte where requires input <= 255 {{ input=255; 0 }} }}"
        ),
        "program postcondition is not proven",
    );
}

#[test]
fn pointer_operations_reject_unsupported_accesses() {
    rejected(
        "fn run() -> true'type { true.* }",
        "dereference requires a pointer",
    );
    rejected("fn run() -> *true'type { true.ref }", "address");
    rejected(
        "struct Cell { value:*bool } fn run(p:*Cell)->Cell {p.*}",
        "pointer reads of non-intrinsic values",
    );
    rejected(
        "struct Cell { value:*bool } fn run(p:*Cell,value:Cell) {p.* = value;}",
        "non-Copy pointer writes",
    );
    accepted("comptime fn run() -> true'type { let value = true; value.ref.* }");
}

#[test]
fn pointer_writes_invalidate_alias_dependent_arithmetic_facts() {
    rejected(
        "typealias Byte = .u8'builtin; fn run() -> Byte { let value: Byte = 0; let pointer = value.ref; pointer.* = 255; value + 1 }",
        "machine arithmetic range is not proven",
    );
    rejected(
        "typealias Byte = .u8'builtin; struct Cell { value: Byte, } fn run() -> Byte { let cell = Cell { value: 0 }; let pointer = cell.ref; pointer.value.* = 255; cell.value + 1 }",
        "machine arithmetic range is not proven",
    );
}

#[test]
fn nested_nominal_methods_share_inherent_coherence_and_parent_contracts() {
    rejected(
        "struct Box where T { fn read() {} } impl Box<U> where U { fn read() {} }",
        "inherent implementation overlap",
    );
    accepted(
        "struct Box where T { fn first() {} } impl Box<U> where U { fn second() {} } fn run() { Box<true'type>.first(); Box<true'type>.second(); }",
    );
    accepted(
        "struct Box where T, requires T == true'type { value: T, fn read(self) -> true'type { self.value } } fn run() -> true'type { Box<true'type>.read(Box<true'type> { value: true }) }",
    );
    rejected(
        "struct Box where T, requires T == true'type { fn read() {} } fn run() { Box<'x''type>.read(); }",
        "type requirement is not satisfied",
    );
}

#[test]
fn pointer_receiver_adaptation_does_not_invent_storage_or_move_permission() {
    rejected(
        "struct Cell { fn read(*self) {} } fn run() { Cell {}.read(); }",
        "address-of requires a local or field place",
    );
    rejected(
        "struct Cell { fn consume(self) {} } fn run() { let cell = Cell {}; cell.ref.consume(); }",
        "by-value pointer receiver requires memory transfer contracts",
    );
    rejected(
        "trait Read { fn read(*self); } struct Cell {} impl Read for Cell { fn read(*self) {} } impl Read for *Cell { fn read(*self) {} } fn run() { let cell = Cell {}; cell.ref.as(Read).read(); }",
        "address-of requires a local or field place",
    );
}

#[test]
fn dereferenced_field_places_reject_invalid_and_owned_deliveries() {
    rejected(
        "struct Cell {} fn run() { let cell = Cell {}; cell.ref.*.missing = true; }",
        "unknown struct field",
    );
    rejected(
        "struct Cell {value:*bool} struct Outer {cell:Cell} fn run(p:*Outer)->Cell {p.*.cell}",
        "pointer reads of non-intrinsic values",
    );
    rejected(
        "struct Cell { fn consume(self) {} } fn run() { let cell = Cell {}; cell.ref.*.consume(); }",
        "by-value pointer receiver requires memory transfer contracts",
    );
    accepted(
        "struct Cell { value: true'type, } comptime fn run() -> true'type { let cell = Cell { value: true }; cell.ref.*.value }",
    );
}

#[test]
fn runtime_callable_capture_assignment_records_environment_and_respects_mutability() {
    accepted(
        "fn first()->bool {true} fn second()->bool {false} fn entry() {var callback=first;let replace=|| {callback=second;};replace();}",
    );
    rejected(
        "fn first()->bool {true} fn second()->bool {false} fn entry() {let callback=first;let replace=|| {callback=second;};}",
        "cannot reassign a let binding",
    );
    accepted(
        "comptime fn first()->bool {true} comptime fn second()->bool {false} comptime fn entry() {var callback=first;let replace=|| {callback=second;};}",
    );
}

#[test]
fn mutable_capture_arithmetic_requires_an_invocation_invariant() {
    for phase in ["", "comptime "] {
        rejected(
            &format!(
                "{phase}fn run() {{var value:.u8'builtin=0;let next=|| {{value = value + 1;}};next();}}"
            ),
            "machine arithmetic range is not proven",
        );
        accepted(&format!(
            "{phase}fn run() {{var value:.u8'builtin=0;let next=|| {{assumes value < 255;value = value + 1;}};next();}}"
        ));
    }
}

#[test]
fn mutable_capture_parent_replacement_invalidates_all_descendants() {
    for source in [
        "struct State {dirty:bool,limit:.u8'builtin,} fn run() {var state=State {dirty:false,limit:7};let next=|| {let result=state.limit + 1;state=State {dirty:false,limit:255};result};next();}",
        "struct Inner {dirty:bool,limit:.u8'builtin,} struct Outer {inner:Inner,} fn run() {let state=Outer {inner:Inner {dirty:false,limit:7}};let next=|| {let result=state.inner.limit + 1;state.inner=Inner {dirty:false,limit:255};result};next();}",
    ] {
        rejected(source, "machine arithmetic range is not proven");
    }
}

#[test]
fn readonly_function_calls_preserve_scalar_field_and_relational_proofs() {
    for source in [
        "fn touch() {} fn run(value:.u8'builtin)->.u8'builtin {assumes value < 255;touch();value + 1}",
        "typealias Byte=.u8'builtin;struct Pair {left:Byte,right:Byte} fn touch() {} fn run(pair:Pair)->Byte {assumes pair.left < 255;touch();pair.left + 1}",
        "typealias Byte=.u8'builtin;fn touch() {} fn run(start:Byte,end:Byte)->Byte {assumes start <= end;touch();end - start}",
        "typealias Byte=.u8'builtin;fn touch() {} fn run(a:Byte,b:Byte,c:Byte)->Byte {assumes a <= b and b <= c;touch();c - a}",
        "typealias Byte=.u8'builtin;fn touch() {} fn run(start:Byte,end:Byte)->Byte {assumes start <= end;let pair=(start,end);touch();let (a,b)=pair;b - a}",
    ] {
        accepted(source);
    }
    // A callback has no route to unaddressed scalar-by-value storage.
    accepted(
        "fn run(value:.u8'builtin,unknown:fn())->.u8'builtin {assumes value < 255;unknown();value + 1}",
    );
    for source in [
        "fn run(value:.u8'builtin,unknown:fn(*.u8'builtin))->.u8'builtin {assumes value < 255;unknown(value.ref);value + 1}",
        "fn write(p:*.u8'builtin) {p.* = 255;} fn run(value:.u8'builtin)->.u8'builtin {assumes value < 255;write(value.ref);value + 1}",
    ] {
        rejected(source, "machine arithmetic range is not proven");
    }
}

#[test]
fn direct_pointer_read_proofs_expire_after_target_writes() {
    for source in [
        "fn bad(n:.u8'builtin)->.u8'builtin {let old=n.ref.*;assumes old < 255;n.ref.* = 255;n.ref.* + 1}",
        "struct State {value:.u8'builtin} fn bad(state:State)->.u8'builtin {let old=state.ref.value.*;assumes old < 255;state.value=255;state.ref.value.* + 1}",
        "fn bad(flag:bool)->.u8'builtin {let n:.u8'builtin=0;if flag {n.ref.* = 255;}n.ref.* + 1}",
        "fn bad(p:*.u8'builtin)->.u8'builtin {p.* + 1}",
    ] {
        rejected(source, "machine arithmetic range is not proven");
    }
}

#[test]
fn possible_pointer_write_does_not_prove_every_candidate_was_overwritten() {
    for source in [
        "fn bad(flag:bool)->.u8'builtin {let first:.u8'builtin=255;let second:.u8'builtin=0;let pointer=if flag {first.ref} else {second.ref};pointer.* = 0;first + 1}",
        "struct Cell {value:.u8'builtin} fn bad(flag:bool)->.u8'builtin {let first=Cell {value:255};let second=Cell {value:0};let pointer=if flag {first.ref} else {second.ref};pointer.value.* = 0;first.value + 1}",
    ] {
        rejected(source, "machine arithmetic range is not proven");
    }
}

#[test]
fn variadic_routes_reject_duplicate_unknown_and_invalid_packages() {
    let declaration = "fn take(...items:(bool,'x''type))->(bool,'x''type) {items}";
    for call in [
        "take(true,'x',.items=(false,'y'))",
        "take(.items=(false,'y'),true,'x')",
        "take(.items=(true,'x'),.items=(false,'y'))",
    ] {
        rejected(
            &format!("{declaration} fn run()->(bool,'x''type) {{{call}}}"),
            "parameter supplied more than once",
        );
    }
    rejected(
        &format!("{declaration} fn run()->(bool,'x''type) {{take(.item=(true,'x'))}}"),
        "no named parameter entry",
    );
    for call in [
        "take(true)",
        "take(true,'x',false)",
        "take('x',true)",
        "take(.items=true)",
    ] {
        rejected(
            &format!("{declaration} fn run()->(bool,'x''type) {{{call}}}"),
            "type",
        );
    }
    rejected(
        "typealias Type = .Type'builtin; fn take(...items:Type)->Type {items} fn run()->Type {take(bool,'x''type)}",
        "collection requires a tuple type",
    );
    rejected(
        "typealias Type = .Type'builtin; fn take(...items:T)->T where T {items} fn run()->Type {take(bool,'x''type)}",
        "collection requires a tuple type",
    );
}

#[test]
fn variadic_function_values_do_not_silently_erase_collection_protocols() {
    rejected(
        "fn take(...items:(bool,bool))->(bool,bool) {items} fn run() {let callback=take;}",
        "variadic function values require parameter protocol metadata",
    );
    rejected(
        "fn take(...items:T)->T where T {items} fn run() {let callback=take<(bool,bool)>;}",
        "variadic function values require parameter protocol metadata",
    );
    rejected(
        "fn take(...items:T)->T where T {items} fn accept(callback:fn((bool,bool))->(bool,bool)) {} fn run() {accept(take);}",
        "variadic function values require parameter protocol metadata",
    );
}

#[test]
fn tuple_collection_respects_equations_without_unfolding_normal_form_heads() {
    let prefix = "typealias Byte=.u8'builtin; pure comptime fn Identity<T> -> .Type'builtin {T} typealias Hidden=Identity<(Byte,bool)>; fn keep(...items:T)->T where T {items}";
    rejected(
        &format!("{prefix} fn make()->Hidden {{keep<Hidden>(7,true)}}"),
        "structural type view remains unresolved",
    );
    rejected(&format!("{prefix} fn make()->Hidden {{(7,true)}}"), "type");
    for (body, message) in [
        ("keep<T>(7,7)", "integer runtime proxy"),
        ("keep<T>(7)", "type"),
        ("keep<T>(7,true,false)", "type"),
    ] {
        rejected(
            &format!(
                "typealias Byte=.u8'builtin; fn keep(...items:T)->T where T {{items}} fn make()->T where T, requires T == (Byte,bool) {{{body}}}"
            ),
            message,
        );
    }
    accepted(&format!(
        "{prefix} fn make()->Hidden where requires Hidden == (Byte,bool) {{keep<Hidden>(7,true)}}"
    ));
}

#[test]
fn tuple_spreads_reject_non_packets_wrong_arity_and_duplicate_supply() {
    rejected("fn take(a:bool) {} fn run() {take(...true);}", "type");
    rejected(
        "fn take(a:bool) {} fn run() {let packet=(true,false);take(...packet);}",
        "spread tuple widths exceed positional slots",
    );
    rejected(
        "fn take(a:bool,b:bool) {} fn run() {take(...(true,false,true));}",
        "spread tuple widths exceed positional slots",
    );
    rejected(
        "fn take(a:bool,b:bool) {} fn run() {take(...(true,));}",
        "missing anchored parameter",
    );
    // The complete package reserves all slots before contextual spread checking.
    rejected(
        "fn keep(...items:(bool,bool)) {} fn run() {let packet=(true,false);keep(...packet,.items=(false,true));}",
        "spread tuple widths exceed positional slots",
    );
}

#[test]
fn tuple_spreads_reject_contextual_packet_element_and_method_shape_errors() {
    rejected(
        "fn apply(callback:fn(.u8'builtin,bool)->bool)->bool {callback(...do {(7,7)})}",
        "integer runtime proxy",
    );
    rejected(
        "struct Holder {} impl Holder {fn take(self,number:.u8'builtin,flag:bool)->bool {flag}} fn run()->bool {Holder {}.take(...do {(7,true,false)})}",
        "spread tuple widths exceed positional slots",
    );
    rejected(
        "fn apply(callback:F)->bool where F, requires F == fn(.u8'builtin,bool)->bool {callback(...do {(7,7)})}",
        "integer runtime proxy",
    );
}

#[test]
fn tuple_spreads_layout_hints_do_not_accept_inconsistent_shapes_or_element_types() {
    let head = "fn take(a:.u8'builtin,b:bool,c:.u16'builtin,d:'x''type) {}";
    rejected(
        &format!(
            "{head} fn run(flag:bool) {{take(...if flag {{(7,true)}} else {{(7,true,false)}},...do {{(9,'x')}});}}"
        ),
        "type",
    );
    rejected(
        &format!("{head} fn run() {{take(...do {{(7,true,false)}},...do {{(9,'x')}});}}"),
        "spread tuple widths exceed positional slots",
    );
    rejected(
        &format!("{head} fn run() {{take(...do {{(7,7)}},...do {{(9,'x')}});}}"),
        "integer runtime proxy",
    );
    rejected(
        &format!("{head} fn run() {{take(...do {{(7,true)}},...do {{(9,true)}});}}"),
        "type",
    );
}

#[test]
fn constructor_spreads_reject_bad_payload_types_arities_and_named_inputs() {
    let head = "enum Choice {pair(.u8'builtin,bool)}";
    for (input, message) in [
        ("Choice.pair(...do {(7,7)})", "integer runtime proxy"),
        (
            "Choice.pair(...do {(7,true,false)})",
            "spread tuple widths exceed positional slots",
        ),
        (
            "Choice.pair(...(7,))",
            "enum payload argument count mismatch",
        ),
        ("Choice.pair(...true)", "type"),
        (
            "Choice.pair(...(7,),.flag=true)",
            "positional enum payload requires positional arguments",
        ),
    ] {
        rejected(&format!("{head} fn run()->Choice {{{input}}}"), message);
    }
    rejected(
        "enum Choice {none} fn run()->Choice {Choice.none(...(true,))}",
        "unit enum variant does not take arguments",
    );
}

#[test]
fn extended_calls_do_not_confuse_properties_with_named_supply() {
    rejected(
        "fn take(.enabled:bool=false)->bool {enabled} fn run()->bool {take {enabled:true}}",
        "properties are not named parameter supply",
    );
    accepted("fn keep(...items:T)->T where T {items} fn run() {keep {value:true};}");
    rejected("fn take(value:bool) {} fn run() {take {'x'};}", "type");
    rejected(
        "fn take(value:bool) {} fn run() {take {};}",
        "missing anchored parameter",
    );
    accepted("struct Holder {enabled:bool} fn run()->Holder {Holder {enabled:true}}");
}

#[test]
fn property_collection_rejects_duplicate_missing_and_ambiguous_entries() {
    let head = "fn keep(...properties:T)->T where T {properties}";
    rejected(
        &format!("{head} fn run() {{keep {{flag:true,flag:false}};}}"),
        "property supplied more than once",
    );
    rejected(
        &format!("{head} fn run()->bool {{let value=keep {{flag:true}};value.missing}}"),
        "unknown record field",
    );
    rejected(
        &format!("{head} fn run() {{keep {{true,flag:false}};}}"),
        "parameter supplied more than once",
    );
    rejected(
        "fn keep(...first:T,...second:U) where T,U {} fn run() {keep {flag:true};}",
        "one variadic entrance",
    );
}

#[test]
fn method_properties_require_a_collector_and_validate_declared_fields() {
    rejected(
        "struct Holder {} impl Holder {fn take(self,.flag:bool=false) {}} fn run() {Holder {}.take {flag:true};}",
        "one variadic entrance",
    );
    let head = "comptime fn schema(...properties:T)->T where T {properties} const SAMPLE=schema {flag:true}; typealias Options=SAMPLE'type; struct Holder {} impl Holder {fn take(self,...properties:Options) {}}";
    rejected(
        &format!("{head} fn run() {{Holder {{}}.take {{flag:true,flag:false}};}}"),
        "property supplied more than once",
    );
    rejected(
        &format!("{head} fn run() {{Holder {{}}.take {{flag:'x'}};}}"),
        "type",
    );
    rejected(
        &format!("{head} fn run() {{Holder {{}}.take {{missing:true}};}}"),
        "type equality",
    );
}

#[test]
fn anonymous_record_places_reject_missing_fields_and_wrong_updates() {
    let head = "fn keep(...properties:T)->T where T {properties}";
    for body in [
        "let value=keep {flag:true};value.missing=false;",
        "let value=keep {flag:true};let p=value.ref;p.missing;",
        "let value=keep {flag:true};let p=value.ref;p.*.missing=false;",
    ] {
        rejected(
            &format!("{head} fn run() {{{body}}}"),
            "unknown record field",
        );
    }
    rejected(
        &format!("{head} fn run() {{let value=keep {{flag:true}};value.flag='x';}}"),
        "type",
    );
    rejected(
        &format!(
            "{head} fn run() {{let value=keep {{flag:true}};let p=value.ref;p.flag.* = 'x';}}"
        ),
        "type",
    );
}

#[test]
fn mixed_property_inputs_reject_duplicate_collection_and_missing_anchors() {
    let head = "fn take(a:bool,...properties:T,b:bool) where T {}";
    rejected(
        &format!("{head} fn run() {{take {{true,flag:true}};}}"),
        "missing anchored parameter",
    );
    rejected(
        &format!("{head} fn run() {{take {{true,false,true,flag:true}};}}"),
        "parameter supplied more than once",
    );
    rejected(
        &format!("{head} fn run() {{take {{true,flag:true,false,flag:false}};}}"),
        "property supplied more than once",
    );
    rejected(
        &format!("{head} fn run() {{take {{'x',flag:true,false}};}}"),
        "type",
    );
}

#[test]
fn property_contexts_reject_overflow_and_do_not_unfold_normal_forms() {
    let schema = "comptime fn schema(...properties:T)->T where T {properties} const SAMPLE=schema {width:do {let n:.u8'builtin=0;n}};typealias Options=SAMPLE'type;";
    rejected(
        &format!(
            "{schema} fn keep(...properties:T)->T where T {{properties}} fn run() {{keep<Options> {{width:256}};}}"
        ),
        "integer does not fit",
    );
    rejected(
        &format!(
            "{schema} fn keep(...properties:T)->(bool,T) where T {{(true,properties)}} fn run()->(bool,Options) {{keep {{width:256}}}}"
        ),
        "integer does not fit",
    );
    rejected(
        &format!(
            "{schema} pure comptime fn Identity<T> -> .Type'builtin {{T}} fn keep(...properties:Identity<Options>) {{}} fn run() {{keep {{width:9}};}}"
        ),
        "structural type view remains unresolved",
    );
}

#[test]
fn property_sibling_contexts_reject_overflow_and_conflicting_record_types() {
    let head = "fn keep(...properties:T)->T where T {properties} fn select(...properties:T,model:T)->T where T {properties}";
    rejected(
        &format!(
            "{head} fn run() {{let n:.u8'builtin=0;let model=keep {{width:n}};select {{width:256,model}};}}"
        ),
        "integer does not fit",
    );
    rejected(
        &format!(
            "{head} fn run() {{let n:.u8'builtin=0;let model=keep {{width:n}};select {{width:'x',model}};}}"
        ),
        "unresolved type equality",
    );
}

#[test]
fn multiple_collection_entries_require_unique_property_routing() {
    let head = "comptime fn schema(...properties:T)->T where T {properties} const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;";
    rejected(
        &format!(
            "{head} fn take(...left:Options,...right:Options) {{}} fn run() {{take {{flag:true}};}}"
        ),
        "one variadic entrance",
    );
    rejected(
        "fn take(...left:(bool,bool),...right:(bool,bool)) {} fn run() {take {true,flag:true,false};}",
        "one variadic entrance",
    );
    rejected(
        "fn take(...left:(bool,bool),...right:(bool,bool)) {} fn run() {take(true,false);}",
        "multiple variadic positional collections",
    );
    rejected(
        "fn take(...left:(bool,bool),...right:(bool,bool)) {} fn run() {take(.left=(true,false),.left=(false,true),true,false);}",
        "parameter supplied more than once",
    );
}

#[test]
fn inherent_generic_methods_reject_incompatible_inputs() {
    rejected(
        "struct Holder {} impl Holder {fn choose(self,a:T,b:T)->T where T {a}} fn run()->bool {Holder {}.choose(true,'x')}",
        "type mismatch",
    );
}

#[test]
fn inherent_generic_methods_do_not_publish_incomplete_instances() {
    rejected(
        "struct Holder {} impl Holder {fn keep(self,value:T)->T where T {value}} fn run() {let value=Holder.keep;}",
        "generic function value requires instantiation",
    );
}

#[test]
fn inherent_generic_methods_require_bound_evidence() {
    rejected(
        "trait Allowed {} struct Holder {} impl Holder {fn keep(self,value:T)->T where T :- Allowed {value}} fn run()->bool {Holder {}.keep(true)}",
        "trait bound is not established",
    );
}

#[test]
fn generic_method_values_reject_missing_bound_evidence() {
    rejected(
        "trait Allowed {} struct Holder {} impl Holder {fn keep(value:T)->T where T :- Allowed {value}} fn run() {let action:fn(bool)->bool=Holder.keep;}",
        "trait bound is not established",
    );
}

#[test]
fn generic_method_values_preserve_fixed_receiver_types() {
    rejected(
        "struct Holder where A {} impl Holder<A> where A {fn keep(self,value:B)->B where B {value}} fn run() {let action:fn(Holder<bool>,bool)->bool=Holder<.u8'builtin>.keep;}",
        "generic function context inference failed",
    );
}

#[test]
fn generic_method_values_do_not_invent_unused_type_arguments() {
    rejected(
        "struct Holder {} impl Holder {fn keep(value:T)->T where T,U {value}} fn run() {let action:fn(bool)->bool=Holder.keep;}",
        "generic function context inference failed",
    );
}

#[test]
fn explicit_generic_methods_check_arity_and_bounds() {
    for call in ["Holder.keep<bool,bool>(true)", "Holder.keep<bool,bool>"] {
        rejected(
            &format!(
                "struct Holder {{}} impl Holder {{fn keep(value:T)->T where T {{value}}}} fn run() {{let value={call};}}"
            ),
            "method type argument count mismatch",
        );
    }
    rejected(
        "trait Allowed {} struct Holder {} impl Holder {fn keep(self,value:T)->T where T :- Allowed {value}} fn run()->bool {Holder {}.keep<bool>(true)}",
        "trait bound is not established",
    );
}

#[test]
fn generic_trait_contracts_match_alpha_renamed_method_binders() {
    let dump = accepted(
        "trait Mapper {fn keep(value:T)->T where T;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U {value}}",
    );
    assert!(dump.contains("(type-parameters"));
    accepted(
        "trait Mapper where A {fn pair(a:A,b:B)->(A,B) where B;} struct Holder where X {} impl Mapper<X> for Holder<X> where X {fn pair(a:X,b:Y)->(X,Y) where Y {(a,b)}}",
    );
    accepted(
        "trait Mapper {fn keep(value:T)->T where T {value}} struct Holder {} impl Mapper for Holder {}",
    );
    accepted(
        "trait Mapper {fn keep(value:T)->T where T {value}} struct Holder {} impl Mapper for Holder {} struct Box where X {} impl Box<X> where X {fn flag()->bool {true}}",
    );
}

#[test]
fn generic_trait_contracts_reject_binder_and_signature_mismatches() {
    rejected(
        "trait Mapper {fn keep(value:T)->T where T;} struct Holder {} impl Mapper for Holder {fn keep(value:bool)->bool {value}}",
        "method generic parameter count",
    );
    rejected(
        "trait Mapper {fn keep(value:T)->T where T;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->bool where U {true}}",
        "does not match its trait signature",
    );
    rejected(
        "trait Mapper {fn keep(value:T)->T where T;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U,requires U==bool {value}}",
        "strengthens trait preconditions",
    );
}

#[test]
fn generic_trait_contracts_preserve_capability_implication() {
    accepted(
        "trait Marker {} trait Mapper {fn keep(value:T)->T where T :- Marker;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U :- Marker {value}}",
    );
    accepted(
        "trait Marker {} trait Mapper {fn keep(value:T)->T where T :- Marker;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U {value}}",
    );
    rejected(
        "trait Marker {} trait Mapper {fn keep(value:T)->T where T;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U :- Marker {value}}",
        "strengthens trait capability requirements",
    );
}

#[test]
fn generic_trait_contracts_use_parent_bounds_and_complete_implementation_world() {
    accepted(
        "trait Parent {} trait Child :- Parent {} trait Mapper {fn keep(value:T)->T where T :- Child;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U :- Parent {value}}",
    );
    for marker in ["impl Marker for bool {}", ""] {
        let source = format!(
            "trait Marker {{}} trait Mapper {{fn keep(value:T)->T where T,requires T==bool;}} struct Holder {{}} impl Mapper for Holder {{fn keep(value:U)->U where U :- Marker,requires U==bool {{value}}}} {marker}"
        );
        if marker.is_empty() {
            rejected(&source, "strengthens trait capability requirements");
        } else {
            accepted(&source);
        }
    }
}

#[test]
fn generic_trait_contracts_publish_inferred_method_arguments() {
    let dump = accepted(
        "trait Mapper {fn keep(value:T)->T where T;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U {value}} fn run()->bool {Holder.as(Mapper).keep(true)}",
    );
    assert!(dump.contains("(method-arguments (primitive bool))"));
    accepted(
        "trait Mapper {fn keep(self,value:T)->T where T;} fn use_mapper(value:S)->bool where S :- Mapper {value.as(Mapper).keep(true)}",
    );
}

#[test]
fn generic_trait_invocation_checks_declared_requirements() {
    rejected(
        "trait Marker {} trait Mapper {fn keep(value:T)->T where T :- Marker;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U {value}} fn run()->bool {Holder.as(Mapper).keep(true)}",
        "trait bound is not established",
    );
    rejected(
        "trait Mapper {fn keep(value:T)->T where T,requires T==bool;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U {value}} fn run()->'x''type {Holder.as(Mapper).keep('x')}",
        "requirement",
    );
}

#[test]
fn generic_trait_invocation_enforces_program_preconditions() {
    rejected(
        "trait Mapper {fn keep(value:T,flag:bool)->T where T,requires flag;} struct Holder {} impl Mapper for Holder {fn keep(value:U,allowed:bool)->U where U {value}} fn run()->bool {Holder.as(Mapper).keep(true,false)}",
        "program precondition is not proven",
    );
}

#[test]
fn generic_trait_method_values_enforce_declared_bounds_and_fixed_types() {
    rejected(
        "trait Marker {} trait Mapper {fn keep(value:T)->T where T :- Marker;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U {value}} fn run() {let action:fn(bool)->bool=Holder.as(Mapper).keep;}",
        "trait bound is not established",
    );
    rejected(
        "trait Mapper {fn keep(value:T)->T where T;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U {value}} fn run() {let action=Holder.as(Mapper).keep;}",
        "generic function value requires instantiation",
    );
    rejected(
        "trait Mapper {fn keep(self,value:T)->T where T;} struct Holder {} impl Mapper for Holder {fn keep(self,value:U)->U where U {value}} fn run() {let action:fn(bool,bool)->bool=Holder.as(Mapper).keep;}",
        "generic function context inference failed",
    );
    rejected(
        "trait Mapper {fn keep(value:T)->T where T;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U {value}} fn run() {let action=Holder.as(Mapper).keep<bool,bool>;}",
        "method type argument count mismatch",
    );
}

#[test]
fn generic_trait_method_values_do_not_erase_program_or_parameter_protocols() {
    rejected(
        "trait Mapper {fn keep(value:T,flag:bool)->T where T,requires flag;} struct Holder {} impl Mapper for Holder {fn keep(value:U,flag:bool)->U where U {value}} fn run() {let action:fn(bool,bool)->bool=Holder.as(Mapper).keep;}",
        "trait method value would erase program contracts",
    );
    rejected(
        "trait Mapper {fn keep(value:T,.flag:bool=true)->T where T;} struct Holder {} impl Mapper for Holder {fn keep(value:U,flag:bool)->U where U {value}} fn run() {let action=Holder.as(Mapper).keep<bool>;}",
        "optional function values require parameter protocol metadata",
    );
}

#[test]
fn associated_values_require_complete_well_typed_implementations() {
    rejected(
        "trait Source {assoc capacity:.u8'builtin;} struct Holder {} impl Source for Holder {}",
        "missing required associated value",
    );
    rejected(
        "trait Source {assoc capacity:.u8'builtin;} struct Holder {} impl Source for Holder {assoc capacity:bool=true;}",
        "does not match its declared type",
    );
    rejected(
        "trait Source {assoc capacity:.u8'builtin;} struct Holder {} impl Source for Holder {assoc capacity:.u8'builtin;}",
        "requires an initializer",
    );
    rejected(
        "trait Source {} struct Holder {} impl Source for Holder {assoc capacity:.u8'builtin=9;}",
        "unknown associated value",
    );
    rejected("assoc capacity:.u8'builtin=9;", "parsing failed");
    rejected(
        "trait Source {assoc capacity:.u8'builtin=300;}",
        "integer does not fit",
    );
}

#[test]
fn associated_values_do_not_confuse_value_and_type_members() {
    rejected(
        "trait Source {assoc capacity:.u8'builtin=9;} struct Holder {} impl Source for Holder {} fn run() {let value:Holder.as(Source).capacity=true;}",
        "associated value used as a type",
    );
    rejected(
        "trait Source {assoc capacity:.u8'builtin;} struct Holder {} impl Source for Holder {fn capacity()->.u8'builtin {9}}",
        "associated value cannot be supplied by a method",
    );
}

#[test]
fn associated_values_do_not_unfold_normal_form_heads() {
    rejected(
        "pure comptime fn Identity<T>->.Type'builtin {T} trait Source {assoc Element:.Type'builtin=Identity<bool>;} struct Holder {} impl Source for Holder {} fn run() {let value:Holder.as(Source).Element=true;}",
        "unresolved type equality",
    );
    rejected(
        "trait Source {assoc capacity:.u8'builtin=9;} struct Holder {} fn run()->.u8'builtin {Holder.as(Source).capacity}",
        "associated value requires an established concrete implementation",
    );
}

#[test]
fn inherited_associated_values_reject_distinct_sources_and_missing_child_views() {
    rejected(
        "trait Left {assoc value:bool=true;} trait Right {assoc value:bool=false;} trait Child :- Left + Right {} struct Holder {} impl Left for Holder {} impl Right for Holder {} impl Child for Holder {} fn run()->bool {Holder.as(Child).value}",
        "ambiguous inherited member",
    );
    rejected(
        "trait Parent {assoc value:bool=true;} trait Child :- Parent {} struct Holder {} impl Parent for Holder {} fn run()->bool {Holder.as(Child).value}",
        "requested interface view is not established",
    );
    rejected(
        "trait Parent where T {assoc value:bool=true;} trait Child :- Parent<bool> + Parent<.u8'builtin> {} struct Holder {} impl Parent<bool> for Holder {} impl Parent<.u8'builtin> for Holder {} impl Child for Holder {} fn run()->bool {Holder.as(Child).value}",
        "ambiguous inherited member",
    );
}

#[test]
fn associated_callable_values_preserve_phase_and_member_contracts() {
    rejected(
        "fn action(value:bool)->bool {value} trait Source {assoc callback:fn(bool)->bool=action;} struct Holder {} impl Source for Holder {} comptime fn run()->bool {Holder.as(Source).callback(true)}",
        "callable type is unavailable in this execution phase",
    );
    rejected(
        "fn action(.value:bool=true)->bool {value} trait Source {assoc callback:fn(bool)->bool=action;} struct Holder {} impl Source for Holder {}",
        "function value",
    );
    rejected(
        "trait Left {assoc value:bool=true;} trait Right {fn value()->bool {false}} trait Child :- Left + Right {} struct Holder {} impl Left for Holder {} impl Right for Holder {} impl Child for Holder {} fn run()->bool {Holder.as(Child).value}",
        "ambiguous inherited member",
    );
}

#[test]
fn comptime_runtime_function_identity_does_not_authorize_invocation() {
    accepted("fn action(value:bool)->bool {value} comptime fn obtain()->fn(bool)->bool {action}");
    rejected(
        "fn action(value:bool)->bool {value} comptime fn bad()->bool {let callback=action;callback(true)}",
        "callable type is unavailable in this execution phase",
    );
    rejected(
        "fn action(value:bool)->bool {value} const RESULT=action(true);comptime fn bad()->bool {RESULT}",
        "runtime function is unavailable during comptime",
    );
}

#[test]
fn computed_runtime_function_identities_reject_calls_and_contract_erasure() {
    rejected(
        "fn action(value:bool)->bool {value} comptime fn choose()->fn(bool)->bool {action} const RESULT=choose()(true);comptime fn run()->bool {RESULT}",
        "callable type is unavailable in this execution phase",
    );
    rejected(
        "fn action(value:bool)->bool where requires value {value} comptime fn choose()->fn(bool)->bool {action} trait Source {assoc action:fn(bool)->bool=choose();} struct Holder {} impl Source for Holder {}",
        "function value would erase program preconditions",
    );
    rejected(
        "trait Allowed {} fn action(value:T)->T where T :- Allowed {value} comptime fn choose()->fn(bool)->bool {action<bool>} const ACTION=choose();comptime fn obtain()->fn(bool)->bool {ACTION}",
        "trait bound is not established",
    );
}

#[test]
fn associated_value_dependencies_reject_cycles_and_failed_initializers() {
    rejected(
        "trait Source {assoc first:bool=Holder.as(Source).second;assoc second:bool=Holder.as(Source).first;} struct Holder {} impl Source for Holder {}",
        "cyclic or excessively deep associated value dependency",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin=Holder.as(Source).Other;assoc Other:.Type'builtin=Holder.as(Source).Element;} struct Holder {} impl Source for Holder {}",
        "cyclic or excessively deep associated value dependency",
    );
    rejected(
        "trait Source {assoc value:bool=true;} struct Holder {} impl Source for Holder {assoc value:bool='x';}",
        "type mismatch",
    );
}

#[test]
fn associated_dependency_depth_is_bounded() {
    for count in [8, 80] {
        let mut source = String::from("trait Source {assoc value:bool;}");
        for index in 0..count {
            let value = if index + 1 == count {
                "true".into()
            } else {
                format!("Node{}.as(Source).value", index + 1)
            };
            source.push_str(&format!(
                "struct Node{index} {{}} impl Source for Node{index} {{assoc value:bool={value};}}"
            ));
        }
        if count == 8 {
            accepted(&source);
        } else {
            rejected(&source, "excessively deep");
        }
    }
}

#[test]
fn computed_trait_method_values_preserve_phase_and_declared_requirements() {
    rejected(
        "trait Mapper {fn keep(value:bool)->bool {value}} struct Holder {} impl Mapper for Holder {} comptime fn choose()->fn(bool)->bool {Holder.as(Mapper).keep} const VALUE=choose()(true);comptime fn run()->bool {VALUE}",
        "callable type is unavailable in this execution phase",
    );
    rejected(
        "trait Mapper {fn keep(value:bool)->bool where requires value;} struct Holder {} impl Mapper for Holder {fn keep(value:bool)->bool {value}} comptime fn choose()->fn(bool)->bool {Holder.as(Mapper).keep} trait Source {assoc action:fn(bool)->bool=choose();} impl Source for Holder {}",
        "trait method value would erase program preconditions",
    );
}

#[test]
fn computed_trait_method_values_do_not_inherit_weaker_implementation_bounds() {
    rejected(
        "trait Allowed {} impl Allowed for bool {} trait Mapper {fn keep(value:T)->T where T :- Allowed;} struct Holder {} impl Mapper for Holder {fn keep(value:U)->U where U {value}} comptime fn choose()->fn('a''type)->'a''type {Holder.as(Mapper).keep<'a''type>} trait Source {assoc action:fn('a''type)->'a''type=choose();} impl Source for Holder {}",
        "trait bound is not established",
    );
}

#[test]
fn associated_callable_values_do_not_recover_declaration_defaults_or_names() {
    let prefix = "trait Choose {fn choose(.flag:bool=true)->bool;} struct Host {} impl Choose for Host {fn choose(value:bool)->bool {value}} trait Source {assoc action:fn(bool)->bool=Host.as(Choose).choose;} struct Holder {} impl Source for Holder {}";
    rejected(
        &format!("{prefix} fn run()->bool {{Holder.as(Source).action()}}"),
        "argument count mismatch",
    );
    rejected(
        &format!("{prefix} fn run()->bool {{Holder.as(Source).action(.flag=true)}}"),
        "named parameter",
    );
}

#[test]
fn associated_variadic_method_values_do_not_recover_collection_entries() {
    rejected(
        "trait Gather {fn gather(...items:(bool,bool))->bool;} struct Host {} impl Gather for Host {fn gather(items:(bool,bool))->bool {let (a,b)=items;a and not b}} trait Source {assoc action:fn((bool,bool))->bool=Host.as(Gather).gather;} struct Holder {} impl Source for Holder {} fn run()->bool {Holder.as(Source).action(true,false)}",
        "argument count mismatch",
    );
}

#[test]
fn repeated_associated_value_queries_respect_the_copy_budget() {
    let prefix = format!(
        "trait Source {{assoc text:\"x\"'type=\"{}\";}} struct Holder {{}} impl Source for Holder {{}}",
        "x".repeat(32768)
    );
    for count in [2, 40] {
        let mut source = prefix.clone();
        for index in 0..count {
            source.push_str(&format!(
                "comptime fn obtain{index}()->\"x\"'type {{Holder.as(Source).text}}"
            ));
        }
        if count == 2 {
            accepted(&source);
        } else {
            rejected(&source, "constant copy budget exhausted");
        }
    }
}

#[test]
fn lexical_associated_references_preserve_cycles_and_value_type_distinction() {
    rejected(
        "trait Source {assoc first:bool;assoc second:bool;} struct Holder {} impl Source for Holder {assoc first:bool=second;assoc second:bool=first;}",
        "cyclic or excessively deep associated value dependency",
    );
    rejected(
        "trait Source {assoc value:bool;fn run();} struct Holder {} impl Source for Holder {assoc value:bool=true;fn run() {let item:value=true;}}",
        "associated value used as a type does not contain a completed Type",
    );
    accepted(
        "trait Source {assoc value:bool=true;fn run()->bool {value}} struct Holder {} impl Source for Holder {assoc value:bool=false;}",
    );
}

#[test]
fn associated_signature_dependencies_reject_cycles_and_contract_mismatches() {
    rejected(
        "trait Source {assoc Element:.Type'builtin;assoc Other:.Type'builtin;fn keep(value:bool)->bool;} struct Holder {} impl Source for Holder {assoc Element:.Type'builtin=Other;assoc Other:.Type'builtin=Element;fn keep(value:Element)->Element {value}}",
        "cyclic or excessively deep associated signature dependency",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin;fn keep(value:bool)->bool;} struct Holder {} impl Source for Holder {assoc Element:.Type'builtin='x''type;fn keep(value:Element)->Element {value}}",
        "does not match its trait signature",
    );
}

#[test]
fn associated_signature_formation_preserves_opaque_heads_and_admission() {
    rejected(
        "pure comptime fn Identity<T>->.Type'builtin {T} trait Source {assoc Element:.Type'builtin;fn keep(value:bool)->bool;} struct Holder {} impl Source for Holder {assoc Element:.Type'builtin=Identity<bool>;fn keep(value:Element)->Element {value}}",
        "does not match its trait signature",
    );
    rejected(
        "trait Source {fn keep(value:bool)->bool;} struct Holder {} impl Source for Holder {assoc Element:.Type'builtin=bool;fn keep(value:Element)->Element {value}}",
        "unknown associated value in implementation",
    );
}

#[test]
fn associated_signature_shared_dependencies_do_not_expand_exponentially() {
    let mut source = String::from("trait Source {");
    for index in 0..16 {
        source.push_str(&format!("assoc Meta{index}:.Type'builtin;"));
    }
    source.push_str("assoc Element:.Type'builtin;fn keep(value:bool)->bool;} struct Holder {} impl Source for Holder {");
    source.push_str("assoc Meta0:.Type'builtin=.Type'builtin;");
    for index in 1..16 {
        let previous = index - 1;
        source.push_str(&format!("assoc Meta{index}:Meta{previous}=Meta{previous};"));
    }
    source.push_str("assoc Element:Meta15=bool;fn keep(value:Element)->Element {value}} fn run()->bool {Holder.as(Source).keep(true)}");
    accepted(&source);
}

#[test]
fn abstract_associated_defaults_do_not_prove_their_own_result_types() {
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;fn value()->Element {true}}",
        "unresolved type equality",
    );
    rejected(
        "trait Source {assoc Element:Element;fn keep(value:Element)->Element {value}}",
        "cyclic or excessively deep associated signature dependency",
    );
}

#[test]
fn selected_associated_method_inputs_reject_the_default_type_after_override() {
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;fn keep(value:Element)->Element {value}} struct Letters {} impl Source for Letters {assoc Element:.Type'builtin='x''type;} fn run()->'x''type {Letters.as(Source).keep(true)}",
        "type mismatch",
    );
}

#[test]
fn associated_override_signature_equations_are_local_acyclic_and_opaque() {
    rejected(
        "trait Source {assoc Element:.Type'builtin;fn keep(value:Element)->Element;} struct Flags {} struct Letters {} impl Source for Flags {assoc Element:.Type'builtin=bool;fn keep(value:bool)->bool {value}} impl Source for Letters {assoc Element:.Type'builtin='x''type;fn keep(value:bool)->bool {value}}",
        "does not match its trait signature",
    );
    rejected(
        "pure comptime fn Identity<T>->.Type'builtin {T} trait Source {assoc Element:.Type'builtin;fn keep(value:Element)->Element;} struct Flags {} impl Source for Flags {assoc Element:.Type'builtin=Identity<bool>;fn keep(value:bool)->bool {value}}",
        "does not match its trait signature",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin = *Element;fn keep(value:Element)->Element;} struct Flags {} impl Source for Flags {fn keep(value:bool)->bool {value}}",
        "cyclic implementation associated type dependency",
    );
}

#[test]
fn explicit_abstract_associated_views_require_evidence_and_preserve_opacity() {
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;} fn keep(value:T.as(Source).Element)->T.as(Source).Element where T {value}",
        "associated value requires an established concrete implementation",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;} fn bad()->T.as(Source).Element where T :- Source {true}",
        "unresolved type equality",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;} trait Child :- Source {} fn keep(value:T.as(Child).Element)->T.as(Source).Element where T :- Source {value}",
        "requested interface view is not established",
    );
    rejected(
        "trait Left {assoc Element:.Type'builtin=bool;} trait Right {assoc Element:.Type'builtin=bool;} fn bad(value:T.as(Left).Element)->T.as(Right).Element where T :- Left + Right {value}",
        "unresolved type equality",
    );
    accepted(
        "trait Source {assoc value:bool=true;} fn read()->bool where T :- Source {T.as(Source).value}",
    );
}

#[test]
fn declaration_associated_views_reject_missing_constraints_and_ambiguous_origins() {
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;} trait Apply {fn keep(value:T.as(Source).Element)->T.as(Source).Element where T;}",
        "declaration associated projection requires a given interface constraint",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;} trait Child :- Source {} trait Apply {fn keep(value:T.as(Child).Element)->T.as(Source).Element where T :- Source;}",
        "declaration associated projection requires a given interface constraint",
    );
    rejected(
        "trait Source where A {assoc Element:.Type'builtin=A;} trait Left :- Source<bool> {} trait Right :- Source<'x''type> {} trait Child :- Left + Right {fn keep(value:Self.as(Child).Element)->bool;}",
        "ambiguous inherited member",
    );
    rejected(
        "trait Source {assoc Element:Self.as(Source).Element;fn keep(value:Self.as(Source).Element)->bool;}",
        "cyclic associated signature dependency",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin;fn keep(value:Self.as(Source).Element)->Self.as(Source).Element;} struct Flags {} impl Source for Flags {assoc Element:.Type'builtin='x''type;fn keep(value:bool)->bool {value}}",
        "does not match its trait signature",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;fn bad()->Self.as(Source).Element {true}}",
        "unresolved type equality",
    );
}

#[test]
fn declaration_associated_views_do_not_export_override_body_equations() {
    rejected(
        "trait Source {assoc Element:.Type'builtin;fn make()->Self.as(Source).Element;} struct Flags {} struct Letters {} impl Source for Flags {assoc Element:.Type'builtin=bool;fn make()->Self.as(Source).Element {true}} impl Source for Letters {assoc Element:.Type'builtin='x''type;fn make()->Self.as(Source).Element {true}}",
        "unresolved type equality",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin;fn make()->Self.as(Source).Element;} struct Flags {} impl Source for Flags {assoc Element:.Type'builtin=bool;fn make()->Self.as(Source).Element {true}} fn bad()->T.as(Source).Element where T :- Source {true}",
        "unresolved type equality",
    );
}

#[test]
fn associated_method_value_contexts_preserve_selected_types_and_phase() {
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;fn keep(value:Element)->Element {value}} struct Letters {} impl Source for Letters {assoc Element:.Type'builtin='x''type;} fn run()->bool {let action:fn(bool)->bool=Letters.as(Source).keep;action(true)}",
        "type mismatch",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;comptime fn keep(value:Element)->Element {value}} struct Flags {} impl Source for Flags {} fn run()->bool {let action:fn(bool)->bool=Flags.as(Source).keep;action(true)}",
        "type mismatch",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;fn keep(value:Element,other:T)->Element where T {value}} struct Letters {} impl Source for Letters {assoc Element:.Type'builtin='x''type;} fn run()->bool {let action:fn(bool,'x''type)->bool=Letters.as(Source).keep;action(true,'x')}",
        "generic function context inference failed",
    );
}

#[test]
fn associated_function_value_contexts_do_not_invent_evidence_or_erase_phase() {
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;} struct Letters {} impl Source for Letters {assoc Element:.Type'builtin='x''type;} fn keep(value:T.as(Source).Element)->T.as(Source).Element where T :- Source {value} fn run()->bool {let action:fn(bool)->bool=keep<Letters>;action(true)}",
        "type mismatch",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;} struct Missing {} fn keep(value:T.as(Source).Element)->T.as(Source).Element where T :- Source {value} fn run()->bool {let action:fn(bool)->bool=keep<Missing>;action(true)}",
        "trait bound is not established",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;} struct Flags {} impl Source for Flags {} comptime fn keep(value:T.as(Source).Element)->T.as(Source).Element where T :- Source {value} fn run()->bool {let action:fn(bool)->bool=keep<Flags>;action(true)}",
        "type mismatch",
    );
}

#[test]
fn inferred_associated_calls_reject_wrong_values_and_do_not_invert_projections() {
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;} struct Letters {} impl Source for Letters {assoc Element:.Type'builtin='x''type;} fn keep(owner:T,value:T.as(Source).Element)->T.as(Source).Element where T :- Source {value} fn run()->bool {keep(Letters {},true)}",
        "generic call inference failed",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;} struct Flags {} impl Source for Flags {} fn keep(value:T.as(Source).Element)->T.as(Source).Element where T :- Source {value} fn run()->bool {keep(true)}",
        "generic call inference failed",
    );
}

#[test]
fn associated_spreads_reject_wrong_packet_types_and_widths() {
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;fn apply(action:fn(Element)->Element,value:Element)->Element {action(value)}} struct Letters {} impl Source for Letters {assoc Element:.Type'builtin='x''type;} fn run()->'x''type {Letters.as(Source).apply(...do {(|value:bool| value,true)})}",
        "type mismatch",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;fn keep(value:Element)->Element {value}} struct Flags {} impl Source for Flags {} fn run()->bool {Flags.as(Source).keep(...do {(true,false)})}",
        "argument count mismatch",
    );
}

#[test]
fn joint_spread_inputs_reject_context_conflicts_and_missing_evidence() {
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;} struct Letters {} impl Source for Letters {assoc Element:.Type'builtin='x''type;} fn apply(owner:T,action:fn(T.as(Source).Element)->T.as(Source).Element,value:T.as(Source).Element)->T.as(Source).Element where T :- Source {action(value)} fn run()->'x''type {apply(Letters {},...do {(|value:bool| value,true)})}",
        "type mismatch",
    );
    rejected(
        "trait Source {assoc Element:.Type'builtin=bool;} struct Missing {} fn apply(owner:T,action:fn(T.as(Source).Element)->T.as(Source).Element,value:T.as(Source).Element)->T.as(Source).Element where T :- Source {action(value)} fn run()->bool {apply(Missing {},...do {(|value| value,true)})}",
        "associated Type implementation selection is unresolved",
    );
}

#[test]
fn abstract_associated_scalars_do_not_become_types_or_unproved_capabilities() {
    rejected(
        "trait Flag {assoc enabled:bool=true;} fn read()->bool where T {T.as(Flag).enabled}",
        "associated value requires an established concrete implementation",
    );
    rejected(
        "trait Flag {assoc enabled:bool=true;fn invalid()->enabled;}",
        "associated value used as a type does not contain a completed Type",
    );
    rejected(
        "trait Source {assoc Item:.Type'builtin;assoc initial:Item;} struct Flags {} impl Source for Flags {assoc Item:.Type'builtin=bool;assoc initial:'x''type='x';}",
        "does not match its declared type",
    );
    rejected(
        "pure comptime fn Identity<T>->.Type'builtin {T} trait Source {assoc Item:.Type'builtin;assoc initial:Item;} struct Flags {} impl Source for Flags {assoc Item:.Type'builtin=Identity<bool>;assoc initial:bool=true;}",
        "does not match its declared type",
    );
}

#[test]
fn associated_callback_selection_does_not_authorize_runtime_invocation_at_comptime() {
    rejected(
        "fn invert(value:bool)->bool {not value} trait Action {assoc callback:fn(bool)->bool=invert;} impl Action for bool {} comptime fn invoke(value:T)->bool where T :- Action {T.as(Action).callback(false)} trait Result {assoc value:bool=invoke(true);} impl Result for bool {}",
        "callable type is unavailable in this execution phase",
    );
}

#[test]
fn dependent_associated_initializers_reject_selected_cycles_and_runtime_calls() {
    rejected(
        "trait Flags {assoc a:bool=not b;assoc b:bool=not a;} impl Flags for bool {}",
        "cyclic",
    );
    rejected(
        "trait Flags {assoc a:bool=Self.as(Flags).a;} impl Flags for bool {}",
        "cyclic",
    );
    rejected(
        "fn compute(value:bool)->bool {value} trait Flags {assoc a:bool=true;assoc b:bool=compute(a);}",
        "runtime",
    );
}

#[test]
fn inherited_associated_initializers_do_not_supply_missing_definition_evidence() {
    rejected(
        "trait Other {assoc seed:bool=true;} trait Child {assoc value:bool=Self.as(Other).seed;} impl Other for bool {} impl Child for bool {}",
        "associated value requires an established concrete implementation or a given interface constraint",
    );
    rejected(
        "trait Allowed {} impl Allowed for bool {} trait Parent {assoc seed:bool=false;} trait Child :- Parent {assoc derived:bool=not Self.as(Parent).seed;} struct Host where X {} impl Parent for Host<X> where X :- Allowed {} impl Child for Host<X> where X :- Allowed {} fn leak(value:T)->bool where T {T.as(Parent).seed}",
        "associated value requires an established concrete implementation or a given interface constraint",
    );
}

#[test]
fn associated_initializer_cycles_through_comptime_calls_are_rejected() {
    rejected(
        "comptime fn read()->bool where T :- Flags {T.as(Flags).value} trait Flags {assoc value:bool=read<Self>();} impl Flags for bool {}",
        "cyclic",
    );
}

#[test]
fn associated_type_predicates_reject_wrong_values_and_opaque_head_unfolding() {
    for value in ["'x''type", "Identity<bool>"] {
        rejected(
            &format!(
                "pure comptime fn Identity<T>->.Type'builtin {{T}} trait Source {{assoc Item:.Type'builtin;}} impl Source for bool {{assoc Item:.Type'builtin={value};}} comptime fn agrees()->bool where T :- Source, requires T.as(Source).Item == bool {{true}} trait Result {{assoc value:bool=agrees<bool>();}} impl Result for bool {{}}"
            ),
            "type requirement",
        );
    }
}

#[test]
fn associated_projection_initializer_order_does_not_accept_wrong_selected_types() {
    let result = "trait Result {assoc value:bool=read<bool>(true);} impl Result for bool {}";
    let source = "trait Source {assoc Item:.Type'builtin=bool;} impl Source for bool {assoc Item:.Type'builtin='x''type;}";
    let function =
        "comptime fn read(value:T.as(Source).Item)->T.as(Source).Item where T :- Source {value}";
    for declarations in [[source, result], [result, source]] {
        rejected(
            &format!("{} {function}", declarations.join(" ")),
            "type mismatch",
        );
    }
}

#[test]
fn projected_trait_subjects_require_evidence_for_the_projected_type() {
    rejected(
        "trait Source {assoc Item:.Type'builtin=bool;} impl Source for bool {} trait Read {fn read()->bool;} impl Read for bool {fn read()->bool {true}} fn forward()->bool where T :- Source {T.as(Source).Item.as(Read).read()}",
        "method implementation is not uniquely established",
    );
    rejected(
        "trait Source {assoc Item:.Type'builtin=bool;} impl Source for bool {assoc Item:.Type'builtin='x''type;} trait Read {fn read()->bool;} impl Read for bool {fn read()->bool {true}} fn forward()->bool where T :- Source, U :- Read, requires U == T.as(Source).Item {T.as(Source).Item.as(Read).read()} fn run()->bool {forward<bool,bool>()}",
        "type requirement is not satisfied",
    );
}

#[test]
fn positive_requires_capabilities_are_checked_at_calls_and_nominal_formation() {
    rejected(
        "trait Read {} fn need() where T, requires T :- Read {} fn run() {need<bool>();}",
        "trait bound is not established",
    );
    rejected(
        "trait Read {} struct Box where T, requires T :- Read {value:T} fn run() {Box<bool> {value:true};}",
        "trait bound is not established",
    );
    rejected(
        "trait Read {} trait Tag {} impl Read for bool {} fn need() where T, requires T :- Read + Tag {} fn run() {need<bool>();}",
        "trait bound is not established",
    );
}

#[test]
fn boolean_trait_requirements_reject_unsupported_or_unproven_calls() {
    rejected(
        "trait Disabled {} impl Disabled for bool {} fn deny()->bool where T, requires not (T :- Disabled) {true} fn run()->bool {deny<bool>()}",
        "type requirement",
    );
    rejected(
        "trait Disabled {} fn deny()->bool where T, requires not (T :- Disabled) {true} fn forward()->bool where U {deny<U>()}",
        "type requirement remains unresolved",
    );
    rejected(
        "trait Read {fn read()->bool;} trait Write {} fn either()->bool where T, requires (T :- Read) or (T :- Write) {T.as(Read).read()}",
        "method implementation is not uniquely established",
    );
    rejected(
        "trait Read {} trait Write {} fn either()->bool where T, requires (T :- Read) or (T :- Write) {true} fn run()->bool {either<bool>()}",
        "type requirement",
    );
    rejected(
        "trait Disabled {} impl Disabled for bool {} struct Box where T, requires not (T :- Disabled) {value:T} fn run() {Box<bool> {value:true};}",
        "type requirement",
    );
}

#[test]
fn conditional_impl_requirements_reject_false_and_recursive_evidence() {
    rejected(
        "trait Disabled {} impl Disabled for bool {} struct Box where T {} trait Read {fn read()->bool;} impl Read for Box<T> where T, requires not (T :- Disabled) {fn read()->bool {true}} fn run()->bool {Box<bool>.as(Read).read()}",
        "method implementation is not uniquely established",
    );
    for condition in ["T :- Loop", "not (T :- Loop)"] {
        rejected(
            &format!(
                "trait Loop {{}} impl Loop for T where T, requires {condition} {{}} struct Box where T {{}} trait Read {{fn read()->bool;}} impl Read for Box<T> where T, requires (T :- Loop) or false {{fn read()->bool {{true}}}} fn run()->bool {{Box<bool>.as(Read).read()}}"
            ),
            "method implementation is not uniquely established",
        );
    }
}

#[test]
fn conditional_inherent_impl_does_not_infer_absence_for_open_or_recursive_queries() {
    rejected(
        "trait Disabled {} struct Box where T {} impl Box<T> where T, requires not (T :- Disabled) {fn read()->bool {true}} fn forward()->bool where U {Box<U>.read()}",
        "inherent method applicability is unresolved",
    );
    for condition in ["T :- Loop", "not (T :- Loop)"] {
        rejected(
            &format!(
                "trait Loop {{}} impl Loop for T where T, requires {condition} {{}} struct Box where T {{}} impl Box<T> where T, requires not (T :- Loop) {{fn read()->bool {{true}}}} fn run()->bool {{Box<bool>.read()}}"
            ),
            "inherent method applicability is unresolved",
        );
    }
}

#[test]
fn method_signature_bounds_do_not_grant_capabilities_to_callers() {
    rejected(
        "trait Source {} trait Service {fn extra()->bool where requires Self :- Source {true}} impl Service for bool {} fn entry()->bool {bool.as(Service).extra()}",
        "trait bound is not established",
    );
    rejected(
        "trait Source {assoc Item:.Type'builtin;} trait Other {} fn value(input:T.as(Source).Item) where T, requires (T :- Source) or (T :- Other) {}",
        "given interface constraint",
    );
}

#[test]
fn boolean_method_requirements_reject_false_and_unproved_conditions() {
    for implementations in ["impl Disabled for bool {} impl Enabled for bool {}", ""] {
        rejected(
            &format!(
                "trait Disabled {{}} trait Enabled {{}} trait Other {{}} {implementations} trait Service {{fn ready()->bool where requires not (Self :- Disabled) and ((Self :- Enabled) or (Self :- Other)) {{true}}}} impl Service for bool {{}} fn entry()->bool {{bool.as(Service).ready()}}"
            ),
            "trait method precondition is not established",
        );
    }
    rejected(
        "trait Disabled {} trait Service {fn ready()->bool where requires not (Self :- Disabled) {true}} fn forward()->bool where T :- Service {T.as(Service).ready()}",
        "trait method precondition is not established",
    );
}

#[test]
fn trait_absence_queries_do_not_guess_equalities_or_ignore_matching_impls() {
    rejected(
        "trait Disabled {} impl Disabled for bool {} fn allowed()->bool where T, requires not (T :- Disabled) {true} fn forward()->bool where T, requires T == bool {allowed<T>()}",
        "type requirement",
    );
    rejected(
        "trait Disabled {} fn allowed()->bool where T, requires not (T :- Disabled) {true} fn forward()->bool where T, requires (T == bool) or (T == 'x''type) {allowed<T>()}",
        "type requirement",
    );
}

#[test]
fn effect_computation_labels_require_valid_instances() {
    rejected(
        "effect echo(value:T)->T where T, requires T == bool; fn take(value:#[echo<'x''type>] bool) {}",
        "type requirement",
    );
    rejected(
        "effect enum IO where T, requires T == bool {read()->T,} fn take(value:#IO<'x''type> bool) {}",
        "type requirement",
    );
    rejected(
        "effect enum IO where T, requires T == bool {read()->T,} fn take(value:#IO<'x''type>.read bool) {}",
        "type requirement",
    );
    rejected(
        "trait Marker {} effect echo(value:T)->T where T :- Marker; fn take(value:#[echo<bool>] bool) {}",
        "trait bound is not established",
    );
    accepted(
        "trait Marker {} impl Marker for bool {} effect echo(value:T)->T where T :- Marker; fn take(value:#[echo<bool>] bool) {}",
    );
    accepted(
        "effect echo(value:T)->T where T, requires T == bool; fn take(value:#[echo<bool>] bool) {}",
    );
    accepted(
        "effect enum IO where T, requires T == bool {read()->T,} fn take(value:#IO<bool> bool) {}",
    );
    rejected(
        "effect echo(value:T)->T where T; fn take(value:#echo bool) {}",
        "effect operation Type argument count mismatch",
    );
    rejected(
        "effect enum IO where T {read()->T,} fn take(value:#IO bool) {}",
        "effect group Type argument count mismatch",
    );
    rejected(
        "struct Data {} fn take(value:#Data bool) {}",
        "effect row entry is not an effect operation or group",
    );
    rejected(
        "effect enum IO {read()->bool,} fn take(value:#IO.missing bool) {}",
        "unknown effect group operation",
    );
}

#[test]
fn effect_row_equality_uses_scoped_label_and_generic_type_evidence() {
    accepted(
        "effect a()->bool; effect b()->bool; fn reorder(value:#[a,b,a] bool)->#[b,a,a] bool {value}",
    );
    accepted(
        "effect echo(value:T)->T where T; fn reorder(value:#[echo<A>,echo<B>] bool)->#[echo<B>,echo<A>] bool where A, B, requires not (A == B) {value}",
    );
    accepted(
        "effect echo(value:T)->T where T; fn reorder(value:#[echo<bool>,echo<'x''type>] bool)->#[echo<'x''type>,echo<bool>] bool {value}",
    );
    rejected(
        "effect a()->bool; effect b()->bool; fn erase(value:#[a,b,a] bool)->#[b,a] bool {value}",
        "effect request has no complete handler route",
    );
    rejected(
        "effect echo(value:T)->T where T; fn change(value:#[echo<A>] bool)->#[echo<B>] bool where A, B {value}",
        "effect operation identity needs further evidence",
    );
}

#[test]
fn effect_qualified_types_are_operands_of_outer_type_requirements() {
    accepted(
        "effect a()->bool; fn same()->bool where requires #a bool == #a bool {true} fn run()->bool {same()}",
    );
    accepted(
        "effect a()->bool; effect b()->bool; fn same()->bool where requires #[a,b] bool == #[b,a] bool {true} fn run()->bool {same()}",
    );
    accepted(
        "effect a()->bool; fn same()->bool where requires #a fn()->bool == (#a fn()->bool) {true} fn run()->bool {same()}",
    );
    rejected(
        "effect a()->bool; effect b()->bool; fn wrong()->bool where requires #a bool == #b bool {true} fn run()->bool {wrong()}",
        "type requirement",
    );
}

#[test]
fn generic_calls_infer_effect_results_instances_and_residual_rows() {
    accepted(
        "effect a()->bool; fn retain(value:#a T)->#a T where T {value} fn caller(value:#a bool)->#a bool {retain(value)}",
    );
    accepted(
        "effect a()->bool; effect b()->bool; fn retain(value:#a T)->#a T where T {value} fn caller(value:#[b,a] bool)->#[b,a] bool {retain(value)}",
    );
    accepted(
        "effect echo(value:T)->T where T; fn retain(value:#[echo<T>,echo<bool>] bool)->#[echo<T>,echo<bool>] bool where T {value} fn caller(value:#[echo<bool>,echo<'x''type>] bool)->#[echo<bool>,echo<'x''type>] bool {retain(value)}",
    );
    accepted(
        "effect echo(value:T)->T where T; fn keep(value:#[echo<T>,echo<U>] bool,witness:(T,U))->(T,U) where T,U {witness} fn caller(value:#[echo<bool>,echo<'x''type>] bool)->(bool,'x''type) {keep(value,(true,'x'))}",
    );
}

#[test]
fn source_effect_requests_require_allowed_rows_and_typed_operation_inputs() {
    accepted(
        "trait Marker {} effect echo(value:T)->T where T :- Marker; fn generic(value:T)->#[echo<T>] T where T :- Marker {echo<T>(value)#}",
    );
    rejected(
        "effect read()->bool; fn missing()->bool {read()#}",
        "effect request requires an explicit allowed effect row",
    );
    rejected(
        "effect a()->bool; effect b()->bool; fn missing()->#a bool {b()#}",
        "effect request has no complete handler route",
    );
    rejected(
        "effect echo(value:bool)->bool; fn bad()->#echo bool {echo('x')#}",
        "type mismatch",
    );
    rejected(
        "effect echo(value:bool)->bool; fn bad()->#echo bool {echo()#}",
        "effect request argument count mismatch",
    );
    rejected(
        "effect read()->bool; comptime fn bad()->#read bool {read()#}",
        "comptime effect requests are not implemented",
    );
}

#[test]
fn source_effect_requests_do_not_call_groups_or_capture_unannotated_lambda_environments() {
    rejected(
        "effect enum IO {read()->bool,} fn bad()->#IO bool {IO()#}",
        "effect group is not an operation",
    );
    rejected(
        "effect read()->bool; fn bad()->#read bool {let closure=|| read()#; read()#}",
        "effect request requires an explicit allowed effect row",
    );
}

#[test]
fn effect_results_keep_machine_arithmetic_checks_and_allow_post_request_guards() {
    rejected(
        "typealias Byte=.u8'builtin; effect read()->Byte; fn overflow()->#read Byte {read()# + 1}",
        "machine arithmetic range is not proven",
    );
    accepted(
        "typealias Byte=.u8'builtin; effect read()->Byte; fn guarded()->#read Byte {let value=read()#; if value < 255 {value + 1} else {0}}",
    );
    rejected(
        "effect a()->bool; effect b()->bool; fn inner()->#b bool {b()#} fn outer()->#a bool {inner()}",
        "effect request has no complete handler route",
    );
}

#[test]
fn direct_passing_declarations_reject_arguments_async_and_missing_outer_supply() {
    rejected(
        "^attrs.effects.direct_passing effect bad(value:bool)->bool;",
        "direct passing effects must be synchronous and have no parameters",
    );
    rejected(
        "^attrs.effects.direct_passing async effect bad()->bool;",
        "direct passing effects must be synchronous and have no parameters",
    );
    rejected(
        "effect enum Config {^attrs.effects.direct_passing bad(value:bool)->bool,}",
        "direct passing effects must be synchronous and have no parameters",
    );
    rejected(
        "^attrs.effects.direct_passing effect setting()->bool; fn run()->bool {setting()# {setting() if false=>true,}}",
        "partial effect handler requires an outer entry",
    );
    rejected(
        "^attrs.effects.direct_passing effect setting()->bool; fn run()->bool {setting()# {setting()=>setting()#,}}",
        "explicit allowed effect row",
    );
    rejected(
        "^attrs.effects.direct_passing effect setting()->bool; fn run()->bool {setting()# {setting()=>'x',}}",
        "type",
    );
    accepted(
        "^attrs.effects.direct_passing inline effect setting()->bool; fn run()->bool {setting()# {setting()=>true,}}",
    );
}

#[test]
fn direct_passing_installation_invalidates_captured_arithmetic_facts_before_subject() {
    rejected(
        "typealias Byte=.u8'builtin; ^attrs.effects.direct_passing effect setting()->bool; fn run()->Byte {var value:Byte=0; do {value + 1}# {setting()=>{value=255;true},}}",
        "machine arithmetic range is not proven",
    );
    accepted(
        "typealias Byte=.u8'builtin; ^attrs.effects.direct_passing effect setting()->bool; fn run()->Byte {var value:Byte=0; do {if value < 255 {value + 1} else {0}}# {setting()=>{value=255;true},}}",
    );
}

#[test]
fn direct_effect_handlers_reject_wrong_signatures_and_missing_outer_routes() {
    rejected(
        "effect read()->bool; fn source()->#read bool {read()#} fn run()->bool {source()# {read(value)=>true,}}",
        "effect handler parameter count mismatch",
    );
    rejected(
        "effect read()->bool; fn source()->#read bool {read()#} fn run()->bool {source()# {read()=>'x',}}",
        "type",
    );
    rejected(
        "effect read(value:bool)->bool; fn source()->#read bool {read(true)#} fn run()->bool {source()# {read(true)=>false,}}",
        "partial effect handler requires an outer entry",
    );
    rejected(
        "effect read()->bool; fn source()->#read bool {read()#} fn run()->bool {source()# {read()=>read()#,}}",
        "explicit allowed effect row",
    );
    rejected(
        "effect read()->bool; fn source()->#read bool {read()#} fn run()->bool {source()# {read() if false=>true,}}",
        "partial effect handler requires an outer entry",
    );
}

#[test]
fn direct_effect_handlers_check_arithmetic_inside_branches_without_capture_facts() {
    let prefix = "typealias Byte=.u8'builtin; effect read(value:Byte)->Byte; fn source()->#read Byte {read(0)#}";
    accepted(&format!(
        "{prefix} fn run()->Byte {{source()# {{read(value)=>if value < 255 {{value + 1}} else {{0}},}}}}"
    ));
    rejected(
        &format!("{prefix} fn run()->Byte {{source()# {{read(value)=>value + 1,}}}}"),
        "machine arithmetic range is not proven",
    );
    rejected(
        &format!(
            "{prefix} fn run()->Byte {{var captured:Byte=0; source()# {{read(value)=>captured + 1,}}}}"
        ),
        "machine arithmetic range is not proven",
    );
    accepted(&format!(
        "{prefix} fn run()->#read Byte {{source()# {{read(value) if value < 255=>value + 1,}}}}"
    ));
}

#[test]
fn direct_effect_handlers_preserve_lexical_return_and_operation_result_distinction() {
    accepted(
        "effect read()->bool; fn source()->#read bool {read()#} fn run()->'x''type {source()# {read()=>return 'x',}; 'y'}",
    );
    rejected(
        "effect read()->bool; fn source()->#read bool {read()#} fn run()->'x''type {source()# {read()=>return true,}; 'y'}",
        "type",
    );
    rejected(
        "effect enum IO {read()->bool,} fn source()->#IO bool {IO.read()#} fn run()->bool {source()# {IO()=>true,}}",
        "effect group is not an operation",
    );
    rejected(
        "effect a()->bool; effect b()->bool; fn source()->#[a,b] bool {a()# and b()#} fn run()->bool {source()# {a()=>b()#, b()=>true,}}",
        "explicit allowed effect row",
    );
    rejected(
        "effect read()->bool; fn source()->#read bool {read()#} fn run()->bool {source()# {read()=> {let f=|| read()#; true},}}",
        "explicit allowed effect row",
    );
}

#[test]
fn direct_effect_handlers_resume_only_the_lexical_branch_and_do_not_cross_closures() {
    accepted("effect read()->bool; fn run()->bool {read()# {read()=> {resume true; false},}}");
    accepted(
        "effect read()->bool; fn run()->bool {read()# {read()=> {resume true while false; false},}}",
    );
    accepted("effect write(); fn run() {write()# {write()=>resume,}}");
    accepted("effect read()->bool; fn run()->bool {let f=|| read()# {read()=>true,}; f()}");
    accepted(
        "effect a()->bool; effect b()->'x''type; fn run()->bool {a()# {a()=> {b()# {b()=>resume 'x',}; resume true},}}",
    );
    rejected(
        "effect read()->bool; fn run()->bool {read()# {read()=>resume 'x',}}",
        "type",
    );
    rejected(
        "effect read()->bool; fn run()->bool {read()# {read()=> {let f=|| {resume true}; false},}}",
        "resume requires an enclosing direct effect branch",
    );
    rejected(
        "fn run()->bool {resume true}",
        "resume requires an enclosing direct effect branch",
    );
    rejected(
        "effect read()->bool; fn run()->bool {read()# {read()=>true,}; resume true}",
        "resume requires an enclosing direct effect branch",
    );
}

#[test]
fn direct_effect_handlers_group_labels_only_with_scoped_identity_evidence() {
    let prefix = "effect echo(value:T)->T where T;";
    rejected(
        &format!(
            "{prefix} fn run(value:T)->T where T,U {{echo<T>(value)# {{echo<T>(x)=>x, echo<U>(y)=>y,}}}}"
        ),
        "effect operation identity needs further evidence",
    );
    accepted(&format!(
        "{prefix} fn run(value:T)->T where T,U,requires T == U {{echo<T>(value)# {{echo<T>(x)=>x, echo<U>(y)=>y,}}}}"
    ));
}

#[test]
fn source_effect_signature_expansion_is_bounded_before_argument_or_branch_checking() {
    let parameter = format!("({})", vec!["T"; 256].join(","));
    let argument = format!("({})", vec!["bool"; 512].join(","));
    let prefix = format!("effect expanded(value:{parameter})->bool where T;");
    // No argument is needed to trigger the request signature guard. It must
    // reject expansion before reporting the later source arity mismatch.
    rejected(
        &format!("{prefix} fn request()->#[expanded<{argument}>] bool {{expanded<{argument}>()#}}"),
        "effect routing budget exhausted",
    );
    // An unused handler must still form its operation signature safely.
    rejected(
        &format!("{prefix} fn install()->bool {{true# {{expanded<{argument}>(_)=>true,}}}}"),
        "effect routing budget exhausted",
    );
    accepted(
        "effect echo(value:T)->T where T; fn run()->bool {echo<bool>(true)# {echo<bool>(value)=>value,}}",
    );
}

#[test]
fn source_effect_handlers_establish_operation_contracts_even_when_unused() {
    let prefix = "trait Marker {} effect echo(value:T)->T where T :- Marker;";
    rejected(
        &format!("{prefix} fn run()->bool {{true# {{echo<bool>(value)=>value,}}}}"),
        "trait bound is not established",
    );
    accepted(&format!(
        "{prefix} impl Marker for bool {{}} fn run()->bool {{true# {{echo<bool>(value)=>value,}}}}"
    ));
    accepted(&format!(
        "{prefix} fn run(value:T)->T where T :- Marker {{echo<T>(value)# {{echo<T>(item)=>item,}}}}"
    ));
}

#[test]
fn generic_effect_requests_reject_wrong_rows_missing_context_and_unproved_contracts() {
    rejected(
        "typealias Char='x''type; effect echo(value:T)->T where T; fn run()->#[echo<bool>] Char {echo('x')#}",
        "effect request has no complete handler route",
    );
    rejected(
        "effect choose()->T where T; fn run()->#[choose<bool>] bool {choose()#;true}",
        "inference",
    );
    rejected(
        "trait Mark {} effect echo(value:T)->T where T :- Mark; fn run()->bool {echo(true)# {echo<bool>(x)=>x,}}",
        "trait bound is not established",
    );
    accepted(
        "trait Mark {} effect echo(value:T)->T where T :- Mark; fn run(value:T)->#[echo<T>] T where T :- Mark {echo(value)#}",
    );
    rejected(
        "effect echo(value:T)->T where T; fn run()->#[echo<bool>] bool {echo(true,false)#}",
        "effect request argument count mismatch",
    );
}

#[test]
fn trait_type_defaults_reject_invalid_argument_routes_and_values() {
    let prefix = "typealias Type=.Type'builtin;";
    for (source, diagnostic) in [
        (
            "trait Pick where .T:Type=bool {} impl Pick<bool> for bool {}",
            "argument count mismatch",
        ),
        (
            "trait Pick where .T:Type=bool {} impl Pick<.Other=bool> for bool {}",
            "unknown named type argument",
        ),
        (
            "trait Pick where .T:Type=bool {} impl Pick<.T=bool,.T=bool> for bool {}",
            "duplicate type argument",
        ),
        (
            "trait Pick where T,.U:Type=bool {} impl Pick for bool {}",
            "argument count mismatch",
        ),
        (
            "trait Pick where .T:Type=T {} impl Pick for bool {}",
            "type default references",
        ),
        (
            "trait Pick where .T:Type=false {} impl Pick for bool {}",
            "type",
        ),
    ] {
        rejected(&format!("{prefix}{source}"), diagnostic);
    }
}

#[test]
fn trait_type_default_definitions_are_checked_even_when_unused_or_overridden() {
    for source in [
        "trait Pick where .T:.Type'builtin=false {}",
        "trait Pick where .T:.Type'builtin=false {} impl Pick<.T=bool> for bool {}",
    ] {
        rejected(source, "type");
    }
    for source in [
        "trait Pick where .T:.Type'builtin=T {}",
        "comptime fn Identity(value:.Type'builtin)->.Type'builtin {value} trait Pick where .T:.Type'builtin=Identity(T) {}",
    ] {
        rejected(source, "type default references");
    }
}

#[test]
fn nominal_type_defaults_reject_invalid_declarations_and_routes() {
    for (source, diagnostic) in [
        (
            "trait Mark {} impl Mark for bool {} struct Holder where .T:.Type'builtin=bool,requires T :- Mark {value:T,} fn bad()->Holder<.T='x''type> {Holder<.T='x''type> {value:'x'}}",
            "trait bound is not established",
        ),
        ("struct Holder where .T:.Type'builtin=false {}", "type"),
        (
            "enum Choice where .T:.Type'builtin=T {empty,}",
            "type default references",
        ),
        (
            "struct Holder where .T:.Type'builtin=bool {} typealias Bad=Holder<bool>;",
            "argument count mismatch",
        ),
        (
            "struct Holder where .T:.Type'builtin=bool {} typealias Bad=Holder<.T=bool,.T=bool>;",
            "duplicate type argument",
        ),
        (
            "struct Holder where .T:.Type'builtin=bool {} typealias Bad=Holder<.Other=bool>;",
            "unknown named type argument",
        ),
        (
            "struct Holder where A,.T:.Type'builtin=bool {} typealias Bad=Holder;",
            "requires explicit type arguments",
        ),
        (
            "struct Holder where .T:.Type'builtin=bool {value:T,} fn bad()->Holder {Holder {value:'x'}}",
            "type",
        ),
        (
            "struct Holder where .T:.Type'builtin=bool {value:T,} fn bad()->Holder<.T='x''type> {Holder {value:'x'}}",
            "type",
        ),
    ] {
        rejected(source, diagnostic);
    }
}

#[test]
fn typealias_defaults_reject_invalid_unused_defaults_and_missing_parameters() {
    for (source, diagnostic) in [
        (
            "pure comptime fn Identity<T> -> .Type'builtin {T} typealias Alias<.T:.Type'builtin=Identity<bool>> = T;fn bad(value:Alias)->bool {value}",
            "unresolved type equality",
        ),
        ("typealias Bad<.T:.Type'builtin=false> = T;", "type"),
        (
            "typealias Bad<.T:.Type'builtin=T> = T;",
            "type default references",
        ),
        (
            "typealias Pair<T,.U:.Type'builtin=T> = (T,U);typealias Bad=Pair;",
            "requires explicit type arguments",
        ),
        (
            "typealias Alias<.T:.Type'builtin=bool> = T;typealias Bad=Alias<bool>;",
            "argument count mismatch",
        ),
        (
            "typealias Alias<.T:.Type'builtin=bool> = T;typealias Bad=Alias<.T=bool,.T=bool>;",
            "duplicate type argument",
        ),
    ] {
        rejected(source, diagnostic);
    }
}

#[test]
fn function_type_defaults_are_not_replaced_by_call_inference() {
    for (source, diagnostic) in [
        (
            "fn keep(value:T)->T where .T:.Type'builtin=bool {value} fn bad() {keep('x');}",
            "type",
        ),
        (
            "fn keep(value:T)->T where .T:.Type'builtin=false {value}",
            "type",
        ),
        (
            "fn keep(value:T)->T where .T:.Type'builtin=T {value}",
            "type default references",
        ),
        (
            "fn keep(value:T)->T where .T:.Type'builtin=bool {value} fn bad() {keep<bool>(true);}",
            "argument count mismatch",
        ),
        (
            "fn keep(value:T)->T where .T:.Type'builtin=bool {value} fn bad() {let f:fn('x''type)->'x''type=keep;}",
            "type",
        ),
    ] {
        rejected(source, diagnostic);
    }
}

#[test]
fn inferred_type_defaults_cannot_be_bypassed_by_argument_or_result_types() {
    let prefix = "typealias Type=.Type'builtin;typealias char=.char'builtin;comptime fn Selected(value:Type)->Type {if value == bool {bool} else {char}}";
    for (source, diagnostic) in [
        (
            "fn keep(first:A,second:B)->B where A,.B:Type=Selected(A) {second} fn bad()->char {keep(true,'x')}",
            "inference",
        ),
        (
            "fn keep(first:A,second:B)->B where A,.B:Type=Selected(A) {second} fn bad() {let f:fn(bool,char)->char=keep;}",
            "inference",
        ),
        (
            "fn keep(value:B)->B where A,.B:Type=Selected(A) {value} fn bad()->bool {keep(true)}",
            "waiting for inferred inputs",
        ),
        (
            "comptime fn Invalid(value:Type)->Type {if value == bool {.BitVec'builtin<-1>} else {bool}} fn keep(first:A)->A where A,.B:Type=Invalid(A) {first} fn bad()->bool {keep(true)}",
            "required comptime evaluation failed",
        ),
    ] {
        rejected(&format!("{prefix}{source}"), diagnostic);
    }
}

#[test]
fn computed_type_defaults_keep_associated_contexts_candidate_local() {
    let prefix = "typealias Type=.Type'builtin; trait Source {assoc Item:Type;} impl Source for bool {assoc Item:Type=bool;} impl Source for 'x''type {assoc Item:Type='x''type;} comptime fn Selected(value:Type)->Type {if value == bool {bool} else {'x''type}} fn apply(value:A,item:B.as(Source).Item,callback:fn(B.as(Source).Item)->bool)->bool where A,.B:Type=Selected(A),requires B :- Source {callback(item)}";
    rejected(
        &format!(
            "{prefix} fn bad()->bool {{let valid=apply(true,false,|item| not item); apply('x','x',|item| not item)}}"
        ),
        "type mismatch: expected (primitive bool), found (primitive char)",
    );
}

#[test]
fn normal_form_type_defaults_reject_bad_routing_and_preserve_opacity() {
    for (source, diagnostic) in [
        (
            "pure comptime fn Identity<.T:.Type'builtin=bool>->.Type'builtin {T} fn bad(value:Identity<>)->bool {value}",
            "type",
        ),
        (
            "pure comptime fn Identity<.T:.Type'builtin=bool>->.Type'builtin {T} fn bad(value:Identity<bool>) {}",
            "argument count mismatch",
        ),
        (
            "pure comptime fn Identity<.T:.Type'builtin=bool>->.Type'builtin {T} fn bad(value:Identity<.T=bool,.T=bool>) {}",
            "duplicate type argument",
        ),
        (
            "pure comptime fn Identity<.T:.Type'builtin=T>->.Type'builtin {T}",
            "itself or a later parameter",
        ),
    ] {
        rejected(source, diagnostic);
    }
}

#[test]
fn mixed_normal_form_defaults_check_definition_scope_and_routing() {
    for (source, diagnostic) in [
        (
            "pure comptime fn Head<.N:bool=N>->.Type'builtin {bool}",
            "itself or a later parameter",
        ),
        (
            "pure comptime fn Head<.N:bool=M,.M:bool=true>->.Type'builtin {bool}",
            "itself or a later parameter",
        ),
        (
            "pure comptime fn Head<.N:bool='x'>->.Type'builtin {bool}",
            "type mismatch",
        ),
        (
            "pure comptime fn Head<N:bool,.M:bool=N>->.Type'builtin {bool} fn bad(value:Head<true,.M=true,.M=false>) {}",
            "duplicate normal-form argument",
        ),
        (
            "pure comptime fn Head<N:bool,.M:bool=N>->.Type'builtin {bool} fn bad(value:Head<>) {}",
            "normal-form argument count mismatch",
        ),
    ] {
        rejected(source, diagnostic);
    }
}

#[test]
fn symbolic_normal_form_default_recovery_does_not_skip_constraints_or_unfold_heads() {
    for (source, diagnostic) in [
        (
            "typealias Type=.Type'builtin;pure comptime fn Head<T,N:bool,.M:bool=N>->Type where requires T == bool {T} fn bad(value:Head<T,true>) where T {}",
            "requirement",
        ),
        (
            "typealias Type=.Type'builtin;pure comptime fn Head<T,N:bool,.M:bool=N>->Type {T} fn bad(value:Head<T,true>)->T where T {value}",
            "type",
        ),
        (
            "typealias Type=.Type'builtin;typealias Integer=.Integer'builtin;comptime fn NeverReturns()->Integer {NeverReturns()} pure comptime fn Head<T,N:Integer,.M:Integer=NeverReturns()>->Type {T} typealias Bad=Head<bool,4>;",
            "required comptime evaluation failed",
        ),
    ] {
        rejected(source, diagnostic);
    }
}

#[test]
fn explicit_type_clauses_preserve_constraints_and_argument_sorts() {
    rejected(
        "fn keep(value:T)->T where T:.Type'builtin, requires T == bool {value} fn run() {keep('x');}",
        "type requirement",
    );
    rejected(
        "struct Box where T:.Type'builtin, requires T == bool {value:T} fn run() {Box<'x''type> {value:'x'};}",
        "type requirement",
    );
    rejected(
        "trait Allowed {} fn keep(value:T)->T where T:.Type'builtin, requires T :- Allowed {value} fn run() {keep(true);}",
        "trait bound is not established",
    );
    rejected(
        "fn keep(value:T)->T where T:.Type'builtin {value} fn run() {keep<true>(true);}",
        "type",
    );
    accepted(
        "typealias Kind=.Type'builtin; fn keep(value:T)->T where T:Kind {value} fn run()->bool {keep(true)}",
    );
    accepted(
        "effect echo(value:T)->T where T:.Type'builtin; effect enum Channel where T:.Type'builtin {send(value:T)->T,} fn source()->#Channel<bool> bool {Channel<bool>.send(true)#}",
    );
}

#[test]
fn comptime_parameters_reject_runtime_inputs_and_failed_required_invocations() {
    rejected(
        "fn choose(comptime flag:bool)->bool {flag} fn run(flag:bool)->bool {choose(flag)}",
        "comptime",
    );
    rejected(
        "fn runtime()->bool {true} fn choose(comptime flag:bool)->bool {flag} fn run()->bool {choose(runtime())}",
        "comptime",
    );
    rejected(
        "comptime fn never()->bool {never()} fn choose(comptime .flag:bool=never())->bool {flag} fn run()->bool {choose()}",
        "comptime",
    );
}

#[test]
fn staged_aggregate_forwarding_does_not_admit_runtime_dependencies() {
    rejected(
        "fn choose(comptime pair:(bool,bool))->bool {let (a,b)=pair;a and b} fn forward(comptime flag:bool,value:bool)->bool {choose((flag,value))}",
        "comptime",
    );
}

#[test]
fn staged_arithmetic_does_not_discharge_unsatisfied_program_contracts() {
    rejected(
        "typealias Byte=.u8'builtin; fn choose(comptime value:Byte)->Byte {value} fn scaled(comptime value:Byte)->Byte where requires value <= 10 {choose(value * value)} fn run()->Byte {scaled(11)}",
        "precondition",
    );
}

#[test]
fn staged_blocks_do_not_hide_runtime_dependencies() {
    for body in [
        "do {let local=value;flag and local}",
        "do {let local=flag;value;local}",
        "do {let local=runtime();flag and local}",
    ] {
        rejected(
            &format!(
                "fn runtime()->bool {{true}} fn choose(comptime flag:bool)->bool {{flag}} fn forward(comptime flag:bool,value:bool)->bool {{choose({body})}}"
            ),
            "comptime",
        );
    }
}

#[test]
fn staged_pattern_bindings_do_not_discard_runtime_dependencies() {
    for input in ["(flag,value)", "(flag,runtime())"] {
        rejected(
            &format!(
                "fn runtime()->bool {{true}} fn choose(comptime flag:bool)->bool {{flag}} fn forward(comptime flag:bool,value:bool)->bool {{choose(do {{let (a,_)={input};a}})}}"
            ),
            "comptime",
        );
    }
}

#[test]
fn deferred_comptime_calls_reject_runtime_inputs() {
    rejected(
        "comptime fn invert(flag:bool)->bool {not flag} fn choose(comptime flag:bool)->bool {flag} fn forward(comptime flag:bool,value:bool)->bool {choose(invert(flag and value))}",
        "comptime",
    );
}

#[test]
fn deferred_comptime_dispatch_preserves_phase_and_trait_requirements() {
    rejected(
        "trait Read {comptime fn read(flag:bool)->bool;} impl Read for bool {comptime fn read(flag:bool)->bool {flag}} fn choose(comptime flag:bool)->bool {flag} fn forward(comptime flag:bool)->bool where T :- Read {choose(T.as(Read).read(flag))} fn run()->bool {forward<'x''type>(true)}",
        "trait",
    );
    rejected(
        "trait Read {fn read(flag:bool)->bool;} impl Read for bool {fn read(flag:bool)->bool {flag}} fn choose(comptime flag:bool)->bool {flag} fn forward(comptime flag:bool)->bool {choose(bool.as(Read).read(flag))}",
        "execution phase",
    );
}

#[test]
fn deferred_comptime_associated_callables_reject_runtime_inputs() {
    rejected(
        "comptime fn keep(flag:bool)->bool {flag} trait Stored {assoc callback:comptime fn(bool)->bool;} impl Stored for bool {assoc callback:comptime fn(bool)->bool=keep;} fn choose(comptime flag:bool)->bool {flag} fn forward(value:bool)->bool {choose(bool.as(Stored).callback(value))}",
        "comptime",
    );
}

#[test]
fn deferred_comptime_local_control_flow_cannot_write_caller_storage() {
    rejected(
        "fn choose(comptime flag:bool)->bool {flag} fn forward(comptime flag:bool)->bool {while true {choose(do {if flag {break;} false});break;}true}",
        "comptime",
    );
    rejected(
        "fn choose(comptime flag:bool)->bool {flag} fn forward(comptime flag:bool)->bool {var outside=flag;choose(do {outside=not outside;outside})}",
        "comptime",
    );
    rejected(
        "fn choose(comptime flag:bool)->bool {flag} fn forward(comptime flag:bool)->bool {choose(do {flag=not flag;flag})}",
        "comptime",
    );
    rejected(
        "fn choose(comptime flag:bool)->bool {flag} fn forward(comptime flag:bool,runtime:bool)->bool {choose(do {var value=flag;while runtime {value=not value;} value})}",
        "comptime",
    );
}

#[test]
fn deferred_comptime_matches_do_not_hide_runtime_dependencies() {
    for body in [
        "value match {true=>flag,false=>not flag,}",
        "flag match {true=>value,false=>false,}",
        "flag match {x if value=>x,_=>false,}",
        "flag match {x and value is true=>x,_=>false,}",
        "flag match {true if runtime()=>true,_=>false,}",
    ] {
        rejected(
            &format!(
                "fn runtime()->bool {{true}} fn choose(comptime value:bool)->bool {{value}} fn forward(comptime flag:bool,value:bool)->bool {{choose({body})}}"
            ),
            "comptime",
        );
    }
}

#[test]
fn deferred_comptime_let_patterns_preserve_stage_and_control_boundaries() {
    for body in [
        "do {while true {let item if value=flag else {break;};} flag}",
        "do {while true {let (item and value is true)=flag else {break;};} flag}",
        "do {while true {let true=flag else {value;break;};break;} flag}",
        "do {let true=flag else {return false;};flag}",
    ] {
        rejected(
            &format!(
                "fn choose(comptime flag:bool)->bool {{flag}} fn forward(comptime flag:bool,value:bool)->bool {{choose({body})}}"
            ),
            "comptime",
        );
    }
}

#[test]
fn deferred_comptime_fields_reject_caller_storage_and_runtime_values() {
    for body in [
        "flag and caller.flag",
        "do {caller.flag=flag;true}",
        "do {let local=Cell {flag:caller.flag};local.flag and flag}",
        "do {let local=Cell {flag:flag};local.flag=caller.flag;local.flag}",
        "do {let local=Cell {flag:flag};let pointer=caller.ref;pointer.flag.*=local.flag;true}",
    ] {
        rejected(
            &format!(
                "struct Cell {{flag:bool}} fn choose(comptime value:bool)->bool {{value}} fn forward(comptime flag:bool,caller:Cell)->bool {{choose({body})}}"
            ),
            "comptime",
        );
    }
}

#[test]
fn deferred_comptime_bitvec_rejects_runtime_reads_and_caller_writes() {
    for body in [
        "(byte'bitvec and value'bitvec).as(Byte)",
        "do {value'bitvec[0..4]=byte'bitvec[0..4];value}",
        "do {byte'bitvec[0..4]=3;byte}",
        "do {var local=byte;local'bitvec[0..4]=value'bitvec[0..4];local}",
    ] {
        rejected(
            &format!(
                "typealias Byte=.u8'builtin;fn choose(comptime value:Byte)->Byte {{value}} fn forward(comptime byte:Byte,value:Byte)->Byte {{choose({body})}}"
            ),
            "comptime",
        );
    }
}

#[test]
fn deferred_comptime_bitvec_still_checks_static_width_contracts() {
    for (body, message) in [
        (
            "byte'bitvec[0..9].as(Byte)",
            "BitVec index or slice is out of bounds",
        ),
        (
            "do {var local=byte;local'bitvec[0..4]=16;local}",
            "integer literal does not fit the BitVec width",
        ),
        (
            "(byte'bitvec ++ byte'bitvec).as(Byte)",
            "BitVec narrowing requires an explicit slice",
        ),
    ] {
        rejected(
            &format!(
                "typealias Byte=.u8'builtin;fn choose(comptime value:Byte)->Byte {{value}} fn forward(comptime byte:Byte)->Byte {{choose({body})}}"
            ),
            message,
        );
    }
}

#[test]
fn deferred_comptime_local_closures_reject_runtime_captures_and_calls() {
    for body in [
        "do {let read:%{value:bool} comptime fn()->bool=|| value;read() and flag}",
        "do {let read:%{flag:bool} comptime fn()->bool=|| {runtime() and flag};read()}",
    ] {
        rejected(
            &format!(
                "fn runtime()->bool {{true}} fn choose(comptime value:bool)->bool {{value}} fn forward(comptime flag:bool,value:bool)->bool {{choose({body})}}"
            ),
            "comptime",
        );
    }
}

#[test]
fn literal_spreads_do_not_promote_runtime_captures_to_comptime() {
    rejected(
        "fn choose(comptime flag:T)->T where T {flag} fn run(value:bool)->bool {choose(...(do {let read:%{value:bool} comptime fn()->bool=|| value;read()},))}",
        "comptime",
    );
}

#[test]
fn computed_spreads_do_not_promote_runtime_packets_to_comptime() {
    rejected(
        "fn packet(flag:bool)->(bool,bool) {(flag,not flag)} fn choose(comptime a:bool,b:bool)->bool {a and b} fn run()->bool {choose(...packet(true))}",
        "comptime",
    );
    rejected(
        "comptime fn packet(flag:bool)->(bool,bool) {(flag,not flag)} fn choose(comptime a:bool,b:bool)->bool {a and b} fn run(flag:bool)->bool {choose(...packet(flag))}",
        "comptime",
    );
}

#[test]
fn comptime_only_inputs_do_not_authorize_runtime_storage() {
    rejected(
        "typealias Type=.Type'builtin;fn bad(T:Type)->bool {true}",
        "comptime-only value",
    );
    rejected(
        "typealias Type=.Type'builtin;fn bad(comptime T:Type)->Type {T}",
        "comptime-only value",
    );
    rejected(
        "typealias Int=.Integer'builtin;fn bad(comptime n:Int)->Int {n}",
        "comptime-only value",
    );
}

#[test]
fn dependent_comptime_type_inputs_reject_wrong_or_unavailable_types() {
    rejected(
        "typealias Type=.Type'builtin; fn identity(comptime T:Type,value:T)->T {value} fn run()->bool {identity(bool,'x')}",
        "type mismatch",
    );
    rejected(
        "typealias Type=.Type'builtin; fn wrong(comptime A:Type,comptime B:Type,value:A)->B {value}",
        "unresolved type equality",
    );
    rejected(
        "typealias Type=.Type'builtin; fn wrong(T:Type,value:T)->T {value}",
        "earlier comptime Type parameter",
    );
    rejected(
        "typealias Type=.Type'builtin; pure comptime fn Identity<T>->Type {T} fn keep(comptime T:Type,value:T)->T {value} fn run()->bool {keep(Identity<bool>,true)}",
        "unresolved type equality",
    );
}

#[test]
fn dependent_type_defaults_reject_runtime_dependencies_and_wrong_values() {
    rejected(
        "typealias Type=.Type'builtin; typealias Char='x''type; comptime fn Select(flag:bool)->Type {if flag {bool} else {Char}} fn choose(flag:bool,comptime .T:Type=Select(flag),value:T)->T {value} fn run()->Char {choose(false,'x')}",
        "dependent comptime input cannot read a runtime argument",
    );
    rejected(
        "typealias Type=.Type'builtin; fn identity(comptime .T:Type=bool,value:T)->T {value} fn run()->bool {identity('x')}",
        "type mismatch",
    );
    rejected(
        "typealias Type=.Type'builtin; comptime fn Broken()->Type {Broken()} fn identity(comptime .T:Type=Broken(),value:T)->T {value} fn run()->bool {identity(true)}",
        "required comptime evaluation failed",
    );
}

#[test]
fn dependent_type_packets_reject_mismatched_elements_and_runtime_producers() {
    rejected(
        "typealias Type=.Type'builtin;typealias Char='x''type;comptime fn packet()->(Type,Char) {(bool,'x')} fn identity(comptime T:Type,value:T)->T {value} fn run()->bool {identity(...packet())}",
        "type mismatch",
    );
    rejected(
        "typealias Type=.Type'builtin;fn packet()->(Type,bool) {(bool,true)} fn identity(comptime T:Type,value:T)->T {value} fn run()->bool {identity(...packet())}",
        "required comptime",
    );
    rejected(
        "typealias Type=.Type'builtin;pure comptime fn Identity<T>->Type {T} comptime fn packet()->(Type,bool) {(Identity<bool>,true)} fn identity(comptime T:Type,value:T)->T {value} fn run()->bool {identity(...packet())}",
        "unresolved type equality",
    );
}

#[test]
fn dependent_comptime_value_domain_refinement_does_not_allow_runtime_type_storage() {
    rejected(
        "typealias Type=.Type'builtin;fn keep(comptime T:Type,value:T)->bool {true} fn run()->bool {keep(Type,bool)}",
        "comptime-only value",
    );
    rejected(
        "typealias Type=.Type'builtin;fn keep(comptime T:Type,comptime value:T)->bool {true} fn run()->bool {keep(bool,bool)}",
        "type mismatch",
    );
}

#[test]
fn mixed_dependent_generic_inputs_check_domains_and_trait_requirements() {
    rejected(
        "typealias Type=.Type'builtin;fn pair(comptime T:Type,value:T,other:G)->(T,G) where G {(value,other)} fn run()->(bool,bool) {pair(bool,'x',true)}",
        "type mismatch",
    );
    rejected(
        "typealias Type=.Type'builtin;trait Allowed {} impl Allowed for bool {} fn pair(comptime T:Type,value:T,other:G)->(T,G) where G :- Allowed {(value,other)} fn run()->(bool,'x''type) {pair(bool,true,'x')}",
        "trait bound is not established",
    );
}

#[test]
fn scoped_generic_calls_do_not_equate_distinct_enclosing_type_inputs() {
    rejected(
        "typealias Type=.Type'builtin;fn pair(comptime T:Type,value:T,other:G)->(T,G) where G {(value,other)} fn wrong(comptime A:Type,comptime B:Type,value:A)->(B,bool) {pair(A,value,true)}",
        "generic call inference failed",
    );
}

#[test]
fn scoped_generic_function_value_contexts_do_not_equate_different_type_inputs() {
    rejected(
        "typealias Type=.Type'builtin;fn identity(value:G)->G where G {value} fn wrong(comptime A:Type,comptime B:Type) {let action:fn(A)->B=identity;}",
        "generic function context inference failed",
    );
}

#[test]
fn scoped_computed_type_defaults_reject_conflicting_callback_contexts() {
    rejected(
        "typealias Type=.Type'builtin;
        comptime fn Choose(T:Type)->Type {if T == bool {bool} else {'x''type}}
        fn pair(value:G,other:H,proof:K)->K where G,H,.K:Type=(G,Choose(H)) {proof}
        fn wrong(comptime A:Type,comptime B:Type) {
            let action:fn(A,bool,(B,bool))->(B,bool)=pair;
        }",
        "generic function context inference failed",
    );
}

#[test]
fn generic_callback_contexts_do_not_supply_missing_trait_evidence() {
    for body in [
        "let action:fn(A)->A=identity;action(value)",
        "apply(identity,value)",
    ] {
        rejected(
            &format!(
                "trait Allowed {{}} impl Allowed for bool {{}}
            fn identity(value:G)->G where G :- Allowed {{value}}
            fn apply(action:fn(G)->G,value:G)->G where G {{action(value)}}
            fn wrong(value:A)->A where A {{{body}}}"
            ),
            "trait bound is not established",
        );
    }
}

#[test]
fn ordinary_type_input_capabilities_reject_wrong_actual_types_and_unpromised_forwarding() {
    for call in ["checked('x''type)", "forward('x''type)"] {
        rejected(
            &format!(
                "typealias Type=.Type'builtin;trait Allowed {{}} impl Allowed for bool {{}}
            fn checked(comptime A:Type)->bool where requires A :- Allowed {{true}}
            fn forward(comptime A:Type)->bool where requires A :- Allowed {{checked(A)}}
            fn run()->bool {{{call}}}"
            ),
            "ordinary input trait bound is not established",
        );
    }
    rejected(
        "typealias Type=.Type'builtin;trait Allowed {} impl Allowed for bool {}
        fn checked(comptime A:Type)->bool where requires A :- Allowed {true}
        fn wrong(comptime A:Type)->bool {checked(A)}",
        "ordinary input trait bound is not established",
    );
}

#[test]
fn ordinary_associated_parameter_types_require_evidence_and_matching_values() {
    let prefix = "typealias Type=.Type'builtin;trait Source {assoc Item:Type;}
        impl Source for bool {assoc Item:Type=bool;}";
    rejected(
        &format!("{prefix} fn bad(comptime A:Type,value:A.as(Source).Item)->bool {{true}}"),
        "requires established trait evidence",
    );
    rejected(
        &format!(
            "{prefix} fn bad(value:A.as(Source).Item,comptime A:Type)->bool where requires A :- Source {{true}}"
        ),
        "Unresolved identifier `A`",
    );
    rejected(
        &format!(
            "{prefix} fn keep(comptime A:Type,value:A.as(Source).Item)->A.as(Source).Item where requires A :- Source {{value}} fn run()->bool {{keep(bool,'x')}}"
        ),
        "type mismatch",
    );
    rejected(
        &format!(
            "{prefix} fn keep(comptime A:Type,comptime value:A.as(Source).Item,comptime .copy:A.as(Source).Item=value)->A.as(Source).Item where requires A :- Source {{copy}} fn run()->bool {{keep(bool,true,.copy='x')}}"
        ),
        "type mismatch",
    );
}

#[test]
fn associated_input_domain_refinement_does_not_accept_the_wrong_value_sort() {
    for call in ["inspect(bool,true)", "inspect('x''type,bool)"] {
        rejected(&format!("typealias Type=.Type'builtin;trait Source {{assoc Item:Type;}}
            impl Source for bool {{assoc Item:Type=Type;}}
            impl Source for 'x''type {{assoc Item:Type=bool;}}
            fn inspect(comptime A:Type,comptime value:A.as(Source).Item)->bool where requires A :- Source {{true}}
            fn run()->bool {{{call}}}"), "type mismatch");
    }
}

#[test]
fn demanded_associated_inputs_and_defaults_preserve_opaque_type_indices() {
    let prefix = "typealias Type=.Type'builtin;trait Source {assoc Item:Type;}
        impl Source for bool {assoc Item:Type=bool;}
        impl Source for 'x''type {assoc Item:Type='x''type;}
        pure comptime fn Tagged<T,value:T>->Type {T}
        fn accept(comptime A:Type,comptime value:A.as(Source).Item,
                  comptime .copy:A.as(Source).Item=value,
                  witness:Tagged<A.as(Source).Item,copy>)->bool where requires A :- Source {true}";
    let dump = accepted(&format!(
        "{prefix}
        fn boolean(witness:Tagged<bool,true>)->bool {{accept(bool,true,witness)}}
        fn character(witness:Tagged<'x''type,'x'>)->bool {{accept('x''type,'x',witness)}}"
    ));
    assert!(dump.contains("(normal-form (head"));
    rejected(
        &format!("{prefix} fn wrong(witness:Tagged<bool,true>)->bool {{accept(bool,'x',witness)}}"),
        "type mismatch",
    );
    rejected(
        &format!(
            "{prefix} fn wrong(witness:Tagged<bool,true>)->bool {{accept(bool,true,witness,.copy=false)}}"
        ),
        "type equality",
    );
}

#[test]
fn demanded_associated_type_domains_accept_type_indices_and_defaults() {
    let prefix = "typealias Type=.Type'builtin;trait Source {assoc Item:Type;}
        impl Source for bool {assoc Item:Type=Type;}
        pure comptime fn Tagged<T,value:T>->Type {T}
        fn accept(comptime A:Type,comptime value:A.as(Source).Item,
                  comptime .copy:A.as(Source).Item=value,
                  witness:Tagged<A.as(Source).Item,copy>)->bool where requires A :- Source {true}";
    accepted(&format!(
        "{prefix}
        fn concrete(witness:Tagged<Type,bool>)->bool {{accept(bool,bool,witness)}}
        fn forwarded(comptime T:Type,witness:Tagged<Type,T>)->bool {{accept(bool,T,witness)}}"
    ));
    let generic = prefix.replace(
        "witness:Tagged<A.as(Source).Item,copy>)->bool where requires A :- Source {true}",
        "witness:Tagged<A.as(Source).Item,copy>,extra:G)->G where G, requires A :- Source {extra}",
    );
    accepted(&format!(
        "{generic}
        fn forwarded(comptime T:Type,witness:Tagged<Type,T>)->bool {{accept(bool,T,witness,true)}}"
    ));
    rejected(
        &format!(
            "{prefix} fn wrong(witness:Tagged<Type,bool>)->bool {{accept(bool,true,witness)}}"
        ),
        "type mismatch",
    );
    rejected(
        &format!(
            "{prefix} fn wrong(witness:Tagged<Type,bool>)->bool {{accept(bool,bool,witness,.copy='x''type)}}"
        ),
        "type equality",
    );
}

#[test]
fn scoped_structural_defaults_do_not_capture_same_index_callee_parameters() {
    rejected(
        "typealias Type=.Type'builtin;fn pair(comptime .T:Type=G,value:T,other:G)->(T,G) where G {(value,other)} fn wrong(comptime A:Type,comptime B:Type,left:A,right:B)->(A,B) {pair(left,right)}",
        "type equality",
    );
}

#[test]
fn ordinary_comptime_values_can_index_opaque_types_and_forward_without_unfolding() {
    let dump = accepted(
        "typealias Type=.Type'builtin;typealias Integer=.Integer'builtin;pure comptime fn Sized<N:Integer>->Type {bool} fn keep(comptime n:Integer,value:Sized<n>)->Sized<n> {value} fn forward(comptime n:Integer,value:Sized<n>)->Sized<n> {keep(n,value)}",
    );
    assert!(dump.contains("(bound (depth 0) (index 0) (sort comptime-value))"));
    assert!(dump.contains("(normal-form (head"));
    rejected(
        "typealias Type=.Type'builtin;typealias Integer=.Integer'builtin;pure comptime fn Sized<N:Integer>->Type {bool} fn bad(n:Integer,value:Sized<n>)->bool {true}",
        "comptime",
    );
    rejected(
        "typealias Type=.Type'builtin;typealias Integer=.Integer'builtin;pure comptime fn Sized<N:Integer>->Type {bool} fn bad(comptime n:bool,value:Sized<n>)->bool {true}",
        "type mismatch",
    );
    rejected(
        "typealias Type=.Type'builtin;pure comptime fn Flag<B:bool>->Type {bool} fn bad(flag:bool,value:Flag<flag>)->bool {true}",
        "comptime",
    );
}

#[test]
fn earlier_comptime_inputs_are_available_to_normal_form_signature_plans() {
    let prefix = "typealias Type=.Type'builtin;typealias Integer=.Integer'builtin;pure comptime fn Shape<N:Integer,.storage:Integer=N>->Type {bool}";
    let dump = accepted(&format!(
        "{prefix} fn keep(comptime n:Integer,value:Shape<n>)->Shape<n,.storage=n> {{value}} fn forward(comptime n:Integer,value:Shape<n>)->Shape<n> {{keep(n,value)}}"
    ));
    assert!(dump.contains("(arguments ((bound (depth 0) (index 0) (sort comptime-value)) (bound (depth 0) (index 0) (sort comptime-value))))"));
    rejected(
        &format!("{prefix} fn bad(value:Shape<n>,comptime n:Integer)->bool {{true}}"),
        "Unresolved identifier `n`",
    );
    rejected(
        "typealias Type=.Type'builtin;pure comptime fn Flag<B:bool,.copy:bool=B>->Type {bool} fn bad(flag:bool,value:Flag<flag>)->bool {true}",
        "local binding is unavailable",
    );
    rejected(
        &format!("{prefix} fn bad(comptime n:bool,value:Shape<n>)->bool {{true}}"),
        "type mismatch",
    );
}

#[test]
fn normal_form_type_defaults_use_the_active_caller_parameter_prefix() {
    let dump = accepted(
        "typealias Type=.Type'builtin;pure comptime fn Wrapper<T,.U:Type=T,.V:Type=U>->Type {T} fn keep(comptime A:Type,marker:bool,comptime B:Type,value:Wrapper<B>)->Wrapper<B> {value}",
    );
    assert!(dump.contains("(arguments ((bound (depth 0) (index 2) (sort type)) (bound (depth 0) (index 2) (sort type)) (bound (depth 0) (index 2) (sort type))))"));
}

#[test]
fn mixed_ordinary_indices_check_the_dependent_value_domain() {
    let head = "typealias Type=.Type'builtin;pure comptime fn Tagged<T,value:T>->Type {T}";
    let dump = accepted(&format!(
        "{head} fn keep(comptime A:Type,comptime v:A,value:Tagged<A,v>)->Tagged<A,v> {{value}} fn forward(comptime A:Type,comptime v:A,value:Tagged<A,v>)->Tagged<A,v> {{keep(A,v,value)}}"
    ));
    assert!(dump.contains("(arguments ((bound (depth 0) (index 0) (sort type)) (bound (depth 0) (index 1) (sort comptime-value))))"));
    rejected(
        &format!(
            "{head} fn wrong(comptime A:Type,comptime B:Type,comptime v:B,value:Tagged<A,v>)->bool {{true}}"
        ),
        "type equality",
    );
    rejected(
        &format!("{head} fn wrong(comptime A:Type,v:A,value:Tagged<A,v>)->bool {{true}}"),
        "comptime",
    );
}

#[test]
fn dependent_type_default_inference_does_not_override_the_computed_type() {
    rejected(
        "typealias Type=.Type'builtin;comptime fn Choose(T:Type)->Type {'x''type} fn pair(comptime .T:Type=Choose(G),value:T,other:G)->(T,G) where G {(value,other)} fn run()->(bool,bool) {pair(true,false)}",
        "type mismatch",
    );
    rejected(
        "typealias Type=.Type'builtin;comptime fn Choose(T:Type)->Type {Choose(T)} fn pair(comptime .T:Type=Choose(G),value:T,other:G)->(T,G) where G {(value,other)} fn run()->(bool,bool) {pair(true,false)}",
        "required comptime evaluation failed",
    );
}

#[test]
fn dependent_type_defaults_report_unresolved_inputs_before_invocation() {
    rejected(
        "typealias Type=.Type'builtin;comptime fn Choose(T:Type)->Type {if T == bool {bool} else {'x''type}} fn keep(comptime .T:Type=Choose(G),value:T,action:fn(G)->G)->T where G {value} fn run()->bool {keep(true,|n| n)}",
        "dependent Type computation awaits inferred type arguments",
    );
}

#[test]
fn ordinary_comptime_capability_indices_reject_wrong_or_unproven_inputs() {
    let prefix = "typealias Type=.Type'builtin;
        trait Source {assoc Item:Type;}
        impl Source for bool {assoc Item:Type=Type;}
        trait Allowed where T {}
        pure comptime fn Tagged<T,value:T>->Type {T}
        impl Allowed<Tagged<Type,bool>> for bool {}
        fn accept(comptime A:Type,comptime value:A.as(Source).Item,
                  comptime .copy:A.as(Source).Item=value)->bool
        where requires A :- Source, requires bool :- Allowed<Tagged<A.as(Source).Item,copy>> {true}";
    for call in ["accept(bool,'x''type)", "accept(bool,bool,.copy='x''type)"] {
        rejected(
            &format!("{prefix} fn run()->bool {{{call}}}"),
            "ordinary input trait bound is not established",
        );
    }
    rejected(
        &format!("{prefix} fn run()->bool {{accept(bool,true)}}"),
        "type mismatch",
    );
    rejected(
        &format!(
            "{prefix} fn forward(comptime A:Type,comptime value:A.as(Source).Item)->bool where requires A :- Source {{accept(A,value)}}"
        ),
        "ordinary input trait bound is not established",
    );
}

#[test]
fn inferred_comptime_capability_domains_do_not_invent_evidence_or_comptime_values() {
    let prefix = "typealias Type=.Type'builtin;
        trait Allowed where Index {}
        pure comptime fn Tagged<T,value:T>->Type {T}
        impl Allowed<Tagged<bool,true>> for bool {}
        fn accept(comptime value:T,extra:G)->G where T,G,
            requires bool :- Allowed<Tagged<T,value>> {extra}";
    for value in ["false", "bool", "'x'"] {
        rejected(
            &format!("{prefix} fn run()->bool {{accept({value},true)}}"),
            "ordinary input trait bound is not established",
        );
    }
    rejected(
        &format!("{prefix} fn run(value:bool)->bool {{accept(value,true)}}"),
        "comptime",
    );
    rejected(
        &format!("{prefix} fn run()->'x''type {{accept(true,false)}}"),
        "type mismatch",
    );
}

#[test]
fn ordinary_capability_dependencies_do_not_assume_missing_or_circular_evidence() {
    let prefix = "typealias Type=.Type'builtin;trait Source {assoc Item:Type;} trait Allowed {} trait Accepts where T {}";
    for constraints in [
        "requires A.as(Source).Item :- Allowed",
        "requires A :- Accepts<A.as(Source).Item>",
        "requires A.as(Source).Item :- Source",
        "requires (A :- Source) or (A :- Allowed), requires A.as(Source).Item :- Allowed",
        "requires not (A :- Source), requires A.as(Source).Item :- Allowed",
    ] {
        rejected(
            &format!("{prefix} fn bad(comptime A:Type)->bool where {constraints} {{true}}"),
            "associated",
        );
    }
    rejected(
        &format!(
            "{prefix} fn bad(comptime A:Type)->bool where requires A.as(Source).Missing :- Allowed, requires A :- Source {{true}}"
        ),
        "type expression",
    );
}

#[test]
fn declaration_capability_prerequisites_do_not_admit_unproven_projections() {
    let prefix = "typealias Type=.Type'builtin;trait Source {assoc Item:Type;} trait Allowed {}";
    for body in [
        "struct Container where A, requires A.as(Source).Item :- Allowed {}",
        "enum Choice where A, requires A.as(Source).Item :- Allowed {none,}",
        "trait Accepts where T {} typealias Alias<A :- Accepts<A.as(Source).Item>> = bool;",
        "trait Checked where A, requires A.as(Source).Item :- Allowed {}",
        "struct Holder where A {} impl Allowed for Holder<A> where A, requires A.as(Source).Item :- Allowed {}",
    ] {
        rejected(
            &format!("{prefix} {body}"),
            "associated projection requires a given interface constraint",
        );
    }
}

#[test]
fn implementation_projection_prerequisites_form_in_reverse_order() {
    accepted(
        "typealias Type=.Type'builtin;trait Source {assoc Item:Type;} trait Allowed {}
        trait Get {fn get()->bool;} struct Holder where A {}
        impl Get for Holder<A> where A,
            requires A.as(Source).Item :- Allowed, requires A :- Source {fn get()->bool {true}}",
    );
}

#[test]
fn implementation_projection_qualification_rejects_wrong_selected_domain() {
    rejected(
        "typealias Type=.Type'builtin;trait Source {assoc Item:Type;} trait Allowed {}
        impl Source for bool {assoc Item:Type='x''type;} impl Allowed for bool {}
        trait Get {fn get()->bool;} struct Holder where A {}
        impl Get for Holder<A> where A,
            requires A.as(Source).Item :- Allowed, requires A :- Source {fn get()->bool {true}}
        fn run()->bool {Holder<bool>.as(Get).get()}",
        "method implementation is not uniquely established",
    );
}

#[test]
fn mixed_sugar_bounds_cannot_extract_source_evidence_from_negative_or_disjunctive_requirements() {
    let prefix = "typealias Type=.Type'builtin;trait Source {assoc Item:Type;} trait Accepts where T {} trait Other {}";
    for tail in [
        "",
        ", requires not (A :- Source)",
        ", requires (A :- Source) or (A :- Other)",
    ] {
        for declaration in [
            format!("struct Bad where A :- Accepts<A.as(Source).Item>{tail} {{}}"),
            format!("fn bad(value:A)->bool where A :- Accepts<A.as(Source).Item>{tail} {{true}}"),
        ] {
            rejected(&format!("{prefix} {declaration}"), "associated");
        }
    }
}

#[test]
fn ordinary_type_predicates_reject_wrong_inputs_and_unproved_forwarding() {
    let prefix = "typealias Type=.Type'builtin;trait Forbidden {} impl Forbidden for bool {}
        fn same(comptime A:Type,comptime B:Type,value:A)->B where requires A == B {value}
        fn choice(comptime T:Type)->bool where requires T == bool or T == 'x''type {true}
        fn distinct(comptime A:Type,comptime B:Type)->bool where requires A != B {true}
        fn allowed(comptime T:Type)->bool where requires not (T :- Forbidden) {true}";
    for body in [
        "fn run()->'x''type {same(bool,'x''type,true)}",
        "fn run()->bool {choice(Type)}",
        "fn run()->bool {distinct(bool,bool)}",
        "fn run()->bool {allowed(bool)}",
        "fn forward(comptime A:Type,comptime B:Type,value:A)->B {same(A,B,value)}",
    ] {
        rejected(
            &format!("{prefix} {body}"),
            "ordinary input type requirement is not established",
        );
    }
}

#[test]
fn scoped_negative_trait_requirements_need_concrete_or_local_evidence() {
    let prefix = "typealias Type=.Type'builtin; trait Forbidden {}
        fn allowed(comptime T:Type)->bool where requires not (T :- Forbidden) {true}";
    rejected(
        &format!("{prefix} fn forward(comptime T:Type)->bool {{allowed(T)}}"),
        "ordinary input type requirement is not established",
    );
    rejected(
        &format!(
            "{prefix} impl Forbidden for bool {{}} fn forward(comptime T:Type)->bool where requires T == bool {{allowed(T)}}"
        ),
        "ordinary input type requirement is not established",
    );
    rejected(
        &format!(
            "{prefix} fn forward(comptime T:Type)->bool where requires T == bool or T == 'x''type {{allowed(T)}}"
        ),
        "ordinary input type requirement is not established",
    );
}

#[test]
fn ordinary_input_method_contracts_use_interface_premises_in_the_method_scope() {
    let prefix = "typealias Type=.Type'builtin; trait Allowed {} trait Other {}";
    accepted(&format!("{prefix}
        trait Service {{fn same(comptime A:Type,comptime B:Type,value:A)->B where requires A == B;}}
        impl Service for bool {{fn same(comptime X:Type,comptime Y:Type,value:X)->Y where requires X == Y {{value}}}}"));
    accepted(&format!("{prefix}
        trait Service {{fn check(comptime A:Type)->bool where requires A :- Allowed;}}
        impl Service for bool {{fn check(comptime X:Type)->bool where requires X :- Allowed {{true}}}}"));
    accepted(&format!(
        "{prefix}
        trait Service {{fn check(comptime A:Type)->bool where requires A :- Allowed;}}
        impl Service for bool {{fn check(comptime X:Type)->bool {{true}}}}"
    ));
    rejected(&format!("{prefix}
        trait Service {{fn check(comptime A:Type)->bool;}}
        impl Service for bool {{fn check(comptime X:Type)->bool where requires X == bool {{true}}}}"),
        "strengthens trait preconditions");
    rejected(&format!("{prefix}
        trait Service {{fn check(comptime A:Type)->bool where requires A :- Allowed;}}
        impl Service for bool {{fn check(comptime X:Type)->bool where requires X :- Other {{true}}}}"),
        "strengthens trait capability requirements");
}

#[test]
fn staged_trait_calls_check_actual_ordinary_inputs() {
    let prefix = "typealias Type=.Type'builtin; trait Allowed {} impl Allowed for bool {}
        trait Service where Tag {
            fn same(comptime A:Type,comptime B:Type)->bool where requires A == B {true}
            fn permitted(comptime A:Type)->bool where requires A :- Allowed {true}
            fn generic(comptime A:Type,value:T)->T where T, requires A == Tag {value}
        } impl Service<bool> for bool {}";
    rejected(
        &format!("{prefix} fn run()->bool {{bool.as(Service<bool>).same(bool,'x''type)}}"),
        "ordinary input type requirement is not established",
    );
    rejected(
        &format!("{prefix} fn run()->bool {{bool.as(Service<bool>).permitted('x''type)}}"),
        "ordinary input trait bound is not established",
    );
    rejected(
        &format!("{prefix} fn run()->bool {{bool.as(Service<bool>).generic('x''type,true)}}"),
        "ordinary input type requirement is not established",
    );
}

#[test]
fn method_signature_equivalence_cannot_use_implementation_only_equalities() {
    rejected("typealias Type=.Type'builtin;
        trait Service {fn same(comptime A:Type,comptime B:Type,value:A)->A;}
        impl Service for bool {fn same(comptime A:Type,comptime B:Type,value:B)->B where requires A == B {value}}",
        "does not match its trait signature");
    rejected("typealias Type=.Type'builtin;
        trait Service {fn same(comptime A:Type,comptime B:Type,value:A)->A where requires A == B or A == bool;}
        impl Service for bool {fn same(comptime A:Type,comptime B:Type,value:B)->B {value}}",
        "does not match its trait signature");
    rejected(
        "typealias Type=.Type'builtin;
        trait Service {fn same(comptime A:Type,value:A)->A;}
        impl Service for bool {fn same(A:Type,value:bool)->bool {value}}",
        "does not match its trait signature",
    );
}

#[test]
fn runtime_default_forwarding_requires_scalar_type_evidence() {
    rejected(
        "typealias Type=.Type'builtin;
        fn copy(comptime T:Type,value:T,.copy:T=value)->T {copy}",
        "default expression requires P3",
    );
    rejected(
        "typealias Type=.Type'builtin; struct Owned {}
        fn copy(comptime T:Type,value:T,.copy:T=value)->T where requires T == Owned {copy}",
        "default expression requires P3",
    );
    rejected("typealias Type=.Type'builtin;
        pure comptime fn Identity<T>->Type {T}
        fn copy(comptime T:Type,value:T,.copy:T=value)->T where requires T == Identity<bool> {copy}",
        "default expression requires P3");
    accepted(
        "typealias Type=.Type'builtin;
        fn copy(comptime T:Type,value:T,.copy:T=value)->T where requires T == bool {copy}",
    );
}

#[test]
fn callback_signature_equivalence_requires_interface_evidence() {
    rejected("typealias Type=.Type'builtin;
        trait Service {fn apply(comptime A:Type,comptime B:Type,callback:fn(A)->bool)->bool;}
        impl Service for bool {fn apply(comptime A:Type,comptime B:Type,callback:fn(B)->bool)->bool where requires A == B {true}}",
        "does not match its trait signature");
    rejected("typealias Type=.Type'builtin;
        trait Service {fn apply(comptime A:Type,comptime B:Type,callback:fn(A)->bool)->bool where requires A == B or A == bool;}
        impl Service for bool {fn apply(comptime A:Type,comptime B:Type,callback:fn(B)->bool)->bool {true}}",
        "does not match its trait signature");
}

#[test]
fn callback_normal_form_congruence_preserves_opaque_head_identity() {
    let prefix = "typealias Type=.Type'builtin; pure comptime fn Identity<T>->Type {T}";
    accepted(&format!("{prefix}
        trait Service {{fn take(comptime A:Type,comptime B:Type,callback:Identity<fn(A)->A>) where requires A == B;}}
        impl Service for bool {{fn take(comptime A:Type,comptime B:Type,callback:Identity<fn(B)->B>) where requires A == B {{}}}}"));
    rejected(&format!("{prefix}
        trait Service {{fn take(comptime A:Type,comptime B:Type,callback:Identity<fn(A)->A>) where requires A == B;}}
        impl Service for bool {{fn take(comptime A:Type,comptime B:Type,callback:fn(B)->B) where requires A == B {{}}}}"),
        "does not match its trait signature");
}

#[test]
fn ordinary_type_container_checks_preserve_field_types_and_exhaustiveness() {
    rejected(
        "typealias Type=.Type'builtin; struct Box where T {value:T,}
        fn wrong(comptime T:Type,value:T)->Box<T> {Box<T>{value:true}}",
        "unresolved type equality",
    );
    rejected("typealias Type=.Type'builtin; struct Box where T {value:T,}
        fn wrong(comptime T:Type,original:Box<T>)->Box<T> {var value=original;value.value=true;value}", "unresolved type equality");
    rejected(
        "typealias Type=.Type'builtin; enum Choice where T {some(T),none,}
        fn wrong(comptime T:Type,value:Choice<T>)->T {value match {.some(item)=>item,}}",
        "non-exhaustive",
    );
}

#[test]
fn ordinary_container_formation_requires_caller_type_evidence() {
    rejected(
        "typealias Type=.Type'builtin; struct Box where T, requires T == bool {value:T,}
        fn wrong(comptime T:Type,value:T)->Box<T> {Box<T>{value:value}}",
        "type requirement",
    );
    rejected(
        "typealias Type=.Type'builtin; enum Choice where T, requires T == bool {some(T),}
        fn wrong(comptime T:Type,value:T)->Choice<T> {Choice<T>.some(value)}",
        "type requirement",
    );
}

#[test]
fn ordinary_container_program_contracts_still_reject_wrong_fields_and_results() {
    let prefix = "typealias Type=.Type'builtin;struct Box where T {value:T,flag:bool,}";
    rejected(&format!("{prefix}
        fn read(comptime T:Type,box:Box<T>)->result:bool where requires box.flag, ensures result {{box.flag}}
        fn run()->bool {{read(bool,Box<bool>{{value:true,flag:false}})}}"), "precondition");
    rejected(&format!("{prefix}
        fn wrong(comptime T:Type,box:Box<T>)->result:bool where requires box.flag, ensures result {{false}}"),
        "postcondition");
}

#[test]
fn ordinary_type_memory_writes_invalidate_changed_field_facts() {
    rejected("typealias Type=.Type'builtin;struct Box where T {value:T,flag:bool,}
        fn clear(comptime T:Type,box:*Box<T>) {box.*.flag=false;}
        fn run()->result:bool where ensures result {var box=Box<bool>{value:true,flag:true};clear(bool,box.ref);box.flag}",
        "postcondition");
    rejected(
        "typealias Type=.Type'builtin;struct Box where T {value:T,flag:bool,}
        fn wrong(comptime T:Type,box:*Box<T>) {box.*.flag='x';}",
        "type mismatch",
    );
}

#[test]
fn pointer_scalar_identity_requires_proof_and_does_not_unfold_heads() {
    let prefix =
        "typealias Type=.Type'builtin;struct Owned {} pure comptime fn Identity<T>->Type {T}";
    for condition in [
        "",
        "where requires T == Owned",
        "where requires T == Identity<bool>",
    ] {
        rejected(
            &format!("{prefix} fn read(comptime T:Type,p:*T)->T {condition} {{p.*}}"),
            "pointer reads of non-intrinsic values",
        );
        rejected(
            &format!(
                "{prefix} fn write(comptime T:Type,p:*T,value:T) {condition} {{p.* = value;}}"
            ),
            "non-Copy pointer writes",
        );
    }
    rejected(
        &format!("{prefix} fn write(comptime T:Type,p:*T) where requires T == bool {{p.* = 'x';}}"),
        "unresolved type equality",
    );
}

#[test]
fn proved_scalar_memory_writes_do_not_preserve_overwritten_facts() {
    rejected("typealias Type=.Type'builtin;struct Box where T {value:T,flag:bool,}
        fn replace(comptime T:Type,box:*Box<T>,value:T) where requires T == bool {box.*.value=value;}
        fn run()->result:bool where ensures result {var box=Box<bool>{value:true,flag:true};replace(bool,box.ref,false);box.value}",
        "postcondition");
}

#[test]
fn dependent_boolean_contracts_do_not_invent_value_or_type_proofs() {
    let prefix = "typealias Type=.Type'builtin;";
    rejected(
        &format!(
            "{prefix} fn keep(comptime T:Type,value:T)->result:T where requires T == bool, requires value, ensures result {{value}} fn run()->bool {{keep(bool,false)}}"
        ),
        "precondition",
    );
    rejected(
        &format!(
            "{prefix} fn wrong(comptime T:Type,value:T)->result:T where requires T == bool, requires value, ensures result {{false}}"
        ),
        "postcondition",
    );
    rejected(
        &format!(
            "{prefix} fn unproved(comptime T:Type,value:T)->result:T where requires T == bool, ensures result {{value}}"
        ),
        "postcondition",
    );
    rejected(
        &format!("{prefix} fn missing(comptime T:Type,value:T) where requires value {{}}"),
        "type equality",
    );
    rejected(
        &format!(
            "{prefix} pure comptime fn Identity<T>->Type {{T}} fn hidden_head(comptime T:Type,value:T) where requires T == Identity<bool>, requires value {{}}"
        ),
        "type equality",
    );
}

#[test]
fn dependent_boolean_operators_require_unambiguous_type_and_value_evidence() {
    let prefix = "typealias Type=.Type'builtin;pure comptime fn Identity<T>->Type {T}";
    for premise in [
        "",
        "where requires T == Identity<bool>",
        "where requires T == bool or T == 'x''type",
    ] {
        rejected(
            &format!(
                "{prefix} fn negate(comptime T:Type,value:T) {premise} {{let ignored=not value;}}"
            ),
            if !premise.is_empty() {
                "structural type view remains unresolved"
            } else {
                "not requires bool or BitVec"
            },
        );
        rejected(
            &format!(
                "{prefix} fn combine(comptime T:Type,value:T) {premise} {{let ignored=value and true;}}"
            ),
            if !premise.is_empty() {
                "structural type view remains unresolved"
            } else {
                "and/or require bool"
            },
        );
    }
    rejected(
        &format!(
            "{prefix} fn wrong(comptime T:Type,value:T)->result:bool where requires T == bool, requires value, ensures result {{not value}}"
        ),
        "postcondition",
    );
    rejected(
        &format!(
            "{prefix} fn wrong(comptime T:Type,value:T)->result:bool where requires T == bool, requires value, ensures result {{value and false}}"
        ),
        "postcondition",
    );
}

#[test]
fn disjunctive_program_facts_do_not_survive_writes_or_authorize_each_arm() {
    rejected(
        "fn wrong(a:bool,b:bool)->result:bool where requires a or b, ensures result {a}",
        "postcondition",
    );
    rejected(
        "fn wrong(a:bool,b:bool)->result:bool where ensures result {var left=a;var right=b;assumes left or right;left=false;right=false;left or right}",
        "postcondition",
    );
    rejected(
        "fn either(a:bool,b:bool)->result:bool where requires a or b, ensures result {a or b} fn wrong()->bool {either(false,false)}",
        "precondition",
    );
}

#[test]
fn disjunctive_copy_facts_do_not_cross_writes_or_missing_sources() {
    let prefix = "fn accept(a:bool,b:bool)->bool where requires a or b {true}";
    rejected(
        &format!(
            "{prefix} fn wrong(a:bool,b:bool)->bool where requires a or b {{let x=a;accept(x,false)}}"
        ),
        "precondition",
    );
    rejected(
        &format!(
            "{prefix} fn wrong(a:bool,b:bool)->bool where requires a or b {{var x=a;var y=b;x=false;y=false;accept(x,y)}}"
        ),
        "precondition",
    );
    rejected(
        &format!(
            "{prefix} fn wrong(a:bool,b:bool,flag:bool)->bool where requires a or b {{var x=false;var y=false;if flag {{x=a;y=b;}};accept(x,y)}}"
        ),
        "precondition",
    );
    rejected(
        &format!(
            "{prefix} fn wrong(a:bool,b:bool)->bool where requires a or b {{let x=a;let call=|| accept(x,false);call()}}"
        ),
        "precondition",
    );
}

#[test]
fn regrouped_disjunctions_do_not_strengthen_or_drop_operands() {
    rejected(
        "fn accept(a:bool,b:bool,c:bool)->bool where requires a or (b and c) {true} fn wrong(a:bool,b:bool,c:bool)->bool where requires a or (b or c) {accept(a,b,c)}",
        "precondition",
    );
    rejected(
        "fn accept(a:bool,b:bool)->bool where requires a or b {true} fn wrong(a:bool,b:bool,c:bool)->bool where requires (a or b) or c {accept(a,b)}",
        "precondition",
    );
}

#[test]
fn dependent_scalar_closures_do_not_inherit_creator_value_guarantees() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;";
    rejected(
        &format!(
            "{prefix} fn wrong(comptime T:Type,value:T)->T where requires T == Byte, requires value < 255 {{let apply=|item:T| item + 1;apply(value)}}"
        ),
        "machine arithmetic range is not proven",
    );
    rejected(
        &format!(
            "{prefix} fn accept(value:bool)->bool where requires value {{value}} fn wrong(comptime T:Type,value:T)->bool where requires T == bool, requires value {{let apply=|item:T| accept(item);apply(value)}}"
        ),
        "precondition",
    );
    rejected(
        &format!(
            "{prefix} fn wrong(comptime T:Type,value:T)->Byte where requires T == bool {{let apply=|item:Byte| item + 1;apply(255)}}"
        ),
        "machine arithmetic range is not proven",
    );
}

#[test]
fn symbolic_type_references_in_runtime_closures_do_not_enable_runtime_type_storage() {
    let prefix = "typealias Type=.Type'builtin;";
    rejected(
        &format!("{prefix} fn wrong(comptime T:Type) {{let call=|| T;call();}}"),
        "comptime-only value cannot inhabit runtime storage",
    );
    rejected(
        &format!("{prefix} fn wrong(comptime T:Type) {{let call=|| {{let saved=T;true}};call();}}"),
        "comptime-only value cannot inhabit runtime storage",
    );
    rejected(
        &format!(
            "{prefix} fn identity(comptime T:Type,value:T)->T {{value}} fn wrong(comptime T:Type,value:T)->T {{let call=|item:T| identity(bool,item);call(value)}}"
        ),
        "type equality",
    );
}

#[test]
fn staged_value_capture_elision_does_not_admit_runtime_integer_storage() {
    rejected(
        "typealias Integer=.Integer'builtin;fn tag(comptime n:Integer)->bool {true} fn wrong(comptime n:Integer) {let call=|| {let saved=n;tag(n)};call();}",
        "comptime-only value cannot inhabit runtime storage",
    );
    rejected(
        "typealias Integer=.Integer'builtin;fn wrong(comptime n:Integer) {let call=|| n;call();}",
        "comptime-only value cannot inhabit runtime storage",
    );
}

#[test]
fn deferred_integer_comparisons_do_not_promote_runtime_dependencies() {
    rejected(
        "fn choose(comptime flag:bool)->bool {flag} fn wrong(value:.u8'builtin)->bool {choose(value < 3)}",
        "comptime",
    );
    rejected(
        "typealias Integer=.Integer'builtin;fn choose(comptime flag:bool)->bool {flag} fn wrong(comptime n:Integer,value:bool)->bool {choose(n == 3 and value)}",
        "comptime",
    );
    rejected(
        "typealias Integer=.Integer'builtin;fn choose(comptime flag:bool)->bool {flag} fn runtime()->bool {true} fn wrong(comptime n:Integer)->bool {choose(n == 3 or runtime())}",
        "comptime",
    );
}

#[test]
fn deferred_float_computations_still_reject_runtime_inputs() {
    rejected(
        "fn choose(comptime flag:bool)->bool {flag} fn wrong(value:.f64'builtin)->bool {choose(value + 1.0 > 0.0)}",
        "comptime",
    );
    rejected(
        "fn choose(comptime flag:bool)->bool {flag} fn wrong(comptime flag:bool,value:.f32'builtin)->bool {choose(if flag {value == 0.0} else {true})}",
        "comptime",
    );
}

#[test]
fn floating_comptime_inputs_reject_width_mismatch_and_runtime_values() {
    rejected(
        "typealias F32=.f32'builtin;typealias F64=.f64'builtin;comptime fn single()->F32 {1.0} fn accept(comptime value:F64)->bool {true} fn wrong()->bool {accept(single())}",
        "type mismatch",
    );
    rejected(
        "typealias F64=.f64'builtin;fn accept(comptime value:F64)->bool {true} fn wrong(value:F64)->bool {accept(value)}",
        "comptime",
    );
}

#[test]
fn staged_constant_availability_does_not_escape_to_runtime_arguments() {
    rejected(
        "const FLAG=true;fn take(value:bool)->bool {value} fn run()->bool {take(FLAG)}",
        "cross-stage Splice",
    );
    rejected(
        "const FLAG=true;fn take(comptime flag:bool,value:bool)->bool {value} fn run()->bool {take(FLAG,FLAG)}",
        "cross-stage Splice",
    );
    rejected(
        "const FLAG=true;fn take(comptime flag:bool)->bool {flag} fn run()->bool {let first=take(FLAG);FLAG}",
        "cross-stage Splice",
    );
    rejected(
        "fn take(comptime flag:bool)->bool {flag} fn run(value:bool)->bool {take(value)}",
        "cannot read a runtime parameter",
    );
}

#[test]
fn staged_input_phase_errors_precede_capture_and_execution() {
    let prefix = "fn supply(comptime flag:bool)->bool {flag} fn runtime()->bool {true} ";
    for body in [
        "runtime()",
        "if false {runtime()} else {true}",
        "do {let flag=runtime();flag}",
    ] {
        rejected(
            &format!("{prefix} fn run()->bool {{supply({body})}}"),
            "runtime call in a comptime input",
        );
    }
    for body in ["value", "not value", "if false {value} else {true}"] {
        rejected(
            &format!("{prefix} fn run(value:bool)->bool {{supply({body})}}"),
            "cannot read a runtime parameter",
        );
    }
    // A pending comptime input remains legal to transport to another stage
    // boundary; failure above must not turn it into a runtime input.
    accepted(
        "fn supply(comptime flag:bool)->bool {flag} fn forward(comptime flag:bool)->bool {supply(not flag)} fn run()->bool {forward(true)}",
    );
}

#[test]
fn anonymous_record_patterns_reject_bad_fields_and_missing_coverage() {
    let prefix = "comptime fn schema(...properties:T)->T where T {properties} const SAMPLE=schema {flag:true,other:false};typealias Options=SAMPLE'type;";
    rejected(
        &format!("{prefix} fn read(options:Options)->bool {{let {{absent}}=options;absent}}"),
        "unknown struct pattern field",
    );
    rejected(
        &format!("{prefix} fn read(options:Options)->bool {{let {{flag:a,flag:b}}=options;a}}"),
        "duplicate struct pattern field",
    );
    rejected(
        &format!(
            "{prefix} fn read(options:Options)->bool {{options match {{{{flag:true}}=>true,}}}}"
        ),
        "non-exhaustive match",
    );
    rejected(
        &format!(
            "{prefix} fn read(options:Options)->bool {{options match {{{{flag:true,other:true}}=>true,{{flag:false,other:false}}=>false,}}}}"
        ),
        "non-exhaustive match",
    );
}

#[test]
fn anonymous_record_patterns_require_evidence_without_unfolding_opaque_heads() {
    rejected(
        "fn read(value:T)->bool where T {let {flag}=value;flag}",
        "record pattern requires a struct type",
    );
    let prefix = "typealias Type=.Type'builtin;comptime fn schema(...properties:T)->T where T {properties} const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;pure comptime fn Identity<T>->Type {T}";
    rejected(
        &format!("{prefix} fn read(value:Identity<Options>)->bool {{let {{flag}}=value;flag}}"),
        "structural type view remains unresolved",
    );
}

#[test]
fn unsigned_division_requires_nonzero_and_does_not_invent_signed_rounding() {
    rejected(
        "fn divide(a:.u32'builtin,b:.u32'builtin)->.u32'builtin {a / b}",
        "divisor is not proven nonzero",
    );
    rejected(
        "fn divide(a:.u8'builtin)->.u8'builtin {a / 0}",
        "divisor is not proven nonzero",
    );
    rejected(
        "fn divide(a:.i32'builtin,b:.i32'builtin)->.i32'builtin where requires b > 0 {a / b}",
        "Integer division operator rounding protocol",
    );
    rejected(
        "fn divide(a:.u32'builtin,b:.u32'builtin)->result:.u32'builtin where requires b != 0, ensures result < a {a / b}",
        "postcondition",
    );
}

#[test]
fn unsigned_division_does_not_reuse_mutated_dividend_evidence() {
    rejected(
        "typealias Int=.u8'builtin;
        fn denominator(p:*Int)->result:Int where ensures result == 1 {p.* = 0;1}
        fn divide(a:Int)->result:Int where ensures result <= a {a / denominator(a.ref)}",
        "postcondition",
    );
}

#[test]
fn unsigned_remainder_rejects_zero_false_contracts_and_unselected_rounding() {
    rejected(
        "fn remainder(a:.u32'builtin,b:.u32'builtin)->.u32'builtin {a % b}",
        "divisor is not proven nonzero",
    );
    rejected(
        "fn remainder(a:.u8'builtin)->.u8'builtin {a % 0}",
        "divisor is not proven nonzero",
    );
    rejected(
        "fn remainder(a:.u8'builtin,b:.u8'builtin)->result:.u8'builtin where requires b != 0, ensures result > 0 {a % b}",
        "postcondition",
    );
    for ty in ["i32", "Integer", "f32", "f64"] {
        rejected(
            &format!(
                "comptime fn remainder(a:.{ty}'builtin,b:.{ty}'builtin)->.{ty}'builtin {{a % b}}"
            ),
            "remainder rounding for this type",
        );
    }
}

#[test]
fn unsigned_remainder_does_not_keep_a_bound_on_a_mutated_divisor() {
    rejected(
        "typealias Int=.u8'builtin;
        fn zero(p:*Int) {p.* = 0;}
        fn remainder(a:Int,b:Int)->result:Int where requires b != 0, ensures result < b {
            let saved=a % b;zero(b.ref);saved
        }",
        "postcondition",
    );
}

#[test]
fn unsigned_round_down_does_not_license_an_unrelated_or_mutated_multiplier() {
    rejected(
        "typealias UInt=.u32'builtin;
        fn bad(n:UInt,alignment:UInt,other:UInt)->UInt where requires alignment != 0 {
            (n / alignment) * other
        }",
        "machine arithmetic range is not proven",
    );
    rejected(
        "typealias UInt=.u32'builtin;
        fn change(p:*UInt) {p.* = 4294967295;}
        fn bad(n:UInt,alignment:UInt)->UInt where requires alignment != 0 {
            let q=n / alignment;change(alignment.ref);q * alignment
        }",
        "machine arithmetic range is not proven",
    );
}

#[test]
fn symbol_indexed_trait_methods_select_complete_interfaces() {
    let source = "typealias Symbol=.Symbol'builtin;
        trait Choose where suffix:Symbol {fn choose(self)->bool;}
        impl Choose<.px> for bool {fn choose(self)->bool {self}}
        impl Choose<.em> for bool {fn choose(self)->bool {not self}}
        fn generic(value:T)->bool where T :- Choose<.px> {value.choose()}
        fn run()->bool {generic(true) and true.as(Choose<.px>).choose() and false.as(Choose<.em>).choose()}";
    for source in [source.to_owned(), source
        .replace("fn generic", "trait Child :- Choose<suffix> where suffix:Symbol {} impl Child<.px> for bool {} fn generic")
        .replace("T :- Choose<.px>", "T :- Child<.px>")] {
        accepted(&source);
        let executed = Sandbox::new(&source).run(&["--interpret", "run"]);
        assert!(executed.status.success(), "{}", details(&executed));
        assert_eq!(String::from_utf8_lossy(&executed.stdout).trim(), "(bool true)");
    }
    rejected(&source.replace("{self}", "{()}"), "mismatch");
}

#[test]
fn symbol_indexed_trait_program_contract_is_enforced() {
    let source = "typealias Symbol=.Symbol'builtin;
        trait Choose where suffix:Symbol {fn choose(self,flag:bool)->bool where requires flag;}
        impl Choose<.px> for bool {fn choose(self,flag:bool)->bool {self}}
        fn run()->bool {true.as(Choose<.px>).choose(true)}";
    let executed = Sandbox::new(source).run(&["--interpret", "run"]);
    assert!(executed.status.success(), "{}", details(&executed));
    let failed =
        Sandbox::new(&source.replace("choose(true)", "choose(false)")).run(&["--interpret", "run"]);
    assert!(!failed.status.success(), "{}", details(&failed));
    assert!(
        details(&failed).contains("program precondition is not proven"),
        "{}",
        details(&failed)
    );
}

#[test]
fn symbol_indexed_trait_generic_methods_infer_local_types() {
    let source = "typealias Symbol=.Symbol'builtin;
        trait Keep where suffix:Symbol {fn keep(self,value:U)->U where U;}
        impl Keep<.px> for bool {fn keep(self,value:U)->U where U {value}}
        fn generic(value:T)->bool where T :- Keep<.px> {value.keep(true)}
        fn run()->bool {generic(false) and false.as(Keep<.px>).keep(true)}";
    for source in [
        source.to_owned(),
        source
            .replace("value:U)->U", "value:U,.flag:bool=true)->bool")
            .replace("where U {value}", "where U {flag}"),
        source.replace(".keep(true)", ".keep<bool>(true)"),
        format!(
            "trait Flag {{}} impl Flag for bool {{}} {}",
            source
                .replace("where U;", "where U :- Flag;")
                .replace("where U {", "where U :- Flag {")
        ),
    ] {
        accepted(&source);
        let executed = Sandbox::new(&source).run(&["--interpret", "run"]);
        assert!(executed.status.success(), "{}", details(&executed));
        assert_eq!(
            String::from_utf8_lossy(&executed.stdout).trim(),
            "(bool true)"
        );
    }
    let constrained = format!(
        "trait Flag {{}} {}",
        source
            .replace("where U;", "where U :- Flag;")
            .replace("where U {", "where U :- Flag {")
    );
    rejected(&constrained, "trait bound");
    rejected(
        &source.replace(".keep(true)", ".keep<bool>(())"),
        "mismatch",
    );
}

#[test]
fn symbol_indexed_trait_default_methods_execute_selected_bodies() {
    let source = "typealias Symbol=.Symbol'builtin;
        trait Choose where suffix:Symbol {fn choose(self)->bool {true}}
        impl Choose<.px> for bool {}
        impl Choose<.em> for bool {fn choose(self)->bool {self}}
        fn generic(value:T)->bool where T :- Choose<.px> {value.choose()}
        fn run()->bool {generic(false) and false.as(Choose<.px>).choose() and not false.as(Choose<.em>).choose()}";
    for source in [
        "typealias Symbol=.Symbol'builtin;
         trait Keep where suffix:Symbol {fn keep(self,value:U)->U where U {value}}
         impl Keep<.px> for bool {}
         fn run()->bool {false.as(Keep<.px>).keep(true)}"
            .to_owned(),
        source.to_owned(),
        source
            .replace(
                "fn choose(self)->bool",
                "fn choose(self,flag:bool)->bool where requires flag",
            )
            .replace(".choose()", ".choose(true)"),
    ] {
        accepted(&source);
        let executed = Sandbox::new(&source).run(&["--interpret", "run"]);
        assert!(executed.status.success(), "{}", details(&executed));
        assert_eq!(
            String::from_utf8_lossy(&executed.stdout).trim(),
            "(bool true)"
        );
    }
}

#[test]
fn symbol_indexed_default_method_delegates_to_selected_member() {
    let source = "typealias Symbol=.Symbol'builtin;
        trait Choose where suffix:Symbol {
            fn choose(self)->bool;
            fn delegated(self)->bool {self.choose()}
        }
        impl Choose<.px> for bool {fn choose(self)->bool {self}}
        impl Choose<.em> for bool {fn choose(self)->bool {not self}}
        fn run()->bool {true.as(Choose<.px>).delegated() and false.as(Choose<.em>).delegated()}";
    let inherited = source.replace("fn delegated(self)->bool {self.choose()}", "")
        .replace("fn run()", "trait Delegate :- Choose<suffix> where suffix:Symbol {fn delegated(self)->bool {self.choose()}} impl Delegate<.px> for bool {} impl Delegate<.em> for bool {} fn run()")
        .replace(".as(Choose<", ".as(Delegate<");
    for source in [source.to_owned(), inherited] {
        accepted(&source);
        let executed = Sandbox::new(&source).run(&["--interpret", "run"]);
        assert!(executed.status.success(), "{}", details(&executed));
        assert_eq!(
            String::from_utf8_lossy(&executed.stdout).trim(),
            "(bool true)"
        );
    }
}

#[test]
fn unresolved_abi_attributes_cannot_publish_default_abi_functions() {
    for source in [
        "^attrs.abi(.c) fn identity(value:bool)->bool {value}",
        "^attrs.abi(.c) inline fn identity(value:bool)->bool {value}",
        "^attrs.abi(.c) pure fn identity(value:bool)->bool {value}",
        "^attrs.abi(.c) fn identity(value:bool)->bool {value} fn entry()->bool {identity(true)}",
        "^attrs.abi(.c) fn identity(value:bool)->bool {value} fn entry()->fn(bool)->bool {identity}",
        "trait Foreign { ^attrs.abi(.c) fn identity(value:bool)->bool; }",
        "mod foreign { ^attrs.abi(.c) fn identity(value:bool)->bool {value} }",
    ] {
        rejected(source, "definition attribute or modifier");
    }
}
