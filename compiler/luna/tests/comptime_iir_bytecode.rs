//! Concrete comptime graphs execute without a source declaration world.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Literal, SourceRange};
use nir::{Primitive, Term};

fn graph(source: &str, entry: &str) -> iir::Package {
    graph_from_package(&support::compile(source), entry)
}

fn graph_from_package(package: &nir::program::Package, entry: &str) -> iir::Package {
    elaboration::materialization::instantiate(
        package,
        &[iir::InstanceKey {
            declaration: support::id(package, entry),
            arguments: vec![].into(),
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
    let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&root, vec![]).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
    assert_eq!(vm.bytecode_invocations(), 0);

    graph
        .functions
        .iter_mut()
        .find(|f| f.key == root)
        .unwrap()
        .function
        .phase = nir::program::FunctionPhase::Comptime;
    assert!(
        graph
            .validate(&mut 100_000)
            .unwrap_err()
            .contains("execution phase")
    );
    assert!(Interpreter::from_iir_for_comptime(&graph, Budget::default()).is_err());
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
    let compilations = compiled.bytecode_compilations();
    let executions = compiled.bytecode_contextual_inputs();
    compiled
        .invoke_instance(
            &graph.roots[0],
            vec![Value::Type(Term::Primitive(Primitive::U8))],
        )
        .unwrap();
    assert_eq!(compiled.bytecode_compilations(), compilations);
    assert!(compiled.bytecode_contextual_inputs() > executions);
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

#[test]
fn graph_sessions_reuse_code_only_for_the_exact_immutable_graph() {
    use comptime_interpretor::CodeSession;
    use std::sync::Arc;
    let graph = Arc::new(graph(
        "comptime fn run(value:bool)->bool {not value}",
        "run",
    ));
    let mut session = CodeSession::for_iir(graph.clone());
    for (index, value) in [true, false, true].into_iter().enumerate() {
        let mut vm = Interpreter::from_iir_for_comptime_in_session(
            &graph,
            &mut session,
            &mut Budget::default(),
        )
        .unwrap();
        assert_eq!(
            vm.with_code_session(&mut session, |vm| vm
                .invoke_instance(&graph.roots[0], vec![Value::Bool(value)]))
                .unwrap(),
            Value::Bool(!value)
        );
        assert!(vm.bytecode_invocations() > 0);
        assert_eq!(vm.bytecode_compilations() == 0, index != 0);
    }
    let mut changed = graph.as_ref().clone();
    let root = changed.roots[0].clone();
    let function = &mut changed
        .functions
        .iter_mut()
        .find(|f| f.key == root)
        .unwrap()
        .function;
    function.body = Expr {
        source: function.body.source,
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Literal(Literal::Bool(true)),
    };
    let mut vm = Interpreter::from_iir_for_comptime_in_session(
        &changed,
        &mut session,
        &mut Budget::default(),
    )
    .unwrap();
    assert_eq!(
        vm.with_code_session(&mut session, |vm| vm
            .invoke_instance(&root, vec![Value::Bool(true)]))
            .unwrap(),
        Value::Bool(true)
    );
    assert!(vm.bytecode_compilations() > 0);
    let mut original =
        Interpreter::from_iir_for_comptime_in_session(&graph, &mut session, &mut Budget::default())
            .unwrap();
    assert_eq!(
        original
            .with_code_session(&mut session, |vm| vm
                .invoke_instance(&root, vec![Value::Bool(true)]))
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(original.bytecode_compilations(), 0);
}

#[test]
fn a_graph_session_keeps_specializations_of_one_declaration_distinct() {
    use comptime_interpretor::CodeSession;
    use std::sync::Arc;
    let package = support::compile("comptime fn selected() -> bool where flag: bool { flag }");
    let declaration = support::id(&package, "selected");
    let roots = [false, true].map(|flag| iir::InstanceKey {
        declaration,
        arguments: vec![Term::Bool(flag)].into(),
    });
    let graph = Arc::new(
        elaboration::materialization::instantiate(
            &package,
            &roots,
            Default::default(),
            Default::default(),
        )
        .unwrap(),
    );
    let mut session = CodeSession::for_iir(graph.clone());
    for warm in [false, true] {
        let mut vm = Interpreter::from_iir_for_comptime_in_session(
            &graph,
            &mut session,
            &mut Budget::default(),
        )
        .unwrap();
        vm.with_code_session(&mut session, |vm| {
            for index in [1, 0, 1] {
                assert_eq!(
                    vm.invoke_instance(&roots[index], vec![]).unwrap(),
                    Value::Bool(index == 1)
                );
            }
        });
        assert_eq!(vm.bytecode_compilations(), if warm { 0 } else { 2 });
    }
}

#[test]
fn graph_validation_reuse_is_metered_and_cannot_authorize_another_snapshot() {
    use comptime_interpretor::CodeSession;
    use std::sync::Arc;
    let graph = Arc::new(graph(
        "comptime fn run(value:bool)->bool where requires value {value}",
        "run",
    ));
    let mut session = CodeSession::for_iir(graph.clone());
    // Insufficient validation fuel must not publish a trusted snapshot.
    for _ in 0..2 {
        let mut budget = Budget {
            steps: 0,
            ..Budget::default()
        };
        assert!(
            Interpreter::from_iir_for_comptime_in_session(&graph, &mut session, &mut budget)
                .is_err()
        );
        assert_eq!(budget.steps, 0);
    }
    let mut cold = Budget::default();
    let mut vm =
        Interpreter::from_iir_for_comptime_in_session(&graph, &mut session, &mut cold).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![Value::Bool(true)])
            .unwrap(),
        Value::Bool(true)
    );
    let mut warm = Budget::default();
    let mut vm =
        Interpreter::from_iir_for_comptime_in_session(&graph, &mut session, &mut warm).unwrap();
    assert!(warm.steps > cold.steps);
    assert!(warm.steps < Budget::default().steps);
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![Value::Bool(false)])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    let mut empty = Budget {
        steps: 0,
        ..Budget::default()
    };
    assert!(matches!(Interpreter::from_iir_for_comptime_in_session(
        &graph, &mut session, &mut empty), Err(error) if error.kind == ErrorKind::StepsExhausted));

    // Identical declaration identities do not authorize a broken root reference.
    let mut invalid = graph.as_ref().clone();
    invalid.roots[0].declaration.local = u32::MAX;
    for _ in 0..2 {
        assert!(matches!(Interpreter::from_iir_for_comptime_in_session(
            &invalid, &mut session, &mut Budget::default()),
            Err(error) if error.kind == ErrorKind::InvalidCallPlan));
    }
    let invalid = Arc::new(invalid);
    let mut invalid_session = CodeSession::for_iir(invalid.clone());
    for _ in 0..2 {
        assert!(matches!(Interpreter::from_iir_for_comptime_in_session(
            &invalid, &mut invalid_session, &mut Budget::default()),
            Err(error) if error.kind == ErrorKind::InvalidCallPlan));
    }
    let mut recovered = Budget::default();
    let mut vm =
        Interpreter::from_iir_for_comptime_in_session(&graph, &mut session, &mut recovered)
            .unwrap();
    assert_eq!(recovered.steps, warm.steps);
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![Value::Bool(true)])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn graph_sessions_reuse_callable_and_declaration_indexes_without_caching_admission() {
    use comptime_interpretor::CodeSession;
    use std::sync::Arc;
    let graph = Arc::new(graph(
        "typealias Type=.Type'builtin;typealias char=.char'builtin;
         fn identity(value:T)->T where T {value}
         pure comptime fn Carrier<T,action:fn(T)->T>->Type {bool}
         trait Probe {comptime fn probe()->bool;}
         impl Probe for bool {comptime fn probe()->bool {true}}
         comptime fn dispatch()->bool where T {true}
         comptime fn run(value:bool)->bool where requires value {
             dispatch<Carrier<bool,identity<bool>>>() and
             dispatch<Carrier<char,identity<char>>>() and bool.as(Probe).probe()
         }",
        "run",
    ));
    assert_eq!(graph.callable_identities.len(), 2);
    assert!(!graph.normal_forms.is_empty());
    assert!(!graph.dispatch.is_empty());
    let mut probe = CodeSession::for_iir(graph.clone());
    let mut cold = Budget::default();
    Interpreter::from_iir_for_comptime_in_session(&graph, &mut probe, &mut cold).unwrap();
    let cold_work = Budget::default().steps - cold.steps;
    assert!(cold_work > 1);

    let mut session = CodeSession::for_iir(graph.clone());
    // Exhaustion after validation/index work must not publish partial indexes.
    let mut insufficient = Budget {
        steps: cold_work - 1,
        ..Budget::default()
    };
    assert!(matches!(Interpreter::from_iir_for_comptime_in_session(
        &graph, &mut session, &mut insufficient), Err(error) if error.kind == ErrorKind::StepsExhausted));
    let mut tiny = Budget {
        steps: 1,
        ..Budget::default()
    };
    assert!(
        Interpreter::from_iir_for_comptime_in_session(&graph, &mut session, &mut tiny).is_err()
    );

    for value in [true, false, true] {
        let mut budget = Budget::default();
        let mut vm =
            Interpreter::from_iir_for_comptime_in_session(&graph, &mut session, &mut budget)
                .unwrap();
        if value == false {
            assert_eq!(Budget::default().steps - budget.steps, 1);
        }
        let result = vm.invoke_instance(&graph.roots[0], vec![Value::Bool(value)]);
        if value {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
    }
    // Declaration IDs alone do not authorize positional indexes. This valid
    // snapshot reverses their backing tables while retaining identical keys.
    let mut reordered = graph.as_ref().clone();
    reordered.callable_identities.reverse();
    reordered.normal_forms.reverse();
    reordered.validate(&mut 1_000_000).unwrap();
    let mut budget = Budget::default();
    let mut vm =
        Interpreter::from_iir_for_comptime_in_session(&reordered, &mut session, &mut budget)
            .unwrap();
    assert!(Budget::default().steps - budget.steps > 1);
    assert_eq!(
        vm.invoke_instance(&reordered.roots[0], vec![Value::Bool(true)])
            .unwrap(),
        Value::Bool(true)
    );
    let mut budget = Budget {
        steps: 1,
        ..Budget::default()
    };
    Interpreter::from_iir_for_comptime_in_session(&graph, &mut session, &mut budget).unwrap();
    assert_eq!(budget.steps, 0);
}

#[test]
fn concrete_comptime_storage_uses_instantiated_nominal_layouts() {
    let graph = graph(
        "newtype Wrapper<T> = T;
        enum Choice where T {empty, some(T)}
        comptime fn run(flag:bool)->bool {
            let selected = Choice<bool>.some(flag) match {
                .empty => false, .some(value) => value,
            };
            let optional: ?bool = selected;
            let wrapped = optional.as(Wrapper<?bool>);
            wrapped.as(?bool) match {value? => value, null => false}
        }",
        "run",
    );
    for compiled in [false, true] {
        let mut vm = if compiled {
            Interpreter::from_iir_for_comptime(&graph, Budget::default())
        } else {
            Interpreter::from_iir_for_comptime_tree(&graph, Budget::default())
        }
        .unwrap();
        for value in [true, false, true] {
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![Value::Bool(value)])
                    .unwrap(),
                Value::Bool(value)
            );
        }
    }
}

