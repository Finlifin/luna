//! Source clause expressions must preserve their trait origin through invocation.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn source(implementation: &str, body: &str) -> String {
    format!(
        r#"
        const staged={{comptime_phase:true}};
        trait Choose {{
            ^staged comptime fn choose(.value:bool=flag)->bool where flag:bool;
        }}
        struct Subject {{}}
        comptime impl Choose for Subject {{ {implementation} }}
        {body}
    "#
    )
}

#[test]
fn source_dynamic_trait_clause_inputs_specialize_defaults_on_each_call() {
    let text = source(
        "comptime fn choose(value:bool)->bool where flag:bool {value}",
        r#"
        comptime fn use_it(flag:bool)->bool {Subject.as(Choose).choose<flag>()}
        comptime fn run()->bool {use_it(true) and not use_it(false)}
    "#,
    );
    let package = support::compile(&text);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn source_dynamic_trait_clause_inputs_retain_explicit_argument_overrides() {
    let text = source(
        "comptime fn choose(value:bool)->bool where flag:bool {value}",
        r#"
        comptime fn use_it(flag:bool)->bool {Subject.as(Choose).choose<flag>(.value=true)}
        comptime fn run()->bool {use_it(false)}
    "#,
    );
    let package = support::compile(&text);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn source_dynamic_trait_clause_inputs_do_not_weaken_interface_postconditions() {
    let text = source(
        "comptime fn choose(value:bool)->bool where flag:bool {value}",
        r#"
        comptime fn use_it(flag:bool)->bool {Subject.as(Choose).choose<flag>()}
        comptime fn run()->bool {use_it(false)}
    "#,
    )
    .replace(
        "choose(.value:bool=flag)->bool where flag:bool;",
        "choose(.value:bool=flag)->result:bool where flag:bool, ensures result;",
    );
    let diagnostic = support::compile_with_protocols(&text, &Default::default()).unwrap_err();
    assert!(
        diagnostic.contains("program postcondition is not proven"),
        "{diagnostic}"
    );
}

#[test]
fn source_dynamic_trait_contracts_specialize_clause_values() {
    let text = source(
        "comptime fn choose(value:bool)->bool where flag:bool {flag}",
        r#"
        comptime fn use_it(flag:bool)->bool {Subject.as(Choose).choose<.flag=flag>()}
        comptime fn run()->bool {use_it(true) and not use_it(false)}
    "#,
    )
    .replace(
        "choose(.value:bool=flag)->bool where flag:bool;",
        "choose(.value:bool=flag)->result:bool where flag:bool, ensures result == flag;",
    )
    .replace("where flag:bool", "where .flag:bool=true");
    let package = support::compile(&text);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn required_invocation_keeps_dynamic_trait_dependencies_and_defaults() {
    let text = source(
        "comptime fn choose(value:bool)->bool where flag:bool {value}",
        r#"
        comptime fn use_it(flag:bool)->bool {Subject.as(Choose).choose<flag>()}
        const selected = use_it(true);
        comptime fn run()->bool {selected}
    "#,
    );
    let package = support::compile(&text);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn source_dynamic_trait_calls_preserve_generic_owner_selection() {
    let text = source(
        "comptime fn choose(value:bool)->bool where flag:bool {value}",
        r#"
        comptime fn use_it(flag:bool)->bool {Subject<bool>.as(Choose).choose<flag>()}
        comptime fn run()->bool {use_it(true)}
    "#,
    )
    .replace("struct Subject {}", "struct Subject where T {}")
    .replace(
        "comptime impl Choose for Subject",
        "comptime impl Choose for Subject<T> where T",
    );
    let package = support::compile(&text);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn required_dynamic_trait_call_retains_the_trait_default_body() {
    let text = source(
        "",
        r#"
        comptime fn use_it(flag:bool)->bool {Subject.as(Choose).choose<flag>()}
        const selected = use_it(true) and not use_it(false);
        comptime fn run()->bool {selected}
    "#,
    )
    .replace(
        "choose(.value:bool=flag)->bool where flag:bool;",
        "choose(.value:bool=flag)->bool where flag:bool {value}",
    );
    let package = support::compile(&text);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn source_dynamic_trait_clauses_evaluate_once_before_explicit_arguments() {
    let text = source(
        "comptime fn choose(value:bool)->bool where flag:bool {value}",
        r#"
        comptime fn take(pointer:*bool)->bool {
            let old = pointer.*;
            pointer.* = not old;
            old
        }
        comptime fn run()->bool {
            let flag = false;
            let selected = Subject.as(Choose).choose<take(flag.ref)>(.value=flag);
            selected and flag
        }
    "#,
    );
    let package = support::compile(&text);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn required_dynamic_trait_calls_prepare_nested_dispatch() {
    let text = source(
        "comptime fn choose(value:bool)->bool where flag:bool {Other.as(Choose).choose<flag>(.value=value)}",
        r#"
        struct Other {}
        comptime impl Choose for Other {
            comptime fn choose(value:bool)->bool where flag:bool {value}
        }
        comptime fn use_it(flag:bool)->bool {Subject.as(Choose).choose<flag>()}
        const selected = use_it(true) and not use_it(false);
        comptime fn run()->bool {selected}
    "#,
    );
    let package = support::compile(&text);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}
