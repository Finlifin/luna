//! Source optional semantics without a library enum or protocol binding.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{calls, compile_with_protocols, id};

#[test]
fn optional_injection_keeps_single_evaluation_and_nested_absence() {
    let source="
        fn flag()->bool {true}
        fn empty()->?bool {null}
        fn wrap()->??bool {empty()}
        fn take(value:?bool)->bool {value match {flag?=>flag,null=>false}}
        fn run()->bool {take(flag()) and (wrap() match {inner?=>inner match {_?=>false,null=>true},null=>false})}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec!["run", "flag", "take", "wrap", "empty"]
    );
    for body in [
        "fn run()->?bool {'x'}",
        "fn run()->?.u8'builtin {256}",
        "fn run()->bool {let value:?bool=true;value}",
    ] {
        assert!(compile_with_protocols(&format!(" {body}"), &Default::default(),).is_err());
    }
}

#[test]
fn generic_optional_injection_preserves_scope_instances_and_call_count() {
    let source="
        fn flag()->bool {true}
        fn keep(value:?T)->?T where T {value}
        fn relay(value:T)->?T where T {keep(value)}
        fn empty()->?bool {null}
        fn nested(value:??T)->??T where T {value}
        fn run()->bool {
            let first=keep(flag());let second=relay(true);let outer=nested(empty());let same=keep(outer);
            (first match {value?=>value,null=>false}) and
            (second match {value?=>value,null=>false}) and
            (same match {inner?=>inner match {_?=>false,null=>true},null=>false})}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec![
            "run", "flag", "keep", "relay", "keep", "empty", "nested", "keep"
        ]
    );
    let instances = graph
        .functions
        .iter()
        .filter(|function| function.function.name == "keep")
        .collect::<Vec<_>>();
    assert_eq!(instances.len(), 2);
    assert!(
        instances
            .iter()
            .any(|function| function.key.arguments
                == vec![nir::Term::Primitive(nir::Primitive::Bool)])
    );
    assert!(instances.iter().any(
        |function| matches!(&function.key.arguments[..],[nir::Term::Optional(payload)]
        if **payload==nir::Term::Primitive(nir::Primitive::Bool))
    ));
}

#[test]
fn optional_propagation_extracts_one_layer_and_stops_later_arguments() {
    let prefix = "
        fn missing()->?bool {null}
        fn present()->?bool {true}
        fn later()->bool {false}
        fn combine(first:bool,second:bool)->bool {first and not second}
        fn failed()->?bool {combine(missing()?,later())}
        fn passed()->?bool {combine(present()?,later())}
        fn one(value:??bool)->??bool {let inner=value?;inner}
        fn run()->bool {
            (failed() match {_?=>false,null=>true}) and
            (passed() match {flag?=>flag,null=>false}) and
            (one(do {let inner:?bool=null;let outer:??bool=inner;outer}) match {inner?=>inner match {_?=>false,null=>true},null=>false}) and
            (one(null) match {_?=>false,null=>true})}";
    let package = compile_with_protocols(prefix, &Default::default()).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec![
            "run", "failed", "missing", "passed", "present", "later", "combine", "one", "one"
        ]
    );
    for definition in ["fn bad()->bool {missing()?}", "fn bad()->?bool {true?}"] {
        assert!(
            compile_with_protocols(&format!("{prefix} {definition}"), &Default::default(),)
                .is_err()
        );
    }
}

