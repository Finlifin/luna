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
        interface: crate::program::FunctionDeclaration {
            source: SourceRange { start: 1, end: 20 },
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
            env_parameters: vec![],
            type_parameters: vec![],
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            result_type_binder: None,
            result_guarantees: vec![],
            entry_snapshots: vec![],
            requirements: vec![],
            capabilities: vec![],
        },
        body: flag(),
    }
}
fn run(package: &Package, fuel: &mut usize) -> Result<BTreeSet<crate::DeclarationId>, Error> {
    arithmetic_obligations_with_budgets(package, &mut 1_000_000, fuel)
        .map(|obligations| obligations.functions)
}

#[test]
fn package_and_detached_checks_share_only_declarations_and_one_allowance() {
    let package = Package {
        functions: vec![function(1, Phase::Comptime), function(2, Phase::Runtime)],
        ..Default::default()
    };
    let mut fuel = 10_000;
    let inspection = Inspection::new(&package);
    inspection.check_package(&mut fuel).unwrap();
    let cold = 10_000 - fuel;
    for allowance in 0..cold {
        assert_eq!(
            Inspection::new(&package)
                .check_package(&mut allowance.clone())
                .unwrap_err()
                .kind,
            ErrorKind::Budget,
        );
    }
    let mut exact = cold;
    Inspection::new(&package).check_package(&mut exact).unwrap();
    assert_eq!(exact, 0);
    fuel = 10_000;
    inspection.check_package(&mut fuel).unwrap();
    let warm = 10_000 - fuel;
    assert!(warm > 1 && warm < cold);
    assert_eq!(
        inspection.check_package(&mut (warm - 1)).unwrap_err().kind,
        ErrorKind::Budget
    );
    inspection.check_package(&mut warm.clone()).unwrap();
    fuel = 10_000;
    inspection.check(&flag(), &mut fuel).unwrap();
    let entry = 10_000 - fuel;
    assert!(entry < warm);
    assert_eq!(
        Inspection::new(&package)
            .check(&flag(), &mut entry.clone())
            .unwrap_err()
            .kind,
        ErrorKind::Budget,
    );

    let mut invalid = package;
    invalid.functions[1].body = Expr {
        kind: K::ComptimeValue(Term::Symbol("wrong domain".into())),
        ..flag()
    };
    let inspection = Inspection::new(&invalid);
    // A valid detached body does not certify any declaration body.
    inspection.check(&flag(), &mut 10_000).unwrap();
    for _ in 0..2 {
        assert_eq!(
            inspection.check_package(&mut 10_000).unwrap_err().kind,
            ErrorKind::Invalid
        );
        inspection.check(&flag(), &mut entry.clone()).unwrap();
    }
}

#[test]
fn detached_inspection_shares_fuel_and_preserves_failure_categories() {
    let package = Package {
        functions: vec![function(1, Phase::Comptime)],
        ..Default::default()
    };
    let body = Expr {
        kind: K::Block {
            statements: vec![flag(); 100],
            result: Some(Box::new(flag())),
        },
        ..flag()
    };
    let mut remaining = 10_000;
    validate_comptime_expression(&package, &body, &mut remaining).unwrap();
    let cost = 10_000 - remaining;
    let mut reserved_width = false;
    for allowance in 0..cost {
        let mut fuel = allowance;
        let failure = validate_comptime_expression(&package, &body, &mut fuel).unwrap_err();
        assert_eq!(
            failure.kind,
            ErrorKind::Budget,
            "allowance={allowance}: {failure:?}"
        );
        reserved_width |= fuel > 0;
    }
    assert!(
        reserved_width,
        "failed width reservations can leave unspent fuel"
    );
    validate_comptime_expression(&package, &body, &mut cost.clone()).unwrap();
    let invalid = Expr {
        kind: K::ComptimeValue(Term::Symbol("symbol".into())),
        ..flag()
    };
    assert_eq!(
        validate_comptime_expression(&package, &invalid, &mut 10_000)
            .unwrap_err()
            .kind,
        ErrorKind::Invalid
    );
    validate_comptime_expression(&package, &body, &mut 10_000).unwrap();
    assert_eq!(package.functions.len(), 1);
}

