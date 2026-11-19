#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

fn source(body: &str) -> String {
    format!("{}{body}", include!("support/text_source.rs"))
}

// This fixture embeds the shipped meta implementation in package zero.
fn compile_text(source: &str) -> nir::program::Package {
    support::compile_with_protocols(source, &support::meta_fixture_protocols()).unwrap()
}

#[test]
fn runtime_function_alias_constants_require_cross_stage_conversion() {
    let error = support::compile_with_protocols(
        &source(
            "fn identity(value: bool) -> bool { value }
        const alias = identity;
        fn run() -> bool { alias(true) }",
        ),
        &support::meta_fixture_protocols(),
    )
    .unwrap_err();
    assert!(error.contains("Splice"), "{error}");
}

#[test]
fn utf8_bytes_and_boundaries_agree_in_both_phases() {
    let body = r#"
        fn run()->bool {
            let size="a🦀".byte_length();
            let empty="".byte_length();
            let first="a🦀".byte(1) match {value?=>value == 240,null=>false};
            let last="a🦀".byte(4) match {value?=>value == 128,null=>false};
            let past="a🦀".byte(5) match {value?=>false,null=>true};
            let far="".byte(18446744073709551615) match {value?=>false,null=>true};
            size == 5 and empty == 0 and first and last and past and far
        }
    "#;
    let staged = body.replace("fn run", "comptime fn staged");
    let package = compile_text(&source(&format!("{body}{staged}")));
    assert!(format!("{package}").contains("(text-query byte-length"));
    assert_eq!(
        luna::invocation::invoke(&package, "staged", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let package = compile_text(&source(&format!(
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
    let package = compile_text(&source(
        r#"
        fn observe(state:*u64)->str {let old=state.*;assumes old == 0;state.* = 1;"az"}
        fn position(state:*u64)->u64 {let index=state.*;assumes index == 1;state.* = 2;index}
        fn run()->bool {
            let state:u64=0;
            let byte=observe(state.ref).byte(position(state.ref));
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
    let package = compile_text(&source(
        "fn run()->bool {str.byte(do {return true;},0);false} comptime fn staged()->bool {str.byte(do {return true;},0);false}",
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
        "str.byte_length(true)",
        "str.byte(\"a\",false)",
        "str.byte(\"a\",-1)",
    ] {
        assert!(
            support::compile_with_protocols(
                &source(&format!("fn invalid() {{{body};}}")),
                &support::meta_fixture_protocols()
            )
            .is_err()
        );
    }
}

#[test]
fn symbol_bytes_observe_utf8_without_materializing_string_views() {
    let aliases = "typealias Symbol=.Symbol'builtin;typealias u64=.u64'builtin;";
    let body = "fn run()->bool {
        let symbol:Symbol=.é;
        let size=.symbol_byte_length'builtin(symbol);
        let first=.symbol_byte'builtin(symbol,0) match {byte?=>byte == 195,null=>false};
        let last=.symbol_byte'builtin(symbol,1) match {byte?=>byte == 169,null=>false};
        let past=.symbol_byte'builtin(symbol,2) match {_?=>false,null=>true};
        let far=.symbol_byte'builtin(symbol,18446744073709551615) match {_?=>false,null=>true};
        size == 2 and first and last and past and far
    }";
    let staged = body.replace("fn run", "comptime fn staged");
    let package = support::compile(&format!(
        "{aliases}{body}{staged} fn verify() {{let result=run();assumes result;}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "staged", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(format!("{package}").contains("(text-query symbol-byte-length"));
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
    let declaration = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration;
    let summary = elaboration::verification::memory::infer_text_regions(
        &package,
        declaration,
        &[],
        &mut 100_000,
    )
    .unwrap();
    assert_eq!(
        summary.reads,
        std::collections::BTreeSet::from([
            elaboration::verification::memory::TextRegionOrigin::Static
        ])
    );
}

#[test]
fn symbol_byte_queries_preserve_operand_order_and_early_exit() {
    let package = support::compile(
        "typealias Symbol=.Symbol'builtin;typealias u64=.u64'builtin;
        fn observe(state:*u64)->Symbol {assumes state.* == 0;state.* = 1;.az}
        fn position(state:*u64)->u64 {assumes state.* == 1;state.* = 2;1}
        fn run()->bool {
            var state:u64=0;
            let byte=.symbol_byte'builtin(observe(state.ref),position(state.ref));
            byte match {value?=>value == 122 and state == 2,null=>false}
        }
        fn early()->bool {.symbol_byte'builtin(do {return true;},0);false}
        comptime fn staged()->bool {.symbol_byte'builtin(do {return true;},0);false}",
    );
    for name in ["run", "early"] {
        assert_eq!(
            luna::execution::interpret(&package, name, 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    assert_eq!(
        luna::invocation::invoke(&package, "staged", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn symbol_byte_queries_keep_distinct_domains_and_checked_indices() {
    use nir::{Primitive as P, Term, program::TextQueryKind as Q};
    let ty = |p| Term::Primitive(p);
    let result = Term::Optional(Box::new(ty(P::U8)));
    assert!(Q::SymbolByte.validate(&Term::Tuple(vec![ty(P::Symbol), ty(P::U64)]), &result));
    assert!(!Q::SymbolByte.validate(&Term::Tuple(vec![ty(P::Str), ty(P::U64)]), &result));
    assert!(!Q::Byte.validate(&Term::Tuple(vec![ty(P::Symbol), ty(P::U64)]), &result));
    assert!(!Q::SymbolByte.validate(&Term::Tuple(vec![ty(P::Symbol), ty(P::U8)]), &result));
    assert!(!Q::SymbolByte.validate(&Term::Tuple(vec![ty(P::Symbol), ty(P::U64)]), &ty(P::U8)));
    for expression in [
        ".symbol_byte'builtin(\"text\",0)",
        ".symbol_byte'builtin(.name,true)",
        ".symbol_byte'builtin(.name)",
        ".symbol_byte_length'builtin(\"text\")",
    ] {
        assert!(
            support::compile_with_protocols(
                &format!("fn invalid() {{{expression};}}"),
                &support::meta_fixture_protocols()
            )
            .is_err(),
            "{expression}"
        );
    }
}

#[test]
fn explicit_runtime_text_trait_does_not_bypass_comptime_phase_checks() {
    let error = support::compile_with_protocols(
        &source("comptime fn invalid()->u64 {\"abc\".as(text.TextBytes).byte_length()}"),
        &support::meta_fixture_protocols(),
    )
    .unwrap_err();
    assert!(!error.contains("parsing failed"), "{error}");
    assert!(
        error.contains("runtime") || error.contains("phase"),
        "{error}"
    );
}

#[test]
fn runtime_symbol_operation_bindings_keep_alias_identity_and_argument_order() {
    let package = support::compile_with_bindings(&source(
        "trait Splice { assoc RuntimeTarget: Type; fn splice(comptime data: Self) -> RuntimeTarget; }
        typealias Length = fn(Symbol) -> u64;
        typealias Byte = fn(Symbol, u64) -> ?u8;
        impl Splice for Length {
            assoc RuntimeTarget: Type = Length;
            fn splice(comptime data: Length) -> Length { data }
        }
        impl Splice for Byte {
            assoc RuntimeTarget: Type = Byte;
            fn splice(comptime data: Byte) -> Byte { data }
        }
        const length = bindings.symbol_byte_length;
        const byte = bindings.symbol_byte;
        fn run() -> bool {
            var trace: u64 = 0;
            let count = length(do {trace = trace * 10 + 1; .hello});
            let last = byte(do {trace = trace * 10 + 2; .hello},
                do {trace = trace * 10 + 3; 4});
            trace == 123 and count == 5
                and (last match {value? => value == 111, null => false})
                and (byte(.hello, 5) match {_? => false, null => true})
        }
        fn early() -> bool {
            byte(do {return true; .hello}, 0);
            false
        }",
    ), &support::meta_fixture_protocols(), Some("Splice")).unwrap();
    for name in ["run", "early"] {
        assert_eq!(
            luna::execution::interpret(&package, name, 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    for name in ["symbol_byte_length", "symbol_byte"] {
        let functions = package
            .functions
            .iter()
            .filter(|function| {
                function.name == format!("<compiler-operation.{name}>")
                    && function.phase == nir::program::FunctionPhase::Runtime
            })
            .collect::<Vec<_>>();
        assert_eq!(
            functions.len(),
            1,
            "aliases must share the original operation body"
        );
    }
}

#[test]
fn comptime_text_slice_and_concat_enforce_utf8_boundaries_and_phase() {
    use nir::{Primitive as P, Term, program::TextQueryKind as Q};
    let ty = |p| Term::Primitive(p);
    assert!(Q::Slice.validate(
        &Term::Tuple(vec![ty(P::Str), ty(P::U64), ty(P::U64)]),
        &Term::Optional(Box::new(ty(P::Str))),
    ));
    assert!(Q::Concat.validate(&Term::Tuple(vec![ty(P::Str), ty(P::Str)]), &ty(P::Str)));
    assert!(!Q::Concat.validate(&Term::Tuple(vec![ty(P::Str), ty(P::U64)]), &ty(P::Str)));

    let package = compile_text(&source(
        r#"
        comptime fn run() -> bool {
            let sample = "a🦀界";
            let full = sample.slice(0, 8) match {
                s? => s.byte_length() == 8 and s.starts_with("a🦀") and s.ends_with("界"),
                null => false,
            };
            let middle = sample.slice(1, 5) match {
                s? => s.byte_length() == 4 and s.starts_with("🦀"),
                null => false,
            };
            let empty_mid = sample.slice(1, 1) match {
                s? => s.byte_length() == 0,
                null => false,
            };
            let empty_end = sample.slice(8, 8) match {
                s? => s.byte_length() == 0,
                null => false,
            };
            let joined = "a".concat("🦀").concat("界");
            let empty_join = "".concat("界").concat("");
            let inverted = sample.slice(5, 1) match { _? => false, null => true };
            let past_end = sample.slice(0, 9) match { _? => false, null => true };
            let start_past = sample.slice(9, 9) match { _? => false, null => true };
            let mid_start = sample.slice(2, 5) match { _? => false, null => true };
            let mid_end = sample.slice(1, 4) match { _? => false, null => true };
            let mid_cjk = sample.slice(6, 8) match { _? => false, null => true };
            full and middle and empty_mid and empty_end
                and joined.byte_length() == 8 and joined.starts_with("a🦀") and joined.ends_with("界")
                and empty_join.byte_length() == 3 and empty_join.starts_with("界")
                and inverted and past_end and start_past and mid_start and mid_end and mid_cjk
        }
        "#,
    ));
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );

    for invalid in [
        "fn bad(s: str) -> ?str { s.as(meta.ComptimeTextBytes).slice(0, 1) }",
        "fn bad(a: str, b: str) -> str { a.as(meta.ComptimeTextBytes).concat(b) }",
    ] {
        let error =
            support::compile_with_protocols(&source(invalid), &support::meta_fixture_protocols())
                .unwrap_err();
        assert!(
            error.contains("runtime") || error.contains("phase") || error.contains("comptime"),
            "{invalid}: {error}"
        );
    }
}