#[test]
fn optional_type_values_construct_dynamic_payloads_once_at_comptime() {
    let source = "typealias Type=.Type'builtin;

        comptime fn Payload()->Type {bool}
        comptime fn Wrap(value:Type)->Type {?value}
        pure comptime fn Opaque<T>->Type {?T}
        comptime fn OpaqueValue()->Type {Opaque<bool>}
        comptime fn Build()->Type {Wrap(Payload())}
        comptime fn Nested()->Type {Wrap(Wrap(Payload()))}
        comptime fn Early()->Type {?do {return bool;}}
        comptime fn VoidType()->Type {Wrap(.void'builtin)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    let wrap = |ty| nir::Term::Optional(Box::new(ty));
    let boolean = nir::Term::Primitive(nir::Primitive::Bool);
    for (name, expected, trace) in [
        (
            "Build",
            wrap(boolean.clone()),
            vec!["Build", "Payload", "Wrap"],
        ),
        (
            "Nested",
            wrap(wrap(boolean.clone())),
            vec!["Nested", "Payload", "Wrap", "Wrap"],
        ),
        ("Early", boolean, vec!["Early"]),
        (
            "VoidType",
            wrap(nir::Term::Primitive(nir::Primitive::Void)),
            vec!["VoidType", "Wrap"],
        ),
        (
            "OpaqueValue",
            nir::Term::NormalForm {
                head: id(&package, "Opaque"),
                sort: nir::Sort::Type,
                arguments: vec![nir::Term::Primitive(nir::Primitive::Bool)],
            },
            vec!["OpaqueValue"],
        ),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, name), vec![]).unwrap(),
            Value::Type(expected.clone())
        );
        assert_eq!(calls(&package, &vm), trace);
        assert_eq!(
            luna::invocation::invoke(&package, name, 100_000).unwrap(),
            Value::Type(expected)
        );
    }
    assert!(package.to_string().contains("optional-type-value"));
    let invalid = source.replace("?value", "?true");
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn partial_optional_branches_preserve_types_bindings_and_execution_order() {
    let prefix = "typealias Byte=.u8'builtin;

        fn choose(value:bool)->bool {value}
        fn touch()->bool {true}
        fn byte()->Byte {9}
        fn source(flag:bool)->?Byte {if flag {null} else {9}}
        fn keep(value:?T)->?T where T {value}";
    for (expression, missing, present) in [
        (
            "if choose(flag) {touch();null} else {byte()}",
            vec!["run", "choose", "touch", "keep"],
            vec!["run", "choose", "byte", "keep"],
        ),
        (
            "when {choose(flag)=>null,false=>9,else=>byte()}",
            vec!["run", "choose", "keep"],
            vec!["run", "choose", "byte", "keep"],
        ),
        (
            "source(flag) match {null=>null,value?=>value}",
            vec!["run", "source", "keep"],
            vec!["run", "source", "keep"],
        ),
    ] {
        for flag in ["true", "false"] {
            let source = format!(
                "{prefix} fn run()->bool {{let flag={flag};let result=keep({expression});
                result match {{value?=>not flag and value==9,null=>flag}}}}"
            );
            let package = compile_with_protocols(&source, &Default::default()).unwrap();
            let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
            let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(
                calls(&package, &vm),
                if flag == "true" {
                    missing.clone()
                } else {
                    present.clone()
                }
            );
            let keep = graph
                .functions
                .iter()
                .find(|function| function.function.name == "keep")
                .unwrap();
            assert_eq!(
                keep.key.arguments,
                vec![nir::Term::Primitive(nir::Primitive::U8)]
            );
        }
    }
    for expression in [
        "if true {null} else {null}",
        "true match {true=>null,false=>null}",
        "when {true=>null,else=>null}",
        "when {true=>null,false=>true,else=>byte()}",
        "source(true) match {value?=>null,null=>value}",
        "source(true) match {null=>null}",
        "if true {let private_value=9;null} else {private_value}",
    ] {
        let source = format!("{prefix} fn run()->bool {{let result=keep({expression});true}}");
        assert!(
            compile_with_protocols(&source, &Default::default()).is_err(),
            "accepted {expression}"
        );
    }
}

