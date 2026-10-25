use super::*;
use nir::{
    Sort,
    program::{Enum, TypePredicate, Variant},
};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 81, local }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn nominal(arguments: Vec<Term>) -> Term {
    Term::Nominal {
        declaration: id(1),
        arguments,
    }
}
fn package() -> Package {
    let parameter = Term::Rigid {
        declaration: id(2),
        sort: Sort::Type,
    };
    let mut package = Package::default();
    package.enums.push(Enum {
        is_intrinsic: false,
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        type_parameters: vec![id(2)],
        declaration: id(1),
        name: "Message".into(),
        variants: vec![
            Variant {
                name: "Empty".into(),
                shape: VariantShape::Unit,
                fields: vec![],
            },
            Variant {
                name: "Data.Pair".into(),
                shape: VariantShape::Tuple,
                fields: vec![
                    parameter.clone(),
                    Term::Pointer(Box::new(nominal(vec![parameter.clone()]))),
                ],
            },
            Variant {
                name: "Data.Record".into(),
                shape: VariantShape::Named(vec!["value\n\"".into()]),
                fields: vec![parameter],
            },
        ],
        groups: vec!["Data".into()],
        encoding: None,
        symbol_encoding: None,
    });
    package
}

#[test]
fn reflection_preserves_groups_order_shapes_and_instantiates_payloads() {
    let package = package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let ty = nominal(vec![boolean()]);
    let result = vm.reflect_enum(&ty).unwrap();
    assert_eq!(result.subject, ty);
    assert_eq!(result.groups, ["Data"]);
    assert_eq!(result.variants.len(), 3);
    assert_eq!(result.variants[0].shape, VariantShape::Unit);
    assert!(result.variants[0].fields.is_empty());
    assert_eq!(result.variants[1].name, "Data.Pair");
    assert_eq!(result.variants[1].shape, VariantShape::Tuple);
    assert_eq!(
        result.variants[1].fields,
        [boolean(), Term::Pointer(Box::new(ty.clone()))]
    );
    assert_eq!(
        result.variants[2].shape,
        VariantShape::Named(vec!["value\n\"".into()])
    );
    assert_eq!(result.variants[2].fields, [boolean()]);
    assert!(result.to_string().contains("value\\n\\\""));
    assert_eq!(vm.reflect_enum(&ty).unwrap(), result);
    assert!(vm.events().is_empty());
    assert!(vm.reflect_struct(&ty).is_err());
}

#[test]
fn rejects_incorrect_inputs_and_runtime_phase_without_poisoning_valid_query() {
    let package = package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for ty in [
        nominal(vec![]),
        nominal(vec![Term::Bool(true)]),
        boolean(),
        Term::Nominal {
            declaration: id(99),
            arguments: vec![],
        },
    ] {
        assert!(vm.reflect_enum(&ty).is_err());
    }
    assert!(vm.reflect_enum(&nominal(vec![boolean()])).is_ok());
    let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        runtime
            .reflect_enum(&nominal(vec![boolean()]))
            .unwrap_err()
            .kind,
        ErrorKind::PhaseMismatch
    );
}

#[test]
fn empty_enum_retains_formation_checks_and_intrinsic_classification() {
    let mut package = package();
    package.enums[0].variants.clear();
    package.enums[0].groups.clear();
    package.enums[0].requirements = vec![TypePredicate::Bool(false)];
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.reflect_enum(&nominal(vec![boolean()])).unwrap_err().kind,
        ErrorKind::TypeContractRejected
    );
    package.enums[0].requirements.clear();
    package.enums[0].is_intrinsic = true;
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(vm.reflect_enum(&nominal(vec![])).is_err());
    let result = vm.reflect_enum(&nominal(vec![boolean()])).unwrap();
    assert!(result.is_intrinsic);
    assert!(result.variants.is_empty());
}

#[test]
fn budget_accounts_for_metadata_and_failure_leaves_world_unchanged() {
    let mut package = package();
    package.enums[0].variants[2].shape = VariantShape::Named(vec!["x".repeat(20_000)]);
    let snapshot = package.clone();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    vm.budget.steps = 10_000;
    assert_eq!(
        vm.reflect_enum(&nominal(vec![boolean()])).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(package, snapshot);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.reflect_enum(&nominal(vec![boolean()]))
            .unwrap()
            .variants
            .len(),
        3
    );
}

#[test]
fn concrete_iir_payloads_agree_with_declaration_reflection() {
    let package = package();
    let ty = nominal(vec![boolean()]);
    let mut definition = package.enums[0].clone();
    let binders = definition.parameter_binders().unwrap();
    for variant in &mut definition.variants {
        for field in &mut variant.fields {
            *field = field
                .instantiate_parameters_in_scope(&binders, &[boolean()], &[])
                .unwrap();
        }
    }
    definition.type_parameters.clear();
    definition.parameter_types.clear();
    let graph = iir::Package {
        nominals: vec![iir::NominalInstance {
            ty: ty.clone(),
            definition: iir::NominalDefinition::Enum(definition),
        }],
        ..iir::Package::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut concrete = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.reflect_enum(&ty).unwrap(),
        concrete.reflect_enum(&ty).unwrap()
    );
}

#[test]
fn opaque_payload_types_keep_their_heads() {
    let mut package = package();
    package.normal_forms.push(nir::normal_forms::Head {
        declaration: id(3),
        role: nir::normal_forms::Role::Opaque,
        parameters: vec![],
        requirements: vec![],
    });
    let opaque = Term::NormalForm {
        head: id(3),
        sort: Sort::Type,
        arguments: vec![],
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let reflected = vm.reflect_enum(&nominal(vec![opaque.clone()])).unwrap();
    assert_eq!(reflected.variants[1].fields[0], opaque);
    assert_eq!(vm.reflect_enum(&opaque).unwrap_err().kind, ErrorKind::Type);
}

#[test]
fn malformed_payload_shapes_or_non_type_fields_cannot_be_reflected() {
    for (shape, fields) in [
        (VariantShape::Unit, vec![boolean()]),
        (VariantShape::Named(vec![]), vec![boolean()]),
        (VariantShape::Tuple, vec![Term::Bool(true)]),
        (
            VariantShape::Tuple,
            vec![Term::Rigid {
                declaration: id(99),
                sort: Sort::Type,
            }],
        ),
    ] {
        let mut package = package();
        package.enums[0].variants = vec![Variant {
            name: "Bad".into(),
            shape,
            fields,
        }];
        if let Ok(mut vm) = Interpreter::for_comptime(&package, Budget::default()) {
            assert!(vm.reflect_enum(&nominal(vec![boolean()])).is_err());
        }
    }
}
