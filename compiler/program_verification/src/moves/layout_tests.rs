use super::*;
use nir::program::{Enum, Struct, Variant, VariantShape};
use nir::{DeclarationId, Primitive, Sort};

fn fixture(is_enum: bool, argument: Term, fields: usize) -> (Package, Term) {
    let declaration = DeclarationId {
        package: 9,
        local: 1,
    };
    let parameter = DeclarationId {
        package: 9,
        local: 2,
    };
    let field = Term::Record(
        (0..fields)
            .map(|i| {
                (
                    format!("field{i}"),
                    Term::Rigid {
                        declaration: parameter,
                        sort: Sort::Type,
                    },
                )
            })
            .collect(),
    );
    let mut package = Package::default();
    if is_enum {
        package.enums.push(Enum {
            declaration,
            name: "Choice".into(),
            type_parameters: vec![parameter],
            parameter_types: vec![Term::Primitive(Primitive::Type)],
            requirements: vec![],
            capabilities: vec![],
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
            is_intrinsic: false,
            variants: vec![
                Variant {
                    name: "first".into(),
                    shape: VariantShape::Unit,
                    fields: vec![],
                },
                Variant {
                    name: "payload".into(),
                    shape: VariantShape::Tuple,
                    fields: vec![field],
                },
                Variant {
                    name: "last".into(),
                    shape: VariantShape::Unit,
                    fields: vec![],
                },
            ],
        });
    } else {
        package.structs.push(Struct {
            declaration,
            name: "Container".into(),
            type_parameters: vec![parameter],
            parameter_types: vec![Term::Primitive(Primitive::Type)],
            requirements: vec![],
            capabilities: vec![],
            fields: vec![("payload".into(), field)],
        });
    }
    (
        package,
        Term::Nominal {
            declaration,
            arguments: vec![argument],
        },
    )
}

fn wide_argument() -> Term {
    Term::Record(
        (0..32)
            .map(|i| {
                (
                    format!("slot{i}"),
                    Term::Pointer(Box::new(Term::Primitive(Primitive::Bool))),
                )
            })
            .collect(),
    )
}

#[test]
fn callable_variant_fields_charge_each_repeated_parameter_occurrence() {
    let argument = wide_argument();
    let (package, ty) = fixture(true, argument, 64);
    let Term::Nominal { arguments, .. } = &ty else {
        unreachable!()
    };
    let source = SourceRange { start: 7, end: 9 };
    let failure = callable_variant_fields(&package.enums[0], arguments, &mut 2000, source)
        .err()
        .expect("expanded variant output must fit the budget");
    assert!(failure.reason.contains("budget"), "{failure}");
    assert_eq!(failure.source, source);
    assert_eq!(
        callable_variant_fields(&package.enums[0], arguments, &mut 100_000, source)
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn owned_projection_bounds_expansion_before_selecting_a_field() {
    let source = SourceRange { start: 7, end: 9 };
    for is_enum in [false, true] {
        let (package, ty) = fixture(is_enum, wide_argument(), 64);
        let path = if is_enum {
            vec![VARIANT_PATH, 1, 0, 63, 31]
        } else {
            vec![0, 63, 31]
        };
        let failure = owned_field_type(Some(&package), &ty, &path, &mut 2000, source)
            .err()
            .expect("projection cannot allocate unbounded intermediate fields");
        assert!(failure.reason.contains("budget"), "{failure}");
        assert_eq!(
            owned_field_type(Some(&package), &ty, &path, &mut 100_000, source).unwrap(),
            Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)))
        );
    }
}

#[test]
fn pointer_origin_queries_bound_expansion_and_recover_the_qualified_path() {
    let source = SourceRange { start: 7, end: 9 };
    for is_enum in [false, true] {
        let (package, ty) = fixture(is_enum, wide_argument(), 64);
        let mut qualified = vec![];
        let query = |fuel: &mut usize, qualified: &mut Vec<usize>| {
            origins::pointer_possible(
                Some(&package),
                &ty,
                &[0, 63, 31],
                &EnumDomains::new(),
                qualified,
                fuel,
                source,
                0,
            )
        };
        let failure = query(&mut 2000, &mut qualified)
            .expect_err("origin queries must account for expanded layout work");
        assert!(failure.reason.contains("budget"), "{failure}");
        assert!(qualified.is_empty());
        assert!(query(&mut 100_000, &mut qualified).unwrap());
        assert!(qualified.is_empty());
    }
}

