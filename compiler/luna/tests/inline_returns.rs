#[path = "support/inline_return.rs"]
mod boundary;
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Interpreter, Value};

#[test]
fn inline_returns_join_early_tail_and_nested_loop_paths() {
    for body in [
        "if flag {return true;} false",
        "while true {if flag {return true;}break;}false",
        "flag match {true=>{return true;},false=>false,}",
        "let true=flag else {return false;};true",
    ] {
        let mut package = support::compile(&format!("fn run(flag:bool)->bool {{{body}}}"));
        boundary::wrap(&mut package, "run");
        let function = package
            .functions
            .iter_mut()
            .find(|f| f.name == "run")
            .unwrap();
        let old = function.body.clone();
        function.body.kind = nir::program::ExprKind::Not(Box::new(old));
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        for value in [true, false] {
            let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![Value::Bool(value)])
                    .unwrap(),
                Value::Bool(!value),
                "{body}"
            );
        }
    }
}

#[test]
fn inline_boundary_leaves_closure_returns_inside_the_closure() {
    let mut package =
        support::compile("fn run(flag:bool)->bool {let callback=|| {return flag;};not callback()}");
    boundary::wrap(&mut package, "run");
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![Value::Bool(true)])
            .unwrap(),
        Value::Bool(false)
    );
    assert!(format!("{graph}").contains("(return"));
}
