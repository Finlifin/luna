//! End-to-end phase inspection must consume one allowance across callables.
use super::*;
use crate::program::{Function, Literal};

fn flag() -> Expr {
    Expr {
        source: SourceRange { start: 10, end: 20 },
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Literal(Literal::Bool(true)),
    }
}
fn function(local: u32, phase: Phase) -> Function {
    Function {
        declaration: crate::DeclarationId { package: 1, local },
        name: format!("function{local}"),
        phase,
        inline: false,
        pure: false,
        is_unsafe: false,
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        },
        parameters: vec![],
        type_parameters: vec![],
        body: flag(),
        defaults: vec![],
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        requirements: vec![],
        capabilities: vec![],
    }
}
fn run(package: &Package, fuel: &mut usize) -> Result<BTreeSet<crate::DeclarationId>, Error> {
    machine_arithmetic_functions_with_budgets(package, &mut 1_000_000, fuel)
}

#[test]
fn storage_and_call_safety_consume_the_same_inspection_allowance() {
    for phase in [Phase::Runtime, Phase::Comptime] {
        let package = Package {
            functions: vec![function(1, phase)],
            ..Default::default()
        };
        let mut fuel = 1000;
        run(&package, &mut fuel).unwrap();
        let cost = 1000 - fuel;
        assert!(cost > 1);
        let mut exact = cost;
        run(&package, &mut exact).unwrap();
        assert_eq!(exact, 0);
        let error = run(&package, &mut (cost - 1)).unwrap_err();
        assert_eq!(error.message, "call safety validation budget exhausted");
        assert_eq!(error.source, package.functions[0].body.source);
        let mut twice = package.clone();
        twice.functions.push(function(2, phase));
        assert!(
            run(&twice, &mut cost.clone())
                .unwrap_err()
                .message
                .contains("budget exhausted")
        );
        run(&twice, &mut 1000).unwrap();
        // A new invocation cannot retain depleted fuel or partial query state.
        run(&package, &mut cost.clone()).unwrap();
    }
}

#[test]
fn captured_values_do_not_receive_a_fresh_allowance_per_expression() {
    let mut callable = function(1, Phase::Comptime);
    callable.body.kind = K::Block {
        statements: (0..64)
            .map(|_| Expr {
                source: SourceRange { start: 80, end: 90 },
                ty: Term::Primitive(Primitive::Symbol),
                kind: K::ComptimeValue(Term::Symbol("x".repeat(20_000).into())),
            })
            .collect(),
        result: Some(Box::new(flag())),
    };
    let mut package = Package {
        functions: vec![callable],
        ..Default::default()
    };
    let error = validate(&package).unwrap_err();
    assert_eq!(error.source, SourceRange { start: 80, end: 90 });
    assert_eq!(
        error.message,
        "captured comptime value validation budget exhausted"
    );
    let K::Block { statements, .. } = &mut package.functions[0].body.kind else {
        unreachable!()
    };
    statements.truncate(1);
    validate(&package).unwrap();
}

#[test]
fn contract_shape_uses_phase_fuel_instead_of_reporting_an_invalid_predicate() {
    let mut callable = function(1, Phase::Comptime);
    callable.preconditions = vec![Expr {
        kind: K::Block {
            statements: (0..10_001).map(|_| flag()).collect(),
            result: Some(Box::new(flag())),
        },
        ..flag()
    }];
    let package = Package {
        functions: vec![callable],
        ..Default::default()
    };
    let error = run(&package, &mut 20).unwrap_err();
    assert_eq!(
        error.message,
        "program predicate inspection budget exhausted"
    );
    assert_eq!(error.source, package.functions[0].preconditions[0].source);
    validate(&package).unwrap();
}

#[test]
fn broad_expression_and_pattern_work_share_phase_inspection_fuel() {
    let fields: Vec<_> = (0..40).map(|_| flag()).collect();
    let mut tuple = function(1, Phase::Comptime);
    tuple.body = Expr {
        source: SourceRange { start: 50, end: 60 },
        ty: Term::Tuple(vec![Term::Primitive(Primitive::Bool); fields.len()]),
        kind: K::Tuple(fields),
    };
    tuple.signature = Term::Function {
        parameters: vec![],
        result: Box::new(tuple.body.ty.clone()),
    };
    let mut matching = function(2, Phase::Comptime);
    matching.body.kind = K::Match {
        value: Box::new(flag()),
        arms: vec![crate::program::MatchArm {
            pattern: Pattern::Or(vec![Pattern::Wild; 40]),
            guard: None,
            body: flag(),
        }],
    };
    for function in [tuple, matching] {
        let source = function.body.source;
        let package = Package {
            functions: vec![function],
            ..Default::default()
        };
        let original = package.to_string();
        let mut fuel = 20;
        let error = run(&package, &mut fuel).unwrap_err();
        assert_eq!(error.message, "phase traversal budget exhausted");
        assert_eq!(error.source, source);
        assert!(fuel > 0 && fuel < 20);
        assert_eq!(package.to_string(), original);
        run(&package, &mut 10_000).unwrap();
    }
}

#[test]
fn narrow_expression_chains_cannot_bypass_worklist_charges() {
    let mut callable = function(1, Phase::Comptime);
    for index in 0..30 {
        callable.body = Expr {
            source: SourceRange {
                start: index,
                end: index + 1,
            },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Not(Box::new(callable.body)),
        };
    }
    let package = Package {
        functions: vec![callable],
        ..Default::default()
    };
    let mut fuel = 10;
    let error = run(&package, &mut fuel).unwrap_err();
    assert_eq!(error.message, "phase traversal budget exhausted");
    assert_eq!(error.source, SourceRange { start: 19, end: 20 });
    assert_eq!(fuel, 0);
    run(&package, &mut 1000).unwrap();
}
