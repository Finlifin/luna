//! Inherited template code gives block-local declarations fresh instance identities.
use comptime_interpretor::Value;
use luna::{
    cli::{self, Command},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn compile(source: &str) -> Result<nir::program::Package, String> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "luna-template-locals-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let Command::Check(options) = cli::parse([
        "check".into(),
        path.clone().into_os_string(),
        "--sysroot".into(),
        library.into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("check command")
    };
    let result = frontend::compile(&options);
    std::fs::remove_file(path).unwrap();
    match result {
        Ok(Artifact::Nir(package)) => Ok(package),
        Ok(_) => panic!("NIR artifact"),
        Err(error) => Err(error.to_string()),
    }
}

fn run(source: &str) -> Value {
    let package = compile(source).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    vm.invoke_instance(&graph.roots[0], vec![]).unwrap()
}

#[test]
fn signature_defaults_can_define_lexical_functions() {
    let source = "
        template fn apply(value: bool, .action: fn(bool) -> bool = do {
            fn invert(input: bool) -> bool { not input }
            invert
        }) -> bool;
        fn first use apply { action(value) }
        fn second use apply { action(value) }
        fn run() -> bool { first(false) and not second(true) }
    ";
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn shared_bodies_can_define_lexical_helpers() {
    let source = "
        template fn with_helper(value: bool, quote lambda body: <current: bool |- bool>)
            -> bool
        {
            fn invert(input: bool) -> bool { not input }
            let current = invert(value);
            ~<body>
        }
        fn first use with_helper { current }
        fn second use with_helper { not current }
        fn run() -> bool { first(false) and second(true) }
    ";
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn local_functions_and_aliases_follow_generic_instance_inputs() {
    let source = "
        template fn with_helper(value: T, quote lambda body: <current: T |- T>)
            -> T where T
        {
            typealias Local = T;
            fn keep(input: Local) -> Local { input }
            let current = keep(value);
            ~<body>
        }
        fn flag use with_helper<bool> { current }
        fn number use with_helper<u8> { current }
        fn generic use with_helper { current }
        fn run() -> bool { flag(true) and number(7) == 7 and generic<bool>(true) }
    ";
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn local_generic_functions_keep_distinct_clause_and_staged_identities() {
    let source = "
        template fn choose(value: bool, .action: fn(bool) -> bool = do {
            fn generic(input: T) -> T where T { input }
            fn staged(comptime T: Type, input: T) -> T { input }
            fn apply(input: bool) -> bool { staged(bool, generic<bool>(input)) }
            apply
        }) -> bool;
        fn first use choose { action(value) }
        fn second use choose { action(value) }
        fn run() -> bool { first(true) and not second(false) }
    ";
    let package = compile(source).unwrap();
    let ids: std::collections::BTreeSet<_> = package
        .functions
        .iter()
        .filter(|function| function.name == "apply")
        .map(|function| function.declaration)
        .collect();
    assert_eq!(
        ids.len(),
        3,
        "source declaration and two fresh template copies"
    );
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn nested_helpers_and_forward_references_stay_in_their_instance() {
    let source = "
        template fn choose(value: bool, .action: fn(bool) -> bool = do {
            fn first(input: bool) -> bool { second(input) }
            fn second(input: bool) -> bool {
                fn inner(flag: bool) -> bool { not flag }
                inner(input)
            }
            first
        }) -> bool;
        fn a use choose { action(value) }
        fn b use choose { action(value) }
        fn run() -> bool { a(false) and not b(true) }
    ";
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn default_local_declarations_do_not_leak_or_gain_runtime_captures() {
    for source in [
        "template fn choose(.action:fn(bool)->bool=do {
            fn hidden(value:bool)->bool {value} hidden
        })->bool;
        fn invalid use choose {hidden(true)}",
        "template fn choose(value:bool,.action:fn()->bool=do {
            fn capture()->bool {value} capture
        })->bool;
        fn invalid use choose {action()}",
    ] {
        let failure = compile(source).unwrap_err();
        assert!(!failure.contains("parsing failed"), "{failure}");
        assert!(
            failure.contains("Unresolved identifier")
                || failure.contains("capture")
                || failure.contains("runtime")
                || failure.contains("local binding is unavailable in this scope"),
            "{failure}"
        );
    }
}

#[test]
fn clause_defaults_rebind_their_local_definitions() {
    let source = "
        template fn choose(value: bool) -> bool
        where .inverted: bool = do {
            typealias Flag = bool;
            let flag: Flag = true;
            flag
        };
        fn first use choose<> { value != inverted }
        fn second use choose<.inverted = false> { value != inverted }
        fn generic use choose { value != inverted }
        fn run() -> bool { first(false) and second(true) and generic<>(false) }
    ";
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn local_comptime_calls_in_clause_defaults_need_precise_lexical_dependencies() {
    let source = "
        template fn choose(value: bool) -> bool
        where .inverted: bool = do {
            pure comptime fn flag() -> bool { true }
            flag()
        };
        fn first use choose<> { value != inverted }
        fn run() -> bool { first(false) }
    ";
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn recursive_helpers_keep_staged_capture_domains_and_instance_values() {
    let source = "
        fn outer(comptime T: Type, comptime seed: T) -> T {
            fn middle() -> T {
                fn inner(done: bool) -> T {
                    if done { seed } else { inner(true) }
                }
                inner(false)
            }
            middle()
        }
        fn run() -> bool {
            outer(bool, true) and not outer(bool, false) and outer(u8, 7) == 7
        }
    ";
    assert_eq!(run(source), Value::Bool(true));
    let invalid = source.replace("outer(u8, 7)", "outer(u8, true)");
    let failure = compile(&invalid).unwrap_err();
    assert!(!failure.contains("parsing failed"), "{failure}");
    assert!(failure.contains("type mismatch"), "{failure}");
}

#[test]
fn local_nominal_definitions_receive_distinct_instance_identities() {
    let source = "
        template fn wrap(value: bool, quote lambda body: <current: bool |- bool>) -> bool {
            newtype Local = bool;
            let wrapped = value.as(Local);
            let current = wrapped.as(bool);
            ~<body>
        }
        fn first use wrap { current }
        fn second use wrap { not current }
        fn run() -> bool { first(true) and second(false) }
    ";
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn copied_default_helpers_keep_public_library_method_resolution() {
    assert_eq!(
        run(include_str!(
            "../../../development/examples/template-local-definitions.fl"
        )),
        Value::Bool(true)
    );
}

fn assert_input_count(package: &nir::program::Package, name: &str, count: usize) {
    let functions: Vec<_> = package
        .functions
        .iter()
        .filter(|f| f.name == name)
        .collect();
    assert!(!functions.is_empty(), "missing function {name}");
    for function in functions {
        assert_eq!(function.type_parameters.len(), count, "inputs of {name}");
    }
}

#[test]
fn unused_lexical_inputs_are_removed_but_explicit_inputs_remain() {
    let source = "
        fn outer() -> bool where unused: bool {
            fn independent() -> bool { true }
            fn explicit() -> bool where unused: bool { true }
            independent() and explicit<false>()
        }
        fn run() -> bool { outer<true>() and outer<false>() }
    ";
    let package = compile(source).unwrap();
    assert_input_count(&package, "outer", 1);
    assert_input_count(&package, "independent", 0);
    assert_input_count(&package, "explicit", 1);
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn callee_binders_do_not_capture_an_ancestor_invocations_inputs() {
    let source = "
        fn outer() -> bool {
            fn parent(stop: bool) -> bool where value: bool {
                fn child() -> bool { parent<false>(true) }
                if stop { value } else { child() }
            }
            parent<true>(false)
        }
        fn run() -> bool { not outer() }
    ";
    let package = compile(source).unwrap();
    assert_input_count(&package, "parent", 1);
    assert_input_count(&package, "child", 0);
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn mutual_callees_propagate_outer_reads_through_the_whole_group() {
    let source = "
        fn outer(comptime seed: bool) -> bool {
            fn first(stop: bool) -> bool {
                if stop { seed } else { second(true) }
            }
            fn second(stop: bool) -> bool {
                if stop { first(true) } else { first(false) }
            }
            second(false)
        }
        fn run() -> bool { outer(true) and not outer(false) }
    ";
    let package = compile(source).unwrap();
    assert_input_count(&package, "first", 1);
    assert_input_count(&package, "second", 1);
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn domain_prerequisites_and_default_only_reads_remain_inputs() {
    let source = "
        fn outer(comptime T: Type, comptime seed: T) -> bool
        where flag: bool
        {
            fn read() -> bool { seed; true }
            fn defaulted(.value: bool = flag) -> bool { value }
            read() and defaulted()
        }
        fn run() -> bool {
            outer<true>(bool, false) and outer<true>(u8, 7)
                and not outer<false>(u8, 8)
        }
    ";
    let package = compile(source).unwrap();
    assert_input_count(&package, "read", 2);
    assert_input_count(&package, "defaulted", 1);
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
#[ignore = "copied defaults still freeze caller invocation slots instead of reifying reads"]
fn staged_default_reads_require_caller_value_reification() {
    let source = "
        fn outer(comptime T: Type, comptime seed: T, comptime flag: bool) -> bool {
            fn read() -> bool { seed; true }
            fn defaulted(.value: bool = flag) -> bool { value }
            read() and defaulted()
        }
        fn run() -> bool {
            outer(bool, false, true) and outer(u8, 7, true)
                and not outer(u8, 8, false)
        }
    ";
    let package = compile(source).unwrap();
    assert_input_count(&package, "read", 2);
    assert_input_count(&package, "defaulted", 1);
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn unused_descendants_keep_scope_without_forcing_parent_captures() {
    let source = "
        fn outer(comptime seed: bool) -> bool {
            fn parent() -> bool {
                fn unused() -> bool { seed }
                true
            }
            parent()
        }
        fn run() -> bool { outer(false) }
    ";
    let package = compile(source).unwrap();
    assert_input_count(&package, "parent", 0);
    assert_input_count(&package, "unused", 1);
    assert_eq!(run(source), Value::Bool(true));
}

#[test]
fn signature_domains_and_inherited_proof_inputs_survive_selection() {
    let source = "
        fn outer(value: T) -> bool
        where T, requires T == bool
        {
            fn domain_only(input: T) -> bool { true }
            fn proof_only() -> bool { true }
            domain_only(value) and proof_only()
        }
        fn run() -> bool { outer<bool>(true) }
    ";
    let package = compile(source).unwrap();
    assert_input_count(&package, "domain_only", 1);
    assert_input_count(&package, "proof_only", 1);
    assert_eq!(run(source), Value::Bool(true));
    let failure = compile(&source.replace("outer<bool>(true)", "outer<u8>(7)")).unwrap_err();
    assert!(!failure.contains("parsing failed"), "{failure}");
    assert!(
        failure.contains("requirement") || failure.contains("constraint"),
        "{failure}"
    );
}
