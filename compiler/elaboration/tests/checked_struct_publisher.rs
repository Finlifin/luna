use comptime_interpretor::{ErrorKind, Interpreter, ReflectedField, StructBuildRequest};
use elaboration::definitions::CheckedStructPublisher;
use nir::{
    DeclarationId, Primitive, Term,
    program::{Package, SourceRange, Struct},
};
use std::sync::Arc;

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 73, local }
}
fn nominal(local: u32) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments: vec![],
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn request(ty: Term) -> StructBuildRequest {
    StructBuildRequest {
        name: "Generated".into(),
        fields: vec![ReflectedField {
            name: "value".into(),
            ty,
        }],
        source: SourceRange { start: 11, end: 23 },
    }
}
fn baseline() -> Package {
    Package {
        structs: vec![Struct {
            declaration: id(0),
            name: "Source".into(),
            fields: vec![("flag".into(), boolean())],
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
        }],
        ..Default::default()
    }
}

#[test]
fn generated_fields_share_checked_baseline_and_prior_publications() {
    let baseline = baseline();
    let publisher = CheckedStructPublisher::new(&baseline);
    let mut steps = 100_000;
    {
        let mut vm = Interpreter::for_comptime(&baseline, Default::default()).unwrap();
        for (index, field) in [(1, nominal(0)), (2, nominal(1))] {
            let prepared = publisher
                .prepare(id(index), &request(field.clone()), &mut steps)
                .unwrap();
            assert_eq!(prepared.result, id(index));
            vm.install_owned_generated_structs_with(prepared.definitions, prepared.commit)
                .unwrap();
            assert_eq!(
                vm.reflect_struct(&nominal(index)).unwrap().fields[0].ty,
                field
            );
        }
    }
    assert_eq!(publisher.published_definition_count(), 2);
    assert!(steps < 100_000);
    assert_eq!(
        publisher.to_string(),
        "(checked-struct-publisher (definitions 2))"
    );
    let retained = publisher.into_definitions();
    assert_eq!(retained[1][0].fields[0].1, nominal(1));
    assert_eq!(baseline.structs.len(), 1);
}

#[test]
fn rejected_candidates_leave_id_and_world_available_for_repair() {
    let baseline = baseline();
    let publisher = CheckedStructPublisher::new(&baseline);
    let mut steps = 100_000;
    for ty in [
        nominal(99),
        Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::Type,
        },
        Term::Rigid {
            declaration: id(12),
            sort: nir::Sort::Type,
        },
        Term::Symbol("not a Type".into()),
        Term::Optional(Box::new(nominal(1))),
    ] {
        let before = steps;
        let error = publisher
            .prepare(id(1), &request(ty), &mut steps)
            .err()
            .unwrap();
        assert_eq!(error.source, Some(request(boolean()).source));
        assert!(steps < before);
        assert_eq!(publisher.published_definition_count(), 0);
        assert!(publisher.last_diagnostic().is_some());
    }
    let mut duplicate = request(boolean());
    duplicate.fields.push(ReflectedField {
        name: "value".into(),
        ty: boolean(),
    });
    assert!(publisher.prepare(id(1), &duplicate, &mut steps).is_err());
    assert!(
        publisher
            .prepare(id(0), &request(boolean()), &mut steps)
            .is_err()
    );
    let prepared = publisher
        .prepare(id(1), &request(boolean()), &mut steps)
        .unwrap();
    (prepared.commit)().unwrap();
    assert_eq!(publisher.published_definition_count(), 1);
    assert!(publisher.last_diagnostic().is_none());
}

#[test]
fn pointer_recursion_is_legal_and_unknown_pointees_are_not() {
    let baseline = baseline();
    let publisher = CheckedStructPublisher::new(&baseline);
    let mut steps = 100_000;
    assert!(
        publisher
            .prepare(
                id(1),
                &request(Term::Pointer(Box::new(nominal(99)))),
                &mut steps
            )
            .is_err()
    );
    let prepared = publisher
        .prepare(
            id(1),
            &request(Term::Optional(Box::new(Term::Pointer(Box::new(nominal(
                1,
            )))))),
            &mut steps,
        )
        .unwrap();
    (prepared.commit)().unwrap();
    assert_eq!(publisher.published_definition_count(), 1);
}