#[test]
fn optional_propagation_context_preserves_nested_payloads_and_loop_exits() {
    let source="typealias Byte=.u8'builtin;

        fn empty()->?T where T {null}
        fn identity(value:T)->T where T {value}
        fn later()->Byte {9}
        fn failed()->?Byte {let flag:bool=empty()?;later()}
        fn nested()->??Byte {let value:?Byte=empty()?;value}
        fn inner_missing()->??Byte {let value:?Byte=identity(do {let inner:?Byte=null;let outer:??Byte=inner;outer})?;value}
        fn loop_exit()->?Byte {while true {let flag:bool=empty()?;} later()}
        fn loop_success()->?Byte {var count:Byte=0;while count < 2 {
            let value:Byte=identity(do {let value:?Byte=1;value})?;assumes count < 2;assumes value == 1;count = count + value;} count}
        fn run()->bool {
            (failed() match {_?=>false,null=>true}) and
            (nested() match {_?=>false,null=>true}) and
            (inner_missing() match {inner?=>inner match {_?=>false,null=>true},null=>false}) and
            (loop_exit() match {_?=>false,null=>true}) and
            (loop_success() match {value?=>value==2,null=>false})}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec![
            "run",
            "failed",
            "empty",
            "nested",
            "empty",
            "inner_missing",
            "identity",
            "loop_exit",
            "empty",
            "loop_success",
            "identity",
            "identity"
        ]
    );
    let empty = graph
        .functions
        .iter()
        .filter(|function| function.function.name == "empty")
        .collect::<Vec<_>>();
    assert_eq!(empty.len(), 2);
    assert!(
        empty
            .iter()
            .any(|f| f.key.arguments == vec![nir::Term::Primitive(nir::Primitive::Bool)])
    );
    assert!(empty.iter().any(
        |f| matches!(&f.key.arguments[..],[nir::Term::Optional(payload)]
        if **payload==nir::Term::Primitive(nir::Primitive::U8))
    ));
    let comptime_source = source.replace("fn ", "comptime fn ");
    let comptime = compile_with_protocols(&comptime_source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&comptime, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "let value:bool=true?;9",
        "let value:Byte=9?;9",
        "let value:bool=identity(true)?;9",
        "let value=empty()?;9",
        "let source:?Byte=9;let value:bool=source?;9",
    ] {
        let invalid = format!("{source} fn bad()->?Byte {{{body}}}");
        assert!(
            compile_with_protocols(&invalid, &Default::default()).is_err(),
            "accepted {body}"
        );
    }
}

#[test]
fn inferred_callback_return_evidence_retry_preserves_captures_and_calls() {
    let source = "typealias Byte=.u8'builtin;

        fn byte()->Byte {9}
        fn touch()->bool {true}
        fn apply(action:%{base:Byte} fn(bool)->?T,flag:bool)->?T where T {action(flag)}
        fn run()->bool {let base=byte();
            let yes=apply(|flag| {touch();if flag {return base;} null},true);
            let no=apply(|flag| {touch();if flag {return base;} null},false);
            (yes match {value?=>value==9,null=>false}) and (no match {_?=>false,null=>true})}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec!["run", "byte", "apply", "touch", "apply", "touch"]
    );
    let apply = graph
        .functions
        .iter()
        .find(|function| function.function.name == "apply")
        .unwrap();
    assert_eq!(
        apply.key.arguments,
        vec![nir::Term::Primitive(nir::Primitive::U8)]
    );
    let comptime_source = source
        .replace("fn ", "comptime fn ")
        .replace("fn(", "comptime fn(");
    let comptime = compile_with_protocols(&comptime_source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&comptime, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let prefix = "
        fn apply(action:fn(bool)->?T)->?T where T {action(true)}";
    for body in [
        "if flag {return true;} 9",
        "let inner=apply(|other| {if other {return 9;} null});null",
        "if flag {return null;} null",
    ] {
        let source = format!("{prefix} fn run()->bool {{let value=apply(|flag| {{{body}}});true}}");
        assert!(
            compile_with_protocols(&source, &Default::default()).is_err(),
            "accepted {body}"
        );
    }
}

