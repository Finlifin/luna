use nir::{
    DeclarationId as Id, NominalAdmissionError as Failure, Primitive, Sort, Term, program as n,
};
use type_resolution::{
    capabilities::World,
    implementation::{Index, Selection},
    ptc::{Budget, Context},
};

fn id(local: u32) -> Id {
    Id { package: 1, local }
}
fn bool_ty() -> Term {
    Term::Primitive(Primitive::Bool)
}

fn fixture(count: u32) -> n::Package {
    let mut package = n::Package::default();
    for entry in 0..count {
        let base = 10 * entry;
        package.traits.push(n::Trait {
            declaration: id(base + 1),
            self_parameter: id(base + 2),
            name: format!("Flag{entry}"),
            comptime_only: false,
            type_parameters: vec![id(base + 3)],
            parameter_types: vec![bool_ty()],
            associated: vec![],
            requirements: vec![],
            capabilities: vec![],
            parents: vec![],
            methods: vec![],
        });
        package.implementations.push(n::Implementation {
            declaration: id(base + 4),
            phases: Default::default(),
            interface: Some(id(base + 1)),
            interface_arguments: vec![Term::Rigid {
                declaration: id(base + 5),
                sort: Sort::ComptimeValue,
            }],
            subject: bool_ty(),
            type_parameters: vec![id(base + 5)],
            parameter_types: vec![bool_ty()],
            requirements: vec![],
            capabilities: vec![],
            methods: vec![],
            associated: vec![],
            associated_initializers: vec![],
        });
    }
    package
}

#[test]
fn one_allowance_covers_all_implementations_and_failure_does_not_publish() {
    let one = fixture(1);
    let many = fixture(32);
    let mut allowance = Budget::default();
    Index::from_package_metered(&one, &mut allowance).unwrap();
    let cost = Budget::default().steps - allowance.steps;
    assert!(cost > 0);
    let mut limited = Budget {
        steps: cost,
        ..Default::default()
    };
    assert!(matches!(
        Index::from_package_metered(&many, &mut limited),
        Err(Failure::Budget)
    ));
    assert!(limited.steps < cost);
    // A failed construction yields no partially usable index. A fresh request
    // must still admit every candidate and supply real ordinary value inputs.
    let mut allowance = Budget::default();
    let index = Index::from_package_metered(&many, &mut allowance).unwrap();
    assert!(Budget::default().steps - allowance.steps > cost);
    let context = Context::from_package(&many).unwrap();
    let world = World {
        index: &index,
        context: &context,
        traits: &many.traits,
        implementations: &many.implementations,
        assumptions: &[],
    };
    for entry in [0, 15, 31] {
        let interface = n::InterfaceInstance {
            declaration: id(10 * entry + 1),
            arguments: vec![Term::Bool(true)],
        };
        let result = world
            .resolve_instance_metered(&interface, &bool_ty(), &mut Budget::default())
            .unwrap();
        assert_eq!(
            result.types.selection,
            Selection::Selected(id(10 * entry + 4))
        );
    }
}

#[test]
fn metadata_copy_and_invalid_domains_are_bounded_before_publication() {
    let mut package = fixture(1);
    let mut small = Budget::default();
    Index::from_package_metered(&package, &mut small).unwrap();
    let cost = Budget::default().steps - small.steps;
    package.structs.push(n::Struct {
        declaration: id(100),
        name: "Wide".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        fields: (0..4096)
            .map(|i| (format!("field{i}"), bool_ty()))
            .collect(),
        requirements: vec![],
        capabilities: vec![],
        packed: false,
    });
    let mut limited = Budget {
        steps: cost,
        ..Default::default()
    };
    assert!(matches!(
        Index::from_package_metered(&package, &mut limited),
        Err(Failure::Budget)
    ));
    package.structs.clear();
    let mut invalid = package.clone();
    invalid.implementations[0].parameter_types[0] = Term::Bool(true);
    let mut allowance = Budget::default();
    assert!(matches!(
        Index::from_package_metered(&invalid, &mut allowance),
        Err(Failure::Invalid(_))
    ));
    assert!(allowance.steps > 0 && allowance.steps < Budget::default().steps);
    let recovered = Index::from_package_metered(&package, &mut allowance).unwrap();
    assert_eq!(recovered.parameter_domains().get(&id(5)), Some(&bool_ty()));
}

#[test]
fn source_index_retains_inherent_domains_without_turning_them_into_trait_candidates() {
    let mut package = fixture(1);
    let mut inherent = package.implementations[0].clone();
    inherent.declaration = id(50);
    inherent.interface = None;
    inherent.interface_arguments.clear();
    inherent.type_parameters = vec![id(51)];
    package.implementations.push(inherent);
    let index = Index::from_package_metered(&package, &mut Budget::default()).unwrap();
    assert!(index.head(id(4)).is_some());
    assert!(index.head(id(50)).is_none());
    assert_eq!(index.parameter_domains().get(&id(51)), Some(&bool_ty()));
    let before = index.to_string();
    // Executable associated initializers are outside structural indexing.
    package.implementations[0].associated.push((
        id(99),
        n::Expr {
            source: n::SourceRange { start: 0, end: 1 },
            ty: bool_ty(),
            kind: n::ExprKind::Literal(n::Literal::Bool(false)),
        },
    ));
    let after = Index::from_package_metered(&package, &mut Budget::default()).unwrap();
    assert_eq!(before, after.to_string());
}
