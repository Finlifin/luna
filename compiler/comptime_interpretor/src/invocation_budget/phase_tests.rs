//! Real constructors must debit body inspection before admitting a source VM.
use super::*;
use crate::{CodeSession, ContractOutcome, DefinitionAdmissionSession, SourceResolvers};
use nir::{DeclarationId, Primitive, Term, program::*};

fn flag() -> Expr {
    Expr {
        source: SourceRange { start: 10, end: 20 },
        ty: Term::Primitive(Primitive::Bool),
        kind: ExprKind::Literal(Literal::Bool(true)),
    }
}

fn package(width: usize) -> Package {
    Package {
        functions: vec![Function {
            interface: nir::program::FunctionDeclaration {
                source: nir::program::SourceRange { start: 0, end: 0 },
                declaration: DeclarationId {
                    package: 9,
                    local: 1,
                },
                name: "entry".into(),
                phase: FunctionPhase::Comptime,
                inline: false,
                pure: false,
                is_unsafe: false,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(flag().ty),
                },
                parameters: vec![],
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
            body: Expr {
                kind: ExprKind::Block {
                    statements: vec![flag(); width],
                    result: Some(Box::new(flag())),
                },
                ..flag()
            },
        }],
        ..Default::default()
    }
}

fn unavailable<T>(_: &Expr, _: &mut u64) -> Result<T, Error> {
    Err(Error {
        kind: ErrorKind::UnmaterializedDispatch,
        source: None,
    })
}

#[test]
fn completed_phase_receipt_reaches_cold_sessions_without_granting_body_authority() {
    let source = std::sync::Arc::new(package(1000));
    let mut fuel = 100_000;
    let phase = nir::phases::Inspection::new(&source)
        .check_package_with_evidence(&mut fuel)
        .unwrap();
    let phase_cost = 100_000 - fuel;
    let copy = std::sync::Arc::new(source.as_ref().clone());
    assert!(CodeSession::with_phase_checked_source(copy.clone(), &phase).is_err());
    let seed = CodeSession::with_phase_checked_source(source.clone(), &phase).unwrap();
    let solver = |_: &TypePredicate, _: &mut u64| Ok(ContractOutcome::Unknown);
    for resolving in [false, true] {
        let mut session = seed.fork_for_source(source.clone());
        let mut budget = Budget {
            steps: phase_cost as u64 - 1,
            ..Default::default()
        };
        let mut vm = if resolving {
            Interpreter::for_comptime_resolving_in_session(
                &source,
                &mut session,
                SourceResolvers {
                    types: &solver,
                    methods: &unavailable,
                    associated: &unavailable,
                },
                &mut budget,
            )
        } else {
            Interpreter::for_comptime_in_session(&source, &mut session, &mut budget)
        }
        .unwrap();
        assert!(budget.steps > 0 && budget.steps < phase_cost as u64 - 1);
        let malformed = Expr {
            kind: ExprKind::ComptimeValue(Term::Symbol("wrong domain".into())),
            ..flag()
        };
        assert!(matches!(
            vm.invoke_comptime_expression(&malformed).unwrap_err().kind,
            ErrorKind::InvalidPhase(_)
        ));
        assert_eq!(
            vm.invoke_comptime_expression(&flag()).unwrap(),
            crate::Value::Bool(true)
        );
    }
    // New owners cannot inherit evidence, even when IDs and contents match.
    let mut changed = seed.fork_for_source(copy.clone());
    let mut short = Budget {
        steps: phase_cost as u64 - 1,
        ..Default::default()
    };
    assert_eq!(
        Interpreter::for_comptime_in_session(&copy, &mut changed, &mut short)
            .err()
            .unwrap()
            .kind,
        ErrorKind::StepsExhausted
    );
    Interpreter::for_comptime_in_session(&copy, &mut changed, &mut Budget::default()).unwrap();
}

#[test]
fn completed_phase_receipt_consumes_budget_and_requires_exact_source() {
    let source = package(0);
    let checked = nir::phases::Inspection::new(&source)
        .check_package_with_evidence(&mut 100_000)
        .unwrap();
    let mut empty = Budget {
        steps: 0,
        ..Default::default()
    };
    assert!(matches!(
        phase_inspection(&source, Some(&checked), &mut empty),
        Err(Error {
            kind: ErrorKind::StepsExhausted,
            ..
        })
    ));
    let mut exact = Budget {
        steps: 1,
        ..Default::default()
    };
    phase_inspection(&source, Some(&checked), &mut exact).unwrap();
    assert_eq!(exact.steps, 0);
    assert!(matches!(
        phase_inspection(&source.clone(), Some(&checked), &mut Budget::default()),
        Err(Error {
            kind: ErrorKind::InvalidCallPlan,
            ..
        })
    ));
}

