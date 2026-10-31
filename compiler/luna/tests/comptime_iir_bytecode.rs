//! Concrete comptime graphs execute without a source declaration world.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Literal, SourceRange};
use nir::{Primitive, Term};

fn graph(source: &str, entry: &str) -> iir::Package {
    let package = support::compile(source);
    elaboration::materialization::instantiate(
        &package,
        &[iir::InstanceKey {
            declaration: support::id(&package, entry),
            arguments: vec![],
        }],
        Default::default(),
        elaboration::ptc::Budget {
            steps: 1_000_000,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn concrete_instances_and_interface_defaults_execute_in_bytecode() {
    let graph = graph(
        "typealias char = .char'builtin;
        comptime fn identity(value:T)->T where T {value}
        trait Probe {comptime fn probe(self,.invert:bool=false)->bool;}
        impl Probe for bool {comptime fn probe(self,flip:bool)->bool {if flip {not self} else {self}}}
        comptime fn run(value:bool)->bool {
            identity<char>('x') == 'x' and not identity<bool>(value).as(Probe).probe()
        }", "run",
    );
    let root = &graph.roots[0];
    let mut compiled = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
    let mut reference = Interpreter::from_iir_for_comptime_tree(&graph, Budget::default()).unwrap();
    for value in [true, false, true] {
        let inputs = vec![Value::Bool(value)];
        assert_eq!(
            compiled.invoke_instance(root, inputs.clone()).unwrap(),
            Value::Bool(!value)
        );
        assert_eq!(
            reference.invoke_instance(root, inputs).unwrap(),
            Value::Bool(!value)
        );
    }
    assert_eq!(compiled.bytecode_invocations(), 12);
    assert_eq!(reference.bytecode_invocations(), 0);
    assert_eq!(compiled.bytecode_default_evaluations(), 3);
    let compilations = compiled.bytecode_compilations();
    compiled
        .invoke_instance(root, vec![Value::Bool(false)])
        .unwrap();
    assert_eq!(compiled.bytecode_compilations(), compilations);
}

#[test]
fn iir_interface_contracts_still_guard_weaker_bodies() {
    let original = graph(
        "trait Service {comptime fn accept(value:bool)->bool;}
        impl Service for bool {comptime fn accept(value:bool)->bool {true}}
        comptime fn run(value:bool)->bool {bool.as(Service).accept(value)}",
        "run",
    );
    for postcondition in [false, true] {
        let mut graph = original.clone();
        let program = &mut graph.interfaces[0].methods[0].program;
        let denied = Expr {
            source: SourceRange { start: 0, end: 0 },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(Literal::Bool(false)),
        };
        if postcondition {
            program.postconditions.push(denied);
        } else {
            program.preconditions.push(denied);
        }
        for compiled in [false, true] {
            let mut vm = if compiled {
                Interpreter::from_iir_for_comptime(&graph, Budget::default())
            } else {
                Interpreter::from_iir_for_comptime_tree(&graph, Budget::default())
            }
            .unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![Value::Bool(true)])
                    .unwrap_err()
                    .kind,
                ErrorKind::ProgramContractRejected
            );
            if compiled {
                assert!(vm.bytecode_invocations() > 0);
            }
        }
    }
}

#[test]
fn graph_phase_authority_is_checked_before_nested_execution() {
    let package = support::compile("fn leaf()->bool {true} fn run()->bool {leaf()}");
    let mut graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let root = graph.roots[0].clone();
    graph
        .functions
        .iter_mut()
        .find(|f| f.key == root)
        .unwrap()
        .function
        .phase = nir::program::FunctionPhase::Comptime;
    let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&root, vec![]).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
    assert_eq!(vm.bytecode_invocations(), 1);
}

#[test]
fn graph_closures_and_structural_values_keep_invocation_state_isolated() {
    let graph = graph(
        "struct Holder {value:bool}
        typealias Counter=%{flag:bool} comptime fn()->bool;
        comptime fn pass(value:bool)->bool {
            let holder=Holder {value:value};
            let optional:?bool=holder.value;
            optional match {answer? => answer, null => false}
        }
        comptime fn run(value:bool)->bool {
            var flag=value;
            let toggle:Counter=|| {flag=not flag;flag};
            pass(toggle())
        }",
        "run",
    );
    let mut compiled = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
    let mut reference = Interpreter::from_iir_for_comptime_tree(&graph, Budget::default()).unwrap();
    for value in [true, false, true] {
        let inputs = vec![Value::Bool(value)];
        assert_eq!(
            compiled
                .invoke_instance(&graph.roots[0], inputs.clone())
                .unwrap(),
            Value::Bool(!value)
        );
        assert_eq!(
            reference.invoke_instance(&graph.roots[0], inputs).unwrap(),
            Value::Bool(!value)
        );
    }
    assert_eq!(compiled.bytecode_invocations(), 9);
    assert_eq!(reference.bytecode_invocations(), 0);
}