#[test]
fn dropped_and_stale_candidates_cannot_publish_or_overwrite() {
    let baseline = baseline();
    let publisher = CheckedStructPublisher::new(&baseline);
    let mut steps = 100_000;
    let discarded = publisher
        .prepare(id(1), &request(boolean()), &mut steps)
        .unwrap();
    let weak = Arc::downgrade(&discarded.definitions);
    drop(discarded);
    assert!(weak.upgrade().is_none());
    assert_eq!(publisher.published_definition_count(), 0);
    let first = publisher
        .prepare(id(1), &request(boolean()), &mut steps)
        .unwrap();
    let stale = publisher
        .prepare(id(2), &request(boolean()), &mut steps)
        .unwrap();
    let mut vm = Interpreter::for_comptime(&baseline, Default::default()).unwrap();
    vm.install_owned_generated_structs_with(first.definitions, first.commit)
        .unwrap();
    let failed = vm.install_owned_generated_structs_with(stale.definitions, stale.commit);
    assert!(
        matches!(failed, Err(comptime_interpretor::StructPublicationError::Commit(error))
        if error.kind == ErrorKind::InvalidCallPlan)
    );
    assert!(vm.reflect_struct(&nominal(2)).is_err());
    assert_eq!(publisher.published_definition_count(), 1);
    assert!(publisher.last_diagnostic().unwrap().contains("stale"));
    let repaired = publisher
        .prepare(id(2), &request(nominal(1)), &mut steps)
        .unwrap();
    vm.install_owned_generated_structs_with(repaired.definitions, repaired.commit)
        .unwrap();
    assert_eq!(
        vm.reflect_struct(&nominal(2)).unwrap().fields[0].ty,
        nominal(1)
    );
    assert_eq!(publisher.published_definition_count(), 2);
}

#[test]
fn exhaustion_during_copy_or_validation_never_commits() {
    let baseline = baseline();
    let publisher = CheckedStructPublisher::new(&baseline);
    let oversized = request(Term::Tuple(vec![boolean(); 4096]));
    for allowance in [0, 1, 50, 100] {
        let mut steps = allowance;
        assert!(publisher.prepare(id(1), &oversized, &mut steps).is_err());
        assert!(steps <= allowance);
        assert_eq!(publisher.published_definition_count(), 0);
    }
    let mut steps = 100_000;
    (publisher
        .prepare(id(1), &request(boolean()), &mut steps)
        .unwrap()
        .commit)()
    .unwrap();
    assert_eq!(publisher.published_definition_count(), 1);
}

#[test]
fn representation_budget_is_retryable_and_not_a_type_error() {
    let mut baseline = baseline();
    baseline.structs[0].fields = (0..128)
        .map(|index| (format!("field{index}"), boolean()))
        .collect();
    let measured = CheckedStructPublisher::new(&baseline);
    let mut full = 1_000_000;
    drop(
        measured
            .prepare(id(1), &request(nominal(0)), &mut full)
            .unwrap(),
    );
    let cost = 1_000_000 - full;

    let publisher = CheckedStructPublisher::new(&baseline);
    let mut short = cost - 1;
    let failure = publisher
        .prepare(id(1), &request(nominal(0)), &mut short)
        .err()
        .unwrap();
    assert_eq!(failure.kind, ErrorKind::StepsExhausted);
    assert_eq!(failure.source, Some(request(nominal(0)).source));
    let diagnostic = publisher.last_diagnostic().unwrap();
    assert!(
        diagnostic.contains("representation") && diagnostic.contains("budget"),
        "{diagnostic}"
    );
    assert_eq!(publisher.published_definition_count(), 0);

    let retry = publisher
        .prepare(id(1), &request(nominal(0)), &mut 1_000_000)
        .unwrap();
    (retry.commit)().unwrap();
    assert_eq!(publisher.published_definition_count(), 1);
    let invalid = publisher
        .prepare(id(2), &request(nominal(2)), &mut 1_000_000)
        .err()
        .unwrap();
    assert_eq!(invalid.kind, ErrorKind::Type);
    assert_eq!(publisher.published_definition_count(), 1);
}

#[test]
fn allocated_publisher_preserves_reserved_frontier_across_discarded_candidates() {
    use elaboration::definitions::{AllocatedStructPublisher, StructBuildService};
    let baseline = baseline();
    let publisher = AllocatedStructPublisher::new(&baseline, 73, 100, 1000);
    let mut steps = 100_000;
    let discarded = publisher.prepare(&request(boolean()), &mut steps).unwrap();
    assert_eq!(discarded.result, id(100));
    drop(discarded);
    {
        let next = publisher.prepare(&request(boolean()), &mut steps).unwrap();
        assert_eq!(next.result, id(101));
        (next.commit)().unwrap();
    }
    assert!(publisher.remaining_structural_fuel() < 1000);
    assert!(publisher.to_string().contains("(next 102)"));
    let (definitions, frontier) = publisher.finish();
    assert_eq!(frontier, 102);
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0][0].declaration, id(101));
}

#[test]
fn allocated_publisher_reports_budget_and_identity_exhaustion_without_publication() {
    use elaboration::definitions::{AllocatedStructPublisher, StructBuildService};
    let baseline = baseline();
    for (frontier, fuel, expected) in [
        (100, 0, "preparation structural budget exhausted"),
        (u32::MAX, 1000, "identity space exhausted"),
    ] {
        let publisher = AllocatedStructPublisher::new(&baseline, 73, frontier, fuel);
        assert!(
            publisher
                .prepare(&request(boolean()), &mut 100_000)
                .is_err()
        );
        assert!(publisher.last_diagnostic().unwrap().contains(expected));
        let (definitions, next) = publisher.finish();
        assert!(definitions.is_empty());
        assert_eq!(next, frontier);
    }
}