fn construct(route: u8, source: &Package, budget: &mut Budget) -> Result<(), Error> {
    let solver = |_: &TypePredicate, _: &mut u64| Ok(ContractOutcome::Unknown);
    match route {
        0 => Interpreter::for_comptime_in_budget(source, &[], budget).map(|_| ()),
        1 => {
            Interpreter::for_comptime_in_scope_in_budget(source, &[], &[], &[], budget).map(|_| ())
        }
        2 => Interpreter::for_comptime_resolving_in_budget(
            source,
            &[],
            &[],
            &[],
            SourceResolvers {
                types: &solver,
                methods: &unavailable,
                associated: &unavailable,
            },
            budget,
        )
        .map(|_| ()),
        3 => DefinitionAdmissionSession::new_with_type_solver(source, &solver, budget).map(|_| ()),
        _ => unreachable!(),
    }
}

#[test]
fn source_constructors_reuse_phase_domains_without_reusing_body_authority() {
    let mut source = package(0);
    source.function_parameters = (0..256)
        .map(|index| nir::function_parameters::Declaration {
            declaration: DeclarationId {
                package: 12,
                local: index * 2,
            },
            parameters: vec![DeclarationId {
                package: 12,
                local: index * 2 + 1,
            }],
            domains: vec![Term::Primitive(Primitive::Type)],
        })
        .collect();
    let solver = |_: &TypePredicate, _: &mut u64| Ok(ContractOutcome::Unknown);
    for resolving in [false, true] {
        let mut budget = Budget::default();
        let mut vm = if resolving {
            Interpreter::for_comptime_resolving_in_budget(
                &source,
                &[],
                &[],
                &[],
                SourceResolvers {
                    types: &solver,
                    methods: &unavailable,
                    associated: &unavailable,
                },
                &mut budget,
            )
        } else {
            Interpreter::for_comptime_in_budget(&source, &[], &mut budget)
        }
        .unwrap();
        let domains = vm
            .phase_inspection
            .as_ref()
            .unwrap()
            .admitted_parameter_domains(&source)
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &domains,
            &vm.declaration_parameter_domains
        ));
        assert_eq!(
            vm.invoke(source.functions[0].declaration, vec![]).unwrap(),
            crate::Value::Bool(true)
        );
    }
    source.functions[0].body.kind = ExprKind::ComptimeValue(Term::Symbol("wrong domain".into()));
    for route in 0..4 {
        assert!(construct(route, &source, &mut Budget::default()).is_err());
    }
}

#[test]
fn all_source_admission_routes_charge_complete_phase_work_and_recover() {
    let source = package(40);
    let mut phase_budget = Budget::default();
    prepare_phase_inspection(&source, &mut phase_budget).unwrap();
    let phase_cost = Budget::default().steps - phase_budget.steps;
    for route in 0..4 {
        let mut generous = Budget::default();
        construct(route, &source, &mut generous).unwrap();
        let cost = Budget::default().steps - generous.steps;
        assert!(cost > phase_cost);
        for steps in 0..cost {
            let mut short = Budget {
                steps,
                ..Default::default()
            };
            assert_eq!(
                construct(route, &source, &mut short).unwrap_err().kind,
                ErrorKind::StepsExhausted,
                "route {route}, allowance {steps}"
            );
            assert_eq!(
                short.steps, 0,
                "failed constructors retain their reservation"
            );
        }
        let mut exact = Budget {
            steps: cost,
            ..Default::default()
        };
        construct(route, &source, &mut exact).unwrap();
        assert_eq!(exact.steps, 0);

        let mut small = Budget::default();
        construct(route, &package(0), &mut small).unwrap();
        assert!(
            small.steps > generous.steps,
            "body width is preparation work"
        );
        let mut invalid = source.clone();
        invalid.functions[0].body = Expr {
            kind: ExprKind::ComptimeValue(Term::Symbol("invalid domain".into())),
            ..flag()
        };
        assert!(matches!(
            construct(route, &invalid, &mut Budget::default())
                .unwrap_err()
                .kind,
            ErrorKind::InvalidPhase(_)
        ));
        construct(route, &source, &mut Budget::default()).unwrap();
    }
}

#[test]
fn failed_phase_preparation_cannot_publish_session_evidence() {
    let source = std::sync::Arc::new(package(40));
    let mut remaining = Budget::default();
    prepare_phase_inspection(&source, &mut remaining).unwrap();
    let phase_cost = Budget::default().steps - remaining.steps;
    let solver = |_: &TypePredicate, _: &mut u64| Ok(ContractOutcome::Unknown);
    for resolving in [false, true] {
        let mut session = CodeSession::new(source.clone());
        let mut cold = None;
        for steps in [phase_cost - 1, phase_cost - 1, 100_000, 100_000] {
            let mut budget = Budget {
                steps,
                ..Default::default()
            };
            let result = if resolving {
                Interpreter::for_comptime_resolving_in_session(
                    &source,
                    &mut session,
                    SourceResolvers {
                        types: &solver,
                        methods: &unavailable,
                        associated: &unavailable,
                    },
                    &mut budget,
                )
            } else {
                Interpreter::for_comptime_in_session(&source, &mut session, &mut budget)
            };
            if steps < phase_cost {
                assert_eq!(result.err().unwrap().kind, ErrorKind::StepsExhausted);
                assert_eq!(budget.steps, 0);
            } else {
                let mut vm = result.unwrap();
                let cost = steps - budget.steps;
                if let Some(previous) = cold {
                    assert!(cost < previous);
                }
                cold = Some(cost);
                assert_eq!(
                    vm.invoke(source.functions[0].declaration, vec![]).unwrap(),
                    crate::Value::Bool(true)
                );
            }
        }
    }
}