#[test]
fn opaque_metadata_consumes_the_phase_declaration_allowance() {
    let package = Package {
        normal_forms: vec![crate::normal_forms::Head {
            declaration: crate::DeclarationId {
                package: 1,
                local: 1,
            },
            role: crate::normal_forms::Role::Opaque,
            collector: None,
            parameters: vec![],
            requirements: vec![crate::program::TypePredicate::Bool(true); 40],
        }],
        ..Default::default()
    };
    let mut fuel = 1000;
    arithmetic_obligations_with_budgets(&package, &mut fuel, &mut 1000).unwrap();
    let cost = 1000 - fuel;
    let error =
        arithmetic_obligations_with_budgets(&package, &mut (cost - 1), &mut 1000).unwrap_err();
    assert_eq!(
        error.message,
        "normal-form metadata validation budget exhausted"
    );
    arithmetic_obligations_with_budgets(&package, &mut cost.clone(), &mut 1000).unwrap();
}

#[test]
fn callable_preparation_cannot_allocate_a_frame_without_inspection_fuel() {
    let package = Package {
        functions: vec![function(1, Phase::Comptime)],
        ..Default::default()
    };
    let error = run(&package, &mut 1).unwrap_err();
    assert_eq!(error.message, "function inspection frame budget exhausted");
    assert_eq!(error.source, package.functions[0].source);
    run(&package, &mut 1000).unwrap();
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
        assert_eq!(error.source, package.functions[0].source);
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
    // Explicit caller fuel replaces the default standalone cap; it is neither
    // split into fresh per-function allowances nor silently limited to it.
    Inspection::new(&package)
        .check_package(&mut 10_000_000)
        .unwrap();
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
    for _ in 0..30 {
        callable.body = Expr {
            source: SourceRange { start: 20, end: 21 },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Not(Box::new(callable.body)),
        };
    }
    callable.body.source = SourceRange { start: 1, end: 2 };
    let package = Package {
        functions: vec![callable],
        ..Default::default()
    };
    let mut fuel = 10;
    let error = run(&package, &mut fuel).unwrap_err();
    assert_eq!(error.message, "phase traversal budget exhausted");
    assert_eq!(error.source, SourceRange { start: 20, end: 21 });
    assert_eq!(fuel, 0);
    run(&package, &mut 1000).unwrap();
}

#[test]
fn a_definition_region_borrows_its_frame_without_publishing_the_owner() {
    let owner = function(7, Phase::Comptime);
    let mut region = crate::program::DefinitionRegion {
        owner: owner.declaration,
        phase: Phase::Comptime,
        signature: owner.signature.clone(),
        parameters: vec![],
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        body: Expr {
            kind: K::Call {
                callee: Box::new(Expr {
                    ty: owner.callable_type(),
                    kind: K::Function(owner.declaration),
                    ..flag()
                }),
                arguments: vec![],
                parameter_bindings: vec![],
                env_arguments: vec![],
            },
            ..flag()
        },
    };
    let failure = inspect_definition_region(&Package::default(), &region).unwrap_err();
    assert_eq!(failure.message, "function phase declaration is missing");
    let imports = BTreeMap::from([(owner.declaration, owner.interface.clone())]);
    let source = Package::default();
    let (owners, _) =
        inspect_definition_region_with_declarations(&source, &imports, &region).unwrap();
    assert!(owners.is_empty());
    assert!(
        crate::callable_source::Callables::new(&source, &mut 1000)
            .unwrap()
            .formation(&owner.declaration)
            .is_none()
    );
    let package = Package {
        functions: vec![owner],
        ..Default::default()
    };
    let duplicate =
        inspect_definition_region_with_declarations(&package, &imports, &region).unwrap_err();
    assert_eq!(
        duplicate.message,
        "invalid or duplicate region declaration identity"
    );
    inspect_definition_region(&package, &region).unwrap();
    region
        .parameters
        .push(crate::program::LocalId { owner: 7, index: 0 });
    let invalid_frame = inspect_definition_region(&package, &region).unwrap_err();
    assert_eq!(invalid_frame.kind, ErrorKind::Invalid);
    assert_eq!(invalid_frame.source, region.body.source);
}
