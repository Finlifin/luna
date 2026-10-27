#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn explicit_strategy_methods_execute_for_independent_subjects() {
    let package = support::compile(
        "trait Flag {fn flag(self)->bool;} derive fn enabled for Flag {fn flag(self)->bool {true}} struct First {} struct Second {} derive Flag.use(enabled) for First; derive Flag.use(enabled) for Second; fn run()->bool {First{}.as(Flag).flag() and Second{}.as(Flag).flag()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(package.implementations.len(), 2);
}

#[test]
fn strategy_applicability_checks_the_complete_interface_instance() {
    let source = "typealias u32 = .u32'builtin; trait Flag where T {fn flag(self)->bool;} derive fn enabled for Flag<bool> {fn flag(self)->bool {true}} struct Record {} derive Flag<u32>.use(enabled) for Record;";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        error.contains("derivation strategy does not match the requested interface instance"),
        "{error}"
    );
}

#[test]
fn generated_implementations_still_check_members_and_overlap() {
    for (source, expected) in [
        (
            "typealias u32=.u32'builtin; trait Flag {fn flag(self)->bool;} derive fn enabled for Flag {fn flag(self)->u32 {1}} struct Record {} derive Flag.use(enabled) for Record;",
            "does not match its trait signature",
        ),
        (
            "trait Flag {fn flag(self)->bool;} derive fn enabled for Flag {} struct Record {} derive Flag.use(enabled) for Record;",
            "missing required trait method",
        ),
        (
            "trait Flag {fn flag(self)->bool;} derive fn enabled for Flag {fn flag(self)->bool {true}} struct Record {} derive Flag.use(enabled) for Record; derive Flag.use(enabled) for Record;",
            "implementation overlap",
        ),
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            error.contains(expected),
            "ordinary impl checking was not reached: {error}"
        );
    }
}

#[test]
fn non_strategy_selections_and_generation_statements_are_not_silently_accepted() {
    for source in [
        "trait Flag {} fn enabled() {} struct Record {} derive Flag.use(enabled) for Record;",
        "trait Flag {} derive fn enabled for Flag {let value = true;} struct Record {} derive Flag.use(enabled) for Record;",
        "trait Flag {} derive fn enabled(flag:bool) for Flag {} struct Record {} derive Flag.use(enabled) for Record;",
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            error.contains("strategy") || error.contains("derivation expansion"),
            "{error}"
        );
    }
}

#[test]
fn matching_parameterized_strategy_executes_without_unfolding_trait_identity() {
    let package = support::compile(
        "trait Flag where T {fn flag(self)->bool;} derive fn enabled for Flag<bool> {fn flag(self)->bool {true}} struct Record {} derive Flag<bool>.use(enabled) for Record; fn run()->bool {Record{}.as(Flag<bool>).flag()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_derivation_preserves_membership_and_function_phases() {
    let source = format!(
        "mod attrs {{{}}} use attrs.comptime_phase as staged; typealias Type=.Type'builtin; trait Flag {{^staged comptime fn flag()->bool;}} derive fn enabled for Flag {{comptime fn flag()->bool {{true}}}} struct Record {{}} comptime derive Flag.use(enabled) for Record; comptime fn run()->bool {{Record.as(Flag).flag()}}",
        include_str!("../../../library/attrs/lib.fl")
    );
    let package = support::compile(&source);
    assert!(package.implementations[0].phases.comptime);
    assert!(!package.implementations[0].phases.runtime);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generated_generic_methods_rebind_their_own_type_parameters() {
    let package = support::compile(
        "trait Echo {fn echo(value:T)->T where T;} derive fn echoing for Echo {fn echo(value:T)->T where T {value}} struct Subject {} derive Echo.use(echoing) for Subject; fn run()->bool {Subject.as(Echo).echo<bool>(true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn strategy_matching_preserves_normal_form_heads() {
    let source = "pure comptime fn Identity<T> -> .Type'builtin {T} trait Flag where T {} derive fn enabled for Flag<Identity<bool>> {} struct Subject {} derive Flag<bool>.use(enabled) for Subject;";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        error.contains("derivation strategy does not match the requested interface instance"),
        "{error}"
    );
}

#[test]
fn block_local_requests_do_not_leave_retired_item_references() {
    let source = "trait Flag {} derive fn enabled for Flag {} struct Record {} fn run()->bool {derive Flag.use(enabled) for Record; true}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("block-local derive requests"), "{error}");
}
