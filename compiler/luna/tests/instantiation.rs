//! Source -> concrete high-level instance graph, without interpreting templates.
use luna::{
    cli::{Emit, Options},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn compile(source: &str) -> nir::program::Package {
    compile_in(source, 0)
}
fn compile_in(source: &str, identity: u64) -> nir::program::Package {
    let path = std::env::temp_dir().join(format!(
        "luna-iir-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let artifact = frontend::compile_in_package(
        &Options {
            input: path.clone(),
            emit: Emit::Nir,
            output: None,
            sysroot: None,
            splice_protocol: None,
            meta_list_type: None,
            index_protocol: None,
            literal_into_protocol: None,
            arithmetic_protocols: Default::default(),
            tuple_type: None,
            optional_type: None,
        },
        identity,
    );
    std::fs::remove_file(path).unwrap();
    let Artifact::Nir(package) = artifact.unwrap() else {
        panic!("expected NIR")
    };
    package
}
#[test]
fn instances_close_recursive_generic_closure_and_dispatch_dependencies() {
    let source = compile(include_str!(
        "../../../development/examples/instantiated-functions.fl"
    ));
    let before = source.to_string();
    let output = luna::instantiation::instantiate(&source, "entry", 100_000).unwrap();
    output.validate(&mut 100_000).unwrap();
    assert_eq!(source.to_string(), before);
    let identities: Vec<_> = output
        .functions
        .iter()
        .filter(|f| f.function.name == "identity")
        .collect();
    assert_eq!(identities.len(), 3);
    assert!(
        identities
            .iter()
            .all(|f| f.function.type_parameters.is_empty())
    );
    assert_eq!(
        output
            .functions
            .iter()
            .filter(|f| f.function.name == "recurse")
            .count(),
        1
    );
    assert!(
        !output
            .functions
            .iter()
            .any(|f| f.function.name == "unreachable")
    );
    assert_eq!(output.dispatch.len(), 1);
    let again = luna::instantiation::instantiate(&source, "entry", 100_000).unwrap();
    assert_eq!(output.to_string(), again.to_string());
    assert!(output.to_string().contains("(closure"));
    std::fs::write("/tmp/luna-instantiated-functions.lisp", output.to_string()).unwrap();
    let mut damaged = output.clone();
    let dependency = damaged
        .functions
        .iter()
        .position(|f| f.function.name == "identity")
        .unwrap();
    damaged.functions.remove(dependency);
    assert!(
        damaged
            .validate(&mut 100_000)
            .unwrap_err()
            .contains("dangling")
    );
    let mut missing_type = output.clone();
    missing_type.nominals.clear();
    assert!(
        missing_type
            .validate(&mut 100_000)
            .unwrap_err()
            .contains("nominal reference is dangling")
    );
    assert!(luna::instantiation::instantiate(&source, "entry", 1).is_err());
    assert_eq!(
        luna::instantiation::instantiate(&source, "entry", 100_000).unwrap(),
        output
    );
}

#[test]
fn instance_obligations_fail_without_poisoning_other_requests() {
    let source = compile(
        "typealias Bool=true'type; fn constrained(value:T)->T where T, requires T == Bool { value } fn identity(value:T)->T where T { value } struct Checked where T, requires T == Bool { value:T }",
    );
    let constrained = source
        .functions
        .iter()
        .find(|f| f.name == "constrained")
        .unwrap()
        .declaration;
    let request = |argument| iir::InstanceKey {
        declaration: constrained,
        arguments: vec![argument],
    };
    let rejected = elaboration::materialization::instantiate(
        &source,
        &[request(nir::Term::Primitive(nir::Primitive::U8))],
        Default::default(),
        Default::default(),
    )
    .unwrap_err();
    assert!(rejected.contains("function type requirement"), "{rejected}");
    let accepted = elaboration::materialization::instantiate(
        &source,
        &[request(nir::Term::Primitive(nir::Primitive::Bool))],
        Default::default(),
        Default::default(),
    )
    .unwrap();
    assert_eq!(accepted.functions.len(), 1);
    let identity = source
        .functions
        .iter()
        .find(|f| f.name == "identity")
        .unwrap()
        .declaration;
    let checked = source
        .structs
        .iter()
        .find(|s| s.name == "Checked")
        .unwrap()
        .declaration;
    let malformed = iir::InstanceKey {
        declaration: identity,
        arguments: vec![nir::Term::Nominal {
            declaration: checked,
            arguments: vec![nir::Term::Primitive(nir::Primitive::U8)],
        }],
    };
    let error = elaboration::materialization::instantiate(
        &source,
        &[malformed],
        Default::default(),
        Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("type formation requirement"), "{error}");
    assert!(
        luna::instantiation::instantiate(&source, "constrained", 100_000)
            .unwrap_err()
            .contains("nongeneric wrapper")
    );
}

#[test]
fn instances_preserve_normal_form_heads_without_invoking_their_bodies() {
    let source = compile(include_str!(
        "../../../development/examples/normal-form-text.fl"
    ));
    let output = luna::instantiation::instantiate(&source, "same", 100_000).unwrap();
    assert_eq!(output.functions.len(), 1);
    assert!(!output.normal_forms.is_empty());
    let nir::Term::Function { parameters, result } = &output.functions[0].function.signature else {
        panic!("signature")
    };
    assert!(matches!(&parameters[0].ty, nir::Term::NormalForm { .. }));
    assert_eq!(&parameters[0].ty, result.as_ref());
    let head = source
        .functions
        .iter()
        .find(|f| f.name == "Named")
        .unwrap()
        .declaration;
    assert!(
        elaboration::materialization::instantiate(
            &source,
            &[iir::InstanceKey {
                declaration: head,
                arguments: vec![]
            }],
            Default::default(),
            Default::default()
        )
        .unwrap_err()
        .contains("normal-form heads")
    );
}

#[test]
fn instantiation_does_not_demand_unused_conditional_interface_members() {
    let source = compile(
        "trait Read where T { fn read() -> true'type; fn restricted() -> true'type where requires T == true'type; } struct S {} impl Read<'x''type> for S { fn read() -> true'type { true } fn restricted() -> true'type where requires 'x''type == true'type { false } } fn entry()->true'type { S.as(Read<'x''type>).read() }",
    );
    let output = luna::instantiation::instantiate(&source, "entry", 100_000).unwrap();
    assert!(
        !output
            .functions
            .iter()
            .any(|f| f.function.name == "restricted")
    );
    assert_eq!(output.dispatch[0].methods.len(), 1);
    output.validate(&mut 100_000).unwrap();
}

#[test]
fn unused_interface_signatures_retain_their_nominal_dependencies() {
    let source = compile(
        "struct Item {} trait API { fn used()->true'type; fn unused()->Item; } struct S {} impl API for S { fn used()->true'type {true} fn unused()->Item {Item {}} } fn entry()->true'type {S.as(API).used()}",
    );
    let output = luna::instantiation::instantiate(&source, "entry", 100_000).unwrap();
    assert!(!output.functions.iter().any(|f| f.function.name == "unused"));
    assert!(
        output.nominals.iter().any(
            |n| matches!(&n.definition, iir::NominalDefinition::Struct(s) if s.name == "Item")
        )
    );
    output.validate(&mut 100_000).unwrap();
}

#[test]
fn concrete_graph_executes_without_source_templates() {
    use comptime_interpretor::{Budget, Event, Interpreter, Value};
    let source = compile(include_str!(
        "../../../development/examples/instantiated-functions.fl"
    ));
    let output = luna::instantiation::instantiate(&source, "entry", 100_000).unwrap();
    drop(source);
    let mut interpreter = Interpreter::from_iir(&output, Budget::default()).unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&output.roots[0], vec![])
            .unwrap(),
        Value::UnsignedInteger(41)
    );
    let recursive = &output
        .functions
        .iter()
        .find(|f| f.function.name == "recurse")
        .unwrap()
        .key;
    assert_eq!(
        interpreter
            .invoke_instance(recursive, vec![Value::UnsignedInteger(20)])
            .unwrap(),
        Value::UnsignedInteger(0)
    );
    for instance in output
        .functions
        .iter()
        .filter(|f| f.function.name == "identity")
    {
        let value = match &instance.key.arguments[0] {
            nir::Term::Primitive(nir::Primitive::Bool) => Value::Bool(false),
            nir::Term::Primitive(nir::Primitive::U8) => Value::UnsignedInteger(7),
            nir::Term::Nominal {
                declaration,
                arguments,
            } => Value::Record {
                declaration: *declaration,
                arguments: arguments.clone(),
                fields: vec![Value::UnsignedInteger(9)],
            },
            _ => panic!("unexpected identity argument"),
        };
        assert_eq!(
            interpreter
                .invoke_instance(&instance.key, vec![value.clone()])
                .unwrap(),
            value
        );
    }
    assert!(
        !interpreter
            .events()
            .iter()
            .any(|e| matches!(e, Event::InstantiateFunction { .. }))
    );
    assert!(
        interpreter
            .invoke_instance(recursive, vec![Value::Bool(true)])
            .is_err()
    );
    assert_eq!(
        interpreter
            .invoke_instance(&output.roots[0], vec![])
            .unwrap(),
        Value::UnsignedInteger(41)
    );
}

