use super::*;

fn list_package() -> Package {
    Package {
        normal_forms: vec![nir::normal_forms::Head {
            declaration: id(1),
            role: nir::normal_forms::Role::TypeList,
            collector: None,
            requirements: vec![],
            parameters: vec![nir::normal_forms::Parameter {
                declaration: id(2),
                binding: LocalId { owner: 1, index: 0 },
                kind: nir::normal_forms::Kind::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
        }],
        ..Default::default()
    }
}

fn list_type(element: Term) -> Term {
    Term::NormalForm {
        head: id(1),
        sort: nir::Sort::Type,
        arguments: vec![element],
    }
}

#[test]
fn empty_list_shapes_pay_for_element_observation_and_recover() {
    let package = list_package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let ty = list_type(Term::Record(vec![(
        "x".repeat(200),
        Term::Primitive(Primitive::Bool),
    )]));
    let value = Value::List {
        ty: ty.clone(),
        values: vec![].into(),
    };
    vm.budget.steps = 2;
    assert_eq!(
        vm.check_value_type(&value, &ty, Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    vm.budget.steps = 10_000;
    vm.check_value_type(&value, &ty, Some(SOURCE)).unwrap();
    assert!(10_000 - vm.remaining_steps() >= 200);
    let mut foreign = ty.clone();
    let Term::NormalForm { head, .. } = &mut foreign else {
        unreachable!()
    };
    *head = id(99);
    let impostor = Value::List {
        ty: foreign.clone(),
        values: vec![].into(),
    };
    assert_eq!(
        vm.check_value_type(&impostor, &foreign, Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert!(vm.events.is_empty());
}

#[test]
fn oversized_lists_are_resource_limits_at_container_boundaries() {
    let package = list_package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let boolean = Term::Primitive(Primitive::Bool);
    let ty = list_type(boolean.clone());
    let value = Value::List {
        ty: ty.clone(),
        values: vec![Value::Bool(true); 100_001].into(),
    };
    vm.budget.steps = 2_000_000;
    assert_eq!(
        vm.check_value_type(&value, &ty, Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::RepresentationSizeLimit
    );
    assert_eq!(
        vm.prepare_list_construction(&ty, std::iter::repeat_n(&boolean, 100_001), SOURCE)
            .unwrap_err()
            .kind,
        ErrorKind::RepresentationSizeLimit
    );
    assert_eq!(
        vm.complete_list_construction(&ty, vec![Value::Bool(true); 100_001], SOURCE)
            .unwrap_err()
            .kind,
        ErrorKind::RepresentationSizeLimit
    );
    let term = Term::NominalValue {
        ty: Box::new(ty.clone()),
        variant: None,
        fields: vec![Term::Bool(true); 100_001],
    };
    assert_eq!(
        vm.decode_immutable_input(&term, &ty).unwrap_err().kind,
        ErrorKind::RepresentationSizeLimit
    );
    vm.budget.steps = 10_000;
    assert!(
        vm.complete_list_construction(&ty, vec![Value::Bool(true)], SOURCE)
            .is_ok()
    );
}

#[test]
fn nominal_decoding_can_spend_more_than_the_old_private_query_ceiling() {
    let width = 40_000;
    let boolean = Term::Primitive(Primitive::Bool);
    let package = Package {
        structs: vec![nir::program::Struct {
            declaration: id(1),
            name: "Wide".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            fields: (0..width)
                .map(|n| (format!("f{n}"), boolean.clone()))
                .collect(),
            capabilities: vec![],
            requirements: vec![],
            packed: nir::program::PackedMode::None,
        }],
        ..Default::default()
    };
    let ty = Term::Nominal {
        declaration: id(1),
        arguments: vec![],
    };
    let term = Term::NominalValue {
        ty: Box::new(ty.clone()),
        variant: None,
        fields: vec![Term::Bool(true); width],
    };
    let mut vm = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 5_000_000,
            ..Budget::default()
        },
    )
    .unwrap();
    vm.budget.steps = 50;
    assert_eq!(
        vm.decode_immutable_input(&term, &ty).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    vm.budget.steps = 5_000_000;
    let Value::Record { fields, .. } = vm.decode_immutable_input(&term, &ty).unwrap() else {
        panic!()
    };
    assert_eq!(fields.len(), width);
    assert!(fields.iter().all(|value| *value == Value::Bool(true)));
    assert!(vm.events.is_empty());
}
