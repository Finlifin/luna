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
        "(struct-admission-session (revision 1))"
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
