use super::*;

fn wide_nominal(kind: usize) -> (Package, Term, Value) {
    let label = "field".repeat(20_001);
    let representation = Term::Record(vec![(
        label.clone(),
        Term::Rigid {
            declaration: id(2),
            sort: nir::Sort::Type,
        },
    )]);
    let argument = Term::Primitive(Primitive::Bool);
    let ty = Term::Nominal {
        declaration: id(1),
        arguments: vec![argument.clone()],
    };
    let payload = Value::AnonymousRecord(vec![(label, Value::Bool(true))]);
    let mut package = Package::default();
    let value = match kind {
        0 => {
            package.structs.push(nir::program::Struct {
                declaration: id(1),
                name: "Packet".into(),
                type_parameters: vec![id(2)],
                parameter_types: vec![],
                fields: vec![("payload".into(), representation)],
                capabilities: vec![],
                requirements: vec![],
            });
            Value::Record {
                declaration: id(1),
                arguments: vec![argument],
                fields: vec![payload],
            }
        }
        1 => {
            package.enums.push(nir::program::Enum {
                declaration: id(1),
                name: "Packet".into(),
                type_parameters: vec![id(2)],
                parameter_types: vec![],
                is_intrinsic: false,
                variants: vec![nir::program::Variant {
                    name: "ready".into(),
                    shape: nir::program::VariantShape::Tuple,
                    fields: vec![representation],
                }],
                groups: vec![],
                encoding: None,
                symbol_encoding: None,
                capabilities: vec![],
                requirements: vec![],
            });
            Value::Variant {
                declaration: id(1),
                arguments: vec![argument],
                index: 0,
                fields: vec![payload],
            }
        }
        2 => {
            package.newtypes.push(nir::newtypes::Newtype {
                declaration: id(1),
                name: "Packet".into(),
                type_parameters: vec![id(2)],
                parameter_types: vec![],
                capture_count: 0,
                representation,
                capabilities: vec![],
                requirements: vec![],
            });
            Value::Newtype {
                ty: ty.clone(),
                value: Box::new(payload),
            }
        }
        _ => unreachable!(),
    };
    (package, ty, value)
}

#[test]
fn nominal_shape_preparation_uses_the_callers_allowance_and_recovers() {
    for kind in 0..3 {
        let (package, ty, value) = wide_nominal(kind);
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 2_000_000,
                ..Budget::default()
            },
        )
        .unwrap();
        let before = package.to_string();
        for steps in [0, 1, 10, 100_000] {
            vm.budget.steps = steps;
            let failure = vm.check_value_type(&value, &ty, Some(SOURCE)).unwrap_err();
            assert_eq!(
                failure.kind,
                ErrorKind::StepsExhausted,
                "kind={kind}, steps={steps}"
            );
            assert_eq!(failure.source, Some(SOURCE));
            assert!(vm.remaining_steps() <= steps);
        }
        vm.budget.steps = 1_000_000;
        vm.check_value_type(&value, &ty, Some(SOURCE)).unwrap();
        let cost = 1_000_000 - vm.remaining_steps();
        assert!(cost > 100_000, "wide preparation cannot hide its cost");
        let pair = Value::Tuple(vec![value.clone(), value.clone()]);
        let pair_type = Term::Tuple(vec![ty.clone(), ty.clone()]);
        vm.budget.steps = cost;
        assert_eq!(
            vm.check_value_type(&pair, &pair_type, Some(SOURCE))
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted,
            "separate payloads must share the allowance"
        );
        vm.budget.steps = 2_000_000;
        vm.check_value_type(&pair, &pair_type, Some(SOURCE))
            .unwrap();
        let mut wrong = value.clone();
        let payload = match &mut wrong {
            Value::Record { fields, .. } | Value::Variant { fields, .. } => &mut fields[0],
            Value::Newtype { value, .. } => value.as_mut(),
            _ => unreachable!(),
        };
        let Value::AnonymousRecord(fields) = payload else {
            unreachable!()
        };
        fields[0].1 = Value::Char('x');
        vm.budget.steps = 1_000_000;
        assert_eq!(
            vm.check_value_type(&wrong, &ty, Some(SOURCE))
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        assert!(vm.events.is_empty());
        assert_eq!(package.to_string(), before);
    }
}

#[test]
fn callable_shape_preparation_preserves_budget_and_input_rejection() {
    let generic = Term::Rigid {
        declaration: id(2),
        sort: nir::Sort::Type,
    };
    let package = Package {
        runtime_function_identities: vec![nir::program::RuntimeFunctionIdentity {
            requirements: vec![],
            capabilities: vec![],
            declaration: id(1),
            type_parameters: vec![id(2)],
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(generic),
            },
        }],
        ..Default::default()
    };
    let domain = Term::Record(vec![(
        "field".repeat(20_001),
        Term::Primitive(Primitive::Bool),
    )]);
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(domain.clone()),
    };
    let value = Value::FunctionInstance {
        declaration: id(1),
        arguments: vec![domain],
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for steps in [1, 10, 100_000] {
        vm.budget.steps = steps;
        assert_eq!(
            vm.check_value_type(&value, &signature, Some(SOURCE))
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
    }
    vm.budget.steps = 2_000_000;
    vm.check_value_type(&value, &signature, Some(SOURCE))
        .unwrap();
    assert!(2_000_000 - vm.remaining_steps() > 100_000);
    let invalid = Value::FunctionInstance {
        declaration: id(1),
        arguments: vec![Term::Bool(true)],
    };
    vm.budget.steps = 2_000_000;
    assert_eq!(
        vm.check_value_type(&invalid, &signature, Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    vm.check_value_type(&value, &signature, Some(SOURCE))
        .unwrap();
    assert!(
        vm.events.is_empty(),
        "signature observation cannot execute the callable"
    );
}
