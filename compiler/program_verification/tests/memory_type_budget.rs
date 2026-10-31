use nir::{Primitive, Term};
use program_verification::memory::bound_type;

#[test]
fn memory_type_admission_checks_closure_contexts_and_record_names() {
    let deep = (0..64).fold(Term::Primitive(Primitive::Bool), |ty, _| {
        Term::Pointer(Box::new(ty))
    });
    let context = Term::ClosureContextValue(vec![deep]);
    assert!(
        bound_type(&context, &mut 10_000)
            .unwrap_err()
            .contains("budget")
    );
    let term = Term::Record(vec![("x".repeat(1000), Term::Primitive(Primitive::Bool))]);
    assert!(bound_type(&term, &mut 100).is_err());
    bound_type(&term, &mut 2000).unwrap();
}
