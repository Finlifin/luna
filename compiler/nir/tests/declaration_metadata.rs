use nir::normal_forms::{self, Head, Kind, Parameter, Role};
use nir::program::{LocalId, Package, TypePredicate};
use nir::{DeclarationId, Primitive, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn opaque(local: u32, ty: Term) -> Head {
    Head {
        declaration: id(local),
        role: Role::Opaque,
        collector: None,
        parameters: vec![Parameter {
            declaration: id(local + 100),
            binding: LocalId {
                owner: local,
                index: 0,
            },
            kind: Kind::Value,
            ty,
        }],
        requirements: vec![],
    }
}
fn wrapper() -> nir::newtypes::Newtype {
    nir::newtypes::Newtype {
        declaration: id(1),
        name: "Wrapper".into(),
        type_parameters: vec![id(2)],
        parameter_types: vec![],
        capture_count: 0,
        requirements: vec![],
        capabilities: vec![],
        representation: Term::Rigid {
            declaration: id(2),
            sort: Sort::Type,
        },
    }
}

#[test]
fn newtype_resources_are_not_invalid_representations_and_retries_are_fresh() {
    use nir::newtypes::{ValidationError, validate_in};
    let mut package = Package {
        newtypes: vec![wrapper()],
        ..Default::default()
    };
    let mut fuel = 10_000;
    validate_in(&package, &mut fuel).unwrap();
    let cost = 10_000 - fuel;
    let mut exact = cost;
    validate_in(&package, &mut exact).unwrap();
    assert_eq!(exact, 0);
    for allowance in 0..cost {
        assert_eq!(
            validate_in(&package, &mut allowance.clone()),
            Err(ValidationError::Budget)
        );
    }
    validate_in(&package, &mut cost.clone()).unwrap();
    package.newtypes[0].representation = Term::Rigid {
        declaration: id(3),
        sort: Sort::Type,
    };
    assert_eq!(
        validate_in(&package, &mut 10_000),
        Err(ValidationError::Invalid(
            "newtype representation has an undeclared captured parameter"
        ))
    );
    package.newtypes[0] = wrapper();
    validate_in(&package, &mut 10_000).unwrap();
}

#[test]
fn borrowed_capture_inspection_keeps_sort_conflicts_and_nested_predicates() {
    let mut value = wrapper();
    let type_input = value.representation.clone();
    let wrong_sort = Term::Rigid {
        declaration: id(2),
        sort: Sort::ComptimeValue,
    };
    value.requirements = vec![TypePredicate::And(vec![TypePredicate::Equal(
        type_input, wrong_sort,
    )])];
    let mut package = Package {
        newtypes: vec![value],
        ..Default::default()
    };
    assert_eq!(
        nir::newtypes::validate(&package),
        Err("newtype contract has an undeclared captured parameter")
    );
    // Each operand is observed separately, so the previous pair has a scope
    // mismatch rather than an intra-term sort conflict. Test both explicitly.
    package.newtypes[0].requirements = vec![TypePredicate::Equal(
        Term::Tuple(vec![
            Term::Rigid {
                declaration: id(2),
                sort: Sort::Type,
            },
            Term::Rigid {
                declaration: id(2),
                sort: Sort::ComptimeValue,
            },
        ]),
        Term::Primitive(Primitive::Bool),
    )];
    assert_eq!(
        nir::newtypes::validate(&package),
        Err("invalid newtype contract parameters")
    );
}

#[test]
fn all_head_domains_and_retained_contracts_share_one_allowance() {
    let mut head = opaque(10, Term::Primitive(Primitive::Bool));
    head.requirements = vec![TypePredicate::And(vec![TypePredicate::Bool(true); 20])];
    let mut package = Package {
        normal_forms: vec![head],
        ..Default::default()
    };
    let mut fuel = 10_000;
    normal_forms::validate_in(&package, &mut fuel).unwrap();
    let cost = 10_000 - fuel;
    assert_eq!(
        normal_forms::validate_in(&package, &mut (cost - 1)),
        Err(normal_forms::ValidationError::Budget)
    );
    normal_forms::validate_in(&package, &mut cost.clone()).unwrap();
    package
        .normal_forms
        .push(opaque(11, Term::Primitive(Primitive::Bool)));
    assert_eq!(
        normal_forms::validate_in(&package, &mut cost.clone()),
        Err(normal_forms::ValidationError::Budget)
    );
    normal_forms::validate_in(&package, &mut 10_000).unwrap();
    package.normal_forms[1].parameters[0].ty =
        Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)));
    assert_eq!(
        normal_forms::validate_in(&package, &mut 10_000),
        Err(normal_forms::ValidationError::Invalid(
            "normal-form value input has an unsupported or forward-dependent type"
        ))
    );
}

#[test]
fn domain_shape_reports_resources_separately_and_uses_the_given_allowance() {
    let domain = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 100_001]);
    let inspect = |ty, fuel: &mut usize| {
        normal_forms::supported_input_type_with_collections_budgeted(
            ty,
            &[],
            |_| false,
            |_| false,
            |_| false,
            None,
            fuel,
        )
    };
    assert!(matches!(
        inspect(&domain, &mut 100),
        Err(nir::type_computations::Error::Budget)
    ));
    assert_eq!(inspect(&domain, &mut 1_000_000).unwrap(), true);
    let unavailable = Term::Rigid {
        declaration: id(99),
        sort: Sort::Type,
    };
    assert_eq!(inspect(&unavailable, &mut 100).unwrap(), false);
    assert_eq!(
        inspect(&Term::Primitive(Primitive::Bool), &mut 100).unwrap(),
        true
    );
}
