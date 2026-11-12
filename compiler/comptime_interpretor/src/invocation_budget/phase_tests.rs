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
            body: Expr {
                kind: ExprKind::Block {
                    statements: vec![flag(); width],
                    result: Some(Box::new(flag())),
                },
                ..flag()
            },
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            requirements: vec![],
            capabilities: vec![],
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

fn construct(route: u8, source: &Package, budget: &mut Budget) -> Result<(), Error> {
    let solver = |_: &TypePredicate, _: &mut u64| ContractOutcome::Unknown;
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
    let solver = |_: &TypePredicate, _: &mut u64| ContractOutcome::Unknown;
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
    let solver = |_: &TypePredicate, _: &mut u64| ContractOutcome::Unknown;
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