#[test]
fn observed_clause_links_preserve_execution_and_reject_stale_evidence() {
    let source = "
        comptime fn pick() -> bool where flag: bool { flag }
        comptime fn run(allowed: bool) -> bool {
            var touched = false;
            let result = pick<do { assumes allowed; assumes not touched; touched = true; true }>();
            result and touched and not pick<false>()
        }";
    let original = graph(source, "run");
    let run = original.roots[0].clone();
    for corrupt in [false, true] {
        let mut graph = original.clone();
        if corrupt {
            // Change what the operand delivers after its original prefix. Keep
            // the linked target and its evidence intact, as in a stale artifact.
            let function = graph.functions.iter_mut().find(|f| f.key == run).unwrap();
            let mut changed = 0;
            nir::visit::expression_mut(&mut function.function.body, &mut 10000, |expr, _| {
                if let K::ComptimeFunctionInstance { inputs, .. } = &mut expr.kind
                    && inputs.observed_argument(0) == Some(&Term::Bool(true))
                {
                    inputs.arguments[0].kind = K::Literal(Literal::Bool(false));
                    changed += 1;
                }
                Ok(())
            })
            .unwrap();
            assert_eq!(changed, 1);
        }
        for compiled in [false, true] {
            let mut vm = if compiled {
                Interpreter::from_iir_for_comptime(&graph, Budget::default())
            } else {
                Interpreter::from_iir_for_comptime_tree(&graph, Budget::default())
            }
            .unwrap();
            for allowed in [true, false, true] {
                let result = vm.invoke_instance(&run, vec![Value::Bool(allowed)]);
                if !allowed {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
                } else if corrupt {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::InvalidCallPlan);
                } else {
                    assert_eq!(result.unwrap(), Value::Bool(true));
                }
            }
        }
    }
    let mut dangling = original;
    dangling
        .functions
        .retain(|function| function.key.arguments.as_ref() != [Term::Bool(true)]);
    assert!(
        dangling
            .validate(&mut 10000)
            .unwrap_err()
            .contains("clause dependency")
    );
}

