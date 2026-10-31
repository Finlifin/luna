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

#[test]
fn concrete_graph_contextual_inputs_execute_in_bytecode() {
    let graph = graph(
        "typealias Type=.Type'builtin;
        pure comptime fn Payload<.T:Type=Type, value:T>->Type {T}
        comptime fn run(element:Type)->Type {Payload<.T=element,17>}",
        "run",
    );
    let mut compiled = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
    let mut reference = Interpreter::from_iir_for_comptime_tree(&graph, Budget::default()).unwrap();
    for primitive in [Primitive::U8, Primitive::U16] {
        let inputs = vec![Value::Type(Term::Primitive(primitive))];
        assert_eq!(
            compiled
                .invoke_instance(&graph.roots[0], inputs.clone())
                .unwrap(),
            reference.invoke_instance(&graph.roots[0], inputs).unwrap()
        );
    }
    assert!(compiled.bytecode_contextual_inputs() > 0);
    assert_eq!(reference.bytecode_contextual_inputs(), 0);
}

#[test]
fn graph_deferred_inputs_preserve_effect_order_and_phase_checks() {
    let original = graph(
        "typealias Type=.Type'builtin;
        pure comptime fn Payload<.T:Type=Type, value:T>->Type {T}
        comptime fn enabled()->bool {true}
        comptime fn run(element:Type)->Type {
            var touched=false;
            Payload<do { touched=not touched; if enabled() {17} else {23} },
                .T=if touched {element} else {bool}>
        }",
        "run",
    );
    for compiled in [false, true] {
        let mut vm = if compiled {
            Interpreter::from_iir_for_comptime(&original, Budget::default())
        } else {
            Interpreter::from_iir_for_comptime_tree(&original, Budget::default())
        }
        .unwrap();
        for primitive in [Primitive::U8, Primitive::U16] {
            let value = vm
                .invoke_instance(
                    &original.roots[0],
                    vec![Value::Type(Term::Primitive(primitive))],
                )
                .unwrap();
            let Value::Type(Term::NormalForm { arguments, .. }) = value else {
                panic!("expected opaque head")
            };
            assert_eq!(arguments[0], Term::Primitive(primitive));
            assert_eq!(
                arguments[1],
                Term::Integer(comptime_runtime::Integer::from_u128(17))
            );
        }
        assert_eq!(vm.bytecode_staged_inputs(), if compiled { 2 } else { 0 });
        assert_eq!(
            vm.invoke_instance(
                &original.roots[0],
                vec![Value::Type(Term::Primitive(Primitive::Bool))]
            )
            .unwrap_err()
            .kind,
            ErrorKind::Type
        );
        assert!(
            vm.invoke_instance(
                &original.roots[0],
                vec![Value::Type(Term::Primitive(Primitive::U8))]
            )
            .is_ok()
        );
        assert_eq!(vm.bytecode_staged_inputs(), if compiled { 4 } else { 0 });
    }
    let mut denied = original.clone();
    denied
        .functions
        .iter_mut()
        .find(|f| f.function.name == "enabled")
        .unwrap()
        .function
        .phase = nir::program::FunctionPhase::Runtime;
    // Changing a retained callee's phase invalidates the graph certificate;
    // rejection must happen before any contextual operand can execute.
    let error = match Interpreter::from_iir_for_comptime(&denied, Budget::default()) {
        Ok(_) => panic!("phase-corrupted graph was admitted"),
        Err(error) => error,
    };
    assert_eq!(error.kind, ErrorKind::InvalidCallPlan);
}
