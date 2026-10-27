//! Explicit iteration identities and the push-effect contract used by consumers.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

fn check(source: &str, selector: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings_for_target(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            iterator: Some(selector),
            ..Default::default()
        },
        Some(luna::frontend::native_test_target().unwrap()),
    )
}

const VALID: &str = "
    typealias Unit = .void'builtin;
    inline effect deliver(value: T) where T;
    trait Stream {
        assoc Item: (.Type'builtin);
        inline fn iterate(self) -> #deliver<Item> Unit;
    }
";

#[test]
fn cli_accepts_one_explicit_iterator_binding() {
    let parse = |args: &[&str]| luna::cli::parse(args.iter().map(std::ffi::OsString::from));
    let luna::cli::Command::Compile(options) = parse(&[
        "input.fl",
        "--emit",
        "nir",
        "--iterator-protocol",
        "input.Stream",
    ])
    .unwrap() else {
        panic!("expected compilation");
    };
    assert_eq!(options.iterator_protocol.as_deref(), Some("input.Stream"));
    for args in [
        vec!["input.fl", "--iterator-protocol"],
        vec![
            "input.fl",
            "--iterator-protocol",
            "input.Stream",
            "--iterator-protocol",
            "input.Other",
        ],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
}

#[test]
fn repository_iterator_is_a_valid_push_protocol() {
    check(
        include_str!("../../../library/builtin/iterator.fl"),
        "input.Iterator",
    )
    .unwrap();
}

#[test]
fn binding_uses_selected_identity_and_declared_operation() {
    check(VALID, "input.Stream").unwrap();
    assert!(check(VALID, "input.Iterator").is_err());
    let source = format!("{VALID} trait Iterator {{}}");
    check(&source, "input.Stream").unwrap();
    let error = check(&source, "input.Iterator").unwrap_err();
    assert!(error.contains("associated Item"), "{error}");
}

#[test]
fn malformed_push_contracts_are_rejected() {
    for (from, to, expected) in [
        ("inline effect", "effect", "direct inline effect"),
        ("value: T", "value: bool", "one Item argument"),
        ("value: T", "value: T, extra: bool", "one Item argument"),
        (
            "iterate(self)",
            "iterate(*self)",
            "runtime fn iterate(self)",
        ),
        (
            "#deliver<Item> Unit",
            ".void'builtin",
            "element-delivery computation",
        ),
        (
            "#deliver<Item> Unit",
            "#deliver<Item> bool",
            "must return void",
        ),
    ] {
        let source = VALID.replace(from, to);
        let error = check(&source, "input.Stream").unwrap_err();
        assert!(error.contains(expected), "{from} -> {to}: {error}");
    }
}
