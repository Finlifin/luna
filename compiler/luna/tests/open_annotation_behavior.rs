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
