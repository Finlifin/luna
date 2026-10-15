use nir::newtypes::Newtype;
use nir::program::Package;
use nir::{DeclarationId, Primitive, Sort, Term};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn generic() -> Newtype {
    Newtype {
        declaration: id(1),
        name: "Id\"\\\n".into(),
        type_parameters: vec![id(2)],
        representation: Term::Rigid {
            declaration: id(2),
            sort: Sort::Type,
        },
        requirements: vec![],
        capabilities: vec![],
    }
}
#[test]
fn representation_substitution_preserves_nominal_and_opaque_heads() {
    let definition = generic();
    for argument in [
        Term::Primitive(Primitive::Bool),
        Term::Nominal {
            declaration: id(3),
            arguments: vec![],
        },
        Term::NormalForm {
            head: id(4),
            sort: Sort::Type,
            arguments: vec![Term::Primitive(Primitive::Bool)],
        },
    ] {
        assert_eq!(
            definition
                .instantiate_representation(&[argument.clone()])
                .unwrap(),
            argument
        );
        assert_ne!(
            Term::Nominal {
                declaration: definition.declaration,
                arguments: vec![argument.clone()]
            },
            argument
        );
    }
    assert!(definition.instantiate_representation(&[]).is_err());
    assert!(
        definition
            .instantiate_representation(&[Term::Bool(true)])
            .is_err()
    );
    assert!(
        definition
            .instantiate_representation(&[
                Term::Primitive(Primitive::Bool),
                Term::Primitive(Primitive::Bool)
            ])
            .is_err()
    );
}
#[test]
fn representation_instantiation_requires_the_callers_scope() {
    let definition = generic();
    let bound = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    assert!(
        definition
            .instantiate_representation(&[bound.clone()])
            .is_err()
    );
    assert_eq!(
        definition
            .instantiate_representation_in_scope(&[bound.clone()], &[vec![Sort::Type]])
            .unwrap(),
        bound
    );
}
#[test]
fn metadata_rejects_duplicate_identity_parameters_and_non_types() {
    let definition = generic();
    let mut package = Package {
        newtypes: vec![definition.clone()],
        ..Default::default()
    };
    assert!(nir::normal_forms::validate(&package).is_ok());
    package.newtypes.push(definition.clone());
    assert!(nir::normal_forms::validate(&package).is_err());
    package.newtypes.pop();
    package.newtypes[0].type_parameters.push(id(2));
    assert!(nir::normal_forms::validate(&package).is_err());
    package.newtypes[0] = definition.clone();
    package.structs.push(nir::program::Struct {
        declaration: definition.declaration,
        name: "collision".into(),
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        fields: vec![],
    });
    assert!(nir::normal_forms::validate(&package).is_err());
    package.structs.clear();
    package.newtypes[0] = definition;
    package.newtypes[0].representation = Term::Bool(true);
    assert!(nir::normal_forms::validate(&package).is_err());
}
#[test]
fn descriptor_dump_escapes_names_and_keeps_representation_separate() {
    let text = generic().to_string();
    assert!(text.starts_with("(newtype (id "));
    assert!(text.contains("(representation (rigid"), "{text}");
    assert!(text.contains("Id\\\"\\\\\\n"), "{text}");
}

