use elaboration::representations::Checker;
use nir::{DeclarationId, Primitive, Sort, Term, program::Struct};
use std::collections::BTreeMap;

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 71, local }
}
fn nominal(local: u32) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments: vec![],
    }
}
fn definition(local: u32, field: Term) -> Struct {
    Struct {
        declaration: id(local),
        name: format!("S{local}"),
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        fields: vec![("field".into(), field)],
    }
}
fn check(field: Term, tuple: Option<DeclarationId>) -> Result<(), &'static str> {
    Checker::new(
        tuple,
        &BTreeMap::from([(id(0), definition(0, field))]),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .check(&nominal(0), &mut 100_000)
}

#[test]
fn inline_storage_wrappers_cannot_hide_recursive_payloads() {
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    for field in [
        nominal(0),
        Term::Optional(Box::new(nominal(0))),
        Term::Record(vec![("nested".into(), nominal(0))]),
        Term::Tuple(vec![nominal(0)]),
        Term::Closure {
            environment: vec![nominal(0)],
            signature: Box::new(signature),
        },
        Term::ErrorResult {
            errors: vec![nominal(0)],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        },
        Term::ErrorResult {
            errors: vec![Term::Primitive(Primitive::Bool)],
            result: Box::new(nominal(0)),
        },
    ] {
        assert!(
            check(field.clone(), None)
                .unwrap_err()
                .contains("recursive by-value"),
            "{field}"
        );
    }
}

#[test]
fn pointer_boundaries_and_opaque_heads_remain_finite() {
    for field in [
        Term::Optional(Box::new(Term::Pointer(Box::new(nominal(0))))),
        Term::Function {
            parameters: vec![],
            result: Box::new(nominal(0)),
        },
        Term::NormalForm {
            head: id(5),
            sort: Sort::Type,
            arguments: vec![nominal(0)],
        },
    ] {
        check(field, None).unwrap();
    }
}

#[test]
fn only_bound_tuple_head_exposes_its_type_list_storage() {
    let field = Term::NormalForm {
        head: id(5),
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(vec![nominal(0)])],
    };
    check(field.clone(), None).unwrap();
    assert!(
        check(field, Some(id(5)))
            .unwrap_err()
            .contains("recursive by-value")
    );
}

#[test]
fn forward_edges_and_failed_roots_do_not_poison_completed_roots() {
    let structs = BTreeMap::from([
        (id(0), definition(0, Term::Optional(Box::new(nominal(1))))),
        (id(1), definition(1, Term::Primitive(Primitive::Bool))),
        (id(2), definition(2, nominal(2))),
    ]);
    let enums = BTreeMap::new();
    let newtypes = BTreeMap::new();
    let mut checker = Checker::new(None, &structs, &enums, &newtypes);
    let mut fuel = 100_000;
    assert!(checker.check(&nominal(2), &mut fuel).is_err());
    checker.check(&nominal(0), &mut fuel).unwrap();
    checker.check(&nominal(1), &mut fuel).unwrap();
    assert!(checker.check(&nominal(2), &mut fuel).is_err());
    assert!(
        checker
            .check(&nominal(99), &mut fuel)
            .unwrap_err()
            .contains("missing")
    );
}

#[test]
fn expanding_generic_recursion_is_bounded_before_materialization() {
    let parameter = id(8);
    let mut def = definition(
        0,
        Term::Nominal {
            declaration: id(0),
            arguments: vec![Term::Optional(Box::new(Term::Rigid {
                declaration: parameter,
                sort: Sort::Type,
            }))],
        },
    );
    def.type_parameters.push(parameter);
    let structs = BTreeMap::from([(id(0), def)]);
    let enums = BTreeMap::new();
    let newtypes = BTreeMap::new();
    let mut checker = Checker::new(None, &structs, &enums, &newtypes);
    let root = Term::Nominal {
        declaration: id(0),
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    assert!(
        checker
            .check(&root, &mut 300)
            .unwrap_err()
            .contains("budget")
    );
    assert!(
        checker
            .check(&root, &mut 100_000)
            .unwrap_err()
            .contains("budget")
    );
}

#[test]
fn exhausted_attempt_can_be_retried_with_fresh_allowance() {
    let structs = BTreeMap::from([(id(0), definition(0, Term::Primitive(Primitive::Bool)))]);
    let enums = BTreeMap::new();
    let newtypes = BTreeMap::new();
    let mut checker = Checker::new(None, &structs, &enums, &newtypes);
    assert!(
        checker
            .check(&nominal(0), &mut 0)
            .unwrap_err()
            .contains("budget")
    );
    checker.check(&nominal(0), &mut 1000).unwrap();
}

#[test]
fn mixed_struct_enum_newtype_cycle_follows_payload_storage() {
    use nir::program::{Enum, Variant, VariantShape};
    let structs = BTreeMap::from([(id(0), definition(0, nominal(1)))]);
    let enums = BTreeMap::from([(
        id(1),
        Enum {
            is_intrinsic: false,
            declaration: id(1),
            name: "Branch".into(),
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
            variants: vec![Variant {
                name: "payload".into(),
                shape: VariantShape::Tuple,
                fields: vec![nominal(2)],
            }],
        },
    )]);
    let mut wrapper = nir::newtypes::Newtype {
        declaration: id(2),
        name: "Wrapper".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        capture_count: 0,
        requirements: vec![],
        capabilities: vec![],
        representation: Term::Optional(Box::new(nominal(0))),
    };
    assert!(
        Checker::new(
            None,
            &structs,
            &enums,
            &BTreeMap::from([(id(2), wrapper.clone())])
        )
        .check(&nominal(0), &mut 100_000)
        .unwrap_err()
        .contains("recursive by-value")
    );
    wrapper.representation = Term::Pointer(Box::new(nominal(0)));
    Checker::new(None, &structs, &enums, &BTreeMap::from([(id(2), wrapper)]))
        .check(&nominal(0), &mut 100_000)
        .unwrap();
}
