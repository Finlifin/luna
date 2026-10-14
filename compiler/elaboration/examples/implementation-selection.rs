//! Run with `cargo run -p elaboration --example implementation-selection`.
//! This exercises type applicability; it does not parse trait source or certify
//! package coherence, phase compatibility or runtime environment delivery.
use elaboration::{
    implementation::{ImplementationHead, Index, Selection},
    ptc::{Budget, Context},
};
use nir::{DeclarationId, Primitive, Sort, Term, program::TypePredicate};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 0, local }
}
fn main() {
    let parameter = Term::Rigid {
        declaration: id(1),
        sort: Sort::Type,
    };
    let boxed = |arg| Term::Nominal {
        declaration: id(10),
        arguments: vec![arg],
    };
    let flag = Term::Primitive(Primitive::Bool);
    let mut index = Index::default();
    index
        .insert(
            ImplementationHead {
                declaration: id(200),
                interface: Some(id(100)),
                interface_arguments: vec![],
                subject: parameter.clone(),
                type_parameters: vec![id(1)],
                requirements: vec![],
            },
            Budget::default(),
        )
        .unwrap();
    index
        .insert(
            ImplementationHead {
                declaration: id(201),
                interface: Some(id(100)),
                interface_arguments: vec![],
                subject: boxed(parameter.clone()),
                type_parameters: vec![id(1)],
                requirements: vec![TypePredicate::Equal(parameter.clone(), flag.clone())],
            },
            Budget::default(),
        )
        .unwrap();
    index
        .insert(
            ImplementationHead {
                declaration: id(202),
                interface: Some(id(100)),
                interface_arguments: vec![],
                subject: boxed(parameter.clone()),
                type_parameters: vec![id(1)],
                requirements: vec![TypePredicate::Equal(
                    parameter,
                    Term::Primitive(Primitive::Char),
                )],
            },
            Budget::default(),
        )
        .unwrap();
    let mut context = Context::default();
    context.admit_nominal_constructor(id(10));
    let coherence = index.check_type_coherence(&context, Budget::default());
    assert!(index.verify_type_coherence(&coherence, &context, Budget::default()));
    let resolution = index
        .resolve_types(id(100), &[], &boxed(flag), &context, Budget::default())
        .unwrap();
    assert_eq!(resolution.selection, Selection::Selected(id(201)));
    println!("(implementation-demo {index} {coherence} {resolution})");
}
