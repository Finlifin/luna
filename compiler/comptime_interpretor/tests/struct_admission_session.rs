use comptime_interpretor::{Budget, ErrorKind, StructAdmissionSession, StructPublicationError};
use nir::{
    DeclarationId, Primitive, Term,
    program::{Package, Struct},
};
use std::{cell::Cell, sync::Arc};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 19, local }
}
fn nominal(local: u32) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments: vec![],
    }
}
fn batch(local: u32, field: Term) -> Arc<[Struct]> {
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
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn budget() -> Budget {
    Budget {
        steps: 1_000_000,
        depth: 128,
    }
}

#[test]
fn source_admission_obtains_metered_formation_evidence_and_recovers() {
    use comptime_interpretor::ContractOutcome;
    use nir::program::{Trait, TraitRequirement, TypePredicate};
    let requirement = TypePredicate::Trait(TraitRequirement {
        interface: id(10),
        interface_arguments: vec![],
        subject: boolean(),
    });
    let mut checked = batch(1, boolean()).to_vec();
    checked[0].requirements.push(requirement.clone());
    let world = Package {
        traits: vec![Trait {
            declaration: id(10),
            self_parameter: id(11),
            name: "Marker".into(),
            comptime_only: false,
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            parents: vec![],
            associated: vec![],
            methods: vec![],
        }],
        structs: checked,
        ..Default::default()
    };
    let calls = Cell::new(0);
    let outcome = Cell::new(ContractOutcome::Unknown);
    let solver = |predicate: &TypePredicate, remaining: &mut u64| {
        assert_eq!(predicate, &requirement);
        calls.set(calls.get() + 1);
        *remaining = remaining.saturating_sub(7);
        outcome.get()
    };
    let mut fuel = budget();
    let mut session =
        StructAdmissionSession::new_with_type_solver(&world, &solver, &mut fuel).unwrap();
    assert_eq!(calls.get(), 0);
    for answer in [ContractOutcome::Unknown, ContractOutcome::Rejected] {
        outcome.set(answer);
        let before = fuel.steps;
        assert!(session.prepare(batch(2, nominal(1)), &mut fuel).is_err());
        assert!(fuel.steps < before);
        assert!(calls.get() > 0);
        assert_eq!(
            session.to_string(),
            "(definition-admission-session (revision 0))"
        );
    }
    outcome.set(ContractOutcome::Satisfied);
    assert!(
        session.prepare(batch(3, nominal(2)), &mut fuel).is_err(),
        "failed formation must not publish a descriptor"
    );
    let receipt = session.prepare(batch(2, nominal(1)), &mut fuel).unwrap();
    session.commit_with(receipt, || Ok::<_, ()>(())).unwrap();
    assert!(session.prepare(batch(3, nominal(2)), &mut fuel).is_ok());
    let calls_before = calls.get();
    assert!(
        session
            .prepare(
                batch(4, nominal(1)),
                &mut Budget {
                    steps: 0,
                    ..budget()
                }
            )
            .is_err()
    );
    assert_eq!(calls.get(), calls_before);
    assert!(session.prepare(batch(4, nominal(1)), &mut budget()).is_ok());
}

#[test]
fn dropped_receipts_are_not_visible_and_committed_receipts_are() {
    let world = Package::default();
    let mut fuel = budget();
    let mut session = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    let descriptions = batch(1, boolean());
    let weak = Arc::downgrade(&descriptions);
    let receipt = session.prepare(descriptions, &mut fuel).unwrap();
    assert!(
        receipt
            .to_string()
            .starts_with("(prepared-struct-admission (revision 0)")
    );
    assert!(session.prepare(batch(2, nominal(1)), &mut fuel).is_err());
    drop(receipt);
    assert!(weak.upgrade().is_none());
    let receipt = session.prepare(batch(1, boolean()), &mut fuel).unwrap();
    let before = fuel.steps;
    session.commit_with(receipt, || Ok::<_, ()>(())).unwrap();
    assert_eq!(fuel.steps, before);
    assert_eq!(
        session.to_string(),
        "(definition-admission-session (revision 1))"
    );
    assert!(session.prepare(batch(2, nominal(1)), &mut fuel).is_ok());
    assert!(session.prepare(batch(1, boolean()), &mut fuel).is_err());
}

#[test]
fn foreign_and_stale_receipts_never_invoke_callbacks() {
    let world = Package::default();
    let mut fuel = budget();
    let mut first = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    let mut second = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    let foreign = first.prepare(batch(1, boolean()), &mut fuel).unwrap();
    let called = Cell::new(false);
    assert!(
        matches!(second.commit_with(foreign, || {called.set(true); Ok::<_, ()>(())}),
        Err(StructPublicationError::Admission(error)) if error.kind == ErrorKind::InvalidCallPlan)
    );
    let current = first.prepare(batch(1, boolean()), &mut fuel).unwrap();
    let stale = first.prepare(batch(2, boolean()), &mut fuel).unwrap();
    first.commit_with(current, || Ok::<_, ()>(())).unwrap();
    assert!(
        matches!(first.commit_with(stale, || {called.set(true); Ok::<_, ()>(())}),
        Err(StructPublicationError::Admission(error)) if error.kind == ErrorKind::InvalidCallPlan)
    );
    assert!(!called.get());
    assert!(first.prepare(batch(3, nominal(2)), &mut fuel).is_err());
    assert!(second.prepare(batch(3, nominal(1)), &mut fuel).is_err());
}

#[test]
fn callback_error_and_unwind_roll_back_without_invalidating_other_receipts() {
    let world = Package::default();
    let mut fuel = budget();
    let mut session = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    let survivor = session.prepare(batch(1, boolean()), &mut fuel).unwrap();
    let rejected = session.prepare(batch(2, boolean()), &mut fuel).unwrap();
    assert!(matches!(
        session.commit_with(rejected, || Err::<(), _>("reject")),
        Err(StructPublicationError::Commit("reject"))
    ));
    let panicking = session.prepare(batch(2, boolean()), &mut fuel).unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = session.commit_with(panicking, || -> Result<(), ()> { panic!("abort commit") });
        }))
        .is_err()
    );
    assert!(session.prepare(batch(3, nominal(2)), &mut fuel).is_err());
    session.commit_with(survivor, || Ok::<_, ()>(())).unwrap();
    assert!(session.prepare(batch(2, nominal(1)), &mut fuel).is_ok());
}

