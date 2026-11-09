use super::*;
use comptime_interpretor::ErrorKind;
use nir::{DeclarationId, Primitive};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn ty(primitive: Primitive) -> Term {
    Term::Primitive(primitive)
}

fn fixture() -> (n::Package, n::Expr, n::Expr) {
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(ty(Primitive::Bool)),
    };
    let method = |local, name: &str| n::MethodContract {
        declaration: id(local),
        name: name.into(),
        phase: Default::default(),
        is_quote: false,
        pure: false,
        is_unsafe: false,
        type_parameters: vec![],
        type_arguments: vec![],
        signature: signature.clone().into(),
        requirements: vec![],
        capabilities: vec![],
        program: Default::default(),
        parameter_bindings: vec![],
        by_value_receiver: false,
        by_reference_receiver: false,
        default: None,
    };
    let implementation = |local, subject| n::Implementation {
        declaration: id(local),
        phases: Default::default(),
        interface: Some(id(1)),
        subject,
        type_parameters: vec![],
        parameter_types: vec![],
        interface_arguments: vec![],
        requirements: vec![],
        capabilities: vec![],
        methods: vec![
            ("first".into(), id(local + 1)),
            ("second".into(), id(local + 2)),
        ],
        associated: vec![],
        associated_initializers: vec![],
    };
    let package = n::Package {
        traits: vec![n::Trait {
            declaration: id(1),
            self_parameter: id(2),
            name: "Read".into(),
            comptime_only: false,
            type_parameters: vec![],
            parameter_types: vec![],
            associated: vec![],
            requirements: vec![],
            capabilities: vec![],
            parents: vec![],
            methods: vec![method(3, "first"), method(4, "second")],
        }],
        implementations: vec![
            implementation(5, ty(Primitive::Bool)),
            implementation(8, ty(Primitive::Char)),
        ],
        newtypes: (100..228)
            .map(|local| nir::newtypes::Newtype {
                declaration: id(local),
                name: format!("Declaration{local}"),
                type_parameters: vec![],
                parameter_types: vec![],
                capture_count: 0,
                requirements: vec![],
                capabilities: vec![],
                representation: ty(Primitive::Bool),
            })
            .collect(),
        ..Default::default()
    };
    let callee = |member, subject| n::Expr {
        source: n::SourceRange { start: 7, end: 11 },
        ty: signature.clone(),
        kind: K::TraitMethod {
            interface: id(1),
            member: id(member),
            subject,
            interface_arguments: vec![],
            method_arguments: vec![],
        },
    };
    (
        package,
        callee(3, ty(Primitive::Bool)),
        callee(4, ty(Primitive::Char)),
    )
}

#[test]
fn distinct_method_demands_share_metadata_but_still_pay_for_selection() {
    let (package, first, second) = fixture();
    let context = Context::from_package(&package).unwrap();
    let queries = SourceInvocationQueries::new(&package, &context, &[]);
    assert!(queries.world.borrow().is_none());
    let mut cold = 100_000;
    assert_eq!(queries.resolve(&first, &mut cold).unwrap().body, id(6));
    let mut warm = 100_000;
    assert_eq!(queries.resolve(&second, &mut warm).unwrap().body, id(10));
    assert!(warm < 100_000 && warm > cold);
    let fresh = SourceInvocationQueries::new(&package, &context, &[]);
    let mut independent = 100_000;
    assert_eq!(
        fresh.resolve(&second, &mut independent).unwrap().body,
        id(10)
    );
    assert!(
        independent < warm,
        "a new invocation must pay for its own query metadata"
    );
    assert!(queries.world.borrow().is_some());
}

