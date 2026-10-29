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

#[test]
fn normal_tuple_observations_require_registered_layout_and_scalar_fields() {
    use nir::{
        DeclarationId, Sort,
        normal_forms::{Head, Role},
        program::{Package, scalar_observation_shape_in_package},
    };
    let head = DeclarationId {
        package: 91,
        local: 1,
    };
    let tuple = |fields| Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(fields)],
    };
    let scalar = Term::Primitive(Primitive::Bool);
    let nested = tuple(vec![tuple(vec![scalar.clone()]), scalar.clone()]);
    let mut package = Package::default();
    assert!(!scalar_observation_shape_in_package(
        Some(&package),
        &nested
    ));
    package.normal_forms.push(Head {
        collector: None,
        declaration: head,
        role: Role::Opaque,
        parameters: vec![],
        requirements: vec![],
    });
    assert!(!scalar_observation_shape_in_package(
        Some(&package),
        &nested
    ));
    package.normal_forms[0].role = Role::Tuple;
    assert!(scalar_observation_shape_in_package(Some(&package), &nested));
    assert!(!scalar_observation_shape(&nested));
    for forbidden in [
        Term::Pointer(Box::new(scalar.clone())),
        Term::Primitive(Primitive::Str),
        Term::Primitive(Primitive::Type),
    ] {
        assert!(!scalar_observation_shape_in_package(
            Some(&package),
            &tuple(vec![forbidden])
        ));
    }
    assert!(!scalar_observation_shape_in_package(
        Some(&package),
        &tuple(vec![scalar; 4096])
    ));
    assert!(scalar_observation_shape_in_package(Some(&package), &nested));
}
