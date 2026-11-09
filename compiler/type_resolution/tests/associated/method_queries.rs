use super::*;
use nir::NominalAdmissionError as Error;
use nir::program::{InterfaceInstance, MethodContract, Package};
use type_resolution::dispatch::DeclarationWorld;

fn fixture() -> (Package, Expr) {
    let mut interface = contract();
    interface.associated.clear();
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(ty(P::Bool)),
    };
    interface.methods.push(MethodContract {
        declaration: id(20),
        name: "accept".into(),
        phase: Default::default(),
        is_quote: false,
        pure: false,
        is_unsafe: false,
        type_parameters: vec![],
        type_arguments: vec![],
        signature: signature.clone().into(),
        requirements: vec![TypePredicate::Bool(false)],
        capabilities: vec![],
        program: Default::default(),
        parameter_bindings: vec![],
        by_value_receiver: false,
        by_reference_receiver: false,
        default: None,
    });
    let mut selected = implementation(ty(P::Bool), ty(P::Bool));
    selected.associated.clear();
    selected.methods.push(("accept".into(), id(21)));
    let callee = Expr {
        source: nir::program::SourceRange { start: 1, end: 2 },
        ty: signature,
        kind: ExprKind::TraitMethod {
            interface: id(1),
            interface_arguments: vec![],
            subject: ty(P::Bool),
            member: id(20),
            method_arguments: vec![],
        },
    };
    (
        Package {
            traits: vec![interface],
            implementations: vec![selected],
            ..Default::default()
        },
        callee,
    )
}

#[test]
fn identity_queries_do_not_copy_or_visit_executable_contracts() {
    let (plain, callee) = fixture();
    let mut large = plain.clone();
    large.traits[0].methods[0].program.preconditions = vec![
        Expr {
            source: callee.source,
            ty: ty(P::Bool),
            kind: ExprKind::Literal(nir::program::Literal::Bool(true)),
        };
        4096
    ];
    let select = |package: &Package| {
        let (mut world, _) = DeclarationWorld::new(package, Budget::default(), None).unwrap();
        let mut budget = Budget::default();
        let selected = world.method_identity(&callee, &mut budget).unwrap();
        assert_eq!(selected.body, id(21));
        (selected, budget.steps)
    };
    // Even a pending/false member requirement belongs to invocation. Identity
    // selection neither proves it nor traverses the executable preconditions.
    assert_eq!(select(&plain), select(&large));
}

#[test]
fn interface_formation_and_parent_evidence_are_required_before_identity_publication() {
    let (mut package, callee) = fixture();
    package.traits[0]
        .requirements
        .push(TypePredicate::Bool(false));
    let (mut world, _) = DeclarationWorld::new(&package, Budget::default(), None).unwrap();
    let mut budget = Budget::default();
    assert!(matches!(
        world.method_identity(&callee, &mut budget),
        Err(Error::Unresolved(_))
    ));
    assert!(budget.steps > 0 && budget.steps < Budget::default().steps);

    package.traits[0].requirements.clear();
    package.traits[0].parents.push(InterfaceInstance {
        declaration: id(40),
        arguments: vec![],
    });
    let mut parent = contract();
    parent.declaration = id(40);
    parent.self_parameter = id(41);
    parent.associated.clear();
    package.traits.push(parent);
    let (mut world, _) = DeclarationWorld::new(&package, Budget::default(), None).unwrap();
    assert!(matches!(
        world.method_identity(&callee, &mut Budget::default()),
        Err(Error::Unresolved(_))
    ));
    let context = Context::from_package(&package).unwrap();
    let assumptions = [TraitRequirement {
        subject: ty(P::Bool),
        interface: id(40),
        interface_arguments: vec![],
    }];
    let (mut world, _) =
        DeclarationWorld::new(&package, Budget::default(), Some((&context, &assumptions))).unwrap();
    assert_eq!(
        world
            .method_identity(&callee, &mut Budget::default())
            .unwrap()
            .body,
        id(21)
    );
}

#[test]
fn exhaustion_and_invalid_members_do_not_poison_the_declaration_world() {
    let (package, callee) = fixture();
    let (mut world, _) = DeclarationWorld::new(&package, Budget::default(), None).unwrap();
    for allowance in [0, 1, 4, 8, 16] {
        let mut budget = Budget {
            steps: allowance,
            ..Default::default()
        };
        assert!(matches!(
            world.method_identity(&callee, &mut budget),
            Err(Error::Budget)
        ));
        assert!(budget.steps <= allowance);
        assert_eq!(
            world
                .method_identity(&callee, &mut Budget::default())
                .unwrap()
                .body,
            id(21)
        );
    }
    let mut invalid = callee.clone();
    let ExprKind::TraitMethod { member, .. } = &mut invalid.kind else {
        unreachable!()
    };
    *member = id(99);
    let mut budget = Budget::default();
    assert!(matches!(
        world.method_identity(&invalid, &mut budget),
        Err(Error::Invalid(_))
    ));
    assert!(budget.steps > 0);
    assert_eq!(
        world.method_identity(&callee, &mut budget).unwrap().body,
        id(21)
    );
}
