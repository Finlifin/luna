//! Construction consumes one allowance without publishing failed contributions.
use nir::{DeclarationId as Id, NominalAdmissionError, Primitive, Term, program as n};
use type_resolution::{
    dispatch::DeclarationWorld,
    implementation::{Error, ImplementationHead, Index},
    ptc::Budget,
};
fn id(local: u32) -> Id {
    Id { package: 31, local }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn head(local: u32) -> ImplementationHead {
    ImplementationHead {
        declaration: id(local),
        interface: Some(id(1000 + local)),
        subject: boolean(),
        interface_arguments: vec![],
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
    }
}
#[test]
fn failed_index_insert_and_domain_registration_leave_no_partial_authority() {
    let mut index = Index::default();
    let revision = index.revision();
    let mut empty = Budget {
        steps: 0,
        ..Budget::default()
    };
    assert!(matches!(
        index.insert_metered(head(1), &mut empty),
        Err(Error::BudgetExhausted)
    ));
    assert_eq!(index.revision(), revision);
    assert!(index.head(id(1)).is_none());
    let mut budget = Budget::default();
    index.insert_metered(head(1), &mut budget).unwrap();
    assert!(budget.steps < Budget::default().steps);
    let revision = index.revision();
    let before = budget.steps;
    assert!(matches!(
        index.insert_metered(head(1), &mut budget),
        Err(Error::DuplicateDeclaration(_))
    ));
    assert_eq!(budget.steps, before);
    assert_eq!(index.revision(), revision);
    assert!(matches!(
        index.register_parameter_domain_metered(id(50), boolean(), &mut 0),
        Err(NominalAdmissionError::Budget)
    ));
    assert!(!index.parameter_domains().contains_key(&id(50)));
    index
        .register_parameter_domain_metered(id(50), boolean(), &mut budget.steps)
        .unwrap();
    let revision = index.revision();
    assert!(matches!(
        index.register_parameter_domain_metered(
            id(50),
            Term::Primitive(Primitive::Char),
            &mut budget.steps
        ),
        Err(NominalAdmissionError::Invalid(_))
    ));
    assert_eq!(index.parameter_domains()[&id(50)], boolean());
    assert_eq!(index.revision(), revision);
}
#[test]
fn implementation_batches_cannot_refresh_their_world_budget() {
    let mut package = n::Package::default();
    for local in 1..20 {
        let h = head(local);
        package.traits.push(n::Trait {
            declaration: h.interface.unwrap(),
            self_parameter: id(2000 + local),
            name: format!("Marker{local}"),
            comptime_only: false,
            parameter_types: vec![],
            type_parameters: vec![],
            parents: vec![],
            methods: vec![],
            requirements: vec![],
            capabilities: vec![],
            associated: vec![],
        });
        package.implementations.push(n::Implementation {
            phases: Default::default(),
            declaration: h.declaration,
            interface: h.interface,
            subject: h.subject,
            interface_arguments: vec![],
            parameter_types: vec![],
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            methods: vec![],
            associated: vec![],
            associated_initializers: vec![],
        });
    }
    let mut full = Budget::default();
    let world = DeclarationWorld::new_metered(&package, &mut full, None).unwrap();
    for local in 1..20 {
        assert!(world.index().head(id(local)).is_some());
    }
    let work = Budget::default().steps - full.steps;
    let mut insufficient = Budget {
        steps: work - 1,
        ..Budget::default()
    };
    assert!(matches!(
        DeclarationWorld::new_metered(&package, &mut insufficient, None),
        Err(NominalAdmissionError::Budget)
    ));
    let mut exact = Budget {
        steps: work,
        ..Budget::default()
    };
    DeclarationWorld::new_metered(&package, &mut exact, None).unwrap();
    assert_eq!(exact.steps, 0);
    // Invalid metadata is distinct from exhaustion and leaves unused work.
    package
        .implementations
        .push(package.implementations[0].clone());
    let mut invalid = Budget::default();
    assert!(matches!(
        DeclarationWorld::new_metered(&package, &mut invalid, None),
        Err(NominalAdmissionError::Invalid(_))
    ));
    assert!(invalid.steps > 0 && invalid.steps < Budget::default().steps);
    package.implementations.pop();
    DeclarationWorld::new_metered(&package, &mut invalid, None).unwrap();
}
