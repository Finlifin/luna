use nir::{Primitive, Term, program::scalar_observation_shape};

#[test]
fn scalar_products_do_not_admit_pointer_or_managed_fields() {
    let symbol = Term::Primitive(Primitive::Symbol);
    assert!(scalar_observation_shape(&Term::Tuple(vec![
        symbol.clone(),
        Term::Record(vec![("flag".into(), Term::Primitive(Primitive::Bool))]),
    ])));
    for forbidden in [
        Term::Pointer(Box::new(symbol.clone())),
        Term::Primitive(Primitive::Str),
        Term::Primitive(Primitive::Type),
    ] {
        assert!(!scalar_observation_shape(&forbidden));
        assert!(!scalar_observation_shape(&Term::Tuple(vec![
            symbol.clone(),
            forbidden
        ])));
    }
}

#[test]
fn observation_shape_budget_is_bounded_and_does_not_poison_later_queries() {
    let scalar = Term::Primitive(Primitive::Bool);
    assert!(!scalar_observation_shape(&Term::Tuple(vec![
        scalar.clone();
        4096
    ])));
    assert!(scalar_observation_shape(&Term::Tuple(vec![scalar; 4])));
}
