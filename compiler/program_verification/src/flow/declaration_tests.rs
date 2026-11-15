use super::*;
use nir::{DeclarationId, Primitive as P, Sort, program::Package};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 7, local }
}

fn value(kind: K, primitive: P) -> Expr {
    Expr {
        source: SourceRange { start: 10, end: 20 },
        ty: Term::Primitive(primitive),
        kind,
    }
}

fn function(local: u32, body: Expr) -> Function {
    Function {
        interface: nir::program::FunctionDeclaration {
            declaration: id(local),
            name: format!("function_{local}"),
            phase: nir::program::FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            type_parameters: vec![],
            parameters: vec![],
            defaults: vec![],
            requirements: vec![],
            capabilities: vec![],
            preconditions: vec![],
            postconditions: vec![],
            entry_snapshots: vec![],
            result_binding: None,
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(body.ty.clone()),
            },
        },
        body,
    }
}

fn package(calls: usize, padding: u32) -> Package {
    let mut target = function(0, value(K::Unit, P::Void));
    target.type_parameters = vec![id(50)];
    target.preconditions = vec![value(K::Literal(Literal::Bool(true)), P::Bool)];
    let call = value(
        K::Call {
            callee: Box::new(Expr {
                ty: target.signature.clone(),
                kind: K::FunctionInstance {
                    declaration: id(0),
                    arguments: vec![Term::Primitive(P::Bool)],
                },
                ..value(K::Unit, P::Void)
            }),
            arguments: vec![],
            parameter_bindings: vec![],
        },
        P::Void,
    );
    Package {
        functions: vec![
            target,
            function(
                1,
                value(
                    K::Block {
                        statements: vec![call; calls],
                        result: None,
                    },
                    P::Void,
                ),
            ),
        ],
        alias_contracts: (0..padding)
            .map(|offset| nir::type_aliases::Contract {
                declaration: id(1000 + offset),
                parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
            })
            .collect(),
        ..Default::default()
    }
}

fn unknown_budget() -> Reason {
    Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into())
}

#[test]
fn contract_directory_exhaustion_is_unknown_and_can_retry() {
    let package = package(1, 1024);
    let mut limits = Limits::default();
    limits.arithmetic.steps = 100;
    let failure = analyze_in_package(&package, &package.functions[1], &mut limits)
        .err()
        .unwrap();
    assert_eq!(failure.reason, unknown_budget());
    assert!(limits.arithmetic.steps < 100);
    limits.arithmetic.steps = 100_000;
    assert!(analyze_in_package(&package, &package.functions[1], &mut limits).is_ok());
}

#[test]
fn repeated_contract_calls_do_not_rescan_unrelated_declarations() {
    let cost = |package: &Package| {
        let mut limits = Limits::default();
        limits.arithmetic.steps = 100_000;
        analyze_in_package(package, &package.functions[1], &mut limits).unwrap();
        100_000 - limits.arithmetic.steps
    };
    let small = cost(&package(8, 0));
    let padded = cost(&package(8, 2048));
    assert!(
        padded > small,
        "directory preparation must still be charged"
    );
    assert!(
        padded - small < 4 * 2048,
        "eight calls repeatedly scanned the package: {small} -> {padded}"
    );
}

#[test]
fn captured_domains_preserve_budget_failure_and_world_identity() {
    let mut package = package(0, 1024);
    package.alias_contracts[0].parameters = vec![id(51)];
    package.alias_contracts[0].parameter_types = vec![Term::Primitive(P::Bool)];
    package.functions[1] = function(
        1,
        value(
            K::ComptimeValue(Term::Rigid {
                declaration: id(51),
                sort: Sort::ComptimeValue,
            }),
            P::Bool,
        ),
    );
    let mut limits = Limits::default();
    limits.arithmetic.steps = 100;
    let failure = analyze_in_package(&package, &package.functions[1], &mut limits)
        .err()
        .unwrap();
    assert_eq!(failure.reason, unknown_budget());
    limits.arithmetic.steps = 100_000;
    assert!(analyze_in_package(&package, &package.functions[1], &mut limits).is_ok());

    let mut other = package.clone();
    other.alias_contracts[0].parameter_types = vec![Term::Primitive(P::U8)];
    let failure = analyze_in_package(&other, &other.functions[1], &mut Limits::default())
        .err()
        .unwrap();
    assert_eq!(failure.reason, Reason::InvalidSignature);
    assert!(analyze_in_package(&package, &package.functions[1], &mut Limits::default()).is_ok());
}

