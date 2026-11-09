use super::*;

const SOURCE: SourceRange = SourceRange { start: 40, end: 60 };
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 95, local }
}
fn record(width: usize) -> Term {
    Term::Record(vec![("x".repeat(width), Term::Primitive(Primitive::Bool))])
}

#[test]
fn abstract_domain_authorization_retains_cost_without_publishing_a_partial_batch() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let boolean = Term::Primitive(Primitive::Bool);
    vm.authorize_abstract_value_parameters(&[(id(1), boolean.clone())])
        .unwrap();
    let prior = vm.abstract_value_parameters.clone();
    let batch = [(id(2), boolean.clone()), (id(3), record(512))];
    vm.budget.steps = 100;
    assert_eq!(
        vm.authorize_abstract_value_parameters(&batch)
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert!(vm.remaining_steps() > 0 && vm.remaining_steps() < 97);
    assert_eq!(vm.abstract_value_parameters, prior);

    vm.budget.steps = 10_000;
    vm.authorize_abstract_value_parameters(&batch).unwrap();
    for (declaration, domain) in &batch {
        assert_eq!(vm.abstract_value_parameters.get(declaration), Some(domain));
    }
    let committed = vm.abstract_value_parameters.clone();
    let conflicting = [(id(4), boolean), (id(1), Term::Primitive(Primitive::Char))];
    assert_eq!(
        vm.authorize_abstract_value_parameters(&conflicting)
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert_eq!(vm.abstract_value_parameters, committed);
    assert!(vm.events.is_empty());
}

#[test]
fn type_equality_shares_description_work_and_does_not_turn_limits_into_false() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let ty = record(512);
    vm.budget.steps = 700;
    let error = vm
        .complete_type_equal(ty.clone(), ty.clone(), SOURCE)
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::StepsExhausted);
    assert_eq!(error.source, Some(SOURCE));
    assert!(vm.remaining_steps() > 0 && vm.remaining_steps() < 200);
    vm.budget.steps = 10_000;
    assert_eq!(
        vm.complete_type_equal(ty.clone(), ty, SOURCE).unwrap(),
        Value::Bool(true)
    );
    vm.budget.steps = 1_000_000;
    let oversized = record(100_001);
    assert_eq!(
        vm.complete_type_equal(oversized.clone(), oversized, SOURCE)
            .unwrap_err()
            .kind,
        ErrorKind::RepresentationSizeLimit
    );
    assert!(vm.events.is_empty());
}

#[test]
fn object_operations_retain_description_failures_before_returning_or_editing_entries() {
    use nir::program::{ObjectEditKind as Edit, ObjectQueryKind as Query};
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let domain = Term::Optional(Box::new(record(512)));
    let value = Value::Optional {
        ty: domain.clone(),
        value: None,
    };
    let object = Object::from_entries(vec![ObjectEntry::Property {
        key: "payload".into(),
        value: ObjectValue {
            ty: domain.clone(),
            value: value.clone(),
        },
    }]);
    vm.budget.steps = 100;
    assert_eq!(
        vm.lookup_object_property(&object, "payload", &domain)
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert!(vm.remaining_steps() > 0 && vm.remaining_steps() < 100);
    vm.budget.steps = 100_000;
    assert!(matches!(
        vm.lookup_object_property(&object, "payload", &domain).unwrap(),
        ObjectProperty::Found(typed) if typed.ty == domain && typed.value == value
    ));

    vm.budget.steps = 100;
    let packet = Value::Tuple(vec![
        Value::Object(object.clone()),
        Value::Integer(comptime_runtime::Integer::from_u128(0)),
    ]);
    let error = vm
        .complete_object_query(
            Query::EntryType,
            &Term::Primitive(Primitive::Type),
            packet,
            SOURCE,
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::StepsExhausted);
    assert_eq!(error.source, Some(SOURCE));
    assert!(vm.remaining_steps() > 0 && vm.remaining_steps() < 100);

    let output = Term::Primitive(Primitive::Object);
    let key_domain = Term::Optional(Box::new(Term::Primitive(Primitive::Symbol)));
    let input = Term::Tuple(vec![output.clone(), key_domain.clone(), domain.clone()]);
    let packet = Value::Tuple(vec![
        Value::Object(object.clone()),
        Value::Optional {
            ty: key_domain,
            value: None,
        },
        value.clone(),
    ]);
    vm.budget.steps = 100;
    assert_eq!(
        vm.complete_object_edit(Edit::Append, &output, &input, packet.clone(), SOURCE)
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert!(vm.remaining_steps() > 0 && vm.remaining_steps() < 100);
    vm.budget.steps = 100_000;
    let Flow::Value(Value::Object(edited)) = vm
        .complete_object_edit(Edit::Append, &output, &input, packet, SOURCE)
        .unwrap()
    else {
        panic!("append did not return an object")
    };
    assert_eq!(edited.entries().len(), 2);
    assert_eq!(edited.entries()[0], object.entries()[0]);
    assert_eq!(edited.entries()[1].value().value, value);
    assert!(edited.entries()[1].key().is_none());
    assert_eq!(object.entries().len(), 1);
    assert!(vm.events.is_empty());
}

#[test]
fn optional_decoding_retains_type_resource_errors_and_recovers() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let domain = Term::Optional(Box::new(record(512)));
    let term = Term::OptionalValue {
        ty: Box::new(domain.clone()),
        value: None,
    };
    vm.budget.steps = 100;
    assert_eq!(
        vm.normal_pattern_value(&term, &domain).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert!(vm.remaining_steps() > 0 && vm.remaining_steps() < 99);
    vm.budget.steps = 100_000;
    assert_eq!(
        vm.normal_pattern_value(&term, &domain).unwrap(),
        Value::Optional {
            ty: domain,
            value: None
        }
    );
    let domain = Term::Optional(Box::new(record(100_001)));
    let term = Term::OptionalValue {
        ty: Box::new(domain.clone()),
        value: None,
    };
    vm.budget.steps = 1_000_000;
    assert_eq!(
        vm.normal_pattern_value(&term, &domain).unwrap_err().kind,
        ErrorKind::RepresentationSizeLimit
    );
    assert!(vm.events.is_empty());
}

#[test]
fn type_pattern_failure_does_not_advance_its_field_cursor() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let input = Value::Type(Term::Tuple(vec![record(512)]));
    for wild in [false, true] {
        let mut state = vm
            .prepare_type_pattern(
                nir::pattern_structure::TypeStructure::Tuple { fields: 1 },
                &input,
            )
            .unwrap()
            .unwrap();
        let before = state.to_string();
        vm.budget.steps = 100;
        assert_eq!(
            vm.type_pattern_field(&mut state, &input, 0, wild)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert!(vm.remaining_steps() > 0 && vm.remaining_steps() < 100);
        assert_eq!(state.to_string(), before);
        vm.budget.steps = 10_000;
        assert_eq!(
            vm.type_pattern_field(&mut state, &input, 0, wild).unwrap(),
            (!wild).then(|| Value::Type(record(512)))
        );
        assert_eq!(
            vm.type_pattern_field(&mut state, &input, 0, wild)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidCallPlan
        );
    }
    assert!(vm.events.is_empty());
}
