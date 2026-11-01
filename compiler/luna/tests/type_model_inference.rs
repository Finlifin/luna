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

#[test]
fn nested_equalities_preserve_symbolic_call_inputs() {
    let package = support::compile(
        "fn accept(value: *T)->bool where T {true}
        fn forward(value: A)->bool where A, U, requires *A == **U {
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
    let expected = nir::Term::Rigid {
        declaration: forward.type_parameters[1],
        sort: nir::Sort::Type,
    };
    let mut found = false;
    iir::visit::expressions(forward, &mut 10000, |expr, _| {
        if let nir::program::ExprKind::FunctionInstance {
            declaration,
            arguments,
        } = &expr.kind
        {
            if *declaration == accept {
                assert_eq!(arguments, &[expected.clone()]);
                found = true;
            }
        }
        Ok(())
    })
    .unwrap();
    assert!(found);
}

#[test]
fn value_indexed_opaque_heads_preserve_inferred_type_arguments() {
    let package = support::compile(
        "typealias Type = .Type'builtin;
        pure comptime fn Tagged<T, flag: bool> -> Type { T }
        fn accept(value: *T)->bool where T {true}
        fn forward(value: A)->bool where A, U, requires *A == **Tagged<U,true> {
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
    let [tagged] = package.normal_forms.as_slice() else {
        panic!("this fixture declares exactly one opaque head")
    };
    let head = tagged.declaration;
    let expected = nir::Term::NormalForm {
        head,
        sort: nir::Sort::Type,
        arguments: vec![
            nir::Term::Rigid {
                declaration: forward.type_parameters[1],
                sort: nir::Sort::Type,
            },
            nir::Term::Bool(true),
        ],
    };
    let mut found = false;
    iir::visit::expressions(forward, &mut 10000, |expr, _| {
        if let nir::program::ExprKind::FunctionInstance {
            declaration,
            arguments,
        } = &expr.kind
        {
            if *declaration == accept {
                assert_eq!(arguments, &[expected.clone()]);
                found = true;
            }
        }
        Ok(())
    })
    .unwrap();
    assert!(found);
}

#[test]
fn excluded_alternative_determines_the_call_input() {
    let package = support::compile(
        "fn accept(value: *T)->bool where T {true}
        fn forward(value: A)->bool
        where A, requires (A == *bool or A == **bool) and A != **bool
        {
            accept(value)
        }",
    );
    assert_call_input(&package, nir::Term::Primitive(nir::Primitive::Bool));
}

#[test]
fn branch_local_equalities_agree_on_one_call_input() {
    let package = support::compile(
        "fn accept(value: *T)->bool where T {true}
        fn forward(value: A)->bool
        where A, B, C,
            requires (A == B and B == *bool) or (A == C and C == *bool)
        {
            accept(value)
        }",
    );
    assert_call_input(&package, nir::Term::Primitive(nir::Primitive::Bool));
}

#[test]
fn equally_shaped_alternatives_do_not_choose_an_arbitrary_input() {
    for alternatives in ["A == *bool or A == **bool", "A == **bool or A == *bool"] {
        let source = format!(
            "fn accept(value: *T)->bool where T {{true}}
            fn forward(value: A)->bool where A, requires {alternatives} {{
                accept(value)
            }}"
        );
        let error = support::compile_with_protocols(&source, &Default::default())
            .expect_err("both branches have pointer shape but disagree on T");
        assert!(
            error.contains("type") || error.contains("inference"),
            "{error}"
        );
        assert!(!error.contains("parsing failed"), "{error}");
    }
}

fn assert_call_input(package: &nir::program::Package, expected: nir::Term) {
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
                assert_eq!(arguments, &[expected.clone()]);
                found = true;
            }
        }
        Ok(())
    })
    .unwrap();
    assert!(found, "the generic call must retain its proven input");
}
