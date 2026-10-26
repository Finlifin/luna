//! Handler arms retain the installer's outer effect row during memory analysis.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

#[test]
fn handler_requests_outer_operation_with_the_original_pointer() {
    for (modifier, outer, declaration) in [
        ("", "touch", "effect touch(value:*bool)->bool;"),
        (
            "inline ",
            "touch",
            "inline effect touch(value:*bool)->bool;",
        ),
        ("", "relay", ""),
        ("inline ", "relay", ""),
    ] {
        let package = support::compile(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
             impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
             {modifier}effect relay(value:*bool)->bool;
             {declaration}
             {modifier}fn nested(value:*bool)->#{outer} bool {{
                 relay(value)# {{relay(pointer)=>{outer}(pointer)#,}}
             }}
             fn run()->bool {{
                 var first=false; var second=false;
                 let result=nested(second.ref)# {{
                     {outer}(pointer)=>do {{pointer.* = true;true}},
                 }};
                 result and not first and second
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
            Value::Bool(true),
            "{modifier}"
        );
    }
}

#[test]
fn outer_handler_context_does_not_extend_a_local_pointer_lifetime() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
         impl Duplicate for bool {} impl Duplicate for *T where T {}
         effect relay(value:*bool)->*bool;
         effect deliver(value:*bool)->*bool;
         fn nested(value:*bool)->#deliver *bool {
             relay(value)# {relay(pointer)=>deliver(pointer)#,}
         }
         fn run()->*bool {
             var local=false;
             nested(local.ref)# {deliver(pointer)=>pointer,}
         }",
    );
    let error = luna::execution::interpret_with_protocols(
        &package,
        "run",
        "Duplicate",
        "Retire",
        1_000_000,
    )
    .unwrap_err();
    assert!(error.contains("escaping provenance"), "{error}");
}

#[test]
fn outer_handler_pointer_result_preserves_the_callers_storage() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
         impl Duplicate for bool {} impl Duplicate for *T where T {}
         effect relay(value:*bool)->*bool;
         effect deliver(value:*bool)->*bool;
         fn nested(value:*bool)->#deliver *bool {
             relay(value)# {relay(pointer)=>deliver(pointer)#,}
         }
         fn forward(value:*bool)->*bool {
             nested(value)# {deliver(pointer)=>pointer,}
         }
         fn run()->bool {
             var first=false; var second=false;
             let pointer=forward(second.ref);
             pointer.* = true;
             not first and second
         }",
    );
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
fn direct_handler_block_tail_cannot_return_local_storage() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
         impl Duplicate for bool {} impl Duplicate for *T where T {}
         effect relay(value:*bool)->*bool;
         fn run()->*bool {
             var local=false;
             relay(local.ref)# {relay(pointer)=>pointer,}
         }",
    );
    let error = luna::execution::interpret_with_protocols(
        &package,
        "run",
        "Duplicate",
        "Retire",
        1_000_000,
    )
    .unwrap_err();
    assert!(error.contains("escaping provenance"), "{error}");
}
