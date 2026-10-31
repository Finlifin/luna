//! Callable reflection remains usable through the public VM API.
use comptime_interpretor::{Budget, CallableReflection, Interpreter};
use nir::{Primitive, Term, program::Package};

#[test]
fn public_callable_view_preserves_the_signature_without_invocation() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    let view: CallableReflection = vm.reflect_callable(&signature).unwrap();
    assert_eq!(view.subject, signature);
    assert_eq!(view.signature(), &signature);
    assert!(!view.is_comptime());
    assert!(vm.events().is_empty());
}