#[test]
fn exhausted_attempts_can_retry_with_a_new_caller_allowance() {
    let world = Package::default();
    let mut fuel = budget();
    let mut session = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    for allowance in [0, 1, 4, 12] {
        let mut low = Budget {
            steps: allowance,
            depth: 128,
        };
        assert!(session.prepare(batch(1, boolean()), &mut low).is_err());
        assert!(low.steps <= allowance);
    }
    let receipt = session.prepare(batch(1, boolean()), &mut fuel).unwrap();
    fuel.steps = 0;
    session.commit_with(receipt, || Ok::<_, ()>(())).unwrap();
    assert_eq!(fuel.steps, 0);
    let mut renewed = budget();
    assert!(session.prepare(batch(2, nominal(1)), &mut renewed).is_ok());
}

#[test]
fn reused_admission_does_not_pay_world_preparation_again() {
    let world = Package {
        structs: (0..256).map(|n| batch(n, boolean())[0].clone()).collect(),
        ..Default::default()
    };
    let mut fuel = budget();
    let mut session = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    let before = fuel.steps;
    let first = session.prepare(batch(1000, nominal(0)), &mut fuel).unwrap();
    let cost = before - fuel.steps;
    session.commit_with(first, || Ok::<_, ()>(())).unwrap();
    let before = fuel.steps;
    let second = session
        .prepare(batch(1001, nominal(1000)), &mut fuel)
        .unwrap();
    assert_eq!(before - fuel.steps, cost);
    session.commit_with(second, || Ok::<_, ()>(())).unwrap();
    let before = fuel.steps;
    let mut cold = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    let _ = cold.prepare(batch(1000, nominal(0)), &mut fuel).unwrap();
    assert!(before - fuel.steps > cost + 256);
}

fn enum_batch(local: u32, field: Term) -> Arc<[nir::program::Enum]> {
    use nir::program::{Enum, Variant, VariantShape};
    vec![Enum {
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
    }]
    .into()
}

#[test]
fn alternating_definition_kinds_share_visibility_and_identity_space() {
    let world = Package::default();
    let mut fuel = budget();
    let mut session =
        comptime_interpretor::DefinitionAdmissionSession::new(&world, &mut fuel).unwrap();
    let receipt = session.prepare(batch(1, boolean()), &mut fuel).unwrap();
    session.commit_with(receipt, || Ok::<_, ()>(())).unwrap();
    let descriptions = enum_batch(2, nominal(1));
    let weak = Arc::downgrade(&descriptions);
    let receipt = session.prepare_enums(descriptions, &mut fuel).unwrap();
    assert!(
        receipt
            .to_string()
            .starts_with("(prepared-enum-admission (revision 1)")
    );
    assert!(session.prepare(batch(3, nominal(2)), &mut fuel).is_err());
    let before = fuel.steps;
    session
        .commit_enums_with(receipt, || Ok::<_, ()>(()))
        .unwrap();
    assert_eq!(before, fuel.steps);
    assert!(weak.upgrade().is_some());
    let receipt = session.prepare(batch(3, nominal(2)), &mut fuel).unwrap();
    session.commit_with(receipt, || Ok::<_, ()>(())).unwrap();
    assert!(
        session
            .prepare_enums(enum_batch(4, nominal(3)), &mut fuel)
            .is_ok()
    );
    assert!(session.prepare(batch(2, boolean()), &mut fuel).is_err());
    assert!(
        session
            .prepare_enums(enum_batch(1, boolean()), &mut fuel)
            .is_err()
    );
    drop(session);
    assert!(weak.upgrade().is_none());
}

