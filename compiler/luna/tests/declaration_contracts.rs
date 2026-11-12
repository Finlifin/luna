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