#[test]
fn callable_paths_preserve_branch_positions_and_pointer_contracts() {
    let source = SourceRange { start: 7, end: 9 };
    let boolean = Term::Primitive(Primitive::Bool);
    let code = Term::Function {
        parameters: vec![],
        result: Box::new(boolean.clone()),
    };
    for is_enum in [false, true] {
        let (package, ty) = fixture(is_enum, code.clone(), 2);
        let prefix = if is_enum {
            vec![VARIANT_PATH, 1, 0]
        } else {
            vec![0]
        };
        let expected: Vec<_> = (0..2)
            .map(|i| {
                let mut path = prefix.clone();
                path.push(i);
                path
            })
            .collect();
        assert_eq!(
            callable_value_paths(Some(&package), &ty, &mut 10_000, source, 0).unwrap(),
            expected
        );
        for path in &expected {
            assert_eq!(
                owned_field_type(Some(&package), &ty, path, &mut 10_000, source).unwrap(),
                code
            );
        }
        let (package, ty) = fixture(is_enum, Term::Pointer(Box::new(boolean.clone())), 2);
        for allowed in [false, true] {
            assert_eq!(
                recoverable_return_capture_layout(
                    Some(&package),
                    &ty,
                    allowed,
                    &mut 10_000,
                    source,
                    0
                )
                .unwrap(),
                allowed
            );
        }
    }
}

#[test]
fn nominal_origin_queries_preserve_tag_exclusion_and_failure_recovery() {
    let source = SourceRange { start: 7, end: 9 };
    let pointer = Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)));
    let (package, ty) = fixture(true, pointer, 2);
    let mut qualified = vec![];
    for (tags, expected) in [(vec![0, 2], false), (vec![1], true), (vec![0, 1, 2], true)] {
        let domains = EnumDomains::from([(vec![], tags.into_iter().collect())]);
        let mut completed = false;
        for budget in 0..500 {
            let result = origins::pointer_possible(
                Some(&package),
                &ty,
                &[0, 1],
                &domains,
                &mut qualified,
                &mut { budget },
                source,
                0,
            );
            assert!(
                qualified.is_empty(),
                "failed queries must restore path state"
            );
            match result {
                Ok(possible) => {
                    assert_eq!(possible, expected);
                    completed = true;
                    break;
                }
                Err(failure) => assert!(failure.reason.contains("budget"), "{failure}"),
            }
        }
        assert!(completed);
    }
}

#[test]
fn nominal_move_views_reject_invalid_arguments_then_accept_valid_queries() {
    let source = SourceRange { start: 7, end: 9 };
    for is_enum in [false, true] {
        let (package, valid) = fixture(is_enum, Term::Primitive(Primitive::Bool), 2);
        let Term::Nominal { declaration, .. } = valid else {
            unreachable!()
        };
        let invalid = Term::Nominal {
            declaration,
            arguments: vec![Term::Bool(true)],
        };
        let path = if is_enum {
            vec![VARIANT_PATH, 1, 0, 0]
        } else {
            vec![0, 0]
        };
        for ty in [&invalid, &valid] {
            assert_eq!(
                callable_value_paths(Some(&package), ty, &mut 10_000, source, 0).is_ok(),
                ty == &valid
            );
            assert_eq!(
                recoverable_return_capture_layout(
                    Some(&package),
                    ty,
                    false,
                    &mut 10_000,
                    source,
                    0
                )
                .is_ok(),
                ty == &valid
            );
            assert_eq!(
                owned_field_type(Some(&package), ty, &path, &mut 10_000, source).is_ok(),
                ty == &valid
            );
            if !is_enum {
                assert_eq!(
                    ownership_shape(&package, ty, &mut 10_000, 0).is_ok(),
                    ty == &valid
                );
            }
        }
    }
}
