//! Source inputs cross closures and local declarations, but internal binders
//! never become inputs supplied by an enclosing invocation.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_interpretor::Value;

#[test]
fn staged_arguments_observe_captures_and_exclude_internal_bindings() {
    for initializer in [
        "do { let inner = flag; inner }",
        "flag match { inner => inner }",
        "do { pure comptime fn capture(inner: bool) -> bool { flag and inner } capture(true) }",
    ] {
        let source = format!(
            "comptime fn Pick(flag: bool) -> Type {{ if flag {{ bool }} else {{ u8 }} }}
            fn consume(comptime flag: bool, value: defer Pick(flag)) -> defer Pick(flag) {{ value }}
            fn forward(comptime flag: bool, value: bool) -> bool {{
                consume({initializer}, value)
            }}
            fn outer(comptime flag: bool, value: bool) -> bool {{ forward(flag, value) }}
            fn run() -> bool {{ outer(true, true) }}"
        );
        let package =
            sysroot::compile(&source).unwrap_or_else(|error| panic!("{initializer}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 200_000).unwrap(),
            Value::Bool(true)
        );
        let graph = luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn captured_runtime_values_cannot_become_staged_arguments() {
    for initializer in [
        "do { let flag = runtime_flag; flag }",
        "(|flag: bool| runtime_flag and flag)(true)",
    ] {
        let source = format!(
            "comptime fn Pick(flag: bool) -> Type {{ if flag {{ bool }} else {{ u8 }} }}
            fn consume(comptime flag: bool, value: defer Pick(flag)) -> defer Pick(flag) {{ value }}
            fn forward(comptime flag: bool, runtime_flag: bool, value: bool) -> bool {{
                consume({initializer}, value)
            }}
            fn run(runtime_flag: bool) -> bool {{ forward(true, runtime_flag, true) }}"
        );
        let failure = sysroot::compile(&source)
            .err()
            .unwrap_or_else(|| panic!("runtime capture accepted: {initializer}"));
        assert!(failure.contains("comptime"), "{initializer}: {failure}");
    }
}

#[test]
fn staged_closure_arguments_keep_captured_invocation_inputs() {
    for input in [
        "(|inner: bool| flag and inner)(true)",
        "do { let capture = |inner: bool| flag and inner; capture(true) }",
        "do { let outer = || { let inner = || flag; inner() }; outer() }",
        "do { let toggle = || { flag = not flag; flag }; toggle(); toggle() }",
        "do { let toggle = || { flag = not flag; flag }; toggle(); flag }",
    ] {
        let source = format!(
            "comptime fn Pick(flag: bool) -> Type {{ if flag {{ bool }} else {{ u8 }} }}
            fn consume(comptime flag: bool, value: defer Pick(flag)) -> defer Pick(flag) {{ value }}
            fn forward(comptime flag: bool, value: defer Pick(flag)) -> defer Pick(flag) {{
                consume({input}, value)
            }}
            fn outer(comptime flag: bool, value: defer Pick(flag)) -> defer Pick(flag) {{
                forward(flag, value)
            }}
            fn run_bool() -> bool {{ outer(true, true) }}
            fn run_u8() -> bool {{ outer(false, 9) == 9 }}"
        );
        let package = sysroot::compile(&source).unwrap_or_else(|error| panic!("{input}: {error}"));
        for (root, expected) in [
            ("run_bool", Value::Bool(true)),
            ("run_u8", Value::Bool(true)),
        ] {
            assert_eq!(
                luna::execution::interpret(&package, root, 200_000).unwrap(),
                expected,
                "{input}: {root}"
            );
            let graph = luna::instantiation::instantiate(&package, root, 200_000).unwrap();
            let mut vm =
                comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                expected,
                "{input}: {root}"
            );
        }
    }
}

#[test]
fn staged_closure_arguments_do_not_restore_invalidated_runtime_inputs() {
    for prefix in [
        "flag = false;",
        "let pointer = flag.ref;",
        "let capture = || flag;",
    ] {
        let source = format!(
            "comptime fn Pick(flag: bool) -> Type {{ if flag {{ bool }} else {{ u8 }} }}
            fn consume(comptime flag: bool, value: defer Pick(flag)) -> defer Pick(flag) {{ value }}
            fn forward(comptime flag: bool, value: bool) -> bool {{
                {prefix}
                consume((|| flag)(), value)
            }}
            fn run() -> bool {{ forward(true, true) }}"
        );
        let failure = sysroot::compile(&source)
            .err()
            .unwrap_or_else(|| panic!("stale runtime input accepted: {prefix}"));
        assert!(failure.contains("not available"), "{prefix}: {failure}");
    }
}
