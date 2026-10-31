//! Model fallback must solve call inputs without granting regional facts.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[test]
fn call_infers_a_type_from_nested_regional_equalities() {
    let package = support::compile(
        "fn accept(value: *T)->bool where T {true}
        fn forward(value: A)->bool where A, requires *A == **bool {
            accept(value)
        }",
    );
    let accept = package
        .functions
        .iter()
        .find(|f| f.name == "accept")
        .unwrap()
        .declaration;
    let forward = package
        .functions
        .iter()
        .find(|f| f.name == "forward")
        .unwrap();
    let mut found = false;
    iir::visit::expressions(forward, &mut 10_000, |expr, _| {
        if let nir::program::ExprKind::FunctionInstance {
            declaration,
            arguments,
        } = &expr.kind
        {
            if *declaration == accept {
                assert_eq!(arguments, &[nir::Term::Primitive(nir::Primitive::Bool)]);
                found = true;
            }
        }
        Ok(())
    })
    .unwrap();
    assert!(
        found,
        "the generic call must retain the inferred bool input"
    );
}

#[test]
fn call_rejects_a_model_that_only_satisfies_one_regional_alternative() {
    let result = support::compile_with_protocols(
        "fn accept(value: *T)->bool where T {true}
        fn forward(value: A)->bool where A, requires A == ?bool or A == *bool {
            accept(value)
        }",
        &Default::default(),
    );
    let error = result.expect_err("a conditional model is not a valid generic call");
    assert!(
        error.contains("type") || error.contains("inference"),
        "{error}"
    );
    assert!(!error.contains("parsing failed"), "{error}");
}