#[test]
fn concrete_contract_execution_matches_normalized_execution() {
    use comptime_interpretor::{Budget, Interpreter, Value};
    let source = compile(include_str!(
        "../../../development/examples/trait-program-contracts.fl"
    ));
    let names = [
        "call",
        "call_twice",
        "generic_call",
        "wide",
        "inherent",
        "generic_implementation",
        "receiver",
        "default_argument",
        "match_call",
        "return_call",
        "parent_call",
        "interface_argument",
    ];
    for name in names {
        let graph = luna::instantiation::instantiate(&source, name, 100_000).unwrap();
        let mut concrete = Interpreter::from_iir(
            &graph,
            Budget {
                steps: 10_000_000,
                ..Budget::default()
            },
        )
        .unwrap();
        let mut normalized = Interpreter::new(
            &source,
            Budget {
                steps: 10_000_000,
                ..Budget::default()
            },
        )
        .unwrap();
        for input in 0..=255 {
            let args = vec![Value::UnsignedInteger(input)];
            let actual = concrete.invoke_instance(&graph.roots[0], args.clone());
            let expected = normalized.invoke(graph.roots[0].declaration, args);
            if input < 99 {
                assert!(expected.is_ok(), "{name}({input}): {expected:?}");
            }
            assert_eq!(actual, expected, "{name}({input})");
        }
    }
}

#[test]
fn concrete_execution_rechecks_results_and_rejects_dangling_graphs() {
    use comptime_interpretor::{Budget, Interpreter, Value};
    let source = compile(
        "typealias Byte=.u8'builtin; fn identity(value:Byte, marker:T)->result:Byte where T, ensures result == value'old {value} fn entry(value:Byte)->Byte {identity(value, true)}",
    );
    let mut graph = luna::instantiation::instantiate(&source, "entry", 100_000).unwrap();
    let identity = graph
        .functions
        .iter_mut()
        .find(|f| f.function.name == "identity")
        .unwrap();
    identity.function.body.kind =
        nir::program::ExprKind::Literal(nir::program::Literal::UnsignedInteger(0));
    let mut interpreter = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&graph.roots[0], vec![Value::UnsignedInteger(9)])
            .unwrap_err()
            .kind,
        comptime_interpretor::ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        interpreter
            .invoke_instance(&graph.roots[0], vec![Value::UnsignedInteger(0)])
            .unwrap(),
        Value::UnsignedInteger(0)
    );
    drop(interpreter);
    graph.functions.retain(|f| f.function.name != "identity");
    assert!(Interpreter::from_iir(&graph, Budget::default()).is_err());
}