fn retain_runtime_declaration(
    package: &mut nir::program::Package,
    declaration: nir::DeclarationId,
) {
    let position = package
        .functions
        .iter()
        .position(|f| f.declaration == declaration)
        .unwrap();
    let f = package.functions.remove(position).interface;
    assert_eq!(f.phase, nir::program::FunctionPhase::Runtime);
    let signature = f.callable_type();
    package
        .runtime_function_identities
        .push(nir::program::RuntimeFunctionIdentity {
            source: f.source,
            requirements: f.requirements,
            capabilities: f.capabilities,
            declaration,
            type_parameters: f.type_parameters,
            signature,
        });
}

#[test]
fn runtime_method_in_an_opaque_head_retains_its_interface_identity_in_both_worlds() {
    let source = "
        typealias Type = .Type'builtin;
        trait Flag { fn read(self) -> bool; }
        impl Flag for bool { fn read(self) -> bool { self } }
        fn execute(value: bool) -> bool { value.as(Flag).read() }
        pure comptime fn Carrier<action: fn(bool) -> bool> -> Type { bool }
        comptime fn selected() -> Type where T { T }
        comptime fn run() -> Type { selected<Carrier<bool.as(Flag).read>>() }
    ";
    let mut package = support::compile(source);
    let root = support::id(&package, "run");
    let expected = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .invoke(root, vec![])
        .unwrap();
    assert!(
        matches!(&expected, Value::Type(Term::NormalForm { arguments, .. })
        if matches!(arguments.as_slice(), [Term::MethodValue(_)]))
    );
    let executable = luna::instantiation::instantiate(&package, "execute", 100_000).unwrap();
    let selected = executable
        .dispatch
        .iter()
        .flat_map(|interface| &interface.methods)
        .next()
        .unwrap()
        .body;
    let mut vm = Interpreter::from_iir(&executable, Budget::default()).unwrap();
    for value in [true, false] {
        assert_eq!(
            vm.invoke_instance(&executable.roots[0], vec![Value::Bool(value)])
                .unwrap(),
            Value::Bool(value)
        );
    }
    retain_runtime_declaration(&mut package, selected);
    let graph = graph_from_package(&package, "run");
    assert!(
        graph
            .callable_identities
            .iter()
            .any(|id| id.key.declaration == selected)
    );
    assert!(
        graph
            .functions
            .iter()
            .all(|f| f.key.declaration != selected)
    );
    assert!(luna::instantiation::instantiate(&package, "execute", 100_000).is_err());
    for compiled in [false, true] {
        let mut vm = if compiled {
            Interpreter::from_iir_for_comptime(&graph, Budget::default())
        } else {
            Interpreter::from_iir_for_comptime_tree(&graph, Budget::default())
        }
        .unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            expected
        );
    }
}