#[test]
fn resolving_view_excludes_physical_dispatch_without_weakening_closed_admission() {
    let clean = package(0);
    let mut polluted = clean.clone();
    let id = DeclarationId {
        package: 99,
        local: 99,
    };
    polluted.dispatch_instances.push(DispatchInstance {
        interface: id,
        interface_arguments: vec![Term::Tuple(vec![Term::Bool(false); 4096])],
        subject: Term::Primitive(Primitive::Bool),
        implementation: id,
        arguments: vec![],
        methods: vec![],
        associated: vec![],
        associated_initializers: vec![],
    });
    for source in [&clean, &polluted] {
        let solver = |_: &TypePredicate, _: &mut u64| Ok(ContractOutcome::Unknown);
        let mut budget = Budget::default();
        let mut vm = Interpreter::for_comptime_resolving_in_budget(
            source,
            &[],
            &[],
            &[],
            SourceResolvers {
                types: &solver,
                methods: &unavailable,
                associated: &unavailable,
            },
            &mut budget,
        )
        .unwrap();
        assert!(vm.method_bodies.is_empty());
        assert!(vm.implemented_interfaces.is_empty());
        assert!(vm.associated_values.is_empty());
        assert!(vm.associated_initializers.is_empty());
        assert_eq!(
            vm.invoke(source.functions[0].declaration, vec![]).unwrap(),
            crate::Value::Bool(true)
        );
        let mut reference = Budget::default();
        construct(2, &clean, &mut reference).unwrap();
        assert_eq!(
            budget.steps, reference.steps,
            "unused table has no admission work"
        );
        let mut admission_budget = Budget::default();
        construct(3, source, &mut admission_budget).unwrap();
        let mut reference = Budget::default();
        construct(3, &clean, &mut reference).unwrap();
        assert_eq!(admission_budget.steps, reference.steps);
    }
    let failure = Interpreter::for_comptime(&polluted, Budget::default())
        .err()
        .unwrap();
    assert_eq!(failure.kind, ErrorKind::InvalidCallPlan);
    polluted.functions[0].body.kind = ExprKind::ComptimeValue(Term::Symbol("wrong domain".into()));
    assert!(
        construct(2, &polluted, &mut Budget::default()).is_err(),
        "declaration phase checking remains mandatory"
    );
    assert!(construct(3, &polluted, &mut Budget::default()).is_err());
}

#[test]
fn source_definition_admission_demands_proof_and_recovers_without_publication() {
    use std::{cell::Cell, sync::Arc};
    let source = package(0);
    let obligation = TypePredicate::Trait(TraitRequirement {
        interface: DeclarationId {
            package: 99,
            local: 1,
        },
        subject: Term::Primitive(Primitive::Bool),
        interface_arguments: vec![],
    });
    let result = Cell::new(ContractOutcome::Rejected);
    let calls = Cell::new(0);
    let solver = |predicate: &TypePredicate, steps: &mut u64| {
        assert_eq!(predicate, &obligation);
        calls.set(calls.get() + 1);
        *steps = steps.checked_sub(1).unwrap();
        Ok(result.get())
    };
    let mut session =
        DefinitionAdmissionSession::new_with_type_solver(&source, &solver, &mut Budget::default())
            .unwrap();
    let definitions: Arc<[Struct]> = vec![Struct {
        declaration: DeclarationId {
            package: 99,
            local: 2,
        },
        name: "Checked".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![obligation.clone()],
        capabilities: vec![],
        fields: vec![("flag".into(), Term::Primitive(Primitive::Bool))],
    }]
    .into();
    for (outcome, expected) in [
        (ContractOutcome::Rejected, ErrorKind::TypeContractRejected),
        (ContractOutcome::Unknown, ErrorKind::TypeContractUnresolved),
    ] {
        result.set(outcome);
        assert_eq!(
            session
                .prepare(definitions.clone(), &mut Budget::default())
                .err()
                .unwrap()
                .kind,
            expected
        );
    }
    result.set(ContractOutcome::Satisfied);
    let ready = session
        .prepare(definitions, &mut Budget::default())
        .unwrap();
    session.commit_with(ready, || Ok::<_, ()>(())).unwrap();
    assert_eq!(calls.get(), 3);
}
