//! The default Once contribution transfers ownership without granting copies.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

fn compile(body: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod resources {{ {} }} use resources.*; {body}",
            include_str!("../../../library/builtin/protocols/resources.fl")
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.resources.Copy"),
            drop: Some("input.resources.Drop"),
            ..Default::default()
        },
    )
}

#[test]
fn blanket_once_transfers_user_defined_values_without_explicit_markers() {
    let package = compile(
        "struct Token {flag:bool}
        fn forward(value:T)->T where T:-Once {value}
        fn run()->bool {forward(Token {flag:true}).flag}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn blanket_once_does_not_grant_copy_or_clone() {
    for bound in ["Clone", "Copy"] {
        assert!(
            compile(&format!(
                "struct Token {{flag:bool}}
            fn require(value:T) where T:-{bound} {{}}
            fn bad() {{require(Token {{flag:true}});}}"
            ))
            .is_err()
        );
    }
    let package = compile(
        "struct Token {flag:bool}
        fn forward(value:T)->T where T:-Once {value}
        fn run()->bool {let token=Token {flag:true};
            let moved=forward(token);token.flag}",
    )
    .unwrap();
    let error = luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100_000)
        .unwrap_err();
    assert!(error.contains("unavailable value"), "{error}");
}