#[test]
fn either_kind_of_commit_invalidates_other_kind_receipts() {
    use comptime_interpretor::EnumPublicationError;
    let world = Package::default();
    let mut fuel = budget();
    let mut session = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    let mut other = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    let called = Cell::new(false);
    let foreign = session
        .prepare_enums(enum_batch(1, boolean()), &mut fuel)
        .unwrap();
    assert!(
        matches!(other.commit_enums_with(foreign, || {called.set(true); Ok::<_, ()>(())}),
        Err(EnumPublicationError::Admission(error)) if error.kind == ErrorKind::InvalidCallPlan)
    );
    let stale = session
        .prepare_enums(enum_batch(1, boolean()), &mut fuel)
        .unwrap();
    let current = session.prepare(batch(2, boolean()), &mut fuel).unwrap();
    session.commit_with(current, || Ok::<_, ()>(())).unwrap();
    assert!(
        matches!(session.commit_enums_with(stale, || {called.set(true); Ok::<_, ()>(())}),
        Err(EnumPublicationError::Admission(error)) if error.kind == ErrorKind::InvalidCallPlan)
    );
    let stale = session.prepare(batch(3, boolean()), &mut fuel).unwrap();
    let current = session
        .prepare_enums(enum_batch(1, boolean()), &mut fuel)
        .unwrap();
    session
        .commit_enums_with(current, || Ok::<_, ()>(()))
        .unwrap();
    assert!(
        matches!(session.commit_with(stale, || {called.set(true); Ok::<_, ()>(())}),
        Err(StructPublicationError::Admission(error)) if error.kind == ErrorKind::InvalidCallPlan)
    );
    assert!(!called.get());
    assert!(
        session
            .prepare_enums(enum_batch(4, nominal(3)), &mut fuel)
            .is_err()
    );
}

#[test]
fn enum_callback_failure_unwind_and_dropped_receipts_publish_nothing() {
    use comptime_interpretor::EnumPublicationError;
    let world = Package::default();
    let mut fuel = budget();
    let mut session = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    let survivor = session.prepare(batch(1, boolean()), &mut fuel).unwrap();
    for mode in 0..3 {
        let descriptions = enum_batch(2, boolean());
        let weak = Arc::downgrade(&descriptions);
        let receipt = session.prepare_enums(descriptions, &mut fuel).unwrap();
        match mode {
            0 => drop(receipt),
            1 => assert!(matches!(
                session.commit_enums_with(receipt, || Err::<(), _>("reject")),
                Err(EnumPublicationError::Commit("reject"))
            )),
            _ => assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = session.commit_enums_with(receipt, || -> Result<(), ()> {
                        panic!("abort enum commit")
                    });
                }))
                .is_err()
            ),
        }
        assert!(weak.upgrade().is_none());
        assert!(session.prepare(batch(3, nominal(2)), &mut fuel).is_err());
    }
    session.commit_with(survivor, || Ok::<_, ()>(())).unwrap();
    assert!(
        session
            .prepare_enums(enum_batch(2, nominal(1)), &mut fuel)
            .is_ok()
    );
}

#[test]
fn enum_admission_budget_failure_is_retryable_and_commit_is_prepaid() {
    let world = Package::default();
    let mut fuel = budget();
    let mut session = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    let before = fuel.steps;
    drop(
        session
            .prepare_enums(enum_batch(1, boolean()), &mut fuel)
            .unwrap(),
    );
    let cost = before - fuel.steps;
    for steps in 0..cost {
        let mut low = Budget { steps, depth: 128 };
        assert!(
            session
                .prepare_enums(enum_batch(1, boolean()), &mut low)
                .is_err()
        );
        assert!(low.steps <= steps);
    }
    let receipt = session
        .prepare_enums(enum_batch(1, boolean()), &mut fuel)
        .unwrap();
    fuel.steps = 0;
    session
        .commit_enums_with(receipt, || Ok::<_, ()>(()))
        .unwrap();
    assert_eq!(fuel.steps, 0);
    assert!(session.prepare(batch(2, nominal(1)), &mut budget()).is_ok());
}