#[test]
fn failed_construction_and_failed_demand_recover_without_unearned_authority() {
    let (package, first, _) = fixture();
    let context = Context::from_package(&package).unwrap();
    let queries = SourceInvocationQueries::new(&package, &context, &[]);
    for allowance in [0, 1, 16, 128] {
        let mut fuel = allowance;
        let error = queries.resolve(&first, &mut fuel).unwrap_err();
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(error.source, Some(first.source));
        assert!(fuel <= allowance);
        assert!(queries.world.borrow().is_none());
    }
    let mut invalid = first.clone();
    let K::TraitMethod { member, .. } = &mut invalid.kind else {
        unreachable!()
    };
    *member = id(999);
    let mut fuel = 100_000;
    assert_eq!(
        queries.resolve(&invalid, &mut fuel).unwrap_err().kind,
        ErrorKind::UnmaterializedDispatch
    );
    assert!(fuel > 0 && fuel < 100_000);
    assert!(
        queries.world.borrow().is_some(),
        "completed metadata survives a rejected member"
    );
    assert_eq!(queries.resolve(&first, &mut fuel).unwrap().body, id(6));
    let mut missing = first.clone();
    let K::TraitMethod { subject, .. } = &mut missing.kind else {
        unreachable!()
    };
    *subject = ty(Primitive::U8);
    for _ in 0..2 {
        let before = fuel;
        assert_eq!(
            queries.resolve(&missing, &mut fuel).unwrap_err().kind,
            ErrorKind::TypeContractUnresolved
        );
        assert!(fuel > 0 && fuel < before);
        assert_eq!(queries.resolve(&first, &mut fuel).unwrap().body, id(6));
    }
    let mut exhausted = 0;
    assert_eq!(
        queries.resolve(&first, &mut exhausted).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(queries.resolve(&first, &mut fuel).unwrap().body, id(6));
}

#[test]
fn source_and_premises_are_owned_by_each_query_scope() {
    let (mut package, first, _) = fixture();
    let mut parent = package.traits[0].clone();
    parent.declaration = id(40);
    parent.self_parameter = id(41);
    parent.methods.clear();
    parent.name = "Parent".into();
    package.traits.push(parent);
    package.traits[0].parents.push(n::InterfaceInstance {
        declaration: id(40),
        arguments: vec![],
    });
    let context = Context::from_package(&package).unwrap();
    let assumptions = [n::TraitRequirement {
        interface: id(40),
        interface_arguments: vec![],
        subject: ty(Primitive::Bool),
    }];
    let proved = SourceInvocationQueries::new(&package, &context, &assumptions);
    let unproved = SourceInvocationQueries::new(&package, &context, &[]);
    let mut changed = package.clone();
    changed.implementations[0].methods[0].1 = id(99);
    let changed_context = Context::from_package(&changed).unwrap();
    let other = SourceInvocationQueries::new(&changed, &changed_context, &assumptions);
    for _ in 0..2 {
        assert_eq!(proved.resolve(&first, &mut 100_000).unwrap().body, id(6));
        assert_eq!(
            unproved.resolve(&first, &mut 100_000).unwrap_err().kind,
            ErrorKind::TypeContractUnresolved
        );
        assert_eq!(other.resolve(&first, &mut 100_000).unwrap().body, id(99));
    }
}

#[test]
fn interrupted_query_reservations_do_not_refill_the_caller() {
    let (package, first, _) = fixture();
    let context = Context::from_package(&package).unwrap();
    let queries = SourceInvocationQueries::new(&package, &context, &[]);
    let guard = queries.world.borrow_mut();
    let mut fuel = 100_000;
    // Force an unwind before a query can settle its reserved allowance.
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        queries.resolve(&first, &mut fuel)
    }));
    assert!(interrupted.is_err());
    assert_eq!(fuel, 0);
    drop(guard);
    assert!(queries.world.borrow().is_none());
    assert_eq!(queries.resolve(&first, &mut 100_000).unwrap().body, id(6));
}

fn mixed_fixture() -> (n::Package, n::Expr, n::Expr, n::TypePredicate) {
    let (mut package, method, _) = fixture();
    package.traits[0].associated.push(n::AssociatedContract {
        phase: Default::default(),
        declaration: id(50),
        name: "enabled".into(),
        ty: ty(Primitive::Bool),
        default: None,
    });
    package.implementations[0].associated.push((
        id(50),
        n::Expr {
            source: method.source,
            ty: ty(Primitive::Bool),
            kind: K::Literal(n::Literal::Bool(true)),
        },
    ));
    let access = n::Expr {
        source: method.source,
        ty: ty(Primitive::Bool),
        kind: K::AssociatedValue {
            interface: n::InterfaceInstance {
                declaration: id(1),
                arguments: vec![],
            },
            subject: ty(Primitive::Bool),
            member: id(50),
        },
    };
    let predicate = n::TypePredicate::Trait(n::TraitRequirement {
        interface: id(1),
        interface_arguments: vec![],
        subject: ty(Primitive::Bool),
    });
    (package, method, access, predicate)
}

#[test]
fn all_three_services_share_one_construction_regardless_of_first_demand() {
    use comptime_interpretor::ContractOutcome;
    let (package, method, access, predicate) = mixed_fixture();
    let context = Context::from_package(&package).unwrap();
    let demand = |queries: &SourceInvocationQueries<'_>, which, fuel: &mut u64| match which {
        0 => assert_eq!(
            queries.check(&predicate, Budget::default(), fuel),
            ContractOutcome::Satisfied
        ),
        1 => assert_eq!(queries.resolve(&method, fuel).unwrap().body, id(6)),
        2 => {
            let selected = queries.associated(&access, fuel).unwrap();
            assert!(matches!(
                selected.initializer.expression().kind,
                K::Literal(n::Literal::Bool(true))
            ));
            assert!(!selected.requires_invocation);
        }
        _ => unreachable!(),
    };
    for first in 0..3 {
        let queries = SourceInvocationQueries::new(&package, &context, &[]);
        demand(&queries, first, &mut 100_000);
        for which in 0..3 {
            let mut warm = 100_000;
            demand(&queries, which, &mut warm);
            let independent = SourceInvocationQueries::new(&package, &context, &[]);
            let mut cold = 100_000;
            demand(&independent, which, &mut cold);
            assert!(
                cold < warm && warm < 100_000,
                "service {which} after {first}"
            );
        }
    }
}

