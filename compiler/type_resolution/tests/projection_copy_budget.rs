use nir::{DeclarationId, Primitive, Sort, Term};
use type_resolution::associated::charge_projection_copy;

#[test]
fn projection_copy_charges_inputs_hidden_in_callable_values() {
    let id = DeclarationId {
        package: 1,
        local: 1,
    };
    let deep = (0..64).fold(Term::Primitive(Primitive::Bool), |ty, _| {
        Term::Pointer(Box::new(ty))
    });
    let term = Term::FunctionValue {
        declaration: id,
        arguments: vec![deep],
    };
    assert!(
        charge_projection_copy(&term, &mut 10_000)
            .unwrap_err()
            .contains("budget")
    );
    let term = Term::ClosureSnapshot {
        factory: id,
        arguments: vec![Term::NormalForm {
            head: id,
            sort: Sort::Type,
            arguments: vec![Term::String("x".repeat(1000).into())],
        }],
    };
    assert!(charge_projection_copy(&term, &mut 100).is_err());
    charge_projection_copy(&term, &mut 2000).unwrap();
}