#[test]
fn concrete_enum_instances_execute_patterns_and_field_defaults() {
    use comptime_interpretor::{Budget, Interpreter, Value};
    let source = compile(include_str!(
        "../../../development/examples/generic-enums.fl"
    ));
    let graph = luna::instantiation::instantiate(&source, "run", 100_000).unwrap();
    drop(source);
    let mut interpreter = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&graph.roots[0], vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn retained_definition_integrity_is_checked_before_execution() {
    let source = compile(include_str!(
        "../../../development/examples/instantiated-functions.fl"
    ));
    let graph = luna::instantiation::instantiate(&source, "entry", 100_000).unwrap();
    let reject = |mutate: fn(&mut iir::Package), expected: &str| {
        let mut damaged = graph.clone();
        mutate(&mut damaged);
        let error = damaged.validate(&mut 100_000).unwrap_err();
        assert!(error.contains(expected), "expected {expected}, got {error}");
        assert!(comptime_interpretor::Interpreter::from_iir(&damaged, Default::default()).is_err());
    };
    reject(
        |p| p.nominals.push(p.nominals[0].clone()),
        "duplicate IIR nominal",
    );
    reject(
        |p| p.nominals[0].ty = nir::Term::Primitive(nir::Primitive::Bool),
        "non-nominal key",
    );
    reject(
        |p| match &mut p.nominals[0].definition {
            iir::NominalDefinition::Struct(s) => s.declaration.local += 1000,
            iir::NominalDefinition::Enum(e) => e.declaration.local += 1000,
            iir::NominalDefinition::Newtype(n) => n.declaration.local += 1000,
        },
        "disagrees with its instance key",
    );
    reject(
        |p| p.interfaces.push(p.interfaces[0].clone()),
        "duplicate IIR interface contract",
    );
    reject(|p| p.interfaces.clear(), "no interface contract");
    reject(
        |p| {
            let duplicate = p.interfaces[0].methods[0].clone();
            p.interfaces[0].methods.push(duplicate);
        },
        "duplicate IIR interface member",
    );
    reject(
        |p| p.interfaces[0].methods[0].parameter_bindings.clear(),
        "parameter bindings",
    );
    reject(
        |p| p.interfaces[0].methods[0].default = Some(p.dispatch[0].methods[0].body),
        "unselected default",
    );
    reject(
        |p| p.dispatch.push(p.dispatch[0].clone()),
        "duplicate IIR dispatch selection",
    );
    reject(|p| p.dispatch.clear(), "no dispatch selection");
    reject(
        |p| {
            let duplicate = p.dispatch[0].methods[0].clone();
            p.dispatch[0].methods.push(duplicate);
        },
        "duplicate IIR dispatch member",
    );
    reject(
        |p| p.dispatch[0].methods[0].member.local += 1000,
        "absent from its interface",
    );
    reject(
        |p| {
            let body = p.dispatch[0].methods[0].body;
            p.functions
                .iter_mut()
                .find(|f| f.key.declaration == body)
                .unwrap()
                .function
                .phase = nir::program::FunctionPhase::Comptime;
        },
        "calling convention",
    );
    reject(
        |p| {
            p.functions
                .iter_mut()
                .find(|f| f.function.name == "identity")
                .unwrap()
                .function
                .parameters
                .clear();
        },
        "parameter bindings",
    );
    graph.validate(&mut 100_000).unwrap();
}

#[test]
fn copy_protocol_selection_uses_identity_and_explicit_declarations() {
    use elaboration::{
        capabilities::World,
        implementation::{ImplementationHead, Index},
        ptc::{Budget, Context},
        verification::copy::{Classification, classify},
    };
    let package = compile(
        "trait Reproduce {} trait Copy {} struct Owned {flag:true'type} struct Shared {flag:true'type} struct Pretender {} struct Box where T {value:T} impl Reproduce for Shared {} impl Copy for Pretender {} impl Reproduce for Box<T> where T, requires T == true'type {} fn transfer(value:Owned)->Owned {value} fn duplicate(value:Owned) {let first=value; let second=value;} fn copy_shared(value:Shared)->Shared {value}",
    );
    let protocol = package
        .traits
        .iter()
        .find(|t| t.name == "Reproduce")
        .unwrap()
        .declaration;
    let mut items = intrinsic::LangItems::new();
    items
        .bind_source_traits(&package, &[(intrinsic::LangItem::Copy, protocol)])
        .unwrap();
    let context = Context::from_package(&package).unwrap();
    let mut index = Index::default();
    for implementation in &package.implementations {
        index
            .insert(
                ImplementationHead {
                    parameter_types: implementation.parameter_types.clone(),
                    declaration: implementation.declaration,
                    interface: implementation.interface,
                    interface_arguments: implementation.interface_arguments.clone(),
                    subject: implementation.subject.clone(),
                    type_parameters: implementation.type_parameters.clone(),
                    requirements: implementation.requirements.clone(),
                },
                Budget::default(),
            )
            .unwrap();
    }
    let world = World {
        index: &index,
        implementations: &package.implementations,
        traits: &package.traits,
        assumptions: &[],
        context: &context,
    };
    assert!(
        luna::protocols::classify_copy(
            &world,
            &intrinsic::LangItems::new(),
            &nir::Term::Primitive(nir::Primitive::Bool),
            Budget::default()
        )
        .is_err()
    );
    assert!(matches!(
        luna::protocols::classify_copy(
            &world,
            &items,
            &nir::Term::Primitive(nir::Primitive::Bool),
            Budget::default()
        )
        .unwrap(),
        Classification::Intrinsic
    ));
    let mut invalid_items = intrinsic::LangItems::new();
    invalid_items
        .set(intrinsic::LangItem::Copy, intrinsic::LangItemDef::Builtin)
        .unwrap();
    assert!(
        luna::protocols::classify_copy(
            &world,
            &invalid_items,
            &nir::Term::Primitive(nir::Primitive::Bool),
            Budget::default()
        )
        .is_err()
    );
    let mut stale_items = intrinsic::LangItems::new();
    stale_items
        .set(
            intrinsic::LangItem::Copy,
            intrinsic::LangItemDef::UserDef(nir::DeclarationId {
                package: protocol.package + 1,
                ..protocol
            }),
        )
        .unwrap();
    assert!(
        luna::protocols::classify_copy(
            &world,
            &stale_items,
            &nir::Term::Primitive(nir::Primitive::Bool),
            Budget::default()
        )
        .is_err()
    );
    for (name, expected_error) in [
        ("transfer", None),
        ("duplicate", Some("unavailable")),
        ("copy_shared", Some("resource legality")),
    ] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let result = luna::protocols::analyze_moves(
            &package,
            &world,
            &items,
            function,
            10000,
            Budget::default(),
        );
        match expected_error {
            Some(message) => assert!(result.unwrap_err().reason.contains(message)),
            None => assert!(result.is_ok(), "{result:?}"),
        }
    }
    let nominal = |name: &str, arguments| nir::Term::Nominal {
        declaration: package
            .structs
            .iter()
            .find(|s| s.name == name)
            .unwrap()
            .declaration,
        arguments,
    };
    assert!(matches!(
        luna::protocols::classify_copy(
            &world,
            &items,
            &nominal("Shared", vec![]),
            Budget::default()
        )
        .unwrap(),
        Classification::Declared { .. }
    ));
    for name in ["Owned", "Pretender"] {
        assert_eq!(
            classify(&world, protocol, &nominal(name, vec![]), Budget::default()).unwrap(),
            Classification::Affine
        );
    }
    let boolean = nir::Term::Primitive(nir::Primitive::Bool);
    assert!(matches!(
        classify(
            &world,
            protocol,
            &nominal("Box", vec![boolean.clone()]),
            Budget::default()
        )
        .unwrap(),
        Classification::Declared { .. }
    ));
    assert_eq!(
        classify(
            &world,
            protocol,
            &nominal("Box", vec![nir::Term::Primitive(nir::Primitive::U8)]),
            Budget::default()
        )
        .unwrap(),
        Classification::Affine
    );
    assert_eq!(
        classify(&world, protocol, &boolean, Budget::default()).unwrap(),
        Classification::Intrinsic
    );
    assert!(matches!(
        classify(
            &world,
            protocol,
            &boolean,
            Budget {
                steps: 0,
                ..Budget::default()
            }
        )
        .unwrap(),
        Classification::Pending(_)
    ));
    let assumption = nir::program::TraitRequirement {
        interface: protocol,
        interface_arguments: vec![],
        subject: nominal("Owned", vec![]),
    };
    let assumed = World {
        assumptions: std::slice::from_ref(&assumption),
        ..world
    };
    assert!(matches!(
        classify(&assumed, protocol, &assumption.subject, Budget::default()).unwrap(),
        Classification::Assumed(_)
    ));
}

#[test]
fn source_package_identity_survives_generic_contract_and_instance_elaboration() {
    use comptime_interpretor::{Budget, Interpreter, Value};
    for identity in [17, 1234567890123] {
        let source = compile_in(
            include_str!("../../../development/examples/instantiated-functions.fl"),
            identity,
        );
        assert!(
            source
                .functions
                .iter()
                .all(|f| f.declaration.package == identity)
        );
        assert!(
            source
                .structs
                .iter()
                .all(|s| s.declaration.package == identity)
        );
        assert!(
            source
                .traits
                .iter()
                .all(|t| t.declaration.package == identity)
        );
        assert!(
            source
                .implementations
                .iter()
                .all(|i| i.declaration.package == identity)
        );
        assert!(!source.to_string().contains("(package 0)"));
        let graph = luna::instantiation::instantiate(&source, "entry", 100_000).unwrap();
        assert_eq!(
            Interpreter::from_iir(&graph, Budget::default())
                .unwrap()
                .invoke_instance(&graph.roots[0], vec![])
                .unwrap(),
            Value::UnsignedInteger(41)
        );
        let contracts = compile_in(
            include_str!("../../../development/examples/trait-program-contracts.fl"),
            identity,
        );
        let graph = luna::instantiation::instantiate(&contracts, "generic_call", 100_000).unwrap();
        assert_eq!(
            Interpreter::from_iir(&graph, Budget::default())
                .unwrap()
                .invoke_instance(&graph.roots[0], vec![Value::UnsignedInteger(41)])
                .unwrap(),
            Value::UnsignedInteger(42)
        );
    }
}

#[test]
fn equal_source_names_in_distinct_packages_do_not_share_nominal_or_opaque_identity() {
    let text = include_str!("../../../development/examples/normal-form-text.fl");
    let first = compile_in(text, 41);
    let second = compile_in(text, 42);
    let signature = |p: &nir::program::Package| {
        p.functions
            .iter()
            .find(|f| f.name == "same")
            .unwrap()
            .signature
            .clone()
    };
    assert_ne!(signature(&first), signature(&second));
    assert_ne!(
        first.normal_forms[0].declaration,
        second.normal_forms[0].declaration
    );
    let first = compile_in("struct Item {} fn entry()->Item {Item {}}", 41);
    let second = compile_in("struct Item {} fn entry()->Item {Item {}}", 42);
    assert_eq!(
        first.structs[0].declaration.local,
        second.structs[0].declaration.local
    );
    assert_ne!(first.structs[0].declaration, second.structs[0].declaration);
    let first_graph = luna::instantiation::instantiate(&first, "entry", 100_000).unwrap();
    let second_graph = luna::instantiation::instantiate(&second, "entry", 100_000).unwrap();
    let mut merged = first_graph.clone();
    merged.roots.extend(second_graph.roots.clone());
    merged.functions.extend(second_graph.functions.clone());
    merged.nominals.extend(second_graph.nominals.clone());
    merged.validate(&mut 100_000).unwrap();
    let mut interpreter =
        comptime_interpretor::Interpreter::from_iir(&merged, Default::default()).unwrap();
    let a = interpreter
        .invoke_instance(&merged.roots[0], vec![])
        .unwrap();
    let b = interpreter
        .invoke_instance(&merged.roots[1], vec![])
        .unwrap();
    assert_ne!(a, b);
}