#[test]
fn unknown_and_rejected_demands_do_not_poison_other_query_services() {
    use comptime_interpretor::ContractOutcome;
    let (package, method, access, predicate) = mixed_fixture();
    let context = Context::from_package(&package).unwrap();
    let queries = SourceInvocationQueries::new(&package, &context, &[]);
    // A per-proof cap is charged inside the invocation; unused outer fuel is
    // retained, but failure cannot turn an incomplete proof into rejection.
    let mut fuel = 100_000;
    let mut charged_partial_work = false;
    for limit in [0, 1, 256] {
        let before = fuel;
        assert_eq!(
            queries.check(
                &predicate,
                Budget {
                    steps: limit,
                    ..Budget::default()
                },
                &mut fuel
            ),
            ContractOutcome::Unknown,
        );
        // A bulk preflight can reject before doing work. Once some metadata
        // has been examined, its work is retained without charging the whole
        // outer allowance or installing a partial world.
        assert!(fuel <= before && before - fuel <= limit as u64);
        charged_partial_work |= fuel < before;
        assert!(queries.world.borrow().is_none());
    }
    assert!(charged_partial_work);
    assert_eq!(
        queries
            .associated(&access, &mut fuel)
            .unwrap()
            .initializer
            .expression()
            .kind,
        K::Literal(n::Literal::Bool(true))
    );
    assert_eq!(
        queries.check(&predicate, Budget::default(), &mut 0),
        ContractOutcome::Unknown
    );
    assert_eq!(queries.resolve(&method, &mut fuel).unwrap().body, id(6));
    let rejected = n::TypePredicate::Equal(ty(Primitive::Bool), ty(Primitive::Char));
    assert_eq!(
        queries.check(&rejected, Budget::default(), &mut fuel),
        ContractOutcome::Rejected
    );
    let mut invalid = access.clone();
    let K::AssociatedValue { member, .. } = &mut invalid.kind else {
        unreachable!()
    };
    *member = id(999);
    let error = queries.associated(&invalid, &mut fuel).err().unwrap();
    assert_eq!(error.kind, ErrorKind::UnmaterializedDispatch);
    assert_eq!(error.source, Some(invalid.source));
    assert_eq!(
        queries.check(&predicate, Budget::default(), &mut fuel),
        ContractOutcome::Satisfied
    );
    assert_eq!(queries.resolve(&method, &mut fuel).unwrap().body, id(6));
    assert!(queries.associated(&access, &mut fuel).is_ok());
}

#[test]
fn restored_proof_context_is_local_to_the_demand() {
    use comptime_interpretor::ContractOutcome;
    let (package, method, access, predicate) = mixed_fixture();
    let context = Context::from_package(&package).unwrap();
    let bound = Term::Bound {
        depth: 0,
        index: 0,
        sort: nir::Sort::Type,
    };
    let scoped_predicate = n::TypePredicate::Equal(bound.clone(), ty(Primitive::Bool));
    let mut restored = context.theory_in_scope(vec![vec![nir::Sort::Type]]);
    restored.assume(ptc::Formula::equal(bound, ty(Primitive::Bool)));
    let queries = SourceInvocationQueries::new(&package, &context, &[]);
    for _ in 0..2 {
        assert_eq!(
            queries.check_in_context(
                &scoped_predicate,
                &restored,
                Budget::default(),
                &mut 100_000
            ),
            ContractOutcome::Satisfied
        );
        assert_eq!(
            queries.check(&scoped_predicate, Budget::default(), &mut 100_000),
            ContractOutcome::Unknown
        );
        assert_eq!(
            queries.check(&predicate, Budget::default(), &mut 100_000),
            ContractOutcome::Satisfied
        );
        assert_eq!(queries.resolve(&method, &mut 100_000).unwrap().body, id(6));
        assert!(queries.associated(&access, &mut 100_000).is_ok());
    }
}

#[test]
fn identical_ids_in_changed_sources_do_not_share_associated_or_type_authority() {
    use comptime_interpretor::ContractOutcome;
    let (package, method, access, predicate) = mixed_fixture();
    let context = Context::from_package(&package).unwrap();
    let original = SourceInvocationQueries::new(&package, &context, &[]);
    let mut changed = package.clone();
    changed.implementations[0].subject = ty(Primitive::U8);
    let changed_context = Context::from_package(&changed).unwrap();
    let other = SourceInvocationQueries::new(&changed, &changed_context, &[]);
    for _ in 0..2 {
        assert_eq!(
            original.check(&predicate, Budget::default(), &mut 100_000),
            ContractOutcome::Satisfied
        );
        assert_ne!(
            other.check(&predicate, Budget::default(), &mut 100_000),
            ContractOutcome::Satisfied
        );
        assert!(original.associated(&access, &mut 100_000).is_ok());
        assert_eq!(
            other.associated(&access, &mut 100_000).err().unwrap().kind,
            ErrorKind::TypeContractUnresolved
        );
        assert_eq!(original.resolve(&method, &mut 100_000).unwrap().body, id(6));
        assert_eq!(
            other.resolve(&method, &mut 100_000).unwrap_err().kind,
            ErrorKind::TypeContractUnresolved
        );
    }
}
