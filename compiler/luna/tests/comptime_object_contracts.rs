//! Metadata construction preserves producer effects under program verification.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[test]
fn object_construction_checks_contracts_in_producer_order() {
    let source = "comptime fn checked(value:bool)->bool where requires value {value}
        comptime fn build()->.Object'builtin where requires true {
            var state=false;
            let object={first:do {state=true;state},second:checked(state)};
            object
        }
        const VALUE=build();";
    support::compile(source);
    let bad = source.replace(
        "first:do {state=true;state},second:checked(state)",
        "second:checked(state),first:do {state=true;state}",
    );
    let error = support::compile_with_protocols(&bad, &Default::default()).unwrap_err();
    assert!(
        error.contains("precondition") || error.contains("contract"),
        "{error}"
    );
}

#[test]
fn object_result_does_not_discard_input_postconditions() {
    let source = "comptime fn build(value:bool)->.Object'builtin where requires value, ensures value {let object={flag:value};object} const VALUE=build(true);";
    support::compile(source);
    let invalid = source.replace("ensures value", "ensures not value");
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        error.contains("postcondition") || error.contains("contract"),
        "{error}"
    );
}
