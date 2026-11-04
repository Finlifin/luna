use super::*;
use nir::program::{Enum, Struct, Variant, VariantShape};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 9, local }
}

fn structure(fields: Vec<Term>) -> Struct {
    Struct {
        declaration: id(1),
        name: "Fields".into(),
        type_parameters: vec![id(2)],
        parameter_types: vec![Term::Primitive(Primitive::Type)],
        requirements: vec![],
        capabilities: vec![],
        fields: fields
            .into_iter()
            .enumerate()
            .map(|(i, ty)| (format!("slot{i}"), ty))
            .collect(),
    }
}

fn enumeration(fields: Vec<Vec<Term>>) -> Enum {
    Enum {
        declaration: id(1),
        name: "Choices".into(),
        type_parameters: vec![id(2)],
        parameter_types: vec![Term::Primitive(Primitive::Type)],
        requirements: vec![],
        capabilities: vec![],
        groups: vec![],
        encoding: None,
        symbol_encoding: None,
        is_intrinsic: false,
        variants: fields
            .into_iter()
            .enumerate()
            .map(|(i, fields)| Variant {
                name: format!("branch{i}"),
                shape: if fields.is_empty() {
                    VariantShape::Unit
                } else {
                    VariantShape::Tuple
                },
                fields,
            })
            .collect(),
    }
}

fn parameter() -> Term {
    Term::Rigid {
        declaration: id(2),
        sort: nir::Sort::Type,
    }
}

fn applied(argument: Term) -> Term {
    Term::Nominal {
        declaration: id(1),
        arguments: vec![argument],
    }
}

#[test]
fn owned_fields_charge_expanded_arguments_before_delivery() {
    let mut package = Package::default();
    package.structs.push(structure(vec![parameter(); 64]));
    let argument = Term::Record(
        (0..32)
            .map(|i| (format!("field{i}"), Term::Primitive(Primitive::Bool)))
            .collect(),
    );
    let ty = applied(argument.clone());
    let failure = owned_fields(Some(&package), &ty, &mut 2000)
        .err()
        .expect("expanded fields must consume the query budget");
    assert!(failure.contains("budget"), "{failure}");
    assert_eq!(
        owned_fields(Some(&package), &ty, &mut 100_000).unwrap(),
        vec![argument; 64]
    );
    assert!(owned_fields(Some(&package), &applied(Term::Bool(true)), &mut 100_000).is_err());
    owned_fields(Some(&package), &ty, &mut 100_000).unwrap();
}

#[test]
fn memory_representation_charges_empty_nominal_alternatives() {
    let mut package = Package::default();
    package.enums.push(enumeration(vec![vec![]; 128]));
    let ty = applied(Term::Primitive(Primitive::Bool));
    let failure = data_representation(Some(&package), &ty, &mut 16)
        .expect_err("empty alternatives still require bounded observation");
    assert!(failure.contains("budget"), "{failure}");
    assert!(data_representation(Some(&package), &ty, &mut 10_000).unwrap());
}

#[test]
fn pointer_free_layout_charges_empty_nominal_alternatives() {
    let mut package = Package::default();
    package.enums.push(enumeration(vec![vec![]; 128]));
    let ty = applied(Term::Primitive(Primitive::Bool));
    let failure = require_pointer_free_owned(Some(&package), &ty, &mut 16, 0)
        .expect_err("empty alternatives still require bounded observation");
    assert!(failure.contains("budget"), "{failure}");
    require_pointer_free_owned(Some(&package), &ty, &mut 10_000, 0).unwrap();
}

