use super::*;
use elaboration::{
    materialization::DeclarationSource,
    verification::copy::{Classification, classify},
};
use nir::{Primitive, Term, program as n};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 41, local }
}

fn subject() -> Term {
    Term::Record(vec![("flag".into(), Term::Primitive(Primitive::Bool))])
}

fn fixture() -> Package {
    let mut package = Package::default();
    for (local, name) in [(1, "Duplicate"), (3, "Retire")] {
        package.traits.push(n::Trait {
            declaration: id(local),
            self_parameter: id(local + 1),
            name: name.into(),
            comptime_only: false,
            type_parameters: vec![],
            parameter_types: vec![],
            associated: vec![],
            requirements: vec![],
            capabilities: vec![],
            parents: vec![],
            methods: vec![],
        });
    }
    package.implementations.push(n::Implementation {
        declaration: id(5),
        phases: Default::default(),
        interface: Some(id(1)),
        interface_arguments: vec![],
        subject: subject(),
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        methods: vec![],
        associated: vec![],
        associated_initializers: vec![],
    });
    package
}

fn bind(source: DeclarationSnapshot, budget: Budget) -> Result<ProtocolContext, String> {
    ProtocolContext::selected_in_source(
        source,
        Selection::Identity(id(1)),
        Some(Selection::Identity(id(3))),
        budget,
    )
}

#[test]
fn bound_resource_source_retains_its_directory_and_releases_the_last_owner() {
    let package = Arc::new(fixture());
    let weak = Arc::downgrade(&package);
    let source = DeclarationSnapshot::new(package);
    assert!(
        bind(
            source.clone(),
            Budget {
                steps: 0,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(source.to_string().ends_with("(metadata pending))"));
    let binding = bind(source.clone(), Budget::default()).unwrap();
    let sibling = bind(
        source.clone(),
        Budget {
            steps: 0,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(std::ptr::eq(binding.package(), sibling.package()));
    assert!(std::ptr::eq(binding.world().index, sibling.world().index));
    let context = Context::default();
    let queries = DeclarationSource::Snapshot(&source)
        .world(
            &mut Budget {
                steps: 1,
                ..Default::default()
            },
            Some((&context, &[])),
        )
        .unwrap();
    assert!(std::ptr::eq(binding.world().index, queries.index()));
    // A retained directory cannot turn an exhausted resource proof into Copy.
    assert!(matches!(
        classify(
            &binding.world(),
            binding.copy,
            &subject(),
            Budget {
                steps: 0,
                ..Default::default()
            }
        )
        .unwrap(),
        Classification::Pending(_)
    ));
    assert!(
        matches!(classify(&binding.world(), binding.copy, &subject(), Budget::default()).unwrap(), Classification::Declared { implementation, .. } if implementation == id(5))
    );
    let dump = binding.to_string();
    assert!(dump.starts_with("(resource-protocol-context (copy "));
    assert!(dump.contains("(source (declaration-snapshot"));
    assert_eq!(dump, sibling.to_string());
    drop(queries);
    drop(source);
    drop(binding);
    assert!(weak.upgrade().is_some());
    assert!(matches!(
        classify(
            &sibling.world(),
            sibling.copy,
            &subject(),
            Budget::default()
        )
        .unwrap(),
        Classification::Declared { .. }
    ));
    drop(sibling);
    assert!(weak.upgrade().is_none());
}

#[test]
fn resource_binding_keeps_exact_source_authority_after_same_id_edits() {
    let mut package = Arc::new(fixture());
    let original = bind(DeclarationSnapshot::new(package.clone()), Budget::default()).unwrap();
    Arc::make_mut(&mut package).implementations.clear();
    let changed_source = DeclarationSnapshot::new(package);
    let changed = bind(changed_source.clone(), Budget::default()).unwrap();
    assert!(!std::ptr::eq(original.world().index, changed.world().index));
    for _ in 0..2 {
        assert!(matches!(
            classify(
                &original.world(),
                original.copy,
                &subject(),
                Budget::default()
            )
            .unwrap(),
            Classification::Declared { .. }
        ));
        assert_eq!(
            classify(
                &changed.world(),
                changed.copy,
                &subject(),
                Budget::default()
            )
            .unwrap(),
            Classification::Affine
        );
    }
    // Warm metadata does not bypass binding validation for another selection.
    assert!(
        ProtocolContext::selected_in_source(
            changed_source.clone(),
            Selection::Identity(id(99)),
            None,
            Budget::default()
        )
        .is_err()
    );
    assert!(
        ProtocolContext::selected_in_source(
            changed_source,
            Selection::Name("Copy"),
            None,
            Budget::default()
        )
        .is_err()
    );
    let mut ambiguous = original.package().clone();
    let mut duplicate = ambiguous.traits[0].clone();
    duplicate.declaration = id(10);
    duplicate.self_parameter = id(11);
    ambiguous.traits.push(duplicate);
    let ambiguous = DeclarationSnapshot::new(Arc::new(ambiguous));
    assert!(
        ProtocolContext::selected_in_source(
            ambiguous.clone(),
            Selection::Name("Duplicate"),
            None,
            Budget::default()
        )
        .is_err()
    );
    assert_eq!(bind(ambiguous, Budget::default()).unwrap().copy, id(1));
}