fn nominal(local: u32, arguments: Vec<Term>) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments,
    }
}
fn wrapper(local: u32, representation: Term) -> Newtype {
    Newtype {
        declaration: id(local),
        name: format!("Wrapper{local}"),
        representation,
        type_parameters: vec![],
        capabilities: vec![],
        requirements: vec![],
    }
}
#[test]
fn conversion_matrix_follows_only_declared_direct_edges() {
    use nir::newtypes::{Direction, conversion};
    let byte = Term::Primitive(Primitive::U8);
    let word = Term::Primitive(Primitive::U16);
    let definitions = vec![
        wrapper(10, byte.clone()),
        wrapper(11, byte.clone()),
        wrapper(12, nominal(10, vec![])),
        wrapper(13, word.clone()),
    ];
    let types = [
        byte,
        word,
        nominal(10, vec![]),
        nominal(11, vec![]),
        nominal(12, vec![]),
        nominal(13, vec![]),
    ];
    // Independent adjacency table: neither equal-width siblings nor the
    // grandparent/grandchild relationship is an explicit conversion edge.
    let edges = [(0, 2, 10), (0, 3, 11), (2, 4, 12), (1, 5, 13)];
    for (i, source) in types.iter().enumerate() {
        for (j, target) in types.iter().enumerate() {
            let expected = edges.iter().find_map(|&(from, to, owner)| {
                if (i, j) == (from, to) {
                    Some((Direction::Wrap, id(owner)))
                } else if (i, j) == (to, from) {
                    Some((Direction::Unwrap, id(owner)))
                } else {
                    None
                }
            });
            let actual = conversion(&definitions, source, target, &[], &mut 10_000).unwrap();
            assert_eq!(
                actual.as_ref().map(|p| (p.direction(), p.declaration())),
                expected,
                "{i}->{j}"
            );
            if let Some(plan) = actual {
                assert_eq!(plan.source(), source);
                assert_eq!(plan.target(), target);
                assert!(plan.to_string().starts_with("(newtype-conversion "));
            }
        }
    }
}
#[test]
fn conversions_preserve_generic_arguments_scope_and_opaque_heads() {
    use nir::newtypes::{ConversionError, conversion};
    let definition = generic();
    let bound = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let head = Term::NormalForm {
        head: id(40),
        sort: Sort::Type,
        arguments: vec![bound.clone()],
    };
    let wrapped = nominal(1, vec![head.clone()]);
    assert!(
        conversion(
            &[definition.clone()],
            &head,
            &wrapped,
            &[vec![Sort::Type]],
            &mut 10_000
        )
        .unwrap()
        .is_some()
    );
    assert!(matches!(
        conversion(&[definition.clone()], &head, &wrapped, &[], &mut 10_000),
        Err(ConversionError::Type(_))
    ));
    assert!(
        conversion(
            &[definition.clone()],
            &bound,
            &wrapped,
            &[vec![Sort::Type]],
            &mut 10_000
        )
        .unwrap()
        .is_none()
    );
    let wrong_arguments = nominal(1, vec![Term::Primitive(Primitive::Bool)]);
    assert!(
        conversion(
            &[definition],
            &head,
            &wrong_arguments,
            &[vec![Sort::Type]],
            &mut 10_000
        )
        .unwrap()
        .is_none()
    );
}
#[test]
fn malformed_conversion_queries_are_errors_not_absent_edges() {
    use nir::newtypes::{ConversionError, conversion};
    let definition = generic();
    let boolean = Term::Primitive(Primitive::Bool);
    for malformed in [nominal(1, vec![]), nominal(1, vec![Term::Bool(true)])] {
        assert!(matches!(
            conversion(
                &[definition.clone()],
                &malformed,
                &boolean,
                &[],
                &mut 10_000
            ),
            Err(ConversionError::Type(_))
        ));
    }
    assert!(matches!(
        conversion(
            &[definition.clone()],
            &Term::Bool(true),
            &boolean,
            &[],
            &mut 10_000
        ),
        Err(ConversionError::Type(_))
    ));
    let wrapped = nominal(1, vec![boolean.clone()]);
    assert_eq!(
        conversion(
            &[definition.clone(), definition],
            &wrapped,
            &boolean,
            &[],
            &mut 10_000
        ),
        Err(ConversionError::DuplicateIdentity(id(1)))
    );
}
#[test]
fn conversion_budget_failure_is_recoverable_and_never_proves_absence() {
    use nir::newtypes::{ConversionError, conversion};
    let definition = generic();
    let representation = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 64]);
    let wrapped = nominal(1, vec![representation.clone()]);
    let mut failures = 0;
    for available in [0, 1, 16, 128] {
        let mut fuel = available;
        let result = conversion(
            &[definition.clone()],
            &wrapped,
            &representation,
            &[],
            &mut fuel,
        );
        assert_eq!(result, Err(ConversionError::Budget));
        assert_eq!(fuel, 0);
        failures += 1;
    }
    assert_eq!(failures, 4);
    assert!(
        conversion(&[definition], &wrapped, &representation, &[], &mut 10_000)
            .unwrap()
            .is_some()
    );
}
#[test]
fn representation_expansion_is_charged_before_substituting_repeated_arguments() {
    use nir::newtypes::ConversionError;
    let mut definition = generic();
    definition.representation = Term::Tuple(vec![definition.representation; 128]);
    let argument = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 128]);
    let mut fuel = 1_000;
    assert_eq!(
        definition.representation_budgeted(&[argument.clone()], &[], &mut fuel),
        Err(ConversionError::Budget)
    );
    assert_eq!(fuel, 0);
    let value = definition
        .representation_budgeted(&[argument.clone()], &[], &mut 100_000)
        .unwrap();
    assert_eq!(value, Term::Tuple(vec![argument; 128]));
}

#[test]
fn package_admission_bounds_newtype_representations_before_recursive_copying() {
    let mut representation = Term::Primitive(Primitive::Bool);
    for _ in 0..140 {
        representation = Term::Pointer(Box::new(representation));
    }
    let mut package = Package {
        newtypes: vec![wrapper(10, representation)],
        ..Default::default()
    };
    assert!(nir::normal_forms::validate(&package).is_err());
    package.newtypes[0].representation = Term::Primitive(Primitive::Bool);
    assert!(nir::normal_forms::validate(&package).is_ok());
}
