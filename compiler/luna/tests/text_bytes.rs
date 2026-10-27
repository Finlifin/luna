#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

fn source(body: &str) -> String {
    format!(
        "typealias str=.str'builtin;typealias u64=.u64'builtin;typealias u8=.u8'builtin;mod text {{{}}}{body}",
        include_str!("../../../library/builtin/text_bytes.fl")
    )
}

#[test]
fn utf8_bytes_and_boundaries_agree_in_both_phases() {
    let body = r#"
        fn run()->bool {
            let size=text.byte_length("a🦀");
            let empty=text.byte_length("");
            let first=text.byte("a🦀",1) match {value?=>value == 240,null=>false};
            let last=text.byte("a🦀",4) match {value?=>value == 128,null=>false};
            let past=text.byte("a🦀",5) match {value?=>false,null=>true};
            let far=text.byte("",18446744073709551615) match {value?=>false,null=>true};
            size == 5 and empty == 0 and first and last and past and far
        }
    "#;
    let staged = body
        .replace("fn run", "comptime fn staged")
        .replace("text.byte_length", "text.comptime_byte_length")
        .replace("text.byte(", "text.comptime_byte(");
    let package = support::compile(&source(&format!("{body}{staged}")));
    assert!(format!("{package}").contains("(text-query byte-length"));
    assert_eq!(
        luna::invocation::invoke(&package, "staged", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let package = support::compile(&source(&format!(
        "{body} fn verify() {{let valid=run();assumes valid;}}"
    )));
    let ir = luna::host::llvm_ir(&package, "verify", 100_000).unwrap();
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
fn byte_operands_execute_once_left_to_right() {
    let package = support::compile(&source(
        r#"
        fn observe(state:*u64)->str {let old=state.*;assumes old == 0;state.* = 1;"az"}
        fn position(state:*u64)->u64 {let index=state.*;assumes index == 1;state.* = 2;index}
        fn run()->bool {
            let state:u64=0;
            let byte=text.byte(observe(state.ref),position(state.ref));
            byte match {value?=>value == 122 and state == 2,null=>false}
        }
    "#,
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn text_queries_preserve_early_exit_and_reject_wrong_input_domains() {
    let package = support::compile(&source(
        "fn run()->bool {text.byte(do {return true;},0);false} comptime fn staged()->bool {text.comptime_byte(do {return true;},0);false}",
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "staged", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "text.byte_length(true)",
        "text.byte(\"a\",false)",
        "text.byte(\"a\",-1)",
    ] {
        assert!(
            support::compile_with_protocols(
                &source(&format!("fn invalid() {{{body};}}")),
                &Default::default()
            )
            .is_err()
        );
    }
}
