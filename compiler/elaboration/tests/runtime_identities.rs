use elaboration::{materialization, ptc::Budget};
use nir::program::{Package, RuntimeFunctionIdentity};
use nir::{DeclarationId, Primitive, Term};
use std::collections::BTreeSet;

#[test]
fn signature_only_runtime_identity_is_not_an_executable_iir_root() {
    let declaration = DeclarationId {
        package: 1,
        local: 1,
    };
    let package = Package {
        runtime_function_identities: vec![RuntimeFunctionIdentity {
            requirements: vec![],
            capabilities: vec![],
            declaration,
            type_parameters: vec![],
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            },
        }],
        ..Default::default()
    };
    assert!(materialization::dispatch_instances(&package, Budget::default()).is_ok());
    let error = materialization::instantiate(
        &package,
        &[iir::InstanceKey {
            declaration,
            arguments: vec![].into(),
        }],
        BTreeSet::new(),
        Budget::default(),
    )
    .unwrap_err();
    assert!(error.contains("no executable definition"), "{error}");
}

#[test]
fn frozen_identity_needs_formation_evidence_before_iir_publication() {
    use nir::program::TypePredicate;
    let declaration = DeclarationId {
        package: 7,
        local: 2,
    };
    let binder = DeclarationId {
        local: 3,
        ..declaration
    };
    let boolean = Term::Primitive(Primitive::Bool);
    let carrier = DeclarationId {
        local: 4,
        ..declaration
    };
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(boolean.clone()),
    };
    let package = Package {
        normal_forms: vec![nir::normal_forms::Head {
            declaration: carrier,
            role: nir::normal_forms::Role::Opaque,
            collector: None,
            parameters: vec![nir::normal_forms::Parameter {
                declaration: DeclarationId {
                    local: 5,
                    ..carrier
                },
                binding: nir::program::LocalId {
                    owner: carrier.local,
                    index: 0,
                },
                kind: nir::normal_forms::Kind::Value,
                ty: signature.clone(),
            }],
            requirements: vec![],
        }],
        runtime_function_identities: vec![RuntimeFunctionIdentity {
            declaration,
            type_parameters: vec![binder],
            signature,
            requirements: vec![TypePredicate::Equal(
                Term::Rigid {
                    declaration: binder,
                    sort: nir::Sort::Type,
                },
                boolean.clone(),
            )],
            capabilities: vec![],
        }],
        ..Default::default()
    };
    let build = |argument| {
        materialization::instantiate_with_types(
            &package,
            &[],
            &[Term::NormalForm {
                head: carrier,
                sort: nir::Sort::Type,
                arguments: vec![Term::FunctionValue {
                    declaration,
                    arguments: vec![argument],
                }],
            }],
            BTreeSet::new(),
            Budget::default(),
        )
    };
    let error = build(Term::Primitive(Primitive::U8)).unwrap_err();
    assert!(
        error.contains("formation requirement is not established"),
        "{error}"
    );
    let graph = build(boolean).unwrap();
    assert!(graph.functions.is_empty());
    assert_eq!(graph.callable_identities.len(), 1);
    assert_eq!(graph.callable_identities[0].key.declaration, declaration);
}
