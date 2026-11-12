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

#[test]
fn inherited_program_contracts_keep_owner_and_method_declaration_domains() {
    for source in [
        r#"
        trait Gate where D, item: D {
            fn check(ok: bool) -> result: bool
            where requires ok, ensures result;
        }
        struct Marker where D, item: D {}
        impl Gate<D, item> for Marker<D, item> where D, item: D {
            fn check(ok: bool) -> bool { ok }
        }
        fn run() -> bool {
            Marker<Type, u8>.as(Gate<Type, u8>).check(true)
                and Marker<bool, true>.as(Gate<bool, true>).check(true)
        }
        "#,
        r#"
        trait Gate {
            fn check(ok: bool) -> result: bool
            where D, item: D, requires ok, ensures result;
        }
        impl Gate for bool {
            fn check(ok: bool) -> bool
            where D, item: D
            {
                ok
            }
        }
        fn run() -> bool {
            bool.as(Gate).check<Type, u8>(true)
                and bool.as(Gate).check<bool, true>(true)
        }
        "#,
    ] {
        assert_eq!(sysroot::run(source), Value::Bool(true));
        let rejected = source.replace("(true)", "(false)");
        let error = sysroot::compile(&rejected)
            .err()
            .expect("inherited entry condition must remain required");
        assert!(error.contains("precondition"), "{error}");
    }
}
