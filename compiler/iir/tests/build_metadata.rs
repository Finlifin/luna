use iir::{FunctionInstance, InstanceKey, Package};
use nir::{
    DeclarationId, Primitive, Sort, Term,
    program::{Expr, ExprKind as K, Function, FunctionPhase, Literal, SourceRange},
};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 94, local }
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        source: SourceRange { start: 1, end: 2 },
        ty,
        kind,
    }
}
fn graph() -> Package {
    let boolean = Term::Primitive(Primitive::Bool);
    let key = InstanceKey {
        declaration: id(1),
        arguments: vec![].into(),
    };
    Package {
        roots: vec![key.clone()],
        functions: vec![FunctionInstance {
            key,
            function: Function {
                declaration: id(1),
                name: "read".into(),
                phase: FunctionPhase::Comptime,
                inline: false,
                pure: true,
                is_unsafe: false,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(boolean.clone()),
                },
                body: expr(
                    boolean,
                    K::BuildMetadata {
                        owner: 100,
                        selector: Box::new(expr(
                            Term::Primitive(Primitive::Str),
                            K::Literal(Literal::String("dependency".into())),
                        )),
                        entries: vec![
                            ("dependency".into(), 200, Some(Term::Bool(true))),
                            ("unavailable".into(), 300, None),
                        ],
                    },
                ),
                parameters: vec![],
                defaults: vec![],
                type_parameters: vec![],
                requirements: vec![],
                capabilities: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
            },
        }],
        ..Default::default()
    }
}
#[test]
fn metadata_snapshots_are_comptime_operations_with_linked_values() {
    let graph = graph();
    graph.validate(&mut 100_000).unwrap();
    let mut runtime = graph.clone();
    runtime.functions[0].function.phase = FunctionPhase::Runtime;
    let error = runtime.validate(&mut 100_000).unwrap_err();
    assert!(error.contains("metadata"), "{error}");
    for value in [
        Term::Nominal {
            declaration: id(99),
            arguments: vec![],
        },
        Term::Rigid {
            declaration: id(98),
            sort: Sort::Type,
        },
    ] {
        let mut invalid = graph.clone();
        let K::BuildMetadata { entries, .. } = &mut invalid.functions[0].function.body.kind else {
            panic!()
        };
        entries.push(("unselected".into(), 400, Some(value)));
        assert!(invalid.validate(&mut 100_000).is_err());
    }
    // Payload domain admission is selected at invocation, not eagerly here.
    let mut deferred = graph.clone();
    let K::BuildMetadata { entries, .. } = &mut deferred.functions[0].function.body.kind else {
        panic!()
    };
    entries.push((
        "another_domain".into(),
        500,
        Some(Term::Primitive(Primitive::Bool)),
    ));
    deferred.validate(&mut 100_000).unwrap();
}
#[test]
fn metadata_aliases_selector_and_validation_budget_are_checked() {
    let graph = graph();
    for alias in ["", "dependency"] {
        let mut invalid = graph.clone();
        let K::BuildMetadata { entries, .. } = &mut invalid.functions[0].function.body.kind else {
            panic!()
        };
        entries.push((alias.into(), 400, None));
        assert!(
            invalid
                .validate(&mut 100_000)
                .unwrap_err()
                .contains("aliases")
        );
    }
    let mut invalid = graph.clone();
    let K::BuildMetadata { selector, .. } = &mut invalid.functions[0].function.body.kind else {
        panic!()
    };
    **selector = expr(
        Term::Primitive(Primitive::Bool),
        K::Literal(Literal::Bool(true)),
    );
    assert!(
        invalid
            .validate(&mut 100_000)
            .unwrap_err()
            .contains("str selector")
    );
    let mut fuel = 100_000;
    graph.validate(&mut fuel).unwrap();
    for steps in 0..100_000 - fuel {
        let mut budget = steps;
        assert!(
            graph.validate(&mut budget).unwrap_err().contains("budget"),
            "{steps}"
        );
    }
    graph.validate(&mut 100_000).unwrap();
}
