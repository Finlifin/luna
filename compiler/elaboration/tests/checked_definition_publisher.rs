use comptime_interpretor::{Budget, ErrorKind, Interpreter};
use elaboration::definitions::CheckedDefinitionPublisher;
use nir::{
    DeclarationId, Primitive, Term,
    program::{Enum, Package, SourceRange, Struct, Variant, VariantShape},
};
use std::sync::Arc;
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 74, local }
}
fn ty(local: u32) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments: vec![],
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn source() -> SourceRange {
    SourceRange { start: 7, end: 19 }
}
fn fuel() -> Budget {
    Budget {
        steps: 1_000_000,
        depth: 128,
    }
}
fn structs(local: u32, field: Term) -> Arc<[Struct]> {
    vec![Struct {
        declaration: id(local),
        name: "Generated".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        fields: vec![("value".into(), field)],
    }]
    .into()
}
fn enumeration(local: u32, field: Term) -> Enum {
    Enum {
        declaration: id(local),
        name: "GeneratedEnum".into(),
        is_intrinsic: false,
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        groups: vec![],
        encoding: None,
        symbol_encoding: None,
        variants: vec![Variant {
            name: "Data".into(),
            shape: VariantShape::Named(vec!["value".into()]),
            fields: vec![field],
        }],
    }
}
fn enums(local: u32, field: Term) -> Arc<[Enum]> {
    vec![enumeration(local, field)].into()
}

#[test]
fn alternating_publications_feed_both_checks_and_the_live_vm() {
    let world = Package::default();
    let publisher = CheckedDefinitionPublisher::new(&world);
    let mut budget = fuel();
    let mut vm = Interpreter::for_comptime(&world, fuel()).unwrap();
    let first = structs(1, boolean());
    let prepared = publisher
        .prepare_structs(Arc::clone(&first), source(), &mut budget)
        .unwrap();
    vm.install_owned_generated_structs_with(first, || prepared.commit())
        .unwrap();
    let second = enums(2, ty(1));
    let weak = Arc::downgrade(&second);
    let prepared = publisher
        .prepare_enums(Arc::clone(&second), source(), &mut budget)
        .unwrap();
    assert_eq!(prepared.to_string(), "(prepared-definitions (count 1))");
    vm.install_owned_generated_enums_with(second, || prepared.commit())
        .unwrap();
    let third = structs(3, ty(2));
    let prepared = publisher
        .prepare_structs(Arc::clone(&third), source(), &mut budget)
        .unwrap();
    vm.install_owned_generated_structs_with(third, || prepared.commit())
        .unwrap();
    assert_eq!(vm.reflect_enum(&ty(2)).unwrap().variants[0].fields, [ty(1)]);
    assert_eq!(vm.reflect_struct(&ty(3)).unwrap().fields[0].ty, ty(2));
    assert_eq!(publisher.published_definition_count(), 3);
    assert_eq!(
        publisher.to_string(),
        "(checked-definition-publisher (definitions 3))"
    );
    let published = publisher.into_definitions();
    assert_eq!(published.structs.len(), 2);
    assert_eq!(published.enums.len(), 1);
    drop(vm);
    assert!(weak.upgrade().is_some());
    drop(published);
    assert!(weak.upgrade().is_none());
}

#[test]
fn invalid_enum_formation_and_representation_leave_no_partial_publication() {
    let world = Package::default();
    let publisher = CheckedDefinitionPublisher::new(&world);
    let mut budget = fuel();
    let survivor = publisher
        .prepare_structs(structs(10, boolean()), source(), &mut budget)
        .unwrap();
    for field in [
        ty(99),
        Term::Symbol("not a Type".into()),
        ty(1),
        Term::Optional(Box::new(ty(1))),
    ] {
        let before = budget.steps;
        let error = publisher
            .prepare_enums(enums(1, field), source(), &mut budget)
            .err()
            .unwrap();
        assert_eq!(error.source, Some(source()));
        assert!(budget.steps < before);
        assert_eq!(publisher.published_definition_count(), 0);
        assert!(publisher.last_diagnostic().is_some());
    }
    // A successful prefix proof must not escape a failed multi-definition batch.
    let batch = vec![enumeration(1, boolean()), enumeration(2, ty(2))].into();
    assert!(
        publisher
            .prepare_enums(batch, source(), &mut budget)
            .is_err()
    );
    assert!(
        publisher
            .prepare_enums(enums(1, ty(1)), source(), &mut budget)
            .is_err()
    );
    survivor.commit().unwrap();
    let prepared = publisher
        .prepare_enums(
            enums(1, Term::Pointer(Box::new(ty(1)))),
            source(),
            &mut budget,
        )
        .unwrap();
    prepared.commit().unwrap();
    assert_eq!(publisher.published_definition_count(), 2);
}

