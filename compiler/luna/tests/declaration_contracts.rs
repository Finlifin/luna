//! Program contracts retain the domains of completed declaration inputs.
#[path = "support/sysroot_execution.rs"]
mod sysroot;
use comptime_interpretor::Value;

#[test]
fn dependent_declaration_domains_can_be_type_in_program_contracts() {
    let source = r#"
        fn admits(ok: bool) -> result: bool
        where D, item: D, requires ok, ensures result
        {
            ok
        }
        fn run() -> bool {
            admits<Type, u8>(true) and admits<bool, true>(true)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let invalid = source.replace("admits<Type, u8>(true)", "admits<Type, u8>(false)");
    let error = sysroot::compile(&invalid)
        .err()
        .expect("a Type input does not prove the runtime precondition");
    assert!(error.contains("precondition"), "{error}");
}

#[test]
fn forwarded_type_inputs_keep_the_callers_scope_in_contracts() {
    let source = r#"
        fn admits(ok: bool) -> result: bool
        where D, item: D, requires ok, ensures result
        {
            ok
        }
        fn forward(comptime T: Type, ok: bool) -> result: bool
        where requires ok, ensures result
        {
            admits<Type, T>(ok)
        }
        fn run() -> bool {
            forward(u8, true) and forward(bool, true)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn purity_checks_dependent_type_inputs_through_their_domains() {
    assert_eq!(
        sysroot::run(
            r#"
        pure fn answer() -> bool where D, item: D { true }
        pure fn run() -> bool { answer<Type, u8>() }
    "#
        ),
        Value::Bool(true)
    );
}

#[test]
fn callable_inputs_and_results_keep_dependent_declaration_domains() {
    assert_eq!(
        sysroot::run(
            r#"
        fn answer() -> bool where D, item: D { true }
        fn call(action: fn() -> bool) -> bool { action() }
        fn factory() -> fn() -> bool where D, item: D { answer<D, item> }
        fn run() -> bool {
            call(answer<Type, u8>) and call(factory<Type, u8>())
        }
    "#
        ),
        Value::Bool(true)
    );
}

#[test]
fn selected_drop_keeps_dependent_declaration_domains() {
    assert_eq!(
        sysroot::run(
            r#"
        struct Marker where D, item: D {}
        impl Drop for Marker<D, item> where D, item: D {
            fn drop(*self) {}
        }
        fn run() -> bool {
            let owned = Marker<Type, u8> {};
            true
        }
    "#
        ),
        Value::Bool(true)
    );
}
