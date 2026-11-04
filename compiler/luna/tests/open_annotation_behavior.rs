//! A deferred annotation with known input domains retains its complete behavior.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

#[test]
fn guarded_indices_keep_branch_facts_and_original_bindings() {
    let expressions = [
        "if n < 255 { n + 1 } else { n }",
        "do { let value = n; if value < 255 { value + 1 } else { value } }",
        "if false { n + 1 } else { if n < 255 { n + 1 } else { n } }",
    ];
    let mut source = String::from(
        "pure comptime fn Index<n: u8> -> Type { bool }
        fn low(value: Index<10>) -> bool { true }
        fn high(value: Index<255>) -> bool { true }",
    );
    for (index, expression) in expressions.iter().enumerate() {
        source.push_str(&format!(
            "comptime fn accept_{index}(n: u8, callback: defer fn(Index<{expression}>) -> bool) -> bool {{ true }}
            const low_{index} = accept_{index}(9, low);
            const high_{index} = accept_{index}(255, high);"
        ));
    }
    source.push_str(
        "comptime fn run() -> bool { low_0 and high_0 and low_1 and high_1 and low_2 and high_2 }",
    );
    let package = sysroot::compile(&source).unwrap();
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration;
    for tree in [false, true] {
        let mut vm = if tree {
            comptime_interpretor::Interpreter::for_comptime_tree(&package, Default::default())
                .unwrap()
        } else {
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap()
        };
        assert_eq!(
            vm.invoke(run, vec![]).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn unsafe_arithmetic_is_rejected_at_definition_without_an_invocation() {
    for expression in [
        "n + 1",
        "if n < 255 { n } else { n + 1 }",
        "do { let value = n; value + 1 }",
    ] {
        let source = format!(
            "pure comptime fn Index<n: u8> -> Type {{ bool }}
            comptime fn accept(n: u8, callback: defer fn(Index<{expression}>) -> bool) -> bool {{ true }}"
        );
        let Err(error) = sysroot::compile(&source) else {
            panic!("unchecked annotation accepted: {expression}");
        };
        assert!(
            error.contains("machine arithmetic range is not proven"),
            "{error}"
        );
    }
}

#[test]
fn result_annotation_is_checked_before_any_call_can_supply_a_safe_value() {
    let source = "pure comptime fn Index<n: u8> -> Type { bool }
        comptime fn factory(n: u8) -> defer fn(Index<n + 1>) -> bool { while true {} }";
    let Err(error) = sysroot::compile(source) else {
        panic!("unchecked result annotation accepted");
    };
    assert!(
        error.contains("machine arithmetic range is not proven"),
        "{error}"
    );
}

#[test]
fn unrelated_pending_domains_do_not_hide_parameter_or_result_arithmetic() {
    let prefix = "pure comptime fn Index<n: u8> -> Type { bool }
        comptime fn Domain(flag: bool) -> Type { if flag { Type } else { bool } }";
    for definition in [
        "comptime fn accept(flag: bool, T: defer Domain(flag), n: u8,
            callback: defer fn(Index<n + 1>) -> bool) -> bool { true }",
        "comptime fn factory(flag: bool, T: defer Domain(flag), n: u8,
            callback: defer fn(T) -> bool) -> defer fn(Index<n + 1>) -> bool { while true {} }",
    ] {
        let Err(error) = sysroot::compile(&format!("{prefix} {definition}")) else {
            panic!("unrelated pending domain suppressed the arithmetic check");
        };
        assert!(
            error.contains("machine arithmetic range is not proven"),
            "{error}"
        );
    }
}

#[test]
fn guarded_annotation_is_checked_despite_unrelated_pending_inputs() {
    let package = sysroot::compile(
        "pure comptime fn Index<n: u8> -> Type { bool }
        comptime fn Domain(flag: bool) -> Type { if flag { Type } else { bool } }
        fn low(value: Index<10>) -> bool { true }
        fn high(value: Index<255>) -> bool { true }
        comptime fn accept(flag: bool, T: defer Domain(flag), n: u8,
            callback: defer fn(Index<if n < 255 { n + 1 } else { n }>) -> bool) -> bool { true }
        comptime fn run() -> bool { accept(true, bool, 9, low) and accept(true, bool, 255, high) }",
    )
    .unwrap();
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration;
    for tree in [false, true] {
        let mut vm = if tree {
            comptime_interpretor::Interpreter::for_comptime_tree(&package, Default::default())
                .unwrap()
        } else {
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap()
        };
        assert_eq!(
            vm.invoke(run, vec![]).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn type_components_and_closure_captures_keep_pending_dependencies() {
    for annotation in [
        "fn(T) -> T",
        "do { let choose = || flag; if choose() { fn(T) -> T } else { fn(bool) -> bool } }",
    ] {
        let source = format!(
            "comptime fn Domain(flag: bool) -> Type {{ if flag {{ Type }} else {{ bool }} }}
            fn identity(value: bool) -> bool {{ value }}
            fn accept(comptime flag: bool, comptime T: defer Domain(flag), callback: defer {annotation}) -> bool {{ true }}
            fn run() -> bool {{ accept(true, bool, identity) }}"
        );
        assert_eq!(
            sysroot::run(&source),
            comptime_interpretor::Value::Bool(true)
        );
    }
}
