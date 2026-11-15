//! Selected associated payloads are checked under one construction allowance.
use super::*;
use nir::{DeclarationId, Primitive, Term, program::*};

fn package(initializer: bool, width: usize, members: usize) -> Package {
    let id = |local| DeclarationId { package: 91, local };
    let bool_ty = Term::Primitive(Primitive::Bool);
    let type_ty = Term::Primitive(Primitive::Type);
    let source = SourceRange { start: 10, end: 20 };
    let value = Expr {
        source,
        ty: type_ty.clone(),
        kind: ExprKind::TypeValue(Term::Function {
            parameters: vec![
                nir::Parameter {
                    sort: nir::Sort::Type,
                    ty: bool_ty.clone()
                };
                width
            ],
            result: Box::new(bool_ty.clone()),
        }),
    };
    let mut contract = Trait {
        declaration: id(1),
        self_parameter: id(2),
        name: "Factory".into(),
        comptime_only: true,
        type_parameters: vec![],
        parameter_types: vec![],
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        parents: vec![],
        methods: vec![],
    };
    let mut implementation = Implementation {
        declaration: id(3),
        phases: ImplementationPhases {
            runtime: false,
            comptime: true,
        },
        type_parameters: vec![],
        parameter_types: vec![],
        subject: bool_ty.clone(),
        interface: Some(id(1)),
        interface_arguments: vec![],
        capabilities: vec![],
        requirements: vec![],
        methods: vec![],
        associated: vec![],
        associated_initializers: vec![],
    };
    for local in 10..10 + members as u32 {
        contract.associated.push(AssociatedContract {
            declaration: id(local),
            name: format!("Generated{local}"),
            phase: TraitPhase::Comptime,
            ty: type_ty.clone(),
            default: None,
        });
        if initializer {
            let body = Expr {
                source,
                ty: type_ty.clone(),
                kind: ExprKind::Block {
                    statements: vec![
                        Expr {
                            source,
                            ty: bool_ty.clone(),
                            kind: ExprKind::Literal(Literal::Bool(true)),
                        };
                        width
                    ],
                    result: Some(Box::new(value.clone())),
                },
            };
            implementation.associated_initializers.push((
                id(local),
                nir::associated_initializers::AssociatedInitializer::new(body, &mut 100_000)
                    .unwrap(),
            ));
        } else {
            implementation.associated.push((id(local), value.clone()));
        }
    }
    let dispatch = DispatchInstance {
        interface: id(1),
        interface_arguments: vec![],
        subject: bool_ty,
        implementation: id(3),
        arguments: vec![],
        methods: vec![],
        associated: implementation.associated.clone(),
        associated_initializers: implementation.associated_initializers.clone(),
    };
    Package {
        traits: vec![contract],
        implementations: vec![implementation],
        dispatch_instances: vec![dispatch],
        ..Default::default()
    }
}

#[test]
fn associated_preparation_debits_all_members_and_preserves_budget_failures() {
    for initializer in [false, true] {
        let mut costs = Vec::new();
        for (width, members) in [(0, 1), (24, 1), (24, 2)] {
            let source = package(initializer, width, members);
            let before = source.to_string();
            let vm = Interpreter::new(&source, Budget::default()).unwrap();
            let cost = Budget::default().steps - vm.remaining_steps();
            assert!(
                vm.events().is_empty(),
                "preparation cannot execute initializers"
            );
            costs.push(cost);
            for steps in 0..cost {
                let failure = Interpreter::new(
                    &source,
                    Budget {
                        steps,
                        ..Budget::default()
                    },
                )
                .err()
                .expect("preparation must stay within the shared allowance");
                assert_eq!(
                    failure.kind,
                    ErrorKind::StepsExhausted,
                    "initializer={initializer}, width={width}, members={members}, steps={steps}"
                );
            }
            let recovered = Interpreter::new(
                &source,
                Budget {
                    steps: cost,
                    ..Budget::default()
                },
            )
            .unwrap();
            assert_eq!(recovered.remaining_steps(), 0);
            assert!(recovered.events().is_empty());
            assert!(
                Interpreter::for_comptime(&source, Budget::default())
                    .unwrap()
                    .events()
                    .is_empty()
            );
            assert_eq!(source.to_string(), before);
        }
        assert!(costs[1] > costs[0], "payload width must be charged");
        assert!(costs[2] > costs[1], "each member must be charged");
    }
}

#[test]
fn associated_preparation_still_rejects_payloads_from_another_selection() {
    for initializer in [false, true] {
        let source = package(initializer, 24, 2);
        let mut changed = source.clone();
        let different = package(initializer, 0, 2);
        changed.dispatch_instances[0] = different.dispatch_instances[0].clone();
        assert_eq!(
            Interpreter::new(&changed, Budget::default())
                .err()
                .unwrap()
                .kind,
            ErrorKind::InvalidCallPlan
        );
        assert!(Interpreter::new(&source, Budget::default()).is_ok());
    }
}

#[test]
fn source_and_closed_sessions_do_not_share_selected_associated_evidence() {
    use crate::{CodeSession, ContractOutcome, SourceResolvers, Value};
    use std::{cell::Cell, sync::Arc};
    let source = Arc::new(package(false, 0, 1));
    let mut code = CodeSession::new(source.clone());
    let requests = Cell::new(0);
    let solver = |_: &TypePredicate, _: &mut u64| ContractOutcome::Unknown;
    let methods = |expr: &Expr, _: &mut u64| {
        Err(crate::Error {
            kind: ErrorKind::UnmaterializedDispatch,
            source: Some(expr.source),
        })
    };
    let associated = |expr: &Expr, _: &mut u64| {
        requests.set(requests.get() + 1);
        Err(crate::Error {
            kind: ErrorKind::TypeContractRejected,
            source: Some(expr.source),
        })
    };
    let access = Expr {
        source: SourceRange { start: 10, end: 20 },
        ty: Term::Primitive(Primitive::Type),
        kind: ExprKind::AssociatedValue {
            interface: source.dispatch_instances[0].interface_instance(),
            member: source.traits[0].associated[0].declaration,
            subject: Term::Primitive(Primitive::Bool),
        },
    };
    for resolving in [false, true, false, true] {
        let mut budget = Budget::default();
        let mut vm = if resolving {
            Interpreter::for_comptime_resolving_in_session(
                &source,
                &mut code,
                SourceResolvers {
                    types: &solver,
                    methods: &methods,
                    associated: &associated,
                },
                &mut budget,
            )
        } else {
            Interpreter::for_comptime_in_session(&source, &mut code, &mut budget)
        }
        .unwrap();
        assert_eq!(vm.associated_values.is_empty(), resolving);
        let result = vm.with_code_session(&mut code, |vm| vm.invoke_comptime_expression(&access));
        if resolving {
            assert_eq!(result.unwrap_err().kind, ErrorKind::TypeContractRejected);
        } else {
            assert!(matches!(result.unwrap(), Value::Type(_)));
        }
    }
    assert_eq!(
        requests.get(),
        2,
        "source demands must reach their own services"
    );
}
