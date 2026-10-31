//! Delayed exact integer formation must preserve instance-specific domains.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;
use support::{compile, compile_with_protocols};

fn native_true(source: &str) {
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100000).unwrap(),
        Value::Bool(true)
    );
    let ir = luna::host::llvm_ir(&package, "run", 100000).unwrap();
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
fn generic_integer_boundaries_and_default_values() {
    native_true(
        "fn minimum()->T where T, requires T == .i8'builtin or T == .i128'builtin { -128 }
        fn maximum()->T where T, requires T == .u8'builtin or T == .u128'builtin {255}
        fn choose(.value:T=255)->T where T, requires T == .u8'builtin or T == .u128'builtin {value}
        fn run()->bool {minimum<.i8'builtin>() == -128 and minimum<.i128'builtin>() == -128
            and maximum<.u8'builtin>() == 255 and maximum<.u128'builtin>() == 255
            and choose<.u8'builtin>() == 255}",
    );
}

#[test]
fn overflow_and_negative_unsigned_do_not_poison_valid_instances() {
    for (literal, valid, invalid, expected) in [
        (
            "256",
            ".u16'builtin",
            ".u8'builtin",
            Value::UnsignedInteger(256),
        ),
        (
            " -1 ",
            ".i8'builtin",
            ".u8'builtin",
            Value::SignedInteger(-1),
        ),
    ] {
        let package = compile(&format!(
            "fn value()->T where T, requires T == {valid} or T == {invalid} {{{literal}}}
            fn good()->{valid} {{value<{valid}>()}} fn bad()->{invalid} {{value<{invalid}>()}}"
        ));
        let error = luna::instantiation::instantiate(&package, "bad", 100000).unwrap_err();
        assert!(error.contains("integer formation"), "{error}");
        assert_eq!(
            luna::execution::interpret(&package, "good", 100000).unwrap(),
            expected
        );
        assert!(luna::instantiation::instantiate(&package, "good", 1).is_err());
        assert!(luna::instantiation::instantiate(&package, "good", 100000).is_ok());
    }
}

#[test]
fn generic_integer_domain_must_be_proven() {
    for source in [
        "fn value()->T where T {1}",
        "fn value()->T where T, requires T == .u8'builtin or T == bool {1}",
        "pure comptime fn Identity<T>->.Type'builtin {T} fn value()->Identity<T> where T {1}",
    ] {
        assert!(
            compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn exact_comptime_integer_and_captured_generic_local() {
    native_true("comptime fn large()->T where T, requires T == .Integer'builtin or T == .u128'builtin {340282366920938463463374607431768211455}
        comptime fn exact()->bool {large<.Integer'builtin>() == 340282366920938463463374607431768211455}
        fn make()->%{value:T} fn()->T where T, requires T == .u8'builtin or T == .u16'builtin {let value:T=255;||value}
        fn run()->bool {const valid=exact();const wide=large<.u128'builtin>();let action=make<.u8'builtin>();valid and action()==255 and wide==340282366920938463463374607431768211455}");
}

#[test]
fn iir_rejects_unformed_integer_with_concrete_annotation() {
    let package = compile("fn run()->.u8'builtin {1}");
    let mut graph = luna::instantiation::instantiate(&package, "run", 100000).unwrap();
    graph.functions[0].function.body.kind =
        nir::program::ExprKind::Literal(nir::program::Literal::PendingInteger {
            spelling: "1".into(),
            negative: false,
        });
    assert!(
        graph
            .validate(&mut 100000)
            .unwrap_err()
            .contains("unformed float or integer")
    );
}

#[test]
fn exact_integer_formation_does_not_authorize_runtime_storage() {
    let source = "fn value()->T where T, requires T == .Integer'builtin or T == .u8'builtin {1}
        fn run()->.Integer'builtin {value<.Integer'builtin>()}";
    let error = match compile_with_protocols(source, &Default::default()) {
        Ok(package) => luna::instantiation::instantiate(&package, "run", 100000).unwrap_err(),
        Err(error) => error,
    };
    assert!(
        error.contains("comptime-only value cannot inhabit runtime storage"),
        "{error}"
    );
}

#[test]
fn generic_integer_formation_survives_quote_and_default_callback() {
    for body in [
        "quote fn produce()->T where T, requires T == .i8'builtin or T == .i128'builtin { -128 }
         fn calculate()->T where T, requires T == .i8'builtin or T == .i128'builtin {produce<T>()}",
        "fn calculate(.action:fn()->T=|| { -128 })->T where T, requires T == .i8'builtin or T == .i128'builtin {action()}",
    ] {
        native_true(&format!(
            "{body} fn run()->bool {{calculate<.i8'builtin>() == -128 and calculate<.i128'builtin>() == -128}}"
        ));
    }
}