#[test]
fn dropped_and_cross_kind_stale_candidates_roll_back_live_vm_installation() {
    let world = Package::default();
    let publisher = CheckedDefinitionPublisher::new(&world);
    let mut budget = fuel();
    let discarded = enums(1, boolean());
    let weak = Arc::downgrade(&discarded);
    drop(
        publisher
            .prepare_enums(discarded, source(), &mut budget)
            .unwrap(),
    );
    assert!(weak.upgrade().is_none());
    assert!(
        publisher
            .prepare_structs(structs(2, ty(1)), source(), &mut budget)
            .is_err()
    );
    let definitions = enums(1, boolean());
    let stale = publisher
        .prepare_enums(Arc::clone(&definitions), source(), &mut budget)
        .unwrap();
    publisher
        .prepare_structs(structs(2, boolean()), source(), &mut budget)
        .unwrap()
        .commit()
        .unwrap();
    let mut vm = Interpreter::for_comptime(&world, fuel()).unwrap();
    assert!(
        matches!(vm.install_owned_generated_enums_with(definitions, || stale.commit()),
        Err(comptime_interpretor::EnumPublicationError::Commit(error)) if error.kind == ErrorKind::InvalidCallPlan)
    );
    assert!(vm.reflect_enum(&ty(1)).is_err());
    assert!(publisher.last_diagnostic().unwrap().contains("stale"));
    let stale = publisher
        .prepare_structs(structs(3, boolean()), source(), &mut budget)
        .unwrap();
    publisher
        .prepare_enums(enums(1, ty(2)), source(), &mut budget)
        .unwrap()
        .commit()
        .unwrap();
    assert_eq!(stale.commit().unwrap_err().kind, ErrorKind::InvalidCallPlan);
    assert!(
        publisher
            .prepare_enums(enums(4, ty(3)), source(), &mut budget)
            .is_err()
    );
    assert!(
        publisher
            .prepare_enums(enums(2, boolean()), source(), &mut budget)
            .is_err()
    );
    assert_eq!(publisher.published_definition_count(), 2);
}

#[test]
fn all_insufficient_budgets_are_retryable_without_losing_representation_checks() {
    let world = Package::default();
    let measured = CheckedDefinitionPublisher::new(&world);
    let mut budget = fuel();
    let before = budget.steps;
    drop(
        measured
            .prepare_enums(enums(1, boolean()), source(), &mut budget)
            .unwrap(),
    );
    let cost = before - budget.steps;
    for steps in 0..cost {
        let publisher = CheckedDefinitionPublisher::new(&world);
        let mut low = Budget { steps, depth: 128 };
        let error = publisher
            .prepare_enums(enums(1, boolean()), source(), &mut low)
            .err()
            .unwrap();
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(error.source, Some(source()));
        assert!(low.steps <= steps);
        assert_eq!(publisher.published_definition_count(), 0);
        assert_eq!(
            publisher
                .prepare_enums(enums(1, ty(1)), source(), &mut fuel())
                .err()
                .unwrap()
                .kind,
            ErrorKind::Type
        );
        let prepared = publisher
            .prepare_enums(enums(1, boolean()), source(), &mut fuel())
            .unwrap();
        prepared.commit().unwrap();
        assert_eq!(publisher.published_definition_count(), 1);
    }
}

#[test]
fn forward_enum_batch_is_retained_together_and_external_rejection_publishes_nothing() {
    let world = Package::default();
    let publisher = CheckedDefinitionPublisher::new(&world);
    let mut budget = fuel();
    let definitions: Arc<[Enum]> = vec![enumeration(1, ty(2)), enumeration(2, boolean())].into();
    let prepared = publisher
        .prepare_enums(Arc::clone(&definitions), source(), &mut budget)
        .unwrap();
    let conflicting = Package {
        enums: vec![enumeration(1, boolean())],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&conflicting, fuel()).unwrap();
    assert!(
        vm.install_owned_generated_enums_with(Arc::clone(&definitions), || prepared.commit())
            .is_err()
    );
    assert_eq!(publisher.published_definition_count(), 0);
    assert!(
        publisher
            .prepare_structs(structs(3, ty(1)), source(), &mut budget)
            .is_err()
    );
    let prepared = publisher
        .prepare_enums(Arc::clone(&definitions), source(), &mut budget)
        .unwrap();
    let mut vm = Interpreter::for_comptime(&world, fuel()).unwrap();
    vm.install_owned_generated_enums_with(definitions, || prepared.commit())
        .unwrap();
    assert_eq!(publisher.published_definition_count(), 2);
    assert_eq!(vm.reflect_enum(&ty(1)).unwrap().variants[0].fields, [ty(2)]);
    publisher
        .prepare_structs(structs(3, ty(1)), source(), &mut budget)
        .unwrap()
        .commit()
        .unwrap();
}
