//! Installed handlers borrow affine capture owners without acquiring Copy.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

const PREFIX: &str = "trait Duplicate {} trait Retire {fn drop(*self);}
    impl Duplicate for bool {} impl Duplicate for *T where T {}
    struct State {target:*bool,next:bool,retired:*bool}
    impl Retire for State {fn drop(*self) {self.*.retired.* = true;}}
    effect touch()->bool;
    fn twice()->#touch bool {let first=touch()#;let second=touch()#;not first and second}";

#[test]
fn affine_pointer_capture_survives_repeated_requests_and_retires_after_installation() {
    let source = format!(
        "{PREFIX}
        fn run()->bool {{
            var flag=false;var retired=false;
            let result=do {{
                var state=State{{target:flag.ref,next:false,retired:retired.ref}};
                let result=twice()# {{touch()=>do {{
                    state.target.* = state.next;
                    state.next=not state.next;
                    state.target.*
                }},}};
                result and not state.next and not retired
            }};
            result and flag and retired
        }}"
    );
    let package = support::compile(&source);
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
fn handler_cannot_consume_the_borrowed_affine_capture() {
    for body in [
        "let moved=state;true",
        "let moved=state;return true;",
        "let moved=state;resume true;",
        "if state.next {let moved=state;true} else {false}",
    ] {
        let source = format!(
            "{PREFIX}
            fn run()->bool {{
                var flag=false;var retired=false;
                var state=State{{target:flag.ref,next:false,retired:retired.ref}};
                twice()# {{touch()=>do {{{body}}},}}
            }}"
        );
        let package = support::compile(&source);
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1_000_000,
        )
        .unwrap_err();
        assert!(
            error.contains("borrowed handler data capture must remain complete"),
            "{body}: {error}"
        );
    }
}

#[test]
fn handler_borrows_callable_environment_with_pointer_capture() {
    let package = support::compile(&format!(
        "{PREFIX}
        fn run()->bool {{
            var flag=false;let pointer=flag.ref;
            let callback=|| {{pointer.* = not pointer.*;pointer.*}};
            let result=twice()# {{touch()=>callback(),}};
            not result and not flag
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