#[test]
fn abstract_associated_values_select_defaults_and_overrides_at_instantiation() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(include_str!(
        "../../../development/examples/associated-values.fl"
    ));
    let before = package.to_string();
    assert!(before.contains("(associated-value "));
    let graph = luna::instantiation::instantiate(&package, "entry", 100000).unwrap();
    assert!(!graph.to_string().contains("(associated-value "));
    assert_eq!(package.to_string(), before);
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert!(luna::instantiation::instantiate(&package, "entry", 1).is_err());
    assert_eq!(
        luna::instantiation::instantiate(&package, "entry", 100000).unwrap(),
        graph
    );
    println!("ASSOCIATED_VALUE_NIR_DUMP={package}");
    println!("ASSOCIATED_VALUE_IIR_DUMP={graph}");
}

#[test]
fn associated_value_origins_keep_inherited_interface_arguments_distinct() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Flag where A {assoc enabled:bool=true;fn flag()->bool {enabled}} trait Child :- Flag<A> where A {} struct Host {} impl Flag<bool> for Host {assoc enabled:bool=false;} impl Flag<'x''type> for Host {} impl Child<bool> for Host {} impl Child<'x''type> for Host {} fn query(value:T,marker:A)->bool where A,T :- Child<A> {T.as(Child<A>).enabled} fn entry()->bool {not query(Host {},true) and query(Host {},'x') and not Host.as(Flag<bool>).flag() and Host.as(Flag<'x''type>).flag()}",
    );
    let graph = luna::instantiation::instantiate(&package, "entry", 100000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn associated_values_use_selected_member_types_and_aggregate_values() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Source {assoc Item:.Type'builtin;assoc initial:Item;fn make()->Item {initial}} impl Source for bool {assoc Item:.Type'builtin=bool;assoc initial:bool=true;} impl Source for 'x''type {assoc Item:.Type'builtin='x''type;assoc initial:'x''type='x';} fn query()->T.as(Source).Item where T :- Source {T.as(Source).initial} fn entry()->(bool,'x''type) {(bool.as(Source).make(),query<'x''type>())}",
    );
    let graph = luna::instantiation::instantiate(&package, "entry", 100000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Tuple(vec![Value::Bool(true), Value::Char('x')])
    );
    let package = compile(
        "trait Pair {assoc values:(bool,bool)=(true,false);fn get()->(bool,bool) {values}} impl Pair for bool {} fn entry()->(bool,bool) {bool.as(Pair).get()}",
    );
    let graph = luna::instantiation::instantiate(&package, "entry", 100000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Tuple(vec![Value::Bool(true), Value::Bool(false)])
    );
}

