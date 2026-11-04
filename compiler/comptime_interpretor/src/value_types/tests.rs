use super::*;
use std::cell::Cell;

const SOURCE: SourceRange = SourceRange { start: 3, end: 8 };

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 91, local }
}

fn fixture() -> (Package, Term, Value) {
    let boolean = Term::Primitive(Primitive::Bool);
    let package = Package {
        structs: vec![nir::program::Struct {
            declaration: id(1),
            name: "Packet".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            fields: vec![("value".into(), boolean.clone())],
            capabilities: vec![nir::program::TraitRequirement {
                interface: id(2),
                subject: boolean,
                interface_arguments: vec![],
            }],
            requirements: vec![],
        }],
        ..Default::default()
    };
    let ty = Term::Nominal {
        declaration: id(1),
        arguments: vec![],
    };
    let value = Value::Record {
        declaration: id(1),
        arguments: vec![],
        fields: vec![Value::Bool(true)],
    };
    (package, ty, value)
}

#[test]
fn active_value_checks_discharge_local_obligations_without_retaining_assumptions() {
    let (package, ty, value) = fixture();
    let outcome = Cell::new(ContractOutcome::Satisfied);
    let calls = Cell::new(0);
    let solver = |_: &nir::program::TypePredicate, fuel: &mut u64| {
        calls.set(calls.get() + 1);
        *fuel = fuel.saturating_sub(7);
        outcome.get()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_budgeted_type_solver(&solver);
    assert!(!vm.has_type(&value, &ty));
    let before = vm.remaining_steps();
    vm.check_value_type(&value, &ty, Some(SOURCE)).unwrap();
    assert!(vm.remaining_steps() < before - 7);
    assert_eq!(calls.get(), 1);
    assert!(!vm.has_type(&value, &ty));
    assert!(vm.assumed_interfaces.is_empty());
    for (result, expected) in [
        (ContractOutcome::Unknown, ErrorKind::TypeContractUnresolved),
        (ContractOutcome::Rejected, ErrorKind::TypeContractRejected),
    ] {
        outcome.set(result);
        assert_eq!(
            vm.check_value_type(&value, &ty, Some(SOURCE))
                .unwrap_err()
                .kind,
            expected
        );
    }
    outcome.set(ContractOutcome::Satisfied);
    vm.check_value_type(&value, &ty, Some(SOURCE)).unwrap();
    assert_eq!(calls.get(), 4);
    assert!(vm.assumed_interfaces.is_empty());
    assert!(vm.events.is_empty());
}

#[test]
fn nested_shapes_finish_before_queries_and_budget_failure_can_retry() {
    let (package, nominal, packet) = fixture();
    let ty = Term::Record(vec![
        ("packet".into(), Term::Optional(Box::new(nominal.clone()))),
        ("flag".into(), Term::Primitive(Primitive::Bool)),
    ]);
    let wrapped = Value::Optional {
        ty: Term::Optional(Box::new(nominal)),
        value: Some(Box::new(packet)),
    };
    let value = |flag| {
        Value::AnonymousRecord(vec![
            ("packet".into(), wrapped.clone()),
            ("flag".into(), flag),
        ])
    };
    let calls = Cell::new(0);
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| {
        calls.set(calls.get() + 1);
        ContractOutcome::Satisfied
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_budgeted_type_solver(&solver);
    assert_eq!(
        vm.check_value_type(&value(Value::Char('x')), &ty, Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert_eq!(calls.get(), 0);
    vm.budget.steps = 2;
    assert_eq!(
        vm.check_value_type(&value(Value::Bool(true)), &ty, Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(calls.get(), 0);
    vm.budget.steps = Budget::default().steps;
    vm.check_value_type(&value(Value::Bool(true)), &ty, Some(SOURCE))
        .unwrap();
    assert_eq!(calls.get(), 1);
    assert!(vm.assumed_interfaces.is_empty());
}

#[test]
fn frozen_nominal_values_recheck_the_receiving_world() {
    let (package, ty, value) = fixture();
    let encoded = Term::NominalValue {
        ty: Box::new(ty.clone()),
        variant: None,
        fields: vec![Term::Bool(true)],
    };
    for result in [ContractOutcome::Satisfied, ContractOutcome::Rejected] {
        let solver = |_: &nir::program::TypePredicate, _: &mut u64| result;
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_budgeted_type_solver(&solver);
        let decoded = vm.decode_immutable_input(&encoded, &ty);
        if result == ContractOutcome::Satisfied {
            assert_eq!(decoded.unwrap(), value);
        } else {
            assert_eq!(decoded.unwrap_err().kind, ErrorKind::TypeContractRejected);
        }
    }
}

#[test]
fn aggregate_formation_failure_precedes_field_effects_on_both_execution_paths() {
    let (package, ty, expected) = fixture();
    let trace = LocalId { owner: 1, index: 0 };
    let expression = |ty, kind| Expr {
        source: SOURCE,
        ty,
        kind,
    };
    let boolean = Term::Primitive(Primitive::Bool);
    let body = expression(
        ty,
        K::Record {
            declaration: id(1),
            fields: vec![expression(
                boolean.clone(),
                K::Block {
                    statements: vec![expression(
                        Term::Primitive(Primitive::Void),
                        K::Assign {
                            place: nir::program::Place {
                                root: trace,
                                fields: vec![],
                            },
                            value: Box::new(expression(
                                boolean.clone(),
                                K::Literal(Literal::Bool(true)),
                            )),
                        },
                    )],
                    result: Some(Box::new(expression(
                        boolean,
                        K::Literal(Literal::Bool(true)),
                    ))),
                },
            )],
        },
    );
    let code = Arc::new(comptime_compiler::compile(&body, &mut 10_000).unwrap());
    for compiled in [false, true] {
        let outcome = Cell::new(ContractOutcome::Unknown);
        let solver = |_: &nir::program::TypePredicate, _: &mut u64| outcome.get();
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_budgeted_type_solver(&solver);
        let mut environment = Environment::from([(trace, Value::Bool(false))]);
        let run = |vm: &mut Interpreter<'_>, environment: &mut Environment| {
            if compiled {
                vm.execute_code(&code, environment)
            } else {
                vm.eval(&body, environment)
            }
        };
        for (result, error) in [
            (ContractOutcome::Unknown, ErrorKind::TypeContractUnresolved),
            (ContractOutcome::Rejected, ErrorKind::TypeContractRejected),
        ] {
            outcome.set(result);
            assert_eq!(run(&mut vm, &mut environment).unwrap_err().kind, error);
            assert_eq!(environment[&trace], Value::Bool(false));
        }
        outcome.set(ContractOutcome::Satisfied);
        assert!(
            matches!(run(&mut vm, &mut environment).unwrap(), Flow::Value(value) if value == expected)
        );
        assert_eq!(environment[&trace], Value::Bool(true));
    }
}

#[test]
fn pending_nominal_domains_use_the_selected_associated_view() {
    let (mut package, _, mut value) = fixture();
    let boolean = Term::Primitive(Primitive::Bool);
    package.structs[0].type_parameters = vec![id(3)];
    package.structs[0].parameter_types = vec![Term::Primitive(Primitive::Type)];
    let Value::Record { arguments, .. } = &mut value else {
        unreachable!()
    };
    arguments.push(boolean.clone());
    let projection = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(9),
            member: id(10),
        }),
        arguments: vec![],
        subject: Box::new(boolean.clone()),
    };
    let ty = Term::Nominal {
        declaration: id(1),
        arguments: vec![projection],
    };
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| ContractOutcome::Satisfied;
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_budgeted_type_solver(&solver);
    vm.associated_values.insert_owned(
        (
            InterfaceInstance {
                declaration: id(9),
                arguments: vec![],
            },
            boolean.clone(),
            id(10),
        ),
        Arc::new(Expr {
            source: SOURCE,
            ty: Term::Primitive(Primitive::Type),
            kind: K::TypeValue(boolean),
        }),
    );
    vm.check_value_type(&value, &ty, Some(SOURCE)).unwrap();
}

#[test]
fn optional_and_error_construction_admit_nominal_payloads_on_both_paths() {
    let (package, nominal, packet) = fixture();
    let input = LocalId { owner: 1, index: 0 };
    let operand = Expr {
        source: SOURCE,
        ty: nominal.clone(),
        kind: K::Local(input),
    };
    let optional = Term::Optional(Box::new(nominal.clone()));
    let result = Term::error_result(vec![Term::Primitive(Primitive::Char)], nominal);
    let cases = [
        (
            Expr {
                source: SOURCE,
                ty: optional.clone(),
                kind: K::OptionalPresent(Box::new(operand.clone())),
            },
            Value::Optional {
                ty: optional,
                value: Some(Box::new(packet.clone())),
            },
        ),
        (
            Expr {
                source: SOURCE,
                ty: result.clone(),
                kind: K::ErrorConvert {
                    conversion: nir::error_results::Conversion::Success,
                    value: Box::new(operand),
                },
            },
            Value::ErrorResult {
                ty: result,
                error: None,
                value: Box::new(packet.clone()),
            },
        ),
    ];
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| ContractOutcome::Satisfied;
    for (body, expected) in cases {
        let code = Arc::new(comptime_compiler::compile(&body, &mut 10_000).unwrap());
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .with_budgeted_type_solver(&solver);
            let mut environment = Environment::from([(input, packet.clone())]);
            let result = if compiled {
                vm.execute_code(&code, &mut environment)
            } else {
                vm.eval(&body, &mut environment)
            };
            assert!(matches!(result.unwrap(), Flow::Value(value) if value == expected));
        }
    }
}
