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
#[ignore = "local function telescopes still capture the enclosing unsupplied clause input"]
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
