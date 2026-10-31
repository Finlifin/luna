//! Handler pointer invariants apply during installation, not forever.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

const PREFIX: &str = "trait Duplicate {} trait Retire {fn drop(*self);}
    impl Duplicate for bool {} impl Duplicate for *T where T {}
    effect touch()->bool;
    fn twice()->#touch bool {let first=touch()#;let second=touch()#;first and second}";

fn execute(body: &str) -> Result<Value, String> {
    let package = support::compile(&format!("{PREFIX} fn run()->bool {{{body}}}"));
    luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 1_000_000)
}

#[test]
fn pointer_capture_can_be_rebound_after_handler_completion() {
    for body in [
        "var left=false;var right=false;var target=left.ref;
         let result=twice()# {touch()=>do {target.* = true;target.*},};
         target=right.ref;target.* = true;result and left and right",
        "var left=false;var right=false;var target=left.ref;target=right.ref;
         let result=twice()# {touch()=>do {target.* = true;target.*},};
         result and not left and right",
        "let callback=|| {var left=false;var right=false;var target=left.ref;
             let result=twice()# {touch()=>do {target.* = true;target.*},};
             target=right.ref;target.* = true;result and left and right};callback()",
    ] {
        assert_eq!(
            execute(body).unwrap_or_else(|error| panic!("{body}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn installed_handler_cannot_publish_a_short_lived_pointer_into_capture() {
    for body in [
        "var local=false;target=local.ref;true",
        "if target.* {var local=false;target=local.ref;};true",
    ] {
        let error = execute(&format!(
            "var flag=true;var target=flag.ref;
            let result=twice()# {{touch()=>do {{{body}}},}};
            result and target.*"
        ))
        .unwrap_err();
        assert!(
            error.contains("handler pointer slot rebinding")
                || error.contains("captured pointer rebinding"),
            "{error}"
        );
    }
}

#[test]
fn existing_alias_to_capture_cell_requires_a_contract() {
    for invocation in [false, true] {
        let body = "var flag=false;var target=flag.ref;let exposed=target.ref;
            twice()# {touch()=>do {target.* = true;target.*},}";
        let body = if invocation {
            format!("let callback=|| {{{body}}};callback()")
        } else {
            body.to_string()
        };
        let error = execute(&body).unwrap_err();
        assert!(error.contains("address exposure"), "{error}");
    }
}

#[test]
fn handler_data_field_updates_preserve_pointer_identity() {
    let package = support::compile(&format!(
        "{PREFIX}
        struct State {{target:*bool,next:bool}}
        fn run()->bool {{var flag=false;var state=State{{target:flag.ref,next:false}};
            let result=twice()# {{touch()=>do {{
                state.next=not state.next;state.target.* = state.next;true
            }},}};
            result and not flag and not state.next
        }}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1_000_000,
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn installation_uses_current_aggregate_and_variant_origins() {
    for (declaration, initial, replacement, body) in [
        (
            "struct Link {target:*bool} impl Duplicate for Link {}",
            "Link{target:left.ref}",
            "captured=Link{target:right.ref};",
            "captured.target.* = true;captured.target.*",
        ),
        (
            "struct Link {target:*bool} impl Duplicate for Link {}",
            "Link{target:left.ref}",
            "captured.target=right.ref;",
            "captured.target.* = true;captured.target.*",
        ),
        (
            "enum Choice {none,some(*bool)} impl Duplicate for Choice {}",
            "Choice.none",
            "captured=Choice.some(right.ref);",
            "captured match {.none=>false,.some(p)=>do {p.* = true;p.*}}",
        ),
    ] {
        let package = support::compile(&format!(
            "{PREFIX} {declaration}
            fn run()->bool {{var left=false;var right=false;var captured={initial};
                {replacement}
                let result=twice()# {{touch()=>do {{{body}}},}};
                result and not left and right
            }}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000,
            )
            .unwrap_or_else(|error| panic!("{replacement}: {error}")),
            Value::Bool(true)
        );
    }
}
