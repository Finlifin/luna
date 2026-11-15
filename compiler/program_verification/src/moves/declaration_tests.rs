//! Exercise unspecialized declaration consumers as well as residual bodies.
use super::*;
use nir::{DeclarationId, Primitive, Sort};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 17, local }
}
fn primitive(value: Primitive) -> Term {
    Term::Primitive(value)
}
fn signature(result: Term) -> Term {
    Term::Function {
        parameters: vec![],
        result: Box::new(result),
    }
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        source: SourceRange { start: 3, end: 7 },
        ty,
        kind,
    }
}
fn function(local: u32, body: Expr) -> Function {
    Function {
        interface: nir::program::FunctionDeclaration {
            declaration: id(local),
            name: format!("f{local}"),
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            type_parameters: vec![],
            parameters: vec![],
            signature: signature(body.ty.clone()),
            requirements: vec![],
            capabilities: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            defaults: vec![],
        },
        body,
    }
}
fn reference(local: u32, result: Term, arguments: Vec<Term>) -> Expr {
    expr(
        signature(result),
        K::FunctionInstance {
            declaration: id(local),
            arguments,
        },
    )
}
fn fixture() -> Package {
    let mut answer = function(
        1,
        expr(
            primitive(Primitive::Bool),
            K::Literal(nir::program::Literal::Bool(true)),
        ),
    );
    answer.type_parameters = vec![id(11), id(12)];
    let mut factory = function(
        2,
        reference(
            1,
            primitive(Primitive::Bool),
            vec![
                Term::Rigid {
                    declaration: id(21),
                    sort: Sort::Type,
                },
                Term::Rigid {
                    declaration: id(22),
                    sort: Sort::ComptimeValue,
                },
            ],
        ),
    );
    factory.type_parameters = vec![id(21), id(22)];
    let function_parameters = [&answer, &factory]
        .into_iter()
        .map(|function| nir::function_parameters::Declaration {
            declaration: function.declaration,
            parameters: function.type_parameters.clone(),
            domains: vec![
                primitive(Primitive::Type),
                Term::Rigid {
                    declaration: function.type_parameters[0],
                    sort: Sort::Type,
                },
            ],
        })
        .collect();
    Package {
        functions: vec![answer, factory],
        function_parameters,
        ..Default::default()
    }
}
fn type_arguments() -> Vec<Term> {
    vec![primitive(Primitive::Type), primitive(Primitive::U8)]
}

#[test]
fn callable_input_uses_the_completed_domain_and_rejects_mismatch() {
    let package = fixture();
    let valid = reference(1, primitive(Primitive::Bool), type_arguments());
    validate_callable_input(Some(&package), &valid, &mut 100_000, 0).unwrap();
    let invalid = reference(
        1,
        primitive(Primitive::Bool),
        vec![primitive(Primitive::Bool), primitive(Primitive::U8)],
    );
    assert!(validate_callable_input(Some(&package), &invalid, &mut 100_000, 0).is_err());
    assert!(validate_callable_input(Some(&package), &valid, &mut 1, 0).is_err());
    validate_callable_input(Some(&package), &valid, &mut 100_000, 0).unwrap();
}

#[test]
fn callable_memory_uses_the_completed_domain() {
    let package = fixture();
    let summary = infer_callable_memory_with_effects(
        &package,
        id(1),
        &type_arguments(),
        &[],
        &[],
        &mut 100_000,
        SourceRange { start: 3, end: 7 },
    )
    .unwrap();
    assert_eq!(summary.signature, signature(primitive(Primitive::Bool)));
    assert!(summary.accesses.is_empty());
    assert!(!summary.may_reject);
}

#[test]
fn returned_callable_retains_dependent_inputs() {
    let package = fixture();
    let callee = reference(2, signature(primitive(Primitive::Bool)), type_arguments());
    let root = function(
        3,
        expr(
            signature(primitive(Primitive::Bool)),
            K::Call {
                callee: Box::new(callee),
                arguments: vec![],
                parameter_bindings: vec![],
            },
        ),
    );
    let report = analyze_in_package(&package, &root, |_| Ok(Delivery::Copy), 100_000).unwrap();
    assert!(report.deferred_calls.is_empty());
    assert_eq!(report.callable_exits.len(), 1);
    assert_eq!(
        report.callable_exits[0],
        Some(reference(1, primitive(Primitive::Bool), type_arguments()))
    );
}

#[test]
fn callable_input_charges_expanded_signature_before_copying() {
    let mut definition = function(
        1,
        expr(
            primitive(Primitive::Bool),
            K::Literal(nir::program::Literal::Bool(true)),
        ),
    );
    definition.type_parameters = vec![id(11)];
    definition.parameters = (0..64).map(|index| LocalId { owner: 1, index }).collect();
    let parameter = |ty| nir::Parameter {
        sort: Sort::Runtime,
        ty,
    };
    definition.signature = Term::Function {
        parameters: vec![
            parameter(Term::Rigid {
                declaration: id(11),
                sort: Sort::Type
            });
            64
        ],
        result: Box::new(primitive(Primitive::Bool)),
    };
    let argument = Term::Tuple(vec![primitive(Primitive::Bool); 32]);
    let expected = Term::Function {
        parameters: vec![parameter(argument.clone()); 64],
        result: Box::new(primitive(Primitive::Bool)),
    };
    let callee = expr(
        expected,
        K::FunctionInstance {
            declaration: id(1),
            arguments: vec![argument],
        },
    );
    let package = Package {
        functions: vec![definition],
        ..Default::default()
    };
    let error = validate_callable_input(Some(&package), &callee, &mut 1000, 0)
        .expect_err("expanded signatures must fit the copy budget");
    assert!(error.reason.contains("budget"), "{error}");
    validate_callable_input(Some(&package), &callee, &mut 100_000, 0).unwrap();
}