fn newtype_batch(local: u32, representation: Term) -> Arc<[nir::newtypes::Newtype]> {
    vec![nir::newtypes::Newtype {
        declaration: id(local),
        name: "GeneratedNewtype".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        capture_count: 0,
        requirements: vec![],
        capabilities: vec![],
        representation,
    }]
    .into()
}

#[test]
fn newtype_receipts_share_revision_visibility_and_identity_with_other_kinds() {
    use comptime_interpretor::NewtypePublicationError;
    let world = Package::default();
    let mut fuel = budget();
    let mut session = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    let structure = session.prepare(batch(1, boolean()), &mut fuel).unwrap();
    let stale = session
        .prepare_newtypes(newtype_batch(2, boolean()), &mut fuel)
        .unwrap();
    assert!(
        stale
            .to_string()
            .starts_with("(prepared-newtype-admission ")
    );
    session.commit_with(structure, || Ok::<_, ()>(())).unwrap();
    let called = Cell::new(false);
    assert!(
        matches!(session.commit_newtypes_with(stale, || {called.set(true); Ok::<_, ()>(())}),
        Err(NewtypePublicationError::Admission(error)) if error.kind == ErrorKind::InvalidCallPlan)
    );
    assert!(!called.get());
    assert!(
        session
            .prepare_newtypes(newtype_batch(1, boolean()), &mut fuel)
            .is_err()
    );
    let current = session
        .prepare_newtypes(newtype_batch(2, nominal(1)), &mut fuel)
        .unwrap();
    let competing = session
        .prepare_enums(enum_batch(3, boolean()), &mut fuel)
        .unwrap();
    session
        .commit_newtypes_with(current, || Ok::<_, ()>(()))
        .unwrap();
    assert!(
        session
            .commit_enums_with(competing, || Ok::<_, ()>(()))
            .is_err()
    );
    assert!(session.prepare(batch(2, boolean()), &mut fuel).is_err());
    assert!(
        session
            .prepare_enums(enum_batch(2, boolean()), &mut fuel)
            .is_err()
    );
    let consumer = session
        .prepare_enums(enum_batch(3, nominal(2)), &mut fuel)
        .unwrap();
    session
        .commit_enums_with(consumer, || Ok::<_, ()>(()))
        .unwrap();
    let next = session
        .prepare_newtypes(newtype_batch(4, nominal(3)), &mut fuel)
        .unwrap();
    session
        .commit_newtypes_with(next, || Ok::<_, ()>(()))
        .unwrap();
}

#[test]
fn newtype_receipt_failures_preserve_revision_and_release_ownership() {
    use comptime_interpretor::NewtypePublicationError;
    let world = Package::default();
    let mut fuel = budget();
    let mut session = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    let survivor = session.prepare(batch(1, boolean()), &mut fuel).unwrap();
    for mode in 0..3 {
        let definitions = newtype_batch(2, boolean());
        let weak = Arc::downgrade(&definitions);
        let receipt = session.prepare_newtypes(definitions, &mut fuel).unwrap();
        match mode {
            0 => drop(receipt),
            1 => assert!(matches!(
                session.commit_newtypes_with(receipt, || Err::<(), _>("reject")),
                Err(NewtypePublicationError::Commit("reject"))
            )),
            _ => assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = session
                        .commit_newtypes_with(receipt, || -> Result<(), ()> { panic!("abort") });
                }))
                .is_err()
            ),
        }
        assert!(weak.upgrade().is_none());
        assert!(session.prepare(batch(3, nominal(2)), &mut fuel).is_err());
    }
    session.commit_with(survivor, || Ok::<_, ()>(())).unwrap();
    let foreign = session
        .prepare_newtypes(newtype_batch(2, nominal(1)), &mut fuel)
        .unwrap();
    let mut other = StructAdmissionSession::new(&world, &mut fuel).unwrap();
    assert!(
        matches!(other.commit_newtypes_with(foreign, || Ok::<_, ()>(())),
        Err(NewtypePublicationError::Admission(error)) if error.kind == ErrorKind::InvalidCallPlan)
    );
    let owned = newtype_batch(2, nominal(1));
    let weak = Arc::downgrade(&owned);
    let receipt = session.prepare_newtypes(owned, &mut fuel).unwrap();
    session
        .commit_newtypes_with(receipt, || Ok::<_, ()>(()))
        .unwrap();
    assert!(weak.upgrade().is_some());
    drop(session);
    assert!(weak.upgrade().is_none());
}
