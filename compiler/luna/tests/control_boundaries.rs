//! Return inference belongs to its callable, including across staging queries.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn inferred_returns_survive_nested_declaration_and_staging_boundaries() {
    let cases = [
        (
            "comptime fn produce()->bool {return true;}",
            "const nested=produce();",
        ),
        (
            "fn choose(.value:bool=true)->bool {value}",
            "let nested=choose();",
        ),
        (
            "struct Packet {value:bool=true}",
            "let nested=Packet {}.value;",
        ),
        (
            "",
            "let inner=|input:bool| {if input {return true;} false};let nested=inner(true);",
        ),
        (
            "comptime fn emit()-><|-bool> {'(true)}",
            "const code=emit();let nested=~<code>;",
        ),
        (
            "quote fn expanded()->bool {return true;}",
            "let nested=expanded();",
        ),
    ];
    for (declaration, nested) in cases {
        let source = format!(
            "{declaration}
            fn input(flag:bool)->!bool bool {{if flag {{true}} else {{error true}}}}
            fn run(flag:bool,early:bool)->bool {{
                let outer=|flag:bool,early:bool| {{
                    if early {{return true;}}
                    input(flag)!;
                    {nested}
                    if not nested {{return false;}}
                    true
                }};
                outer(flag,early) match {{value! => value, error failure => failure}}
            }}"
        );
        let package = support::compile(&source);
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        for (flag, early) in [(true, false), (false, false), (false, true)] {
            let mut vm =
                comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
            let result =
                vm.invoke_instance(&graph.roots[0], vec![Value::Bool(flag), Value::Bool(early)]);
            assert_eq!(result.unwrap(), Value::Bool(true), "{source}");
        }
    }
}

#[test]
fn required_initializers_cannot_return_or_break_from_the_runtime_caller() {
    for body in [
        "const value=do {return true;false};value",
        "while true {const value=do {break;true};} true",
        "while true {const value=do {continue;true};} true",
    ] {
        let source = format!("fn run()->bool {{{body}}}");
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(!error.contains("parsing failed"), "{source}: {error}");
        assert!(
            error.contains("boundary") || error.contains("outside"),
            "{source}: {error}"
        );
    }
}

#[test]
fn handlers_retain_the_installers_return_and_loop_targets() {
    let source = "effect signal()->bool;
        fn run()->bool {
            let callback=|| {
                while true {signal()# {signal()=>break,};}
                signal()# {signal()=>return true,};
                false
            };
            callback()
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}
