#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;
use support::{compile, compile_with_protocols};

#[test]
fn generated_symbol_guards_preserve_substitution_and_local_state() {
    let source = "typealias Symbol=.Symbol'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        comptime fn build()-><input:Symbol,key:Symbol,flag:bool|-bool> {
            '(input match {
                tag if do {var same=tag==key;same=same and flag;same}=>true,
                tag if tag!=key or not flag=>false
            })
        }
        fn compare(value:Symbol,other:Symbol,enabled:bool)->bool {
            ~[input=value,key=other,flag=enabled]<build()>
        }
        fn run()->bool {compare(.x,.x,true) and not compare(.x,.y,true)
            and not compare(.x,.x,false) and not compare(.x,.y,false)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        Value::Bool(true)
    );
    let invalid = source.replace(
        "tag if tag!=key or not flag=>false",
        "tag if tag!=key=>false",
    );
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn repeated_splices_keep_distinct_symbol_guard_environments() {
    let source = "typealias Symbol=.Symbol'builtin;
        comptime fn build()-><input:Symbol,key:Symbol|-bool> {
            '(input match {tag if tag==key=>true,tag if tag!=key=>false})
        }
        fn run()->bool {
            let value:Symbol=.x;let same:Symbol=.x;let different:Symbol=.y;
            const code=build();
            let first=~[input=value,key=same]<code>;
            let second=~[input=value,key=different]<code>;
            first and not second
        }";
    assert_eq!(
        luna::execution::interpret(&compile(source), "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}
