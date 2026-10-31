//! Frozen callable evidence uses immutable cores, not active type state.
use iir::{CallableIdentity, InstanceKey};
use nir::{DeclarationId, Primitive, Term, TypeInterner};
fn key(local: u32) -> InstanceKey {
    InstanceKey {
        declaration: DeclarationId { package: 1, local },
        arguments: vec![].into(),
    }
}
fn signature(result: Term) -> Term {
    Term::Function {
        parameters: vec![],
        result: Box::new(result),
    }
}
#[test]
fn signature_interning_is_bounded_and_failed_admission_does_not_publish() {
    let mut pool = TypeInterner::default();
    let ty = signature(Term::Primitive(Primitive::Bool));
    assert!(CallableIdentity::intern(key(1), ty.clone(), &mut pool, &mut 0).is_err());
    assert!(pool.is_empty());
    let unresolved = signature(Term::Rigid {
        declaration: key(9).declaration,
        sort: nir::Sort::Type,
    });
    assert!(CallableIdentity::intern(key(2), unresolved, &mut pool, &mut 10_000).is_err());
    assert!(pool.is_empty());
    let first = CallableIdentity::intern(key(1), ty.clone(), &mut pool, &mut 10_000).unwrap();
    let second = CallableIdentity::intern(key(2), ty.clone(), &mut pool, &mut 10_000).unwrap();
    assert_eq!(pool.len(), 1);
    assert_ne!(first.key, second.key);
    assert!(std::sync::Arc::ptr_eq(
        first.signature.core(),
        second.signature.core()
    ));
    assert_eq!(first.closed_signature().unwrap(), &ty);
    assert!(first.to_string().contains("type-ref"));
}

#[test]
fn repeated_type_links_do_not_rescan_the_callable_catalog() {
    let mut pool = TypeInterner::default();
    let mut graph = iir::Package::default();
    for local in 0..256 {
        let mut instance = key(local);
        if local != 0 {
            instance.arguments = vec![Term::FunctionValue {
                declaration: key(0).declaration,
                arguments: vec![],
            }]
            .into();
        }
        graph.callable_identities.push(
            CallableIdentity::intern(
                instance,
                signature(Term::Primitive(Primitive::Bool)),
                &mut pool,
                &mut 1000,
            )
            .unwrap(),
        );
    }
    graph.validate(&mut (256 * 64)).unwrap();
    let mut missing = graph.clone();
    missing.callable_identities.remove(0);
    assert!(
        missing
            .validate(&mut (256 * 64))
            .unwrap_err()
            .contains("no signature evidence")
    );
    assert!(graph.validate(&mut 1).is_err());
    graph.validate(&mut (256 * 64)).unwrap();
}

#[test]
fn index_admission_bounds_keys_before_hashing_and_recovers() {
    let mut graph = iir::Package::default();
    let mut pool = TypeInterner::default();
    let identity = CallableIdentity::intern(
        key(1),
        signature(Term::Primitive(Primitive::Bool)),
        &mut pool,
        &mut 1000,
    )
    .unwrap();
    graph.callable_identities.push(identity.clone());
    assert!(
        graph
            .validate_index_with_budget(&mut 0)
            .unwrap_err()
            .contains("budget")
    );
    graph.validate_index_with_budget(&mut 1000).unwrap();
    let mut deep = Term::Primitive(Primitive::Bool);
    for _ in 0..140 {
        deep = Term::Pointer(Box::new(deep));
    }
    graph.callable_identities[0].key.arguments = vec![deep].into();
    assert!(
        graph
            .validate_index_with_budget(&mut 100_000)
            .unwrap_err()
            .contains("budget")
    );
    graph.callable_identities[0] = identity.clone();
    graph.callable_identities.push(identity);
    assert!(
        graph
            .validate_index_with_budget(&mut 1000)
            .unwrap_err()
            .contains("duplicate")
    );
    graph.callable_identities.pop();
    graph.validate(&mut 1000).unwrap();
}

#[test]
fn instance_input_frames_share_storage_but_compare_by_content() {
    use std::sync::Arc;
    let mut first = key(7);
    first.arguments = vec![
        Term::Bool(true),
        Term::String("retained".repeat(256).into()),
    ]
    .into();
    let cloned = first.clone();
    assert!(Arc::ptr_eq(&first.arguments, &cloned.arguments));
    let mut independent = key(7);
    independent.arguments = first.arguments.to_vec().into();
    assert!(!Arc::ptr_eq(&first.arguments, &independent.arguments));
    assert_eq!(first, independent);
    let identities = std::collections::HashSet::from([first.clone()]);
    assert!(identities.contains(&independent));
    Arc::make_mut(&mut independent.arguments)[0] = Term::Bool(false);
    assert_ne!(first, independent);
    assert!(!identities.contains(&independent));
    assert_eq!(cloned.arguments[0], Term::Bool(true));
    let mut pool = TypeInterner::default();
    let identity = CallableIdentity::intern(
        first,
        signature(Term::Primitive(Primitive::Bool)),
        &mut pool,
        &mut 1000,
    )
    .unwrap();
    let graph = iir::Package {
        callable_identities: vec![identity],
        ..Default::default()
    };
    graph.validate(&mut 100_000).unwrap();
    let copied = graph.clone();
    assert!(Arc::ptr_eq(
        &graph.callable_identities[0].key.arguments,
        &copied.callable_identities[0].key.arguments
    ));
    assert_eq!(graph.to_string(), copied.to_string());
}
