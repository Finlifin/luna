//! Frozen callable evidence uses immutable cores, not active type state.
use iir::{CallableIdentity, InstanceKey};
use nir::{DeclarationId, Primitive, Term, TypeInterner};
fn key(local: u32) -> InstanceKey {
    InstanceKey {
        declaration: DeclarationId { package: 1, local },
        arguments: vec![],
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