#[test]
fn associated_scalar_values_supply_declaration_ordered_parameter_defaults() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Flag {assoc enabled:bool=true;} impl Flag for bool {assoc enabled:bool=false;} impl Flag for 'x''type {} fn query(value:T,.selected:bool=T.as(Flag).enabled)->bool where T :- Flag {selected} fn entry()->bool {not query(true) and query('x') and query(true,.selected=true)}",
    );
    let graph = luna::instantiation::instantiate(&package, "entry", 100000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn associated_value_nodes_validate_member_identity_type_and_selection() {
    use nir::program::ExprKind as K;
    let package = compile(
        "trait Flag {assoc enabled:bool=true;} impl Flag for bool {} fn query(value:T)->bool where T :- Flag {T.as(Flag).enabled} fn entry()->bool {query(true)}",
    );
    for mutation in 0..4 {
        let mut damaged = package.clone();
        let owner = damaged
            .functions
            .iter_mut()
            .find(|f| f.name == "query")
            .unwrap();
        iir::visit::expression_mut(&mut owner.body, &mut 100000, |expr, _| {
            if let K::AssociatedValue {
                interface, member, ..
            } = &mut expr.kind
            {
                match mutation {
                    0 => expr.ty = nir::Term::Primitive(nir::Primitive::Char),
                    1 => *member = interface.declaration,
                    2 => interface
                        .arguments
                        .push(nir::Term::Primitive(nir::Primitive::Bool)),
                    _ => {}
                }
            }
            Ok(())
        })
        .unwrap();
        if mutation == 3 {
            damaged.implementations.clear();
        }
        assert!(
            luna::instantiation::instantiate(&damaged, "entry", 100000).is_err(),
            "mutation {mutation}"
        );
    }
    let mut graph = luna::instantiation::instantiate(&package, "entry", 100000).unwrap();
    let template = &package
        .functions
        .iter()
        .find(|f| f.name == "query")
        .unwrap()
        .body;
    let function = graph
        .functions
        .iter_mut()
        .find(|f| f.function.name == "query")
        .unwrap();
    function.function.body = template.clone();
    assert!(
        graph
            .validate(&mut 100000)
            .unwrap_err()
            .contains("associated value selection")
    );
}

#[test]
fn inline_function_intent_survives_generic_and_staged_instantiation() {
    use comptime_interpretor::{Interpreter, Value};
    // Keep one ordinary generic call outside the staged expansion and one
    // recursive staged fallback: these are actual residual instance boundaries.
    let package = compile(
        "inline fn identity(value:T)->T where T {value}
        inline fn select(comptime choose:bool,value:bool)->bool {if choose {select(false,not value)} else {not value}}
        fn entry()->bool {identity(true) and select(true,identity(true))}",
    );
    for name in ["identity", "select"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        assert!(function.inline);
        assert_eq!(function.phase, nir::program::FunctionPhase::Runtime);
        assert!(!function.pure);
        assert!(function.to_string().contains("(inline true)"));
    }
    assert!(
        !package
            .functions
            .iter()
            .find(|f| f.name == "entry")
            .unwrap()
            .inline
    );
    let graph = luna::instantiation::instantiate(&package, "entry", 100_000).unwrap();
    for name in ["identity", "select"] {
        let instances: Vec<_> = graph
            .functions
            .iter()
            .filter(|f| f.function.name == name)
            .collect();
        assert!(!instances.is_empty(), "{name}");
        assert!(instances.iter().all(|f| f.function.inline), "{name}");
    }
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn inline_function_calls_preserve_argument_order_and_effects() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("struct Counter {value:.u8'builtin}
        fn tick(counter:*Counter)->.u8'builtin {assumes counter.*.value < 255;counter.*.value = counter.*.value + 1;counter.*.value}
        inline fn use_twice(.left:.u8'builtin=0,.right:.u8'builtin=0)->bool {left == 2 and right == 1 and left == 2}
        fn entry()->bool {let counter=Counter {value:0};use_twice(.right=tick(counter.ref),.left=tick(counter.ref)) and counter.value == 2}");
    let graph = luna::instantiation::instantiate(&package, "entry", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn inline_function_deferred_signature_preserves_intent() {
    // Recursive fallback retains a callable whose deferred domain has completed.
    // A nonrecursive identity is now eligible for late expansion instead.
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        inline fn identity(comptime flag:bool,value:defer Pick(flag),again:bool)->defer Pick(flag) {
            if again {identity(flag,value,false)} else {value}
        }
        fn entry()->bool {identity(true,true,true)}",
    );
    let graph = luna::instantiation::instantiate(&package, "entry", 100_000).unwrap();
    assert!(
        graph
            .functions
            .iter()
            .any(|f| f.function.name == "identity" && f.function.inline)
    );
    assert_eq!(
        luna::execution::interpret(&package, "entry", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn inline_function_preserves_phase_purity_and_selected_methods() {
    use comptime_interpretor::Value;
    let package = compile(
        "comptime inline fn invert(value:bool)->bool {not value}
        pure inline fn stable(value:bool)->bool {value}
        trait Read {fn read(self)->bool;}
        struct Reader {}
        impl Read for Reader {inline fn read(self)->bool {true}}
        const FLAG=invert(false);
        fn entry()->bool {stable(true) and Reader {}.as(Read).read()}",
    );
    let invert = package
        .functions
        .iter()
        .find(|f| f.name == "invert")
        .unwrap();
    assert!(invert.inline);
    assert_eq!(invert.phase, nir::program::FunctionPhase::Comptime);
    let stable = package
        .functions
        .iter()
        .find(|f| f.name == "stable")
        .unwrap();
    assert!(stable.inline && stable.pure);
    let graph = luna::instantiation::instantiate(&package, "entry", 100_000).unwrap();
    assert!(
        graph
            .functions
            .iter()
            .any(|f| f.function.name == "read" && f.function.inline)
    );
    assert_eq!(
        luna::execution::interpret(&package, "entry", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn optional_inline_pass_expands_calls_with_hygienic_returns_and_defaults() {
    use comptime_interpretor::{Interpreter, Value};
    let mut package = compile(
        "inline fn identity(value:T)->T where T {return value;}
        inline fn pick(value:bool,.fallback:bool=not value)->bool {
            if value {return identity(value);}fallback
        }
        fn entry()->bool {pick(true) and pick(false)}",
    );
    let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    assert!(report.expanded >= 3, "{report}");
    assert!(report.to_string().starts_with("(inline-report "));
    assert!(!package.internal_return_packets.is_empty());
    let graph = luna::instantiation::instantiate(&package, "entry", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn optional_inline_pass_retains_recursive_and_indirect_calls() {
    use comptime_interpretor::{Interpreter, Value};
    let mut package = compile(
        "inline fn recur(stop:bool)->bool {if stop {true} else {recur(true)}}
        fn entry()->bool {let callback=recur;callback(false) and recur(false)}",
    );
    let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    assert!(report.expanded > 0);
    let graph = luna::instantiation::instantiate(&package, "entry", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn optional_inline_pass_small_budget_keeps_original_program_usable() {
    let original = compile(
        "inline fn invert(value:bool)->bool {not value}
        fn entry()->bool {invert(false)}",
    );
    for budget in [0, 1, 10, 30, 60, 100] {
        let mut package = original.clone();
        let report = elaboration::program::inlining::expand(&mut package, &mut { budget });
        if !matches!(report,Ok(report) if report.expanded>0) {
            assert_eq!(package.expansion_sources, original.expansion_sources);
            assert_eq!(package.enums, original.enums);
            assert_eq!(
                package.internal_return_packets,
                original.internal_return_packets
            );
            assert_eq!(
                package.internal_return_empty_replacements,
                original.internal_return_empty_replacements
            );
            assert_eq!(
                package.quote_identity_cursor,
                original.quote_identity_cursor
            );
        }
        // Regardless of whether preparation or a candidate ran out of budget,
        // no half-expanded call or leaked private packet may escape.
        assert_eq!(
            luna::execution::interpret(&package, "entry", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true),
            "{budget}"
        );
    }
}

#[test]
fn optional_inline_pass_mutual_recursion_remains_bounded() {
    let original = compile(
        "inline fn first(stop:bool)->bool {if stop {true} else {second(true)}}
        inline fn second(stop:bool)->bool {if stop {first(true)} else {first(false)}}
        fn entry()->bool {first(false)}",
    );
    for budget in [1000, 10000, 100000] {
        let mut package = original.clone();
        if let Ok(report) = elaboration::program::inlining::expand(&mut package, &mut { budget }) {
            assert!(report.expanded <= 9, "{report}");
        }
        assert_eq!(
            luna::execution::interpret(&package, "entry", 1000000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn optional_inline_pass_evaluates_even_unused_inputs_once() {
    use comptime_interpretor::{Event, Interpreter, Value};
    let mut package=compile("fn mark(slot:*bool)->bool {slot.* = not slot.*;slot.*}
        inline fn twice(value:bool,.fallback:bool=not value)->bool {value == value and fallback}
        inline fn ignore(value:bool)->bool {true}
        fn entry()->bool {var flag=false;let first=twice(mark(flag.ref));let second=ignore(mark(flag.ref));not first and second and not flag}");
    elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    let graph = luna::instantiation::instantiate(&package, "entry", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let mark = package
        .functions
        .iter()
        .find(|f| f.name == "mark")
        .unwrap()
        .declaration;
    assert_eq!(
        vm.events()
            .iter()
            .filter(|event| matches!(event,Event::EnterFunction(id) if *id==mark))
            .count(),
        2
    );
}

#[test]
fn optional_inline_pass_selects_trait_bodies_and_keeps_interface_contracts() {
    use comptime_interpretor::{ErrorKind, Event, Interpreter, Value};
    let original = compile(
        "trait Check {fn check(self,value:bool)->bool where requires value == value, ensures true;}
        struct Reader {} impl Check for Reader {inline fn check(self,value:bool)->bool {value}}
        fn mark(slot:*bool)->bool {slot.* = not slot.*;slot.*}
        fn entry()->bool {var flag=false;Reader {}.as(Check).check(mark(flag.ref))}",
    );
    let body = original
        .functions
        .iter()
        .find(|f| f.name == "check")
        .unwrap()
        .declaration;
    for failure in ["none", "entry", "result"] {
        let mut package = original.clone();
        if failure != "none" {
            let contract = &mut package
                .traits
                .iter_mut()
                .find(|t| t.name == "Check")
                .unwrap()
                .methods[0]
                .program;
            let predicates = if failure == "entry" {
                &mut contract.preconditions
            } else {
                &mut contract.postconditions
            };
            assert!(!predicates.is_empty());
            predicates[0].kind =
                nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
        }
        let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
        assert_eq!(report.expanded, 1, "{report}");
        let graph = luna::instantiation::instantiate(&package, "entry", 100_000).unwrap();
        let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
        let result = vm.invoke_instance(&graph.roots[0], vec![]);
        if failure != "none" {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        } else {
            assert_eq!(result.unwrap(), Value::Bool(true));
        }
        assert!(
            !vm.events()
                .iter()
                .any(|event| matches!(event, Event::EnterFunction(id) if *id == body))
        );
        let mark = package
            .functions
            .iter()
            .find(|f| f.name == "mark")
            .unwrap()
            .declaration;
        assert_eq!(
            vm.events()
                .iter()
                .filter(|event| matches!(event, Event::EnterFunction(id) if *id == mark))
                .count(),
            1
        );
    }
}

#[test]
fn optional_inline_pass_specializes_generic_trait_methods_inside_closures() {
    use comptime_interpretor::{Event, Interpreter, Value};
    let mut package = compile("trait Identity {fn keep(self,value:U)->U where U;}
        struct Holder where T {value:T}
        impl Identity for Holder<T> where T {
            inline fn keep(self,value:U)->U where U {return value;}
        }
        fn entry()->bool {
            let choose=|value:bool| {Holder<bool> {value:false}.as(Identity).keep(value)};
            choose(true) and Holder<bool> {value:false}.as(Identity).keep(true) and Holder<bool> {value:false}.as(Identity).keep('a') == 'a' and Holder<.char'builtin> {value:'z'}.as(Identity).keep(true)
        }");
    let method = package
        .functions
        .iter()
        .find(|f| f.name == "keep")
        .unwrap()
        .declaration;
    let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    assert_eq!(report.expanded, 4, "{report}");
    assert_eq!(report.selected_methods, 3, "{report}");
    assert_eq!(report.reused_method_selections, 1, "{report}");
    let graph = luna::instantiation::instantiate(&package, "entry", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert!(
        !vm.events()
            .iter()
            .any(|event| matches!(event, Event::EnterFunction(id) if *id == method))
    );
}

#[test]
fn optional_inline_effect_replaces_requests_with_local_matching() {
    use comptime_interpretor::Value;
    let original = compile(
        "inline effect ask(value:bool)->bool;
        inline fn subject(value:bool)->#ask bool {ask(value)# or ask(false)#}
        fn entry()->bool {var observed=false;let result=subject(true)# {
            ask(true)=>{observed=true;false},ask(false)=>observed,
        };result and observed}",
    );
    let mut package = original.clone();
    let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    assert_eq!(report.expanded_handlers, 1, "{report}");
    assert_eq!(report.expanded_requests, 2, "{report}");
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "entry")
        .unwrap();
    nir::visit::expressions(entry, &mut 100_000, |expr, _| {
        assert!(!matches!(
            expr.kind,
            nir::program::ExprKind::EffectRequest { .. }
                | nir::program::ExprKind::EffectHandle { .. }
        ));
        Ok(())
    })
    .unwrap();
    for source in [&original, &package] {
        assert_eq!(
            luna::execution::interpret(source, "entry", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn optional_inline_effect_retains_unsupported_control_and_unexpanded_calls() {
    use comptime_interpretor::Value;
    for (modifier, arm) in [("", "true")] {
        let mut package = compile(&format!(
            "inline effect ask()->bool;
            {modifier} fn subject()->#ask bool {{ask()#}}
            fn entry()->bool {{subject()# {{ask()=>{arm},}}}}"
        ));
        let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
        assert_eq!(report.expanded_handlers, 0, "{report}");
        assert_eq!(report.expanded_requests, 0, "{report}");
        assert_eq!(
            luna::execution::interpret(&package, "entry", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn optional_inline_effect_without_inline_callee_evaluates_request_inputs_once() {
    use comptime_interpretor::{Event, Interpreter, Value};
    let mut package = compile(
        "inline effect ask(value:bool)->bool;
        fn mark(pointer:*bool)->bool {pointer.* = not pointer.*;pointer.*}
        fn subject()->#ask bool {true}
        fn entry()->bool {var flag=false;let result=(do {ask(mark(flag.ref))#})# {
            ask(value)=>value and value,
        };result and flag}",
    );
    let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    assert_eq!(report.expanded_handlers, 1, "{report}");
    let graph = luna::instantiation::instantiate(&package, "entry", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let mark = package
        .functions
        .iter()
        .find(|f| f.name == "mark")
        .unwrap()
        .declaration;
    assert_eq!(
        vm.events()
            .iter()
            .filter(|event| matches!(event,Event::EnterFunction(id) if *id==mark))
            .count(),
        1
    );
}

#[test]
fn optional_inline_effect_keeps_operation_instances_and_loop_requests_distinct() {
    use comptime_interpretor::Value;
    let original = compile(
        "inline effect choose()->T where T;
        inline fn subject()->#[choose<bool>,choose<.char'builtin>] bool {
            (choose<bool>()#) and (choose<.char'builtin>()#)=='x'
        }
        fn entry()->bool {var flag=false;var result=false;
            while not flag {result=subject()# {
                choose<bool>()=>{flag=true;true},choose<.char'builtin>()=>'x',
            };}result and flag}",
    );
    let mut package = original.clone();
    let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    assert_eq!(report.expanded_handlers, 1, "{report}");
    assert_eq!(report.expanded_requests, 2, "{report}");
    for source in [&original, &package] {
        assert_eq!(
            luna::execution::interpret(source, "entry", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn optional_inline_effect_expands_independent_nested_installations() {
    use comptime_interpretor::Value;
    for nested in [
        "let inside=read()# {read()=>true,};inside and (poll()#)",
        "poll()#",
    ] {
        let handler = if nested == "poll()#" {
            "read()# {read()=>do {resume true;},}"
        } else {
            "true"
        };
        let original = compile(&format!(
            "inline effect read()->bool;inline effect poll()->bool;
            inline fn subject()->#poll bool {{{nested}}}
            fn entry()->bool {{subject()# {{poll()=>{handler},}}}}"
        ));
        let mut package = original.clone();
        let report = elaboration::program::inlining::expand(&mut package, &mut 1000000).unwrap();
        let entry = package
            .functions
            .iter()
            .find(|f| f.name == "entry")
            .unwrap();
        assert!(
            report.expanded_handlers >= 2,
            "{nested}: {report}\n{}",
            entry.body
        );
        let mut remaining = 0;
        nir::visit::expression(&entry.body, &mut 100000, |expr, _| {
            if matches!(
                expr.kind,
                nir::program::ExprKind::EffectHandle { .. }
                    | nir::program::ExprKind::EffectRequest { .. }
            ) {
                remaining += 1;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(remaining, 0, "{nested}: {report}");
        for package in [&original, &package] {
            assert_eq!(
                luna::execution::interpret(package, "entry", 1000000).unwrap(),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn optional_inline_effect_maps_subset_and_reordered_callee_rows() {
    use comptime_interpretor::Value;
    for (row, body, requests) in [
        ("poll", "not (poll()#)", 1),
        ("[poll,read]", "not (poll()#) and (read()#)", 2),
    ] {
        let original = compile(&format!(
            "inline effect read()->bool;inline effect poll()->bool;
            inline fn inner()->#{row} bool {{{body}}}
            inline fn subject()->#[read,poll] bool {{inner()}}
            fn entry()->bool {{subject()# {{read()=>true,poll()=>false,}}}}"
        ));
        let mut package = original.clone();
        let report = elaboration::program::inlining::expand(&mut package, &mut 1000000).unwrap();
        assert_eq!(report.expanded_handlers, 1, "{row}: {report}");
        assert_eq!(report.expanded_requests, requests, "{row}: {report}");
        for package in [&original, &package] {
            assert_eq!(
                luna::execution::interpret(package, "entry", 1000000).unwrap(),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn optional_inline_effect_rolls_back_a_partially_transformed_installation() {
    use comptime_interpretor::Value;
    let mut original = compile(
        "inline effect ask()->bool;
        fn ordinary()->#ask bool {ask()#}
        inline fn yes()->bool {return true;}
        inline fn partially()->#ask bool {let first=ask()#;ordinary()}
        fn entry()->bool {partially()# {ask()=>do {resume yes();},}}",
    );
    original.effects[0].inline = false;
    elaboration::program::inlining::expand(&mut original, &mut 100_000).unwrap();
    original.effects[0].inline = true;
    for function in &mut original.functions {
        function.inline = false;
    }
    for budget in [100_000, 500, 200, 50] {
        let mut package = original.clone();
        if let Ok(report) = elaboration::program::inlining::expand(&mut package, &mut { budget }) {
            assert_eq!(report.expanded_handlers, 0, "{report}");
            assert_eq!(report.expanded_requests, 0, "{report}");
        }
        assert_eq!(
            package, original,
            "failed installation changed IR or metadata at budget {budget}"
        );
        assert_eq!(
            luna::execution::interpret(&package, "entry", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn optional_inline_effect_nested_outer_requests_keep_routing_and_rollback() {
    use comptime_interpretor::Value;
    let mut original = compile(
        "inline effect read()->bool;inline effect poll()->bool;
        inline fn inside()->#[read,poll] bool {let first=read()#;first and (poll()#)}
        fn entry()->bool {(inside()# {read()=>do {resume true;},})# {poll()=>true,}}",
    );
    for effect in &mut original.effects {
        effect.inline = false;
    }
    elaboration::program::inlining::expand(&mut original, &mut 1000000).unwrap();
    for effect in &mut original.effects {
        effect.inline = true;
    }
    for function in &mut original.functions {
        function.inline = false;
    }
    for budget in [1000000, 1000, 200, 1] {
        let mut package = original.clone();
        let report = match elaboration::program::inlining::expand(&mut package, &mut { budget }) {
            Ok(report) => report,
            Err(error) => {
                assert!(error.contains("budget exhausted"), "{error}");
                assert_eq!(package, original, "failed preparation at budget {budget}");
                Default::default()
            }
        };
        if budget == 1000000 {
            assert_eq!(report.expanded_handlers, 2, "{report}");
            assert_eq!(report.expanded_requests, 2, "{report}");
        }
        if report.expanded_handlers == 0 {
            assert_eq!(
                package, original,
                "rollback metadata changed at budget {budget}"
            );
        }
        assert_eq!(
            luna::execution::interpret(&package, "entry", 1000000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn optional_inline_effect_invalid_row_maps_do_not_publish_partial_rewrites() {
    let mut original = compile(
        "inline effect read()->bool;inline effect poll()->bool;
        inline fn inner()->#poll bool {poll()#}
        inline fn subject()->#[read,poll] bool {inner()}
        fn entry()->bool {subject()# {read()=>false,poll()=>true,}}",
    );
    for effect in &mut original.effects {
        effect.inline = false;
    }
    elaboration::program::inlining::expand(&mut original, &mut 1000000).unwrap();
    for effect in &mut original.effects {
        effect.inline = true;
    }
    for function in &mut original.functions {
        function.inline = false;
    }
    for invalid in [vec![0], vec![usize::MAX], vec![]] {
        let mut package = original.clone();
        let entry = package
            .functions
            .iter_mut()
            .find(|f| f.name == "entry")
            .unwrap();
        let mut changed = false;
        nir::visit::expression_mut(&mut entry.body, &mut 100000, |expr, _| {
            if let nir::program::ExprKind::EffectRun { slots, .. } = &mut expr.kind {
                if slots.as_slice() == [1] {
                    *slots = invalid.clone();
                    changed = true;
                }
            }
            Ok(())
        })
        .unwrap();
        assert!(changed);
        let before = package.clone();
        let report = elaboration::program::inlining::expand(&mut package, &mut 1000000).unwrap();
        assert_eq!(report.expanded_handlers, 0, "{invalid:?}: {report}");
        assert_eq!(package, before, "{invalid:?}");
    }
}

#[test]
fn optional_inline_effect_inner_commit_survives_outer_retention() {
    use comptime_interpretor::Value;
    let mut original = compile(
        "inline effect read()->bool;inline effect poll()->bool;
        fn ordinary()->#poll bool {poll()#}
        inline fn subject()->#poll bool {
            let inside=read()# {read()=>do {resume true;},};
            ordinary()
        }
        fn entry()->bool {subject()# {poll()=>true,}}",
    );
    for effect in &mut original.effects {
        effect.inline = false;
    }
    elaboration::program::inlining::expand(&mut original, &mut 1000000).unwrap();
    for effect in &mut original.effects {
        effect.inline = true;
    }
    for function in &mut original.functions {
        function.inline = false;
    }
    let mut package = original.clone();
    let report = elaboration::program::inlining::expand(&mut package, &mut 1000000).unwrap();
    assert!(report.expanded_handlers > 0, "{report}");
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "entry")
        .unwrap();
    let mut handlers = 0;
    nir::visit::expression(&entry.body, &mut 100000, |expr, _| {
        if matches!(expr.kind, nir::program::ExprKind::EffectHandle { .. }) {
            handlers += 1;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(handlers, 1, "{report}");
    assert_eq!(
        luna::execution::interpret(&package, "entry", 1000000).unwrap(),
        Value::Bool(true)
    );
    let previous = package.clone();
    let repeated = elaboration::program::inlining::expand(&mut package, &mut 1000000).unwrap();
    assert_eq!(repeated.expanded_handlers, 0, "{repeated}");
    assert_eq!(package, previous);
}

#[test]
fn optional_inline_effect_resume_returns_to_each_request_not_the_caller() {
    use comptime_interpretor::Value;
    let original=compile("inline effect ask(value:bool)->bool;
        inline fn subject()->#ask bool {let first=ask(true)#;let second=ask(false)#;first and second}
        fn entry()->bool {var seen=false;let result=subject()# {
            ask(value)=>{if value {seen=true;resume true;}resume seen;seen=false;false},
        };result and seen}");
    let mut package = original.clone();
    let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    assert_eq!(report.expanded_handlers, 1, "{report}");
    assert_eq!(report.expanded_requests, 2, "{report}");
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "entry")
        .unwrap();
    nir::visit::expressions(entry, &mut 100_000, |expr, _| {
        assert!(!matches!(
            expr.kind,
            nir::program::ExprKind::EffectResume(_)
                | nir::program::ExprKind::EffectRequest { .. }
                | nir::program::ExprKind::EffectHandle { .. }
        ));
        Ok(())
    })
    .unwrap();
    for source in [&original, &package] {
        assert_eq!(
            luna::execution::interpret(source, "entry", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn optional_inline_effect_does_not_capture_returns_from_request_arguments() {
    use comptime_interpretor::Value;
    let original = compile(
        "inline effect ask(value:bool)->bool;
        fn entry()->bool {(do {ask(do {return true;false})#})# {
            ask(value)=>do {resume false;},
        }}",
    );
    let mut package = original.clone();
    let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    assert_eq!(report.expanded_handlers, 1, "{report}");
    for source in [&original, &package] {
        assert_eq!(
            luna::execution::interpret(source, "entry", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn optional_inline_effect_handler_loops_keep_local_and_caller_targets_separate() {
    use comptime_interpretor::Value;
    let original = compile(
        "inline effect poll()->bool;
        inline fn subject()->#poll bool {poll()#}
        fn entry()->bool {var finished=false;while:outside true {
            subject()# {poll()=>do {
                var first=true;while:inside true {
                    if first {first=false;continue inside;}break inside;
                }
                finished=not first;break outside;
            },};assumes false;
        }finished}",
    );
    let mut package = original.clone();
    let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    assert_eq!(report.expanded_handlers, 1, "{report}");
    for source in [&original, &package] {
        assert_eq!(
            luna::execution::interpret(source, "entry", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn optional_inline_effect_keeps_callee_closure_invocation_separate() {
    use comptime_interpretor::Value;
    let original = compile(
        "inline effect poll()->bool;
        inline fn subject(value:bool)->#poll bool {
            let callback=|input:bool| {return input and value;};
            let answer=poll()#;callback(answer)
        }
        fn entry()->bool {subject(true)# {poll()=>true,}}",
    );
    let mut package = original.clone();
    let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    assert_eq!(report.expanded_handlers, 1, "{report}");
    for source in [&original, &package] {
        assert_eq!(
            luna::execution::interpret(source, "entry", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn optional_inline_effect_retains_a_closure_with_its_own_effect_row() {
    use comptime_interpretor::Value;
    let mut package = compile(
        "inline effect poll()->bool;
        typealias Callback=%{value:bool} fn()->#poll bool;
        inline fn subject(value:bool)->#poll bool {
            let callback:Callback=|| {poll()# and value};callback()
        }
        fn entry()->bool {subject(true)# {poll()=>true,}}",
    );
    let report = elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    assert_eq!(report.expanded_handlers, 0, "{report}");
    assert_eq!(
        luna::execution::interpret(&package, "entry", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn inline_effect_iteration_pipeline_forwards_each_item_once() {
    use comptime_interpretor::Value;
    let original = compile(include_str!(
        "../../../development/examples/inline-iteration.fl"
    ));
    let mut expanded = original.clone();
    let report = elaboration::program::inlining::expand(&mut expanded, &mut 100_000).unwrap();
    assert!(report.expanded_handlers >= 3, "{report}");
    let entry = expanded
        .functions
        .iter()
        .find(|function| function.name == "entry")
        .unwrap();
    nir::visit::expressions(entry, &mut 100_000, |expr, _| {
        assert!(!matches!(
            expr.kind,
            nir::program::ExprKind::EffectHandle { .. }
                | nir::program::ExprKind::EffectRequest { .. }
                | nir::program::ExprKind::EffectRun { .. }
                | nir::program::ExprKind::EffectComputation { .. }
        ));
        Ok(())
    })
    .unwrap();
    for package in [&original, &expanded] {
        assert_eq!(
            luna::execution::interpret(package, "entry", 100_000).unwrap(),
            Value::UnsignedInteger(7)
        );
    }
}

#[test]
fn inline_effect_partial_handlers_forward_without_removing_the_inner_layer() {
    use comptime_interpretor::Value;
    let original=compile("inline effect ask(value:bool)->bool;
        inline fn produce()->#ask bool {
            let a=ask(true)#;let b=ask(false)#;let c=ask(true)#;let d=ask(false)#;
            not a and b and not c and d
        }
        inline fn filtered()->#ask bool {produce()# {ask(true)=>false,}}
        fn entry()->bool {var flag=false;let result=filtered()# {ask(value)=>{flag=not flag;not value},};result and not flag}");
    let mut expanded = original.clone();
    let report = elaboration::program::inlining::expand(&mut expanded, &mut 1000000).unwrap();
    assert!(report.expanded_handlers >= 2, "{report}");
    let entry = expanded
        .functions
        .iter()
        .find(|f| f.name == "entry")
        .unwrap();
    nir::visit::expressions(entry, &mut 100000, |expr, _| {
        assert!(!matches!(
            expr.kind,
            nir::program::ExprKind::EffectHandle { .. }
                | nir::program::ExprKind::EffectRequest { .. }
        ));
        Ok(())
    })
    .unwrap();
    for package in [&original, &expanded] {
        assert_eq!(
            luna::execution::interpret(package, "entry", 1000000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn generic_trait_inline_producers_specialize_associated_effect_inputs() {
    use comptime_interpretor::Value;
    let original=compile("typealias Unit=.void'builtin;
        inline effect emit(value:T) where T;
        trait Producer {assoc Item:.Type'builtin;fn produce(self)->#emit<Item> Unit;}
        struct Flag {value:bool}
        impl Producer for Flag {
            assoc Item:.Type'builtin=bool;
            inline fn produce(self)->#emit<bool> Unit {emit<bool>(self.value)#;}
        }
        struct Letter {value:.char'builtin}
        impl Producer for Letter {
            assoc Item:.Type'builtin=.char'builtin;
            inline fn produce(self)->#emit<.char'builtin> Unit {emit<.char'builtin>(self.value)#;}
        }
        inline fn invoke(value:T)->#emit<T.as(Producer).Item> Unit where T:-Producer {value.produce()}
        fn entry()->bool {var result=false;invoke(Flag {value:true})# {emit<bool>(value)=>{result=value;},};var letter=false;invoke(Letter {value:'x'})# {emit<.char'builtin>(value)=>{letter=value=='x';},};result and letter}");
    let mut expanded = original.clone();
    let report = elaboration::program::inlining::expand(&mut expanded, &mut 1000000).unwrap();
    assert!(report.selected_methods > 0, "{report}");
    assert_eq!(report.expanded_handlers, 2, "{report}");
    let entry = expanded
        .functions
        .iter()
        .find(|f| f.name == "entry")
        .unwrap();
    nir::visit::expressions(entry, &mut 100000, |expr, _| {
        assert!(!matches!(
            expr.kind,
            nir::program::ExprKind::EffectHandle { .. }
                | nir::program::ExprKind::EffectRequest { .. }
                | nir::program::ExprKind::TraitMethod { .. }
        ));
        Ok(())
    })
    .unwrap();
    for package in [&original, &expanded] {
        assert_eq!(
            luna::execution::interpret(package, "entry", 1000000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn inline_effect_associated_rows_do_not_require_a_method_call() {
    use comptime_interpretor::Value;
    let original=compile("typealias Unit=.void'builtin;
        inline effect emit(value:T) where T;
        trait Source {assoc Item:.Type'builtin;}
        impl Source for bool {assoc Item:.Type'builtin=bool;}
        inline fn send(value:T.as(Source).Item)->#emit<T.as(Source).Item> Unit where T:-Source {emit<T.as(Source).Item>(value)#;}
        fn entry()->bool {var result=false;send<bool>(true)# {emit<bool>(value)=>{result=value;},};result}");
    let mut expanded = original.clone();
    let report = elaboration::program::inlining::expand(&mut expanded, &mut 1000000).unwrap();
    assert_eq!(report.selected_methods, 0, "{report}");
    assert_eq!(report.expanded_handlers, 1, "{report}");
    assert_eq!(report.expanded_requests, 1, "{report}");
    for package in [&original, &expanded] {
        assert_eq!(
            luna::execution::interpret(package, "entry", 1000000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn inline_weakened_trait_contracts_preserve_entry_guards_and_check_counts() {
    use comptime_interpretor::{Event, Interpreter, Value};
    for domain in ["requires value", "requires value, requires other"] {
        for tail in ["answer", "return answer;"] {
            let source="trait Gate {fn apply(self,value:bool,other:bool)->result:bool where requires value, ensures result;}
        impl Gate for bool {inline fn apply(self,value:bool,other:bool)->result:bool where requires value == value, ensures result == value'old {let answer=value;value=false;answer}}
        fn entry()->bool {let receiver=true;receiver.as(Gate).apply(true,true)}";
            let original = compile(
                &source
                    .replace(
                        "requires value, ensures result",
                        &format!("{domain}, ensures result"),
                    )
                    .replace("value=false;answer", &format!("value=false;{tail}")),
            );
            let mut expected = None;
            for expanded in [false, true] {
                let mut package = original.clone();
                if expanded {
                    let report =
                        elaboration::program::inlining::expand(&mut package, &mut 1000000).unwrap();
                    assert!(
                        report.selected_methods > 0 && report.expanded > 0,
                        "{report}"
                    );
                }
                let graph = luna::instantiation::instantiate(&package, "entry", 1000000).unwrap();
                let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
                assert_eq!(
                    vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                    Value::Bool(true)
                );
                let count = vm
                    .events()
                    .iter()
                    .filter(|event| matches!(event, Event::CheckPredicate { .. }))
                    .count();
                if let Some(expected) = expected {
                    assert_eq!(count, expected, "expanded={expanded}");
                } else {
                    expected = Some(count);
                }
            }
        }
    }
}

#[test]
fn symbol_indexed_default_trait_method_has_a_complete_instance_key() {
    let package = compile(
        "typealias Symbol=.Symbol'builtin;
        trait Choose where suffix:Symbol {fn flag(self)->bool;fn choose(self)->bool {self.flag()}}
        impl Choose<.px> for bool {fn flag(self)->bool {self}} impl Choose<.em> for bool {fn flag(self)->bool {not self}}
        fn run()->bool {true.as(Choose<.px>).choose() and false.as(Choose<.em>).choose()}",
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    graph.validate(&mut 100_000).unwrap();
    assert_eq!(
        graph
            .functions
            .iter()
            .filter(|f| f.function.name == "choose")
            .count(),
        2
    );
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let dump = graph.to_string();
    assert!(dump.contains("px") && dump.contains("em"), "{dump}");
}

#[test]
fn staged_inline_expansion_keeps_results_without_requiring_residual_entrypoints() {
    use comptime_interpretor::{Interpreter, Value};
    for source in [
        "inline fn identity(value:T)->T where T {value}
         inline fn select(comptime choose:bool,value:bool)->bool {if choose {value} else {not value}}
         fn entry()->bool {select(true,identity(true))}",
        "typealias Type=.Type'builtin;
         comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
         inline fn identity(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
         fn entry()->bool {identity(true,true)}",
    ] {
        let package=compile(source);
        assert!(package.functions.iter().filter(|f|f.name=="identity" || f.name=="select")
            .all(|f|f.inline && f.phase==nir::program::FunctionPhase::Runtime));
        let graph=luna::instantiation::instantiate(&package,"entry",100_000).unwrap();
        assert!(!graph.functions.iter().any(|f|f.function.name=="identity" || f.function.name=="select"));
        let mut vm=Interpreter::from_iir(&graph,Default::default()).unwrap();
        assert_eq!(vm.invoke_instance(&graph.roots[0],vec![]).unwrap(),Value::Bool(true));
    }
}
