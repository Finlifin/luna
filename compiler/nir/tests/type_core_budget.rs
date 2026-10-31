use nir::{DeclarationId, Parameter, Primitive, RuntimeId, Sort, Term, TypeError, TypeInterner};
use std::sync::Arc;

fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn dependent(value: Term) -> Term {
    Term::NormalForm {
        sort: Sort::Type,
        head: DeclarationId {
            package: 1,
            local: 1,
        },
        arguments: vec![value],
    }
}
fn deep() -> Term {
    (0..140).fold(boolean(), |ty, _| Term::Pointer(Box::new(ty)))
}

#[test]
fn core_admission_bounds_body_contracts_and_payloads_before_publication() {
    let mut pool = TypeInterner::default();
    for (holes, body) in [
        (vec![], deep()),
        (
            vec![Parameter {
                sort: Sort::Runtime,
                ty: deep(),
            }],
            boolean(),
        ),
        (vec![], dependent(Term::String("x".repeat(100_001).into()))),
    ] {
        assert_eq!(pool.intern(holes, body), Err(TypeError::ConstructionBudget));
        assert!(pool.is_empty());
    }
    assert_eq!(
        pool.intern_with_budget(vec![], boolean(), &mut 0),
        Err(TypeError::ConstructionBudget)
    );
    let first = pool
        .intern_with_budget(vec![], boolean(), &mut 100)
        .unwrap();
    // Even a hit must inspect the submitted structure under this caller's fuel.
    assert_eq!(
        pool.intern_with_budget(vec![], boolean(), &mut 0),
        Err(TypeError::ConstructionBudget)
    );
    let second = pool
        .intern_with_budget(vec![], boolean(), &mut 100)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(pool.len(), 1);
}

#[test]
fn runtime_abstraction_exhaustion_is_atomic_and_retry_preserves_identity() {
    let mut pool = TypeInterner::default();
    let id = RuntimeId {
        instance: 1,
        local: 0,
    };
    let body = dependent(Term::Runtime(id));
    let contracts = [(id, boolean())];
    // Exercise every failure boundary through abstraction and core admission.
    let mut failures = 0;
    let first = (0..100)
        .find_map(|mut budget| {
            match pool.abstract_runtime_with_budget(&body, &contracts, &mut budget) {
                Ok(reference) => Some(reference),
                Err(TypeError::ConstructionBudget) => {
                    failures += 1;
                    assert!(pool.is_empty());
                    None
                }
                Err(error) => panic!("unexpected admission error: {error}"),
            }
        })
        .expect("small dependency must fit");
    assert!(failures > 1);
    assert_eq!(first.materialize().unwrap(), body);
    let second_id = RuntimeId {
        instance: 2,
        local: 0,
    };
    let second = pool
        .abstract_runtime_with_budget(
            &dependent(Term::Runtime(second_id)),
            &[(second_id, boolean())],
            &mut 1000,
        )
        .unwrap();
    assert!(Arc::ptr_eq(first.core(), second.core()));
    assert_ne!(first, second);
    assert_eq!(pool.len(), 1);
    assert_eq!(
        pool.abstract_runtime(&body, &[(id, deep())]),
        Err(TypeError::ConstructionBudget)
    );
    assert_eq!(
        pool.abstract_runtime(&deep(), &contracts),
        Err(TypeError::ConstructionBudget)
    );
    assert_eq!(pool.len(), 1);
}