#[test]
fn nominal_pointer_layout_preserves_empty_branches_and_storage_guards() {
    let mut package = Package::default();
    package.enums.push(enumeration(vec![
        vec![],
        vec![parameter()],
        vec![],
        vec![Term::Optional(Box::new(parameter()))],
    ]));
    let boolean = Term::Primitive(Primitive::Bool);
    let pointer = Term::Pointer(Box::new(boolean.clone()));
    let ty = applied(pointer.clone());
    let leaves = pointer_input_layout(Some(&package), &ty, &mut 100_000).unwrap();
    assert_eq!(leaves.len(), 2);
    assert_eq!(leaves[0].input_fields, vec![VARIANT_FIELD, 1, 0]);
    assert_eq!(leaves[0].storage_fields, vec![0]);
    assert_eq!(leaves[0].guards, vec![(vec![], 1)]);
    assert_eq!(
        leaves[1].input_fields,
        vec![VARIANT_FIELD, 3, 0, VARIANT_FIELD, 1, 0]
    );
    assert_eq!(leaves[1].storage_fields, vec![0, 0]);
    assert_eq!(leaves[1].guards, vec![(vec![], 3), (vec![0], 1)]);
    for leaf in &leaves {
        assert_eq!(leaf.pointee, boolean);
        let projection =
            project_owned_storage(Some(&package), &ty, &leaf.input_fields, &mut 100_000).unwrap();
        assert_eq!(projection.guards, leaf.guards);
        assert_eq!(projection.ty, pointer);
        assert_eq!(
            owned_path_type(Some(&package), &ty, &leaf.input_fields, &mut 100_000).unwrap(),
            pointer
        );
    }
    for tag in [0, 2] {
        assert!(
            owned_path_type(Some(&package), &ty, &[VARIANT_FIELD, tag, 0], &mut 100_000).is_err()
        );
    }
    assert!(!data_representation(Some(&package), &ty, &mut 100_000).unwrap());
    assert!(require_pointer_free_owned(Some(&package), &ty, &mut 100_000, 0).is_err());
    pointer_input_layout(Some(&package), &ty, &mut 100_000).unwrap();
}

#[test]
fn empty_nominal_payloads_still_validate_the_full_argument_frame() {
    for is_enum in [false, true] {
        let mut package = Package::default();
        if is_enum {
            package.enums.push(enumeration(vec![vec![], vec![]]));
        } else {
            package.structs.push(structure(vec![]));
        }
        let valid = applied(Term::Primitive(Primitive::Bool));
        let invalid = applied(Term::Bool(true));
        let check = |ty: &Term, fuel: &mut usize| {
            if is_enum {
                owned_variant_fields(Some(&package), ty, fuel).map(|_| ())
            } else {
                owned_fields(Some(&package), ty, fuel).map(|_| ())
            }
        };
        for ty in [&invalid, &valid] {
            assert_eq!(check(ty, &mut 10_000).is_ok(), ty == &valid);
            assert_eq!(
                data_representation(Some(&package), ty, &mut 10_000).is_ok(),
                ty == &valid
            );
            assert_eq!(
                require_pointer_free_owned(Some(&package), ty, &mut 10_000, 0).is_ok(),
                ty == &valid
            );
            assert_eq!(
                effect_data_layout(Some(&package), ty, &mut 10_000, 0).is_ok(),
                ty == &valid
            );
            assert_eq!(
                effect_borrowed_code_layout(Some(&package), ty, &mut 10_000).is_ok(),
                ty == &valid
            );
        }
        assert!(check(&valid, &mut 0).is_err());
        check(&valid, &mut 10_000).unwrap();
    }
}

#[test]
fn effect_storage_views_keep_code_and_pointer_permissions_separate() {
    let boolean = Term::Primitive(Primitive::Bool);
    let code = Term::Function {
        parameters: vec![],
        result: Box::new(boolean.clone()),
    };
    for is_enum in [false, true] {
        let mut package = Package::default();
        if is_enum {
            package
                .enums
                .push(enumeration(vec![vec![], vec![parameter()]]));
        } else {
            package.structs.push(structure(vec![parameter()]));
        }
        for (leaf, data, borrowed_code) in [
            (boolean.clone(), true, true),
            (code.clone(), false, true),
            (Term::Pointer(Box::new(boolean.clone())), false, false),
        ] {
            let ty = applied(leaf);
            assert_eq!(
                effect_data_layout(Some(&package), &ty, &mut 10_000, 0).unwrap(),
                data
            );
            assert_eq!(
                effect_borrowed_code_layout(Some(&package), &ty, &mut 10_000).unwrap(),
                borrowed_code
            );
            assert!(effect_data_layout(Some(&package), &ty, &mut 1, 0).is_err());
            assert_eq!(
                effect_data_layout(Some(&package), &ty, &mut 10_000, 0).unwrap(),
                data
            );
        }
    }
}
