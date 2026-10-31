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