#[test]
fn method_contract_domains_do_not_conflate_budget_with_invalid_inputs() {
    let mut package = package(0, 1024);
    let signature = package.functions[1].signature.clone();
    package.traits.push(nir::program::Trait {
        declaration: id(60),
        name: "Contract".into(),
        self_parameter: id(61),
        type_parameters: vec![id(62)],
        parameter_types: vec![Term::Primitive(P::Bool)],
        comptime_only: false,
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        parents: vec![],
        methods: vec![nir::program::MethodContract {
            declaration: id(63),
            name: "invoke".into(),
            phase: nir::program::TraitPhase::Runtime,
            is_quote: false,
            pure: true,
            is_unsafe: false,
            type_arguments: vec![],
            type_parameters: vec![],
            capabilities: vec![],
            program: nir::contracts::ProgramContract {
                preconditions: vec![value(
                    K::ComptimeValue(Term::Rigid {
                        declaration: id(62),
                        sort: Sort::ComptimeValue,
                    }),
                    P::Bool,
                )],
                ..Default::default()
            },
            parameter_bindings: vec![],
            by_value_receiver: false,
            by_reference_receiver: false,
            signature: signature.clone().into(),
            requirements: vec![],
            default: None,
        }],
    });
    let call = |flag| {
        value(
            K::Call {
                callee: Box::new(Expr {
                    kind: K::TraitMethod {
                        interface: id(60),
                        interface_arguments: vec![Term::Bool(flag)],
                        member: id(63),
                        method_arguments: vec![],
                        subject: Term::Primitive(P::Bool),
                    },
                    ty: signature.clone(),
                    ..value(K::Unit, P::Void)
                }),
                arguments: vec![],
                parameter_bindings: vec![],
            },
            P::Void,
        )
    };
    package.functions[1].body = call(true);
    let mut limits = Limits::default();
    limits.arithmetic.steps = 100;
    let failure = analyze_in_package(&package, &package.functions[1], &mut limits)
        .err()
        .unwrap();
    assert_eq!(failure.reason, unknown_budget());
    limits.arithmetic.steps = 100_000;
    assert!(analyze_in_package(&package, &package.functions[1], &mut limits).is_ok());
    package.functions[1].body = call(false);
    let failure = analyze_in_package(&package, &package.functions[1], &mut Limits::default())
        .err()
        .unwrap();
    assert_eq!(failure.reason, Reason::MissingPreconditionProof);
}

#[test]
fn contract_specialization_uses_the_completed_declaration_domains() {
    let mut package = package(1, 0);
    package.functions[0].type_parameters = vec![id(50), id(51)];
    package
        .function_parameters
        .push(nir::function_parameters::Declaration {
            declaration: id(0),
            parameters: vec![id(50), id(51)],
            domains: vec![
                Term::Primitive(P::Type),
                Term::Rigid {
                    declaration: id(50),
                    sort: Sort::Type,
                },
            ],
        });
    for (arguments, accepted) in [
        (vec![Term::Primitive(P::Type), Term::Primitive(P::U8)], true),
        (vec![Term::Primitive(P::Bool), Term::Bool(true)], true),
        (
            vec![Term::Primitive(P::Bool), Term::Primitive(P::U8)],
            false,
        ),
    ] {
        let K::Block { statements, .. } = &mut package.functions[1].body.kind else {
            unreachable!()
        };
        let K::Call { callee, .. } = &mut statements[0].kind else {
            unreachable!()
        };
        let K::FunctionInstance {
            arguments: actual, ..
        } = &mut callee.kind
        else {
            unreachable!()
        };
        *actual = arguments;
        let result = analyze_in_package(&package, &package.functions[1], &mut Limits::default());
        if accepted {
            assert!(
                result.is_ok(),
                "{}: {:?}",
                package.functions[1].body,
                result.err()
            );
        } else {
            assert_eq!(result.err().unwrap().reason, Reason::InvalidSignature);
        }
    }
}

#[test]
fn contract_specialization_keeps_the_callers_binder_scope() {
    let mut package = package(1, 0);
    package.functions[1].parameters = vec![LocalId { owner: 1, index: 0 }];
    package.functions[1].signature = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Type,
            ty: Term::Primitive(P::Type),
        }],
        result: Box::new(Term::Primitive(P::Void)),
    };
    for (depth, accepted) in [(0, true), (1, false)] {
        let K::Block { statements, .. } = &mut package.functions[1].body.kind else {
            unreachable!()
        };
        let K::Call { callee, .. } = &mut statements[0].kind else {
            unreachable!()
        };
        let K::FunctionInstance { arguments, .. } = &mut callee.kind else {
            unreachable!()
        };
        *arguments = vec![Term::Bound {
            depth,
            index: 0,
            sort: Sort::Type,
        }];
        let result = analyze_in_package(&package, &package.functions[1], &mut Limits::default());
        if accepted {
            assert!(result.is_ok(), "{:?}", result.err());
        } else {
            assert_eq!(result.err().unwrap().reason, Reason::InvalidSignature);
        }
    }
}

#[test]
fn contract_copy_charges_type_payloads_in_unexecuted_branches() {
    let mut package = package(1, 0);
    let payload = value(
        K::TypeValue(Term::Tuple(vec![Term::Primitive(P::Bool); 512])),
        P::Type,
    );
    let truth = value(K::Literal(Literal::Bool(true)), P::Bool);
    package.functions[0].preconditions = vec![value(
        K::If {
            condition: Box::new(truth.clone()),
            then_branch: Box::new(truth),
            else_branch: Some(Box::new(value(
                K::TypeEqual(Box::new(payload.clone()), Box::new(payload)),
                P::Bool,
            ))),
        },
        P::Bool,
    )];
    let mut limits = Limits::default();
    limits.arithmetic.steps = 256;
    let failure = analyze_in_package(&package, &package.functions[1], &mut limits)
        .err()
        .expect("copying the contract must charge hidden type payloads before choosing a branch");
    assert_eq!(failure.reason, unknown_budget());
    limits.arithmetic.steps = 100_000;
    assert!(analyze_in_package(&package, &package.functions[1], &mut limits).is_ok());
}
