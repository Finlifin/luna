//! Closure construction and invocation must both use the bytecode carrier.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

#[test]
fn closures_keep_private_capture_cells_across_compiled_invocations() {
    let package = support::compile(
        "typealias Counter=%{flag:bool} comptime fn()->bool;
        comptime fn make()->Counter {
            var flag=false;
            || {flag=not flag;flag}
        }
        comptime fn run()->bool {
            let left=make();
            let right=make();
            let first=left();
            let other=right();
            let second=left();
            first and other and not second
        }
        comptime fn local()->bool {
            var flag=false;
            let toggle:Counter=|| {flag=not flag;flag};
            let first=toggle();
            let second=toggle();
            first and not second and not flag
        }
        comptime fn empty()->bool {
            let identity:comptime fn(bool)->bool=|value| value;
            identity(true)
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for (name, count) in [("run", 6), ("local", 3), ("empty", 2)] {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(support::id(&package, name), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(vm.bytecode_invocations() - before, count, "{name}");
        assert_eq!(
            tree.invoke(support::id(&package, name), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
    assert_eq!(
        vm.events()
            .iter()
            .filter(|event| matches!(event, comptime_interpretor::Event::CreateClosure { .. }))
            .count(),
        4
    );
    // Both instances from make share immutable body code, never capture cells.
    assert_eq!(vm.bytecode_compilations(), 7);
}

#[test]
fn callback_loops_reuse_code_but_execute_every_state_change() {
    let package = support::compile(
        "typealias Integer=.Integer'builtin;
        comptime fn run()->bool {
            var flag=false;
            let toggle:%{flag:bool} comptime fn()->bool=|| {flag=not flag;flag};
            var index:Integer=0;
            var result=false;
            while index < 100 {
                result=toggle();
                index = index + 1;
            }
            not result and not flag
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for _ in 0..2 {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(vm.bytecode_invocations() - before, 101);
        assert_eq!(vm.bytecode_compilations(), 2);
    }
}