#[test]
fn transporting_runtime_functions_needs_their_declarations_and_calling_needs_their_bodies() {
    let mut package = support::compile(
        "
        typealias Type = .Type'builtin;
        trait Source { assoc Item: Type; }
        impl Source for bool { assoc Item: Type = bool; }
        fn keep(value: T.as(Source).Item) -> T.as(Source).Item
        where T :- Source
        {
            value
        }
        fn flip(value: bool) -> bool { not value }
        comptime fn select(first: bool) -> fn(bool) -> bool {
            if first { keep<bool> } else { flip }
        }
        fn run(value: bool) -> bool { keep<bool>(value) }
    ",
    );
    let root = |name: &str| iir::InstanceKey {
        declaration: support::id(&package, name),
        arguments: vec![].into(),
    };
    let select = root("select");
    let run = root("run");
    let keep = root("keep").declaration;
    let flip = root("flip").declaration;
    let instantiate = |package: &nir::program::Package, roots: &[iir::InstanceKey]| {
        elaboration::materialization::instantiate(
            package,
            roots,
            Default::default(),
            Default::default(),
        )
    };
    let cases = [
        (true, keep, vec![Term::Primitive(Primitive::Bool)]),
        (false, flip, vec![]),
    ];
    for declarations_only in [false, true] {
        if declarations_only {
            for (_, declaration, _) in &cases {
                retain_runtime_declaration(&mut package, *declaration);
            }
        }
        let graph = instantiate(&package, std::slice::from_ref(&select)).unwrap();
        assert_eq!(graph.functions.len(), 1);
        assert_eq!(graph.callable_identities.len(), cases.len());
        for reference in [false, true] {
            let mut vm = if reference {
                Interpreter::from_iir_for_comptime_tree(&graph, Budget::default())
            } else {
                Interpreter::from_iir_for_comptime(&graph, Budget::default())
            }
            .unwrap();
            for (first, declaration, arguments) in &cases {
                assert_eq!(
                    vm.invoke_instance(&select, vec![Value::Bool(*first)])
                        .unwrap(),
                    Value::FunctionInstance {
                        declaration: *declaration,
                        arguments: arguments.clone()
                    }
                );
            }
        }
        let executable = instantiate(&package, &[select.clone(), run.clone()]);
        if declarations_only {
            assert!(executable.unwrap_err().contains("no executable definition"));
        } else {
            let mut both = executable.unwrap();
            let mut vm = Interpreter::from_iir(&both, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&run, vec![Value::Bool(true)]).unwrap(),
                Value::Bool(true)
            );
            let mut ct = Interpreter::from_iir_for_comptime(&both, Budget::default()).unwrap();
            assert_eq!(
                ct.invoke_instance(&select, vec![Value::Bool(true)])
                    .unwrap(),
                Value::FunctionInstance {
                    declaration: keep,
                    arguments: cases[0].2.clone()
                }
            );
            let mut inconsistent = both.clone();
            let identity = inconsistent
                .callable_identities
                .iter_mut()
                .find(|i| i.key.declaration == keep)
                .unwrap();
            *identity = iir::CallableIdentity::intern(
                identity.key.clone(),
                identity
                    .closed_signature()
                    .unwrap()
                    .clone()
                    .with_unsafe_call_requirement(true),
                &mut nir::TypeInterner::default(),
                &mut 100_000,
            )
            .unwrap();
            assert!(
                inconsistent
                    .validate(&mut 100_000)
                    .unwrap_err()
                    .contains("disagree on signature")
            );
            both.functions.retain(|f| f.key.declaration != keep);
            assert!(
                both.validate(&mut 100_000)
                    .unwrap_err()
                    .contains("callable reference is dangling")
            );
        }
    }
}