#[test]
fn native_optional_source_runs_through_llvm_without_enum_binding() {
    use std::os::unix::fs::PermissionsExt;
    let source = "fn empty()->?bool {null}
        fn wrap()->??bool {empty()}
        fn propagate(value:?bool)->?bool {value?}
        fn run()->bool {
            (wrap() match {inner?=>inner match {null=>true,_?=>false},null=>false}) and
            (propagate(null) match {null=>true,_?=>false}) and
            (propagate(true) match {value?=>value,null=>false})
        }";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    let ir = luna::host::llvm_ir(&package, "run", 100_000).unwrap();
    let directory =
        std::env::temp_dir().join(format!("luna-native-optional-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let executable = directory.join("run");
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let toolchain = luna::host::Toolchain {
            optimization,
            ..Default::default()
        };
        let binary = luna::host::executable(&ir, &toolchain).unwrap();
        std::fs::write(&executable, binary).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = std::process::Command::new(&executable).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"(bool true)\n");
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn optional_payload_contracts_keep_absence_and_numeric_proofs_separate() {
    let prefix = "typealias Byte=.u8'builtin;";
    let safe = format!(
        "{prefix}
        fn divide(value:?Byte)->Byte {{value match {{
            null=>0, n?=>if n != 0 {{10 / n}} else {{0}}
        }}}}
        fn run()->bool {{divide(null)==0 and divide(2)==5 and divide(0)==0}}"
    );
    let package = compile_with_protocols(&safe, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "value match {null=>0,n?=>10 / n}",
        "value match {null=>10 / 0,_?=>0}",
    ] {
        let invalid = format!("{prefix} fn bad(value:?Byte)->Byte {{{body}}}");
        assert!(
            compile_with_protocols(&invalid, &Default::default()).is_err(),
            "accepted {body}"
        );
    }
}

#[test]
fn repository_optional_methods_preserve_nested_absence_and_lazy_callbacks() {
    let source = format!(
        "{}\n
        fn absent(value:bool)->?bool {{null}}
        fn invert(value:bool)->bool {{not value}}
        fn forbidden()->bool {{forbidden()}}
        fn run()->bool {{
            let present:?bool=true;
            let missing:?bool=null;
            let mapped=present.as(OptionalOps<bool>).map(absent);
            let chained=present.as(OptionalOps<bool>).and_then(absent);
            let fallback=missing.as(OptionalOps<bool>).unwrap_or_else(|| true);
            present.as(OptionalOps<bool>).is_some() and missing.as(OptionalOps<bool>).is_none() and
            present.as(OptionalOps<bool>).unwrap_or(false) and missing.as(OptionalOps<bool>).unwrap_or(true) and
            present.as(OptionalOps<bool>).unwrap_or_else(forbidden) and fallback and
            not present.as(OptionalOps<bool>).map(invert).as(OptionalOps<bool>).unwrap_or(true) and
            missing.as(OptionalOps<bool>).map(invert).as(OptionalOps<bool>).is_none() and
            (mapped match {{inner?=>inner.as(OptionalOps<bool>).is_none(),null=>false}}) and
            chained.as(OptionalOps<bool>).is_none()
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        format!(
            "mod primitives {{\n{}\n}}\nmod qualified {{\n{}\n}} pub use qualified.*;",
            include_str!("../../../library/builtin/primitives.fl"),
            include_str!("../../../library/builtin/types/option.fl")
        )
    );
    let package = compile_with_protocols(
        &source,
        &elaboration::program::Protocols {
            builtin_implementation_packages: [0].into(),
            ..Default::default()
        },
    )
    .unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let trace = calls(&package, &vm);
    assert_eq!(
        trace
            .iter()
            .filter(|name| name.as_str() == "absent")
            .count(),
        2
    );
    assert_eq!(
        trace
            .iter()
            .filter(|name| name.as_str() == "invert")
            .count(),
        1
    );
    assert!(!trace.iter().any(|name| name == "forbidden"));
    assert!(package.enums.is_empty());
    let ir = luna::host::llvm_ir(&package, "verify", 200_000).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}
