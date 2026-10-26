//! Local handlers use conditional closure input footprints, checked at each call.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

const PREFIX: &str = "trait Duplicate {} trait Retire {fn drop(*self);}
    impl Duplicate for bool {} impl Duplicate for *T where T {}
    struct Link {target:*bool} impl Duplicate for Link {}
    effect touch()->bool;
    fn twice()->#touch bool {let first=touch()#;let second=touch()#;first and second}";

fn check(source: &str) {
    let package = support::compile(&format!(
        "{PREFIX} {source}
        fn verify() {{let passed=run();assumes passed;}}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1_000_000,
        )
        .unwrap(),
        Value::Bool(true)
    );
    let copy = package
        .traits
        .iter()
        .find(|item| item.name == "Duplicate")
        .unwrap()
        .declaration;
    let drop = package
        .traits
        .iter()
        .find(|item| item.name == "Retire")
        .unwrap()
        .declaration;
    let prepared =
        luna::host::prepare_with_protocols(&package, "verify", 1_000_000, copy, drop).unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 1_000_000).unwrap();
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

#[test]
fn closure_installs_handler_over_captured_scalar_storage() {
    for (capture, pointer) in [
        ("let captured=flag.ref;", "captured"),
        ("let captured=Link{target:flag.ref};", "captured.target"),
    ] {
        check(&format!(
            "fn run()->bool {{var flag=false;{capture}
            let callback=|| {{twice()# {{touch()=>do {{{pointer}.* = true;{pointer}.*}},}}}};
            let first=callback();flag=false;let second=callback();
            first and second and flag
        }}"
        ));
    }
}

#[test]
fn closure_installs_handler_over_pointer_argument() {
    check(
        "fn run()->bool {var flag=false;
        let callback=|target:*bool| {twice()# {touch()=>do {target.* = true;target.*},}};
        let result=callback(flag.ref);result and flag
    }",
    );
}

#[test]
fn capture_rebinding_between_installations_uses_the_new_target() {
    check(
        "fn run()->bool {var left=false;var right=false;var target=left.ref;let other=right.ref;
        let callback=|| {
            let result=twice()# {touch()=>do {target.* = true;target.*},};
            target=other;result
        };
        let first=callback();let separated=left and not right;
        let second=callback();first and second and separated and right
    }",
    );
}

#[test]
fn conditional_input_premises_do_not_make_dead_captures_live() {
    for capture in ["local.ref", "Link{target:local.ref}"] {
        let pointer = if capture == "local.ref" {
            "captured"
        } else {
            "captured.target"
        };
        let package = support::compile(&format!(
            "{PREFIX}
            fn run()->bool {{let callback=do {{var local=false;let captured={capture};
                || {{twice()# {{touch()=>do {{{pointer}.* = true;{pointer}.*}},}}}}
            }};callback()}}"
        ));
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1_000_000,
        )
        .unwrap_err();
        assert!(
            error.contains("unavailable")
                || error.contains("escaping provenance")
                || error.contains("live"),
            "{error}"
        );
    }
}

#[test]
fn handler_access_still_requires_the_affine_pointee_owner() {
    let source = "struct Owner {value:bool} fn consume(value:Owner) {}
        fn run()->bool {let owner=Owner{value:false};let target=owner.value.ref;
            let callback=|| {twice()# {touch()=>do {target.* = true;target.*},}};
            let result=callback();result and owner.value
        }";
    check(source);
    let package = support::compile(&format!(
        "{PREFIX} {}",
        source
            .replace(
                "let result=callback();",
                "consume(owner);let result=callback();"
            )
            .replace("result and owner.value", "result")
    ));
    let error = luna::execution::interpret_with_protocols(
        &package,
        "run",
        "Duplicate",
        "Retire",
        1_000_000,
    )
    .unwrap_err();
    assert!(
        error.contains("unavailable") || error.contains("live"),
        "{error}"
    );
}
