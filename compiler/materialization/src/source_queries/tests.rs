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
        source: nir::program::SourceRange { start: 0, end: 0 },
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
fn direct_type_answers_leave_implementation_metadata_cold() {
    use comptime_interpretor::ContractOutcome;
    use n::TypePredicate as P;
    let (package, _, _, _) = mixed_fixture();
    let context = Context::from_package(&package).unwrap();
    let queries = SourceInvocationQueries::new(&package, &context, &[]);
    let different = P::Equal(ty(Primitive::Bool), ty(Primitive::U8));
    let mut fuel = 100_000;
    assert_eq!(
        queries.check(&different, Budget::default(), &mut fuel),
        Ok(ContractOutcome::Rejected)
    );
    assert_eq!(
        queries.check(&P::Not(Box::new(different)), Budget::default(), &mut fuel),
        Ok(ContractOutcome::Satisfied)
    );
    assert!(fuel < 100_000);
    assert!(queries.world.borrow().is_none());
}

#[test]
fn traits_in_restored_premises_can_decide_an_equality_goal() {
    use comptime_interpretor::ContractOutcome;
    let (package, _, _, implements) = mixed_fixture();
    let context = Context::from_package(&package).unwrap();
    let queries = SourceInvocationQueries::new(&package, &context, &[]);
    let equality = n::TypePredicate::Equal(
        Term::Rigid {
            declaration: id(120),
            sort: nir::Sort::Type,
        },
        ty(Primitive::Bool),
    );
    let mut restored = context.fork();
    restored.assume(
        ptc::Formula::from_predicate(&implements).implies(ptc::Formula::from_predicate(&equality)),
    );
    assert_eq!(
        queries.check_in_context(&equality, &restored, Budget::default(), &mut 100_000),
        Ok(ContractOutcome::Satisfied)
    );
    assert!(queries.world.borrow().is_some());
    assert_ne!(
        queries.check(&equality, Budget::default(), &mut 100_000),
        Ok(ContractOutcome::Satisfied)
    );
}

#[test]
fn all_three_services_share_one_construction_regardless_of_first_demand() {
    use comptime_interpretor::ContractOutcome;
    let (package, method, access, predicate) = mixed_fixture();
    let context = Context::from_package(&package).unwrap();
    let demand = |queries: &SourceInvocationQueries<'_>, which, fuel: &mut u64| match which {
        0 => assert_eq!(
            queries.check(&predicate, Budget::default(), fuel),
            Ok(ContractOutcome::Satisfied)
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
fn exhausted_and_rejected_demands_do_not_poison_other_query_services() {
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
            Err(comptime_interpretor::ErrorKind::StepsExhausted),
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
        Err(comptime_interpretor::ErrorKind::StepsExhausted)
    );
    assert_eq!(queries.resolve(&method, &mut fuel).unwrap().body, id(6));
    let rejected = n::TypePredicate::Equal(ty(Primitive::Bool), ty(Primitive::Char));
    assert_eq!(
        queries.check(&rejected, Budget::default(), &mut fuel),
        Ok(ContractOutcome::Rejected)
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
        Ok(ContractOutcome::Satisfied)
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
    let open = restored.fork();
    let n::TypePredicate::Trait(mut requirement) = predicate.clone() else {
        unreachable!("fixture's implemented interface")
    };
    requirement.subject = bound.clone();
    let scoped_trait = n::TypePredicate::Trait(requirement);
    restored.assume(ptc::Formula::equal(bound, ty(Primitive::Bool)));
    let queries = SourceInvocationQueries::new(&package, &context, &[]);
    for _ in 0..2 {
        assert_eq!(
            queries.check_in_context(&scoped_trait, &open, Budget::default(), &mut 100_000),
            Ok(ContractOutcome::Unknown)
        );
        assert_eq!(
            queries.check_in_context(&scoped_trait, &restored, Budget::default(), &mut 100_000),
            Ok(ContractOutcome::Satisfied)
        );
        assert_eq!(
            queries.check_in_context(
                &scoped_predicate,
                &restored,
                Budget::default(),
                &mut 100_000
            ),
            Ok(ContractOutcome::Satisfied)
        );
        assert!(matches!(
            queries.check(&scoped_predicate, Budget::default(), &mut 100_000),
            Err(comptime_interpretor::ErrorKind::InvalidTypeContract(_))
        ));
        assert_eq!(
            queries.check(&predicate, Budget::default(), &mut 100_000),
            Ok(ContractOutcome::Satisfied)
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
            Ok(ContractOutcome::Satisfied)
        );
        assert_ne!(
            other.check(&predicate, Budget::default(), &mut 100_000),
            Ok(ContractOutcome::Satisfied)
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

#[test]
fn explicit_directory_preparation_is_atomic_and_shared_with_invocation_queries() {
    use std::sync::Arc;
    let (package, method, _, _) = mixed_fixture();
    let context = Context::from_package(&package).unwrap();
    let snapshot = DeclarationSnapshot::new(Arc::new(package));
    for allowance in [0, 1] {
        let mut budget = Budget {
            steps: allowance,
            ..Default::default()
        };
        assert!(matches!(
            snapshot.prepare(&mut budget),
            Err(nir::NominalAdmissionError::Budget)
        ));
        assert!(
            snapshot
                .preparation()
                .to_string()
                .ends_with("(metadata pending))")
        );
    }
    let mut budget = Budget::default();
    snapshot.prepare(&mut budget).unwrap();
    assert!(budget.steps < Budget::default().steps);
    let mut empty = Budget {
        steps: 0,
        ..Default::default()
    };
    snapshot.prepare(&mut empty).unwrap();
    let proof = snapshot.capabilities(&context, &[], &mut empty).unwrap();
    let queries =
        SourceInvocationQueries::in_source(DeclarationSource::Snapshot(&snapshot), &context, &[]);
    assert_eq!(
        queries.resolve(&method, &mut 0).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(queries.resolve(&method, &mut 100_000).unwrap().body, id(6));
    assert!(std::ptr::eq(
        proof.index,
        queries.world.borrow().as_ref().unwrap().index()
    ));
}

#[test]
fn closed_declarations_charge_both_services_before_serving_vm_queries() {
    use std::sync::Arc;
    let (package, method, _, predicate) = mixed_fixture();
    let package = Arc::new(package);
    let source = DeclarationSnapshot::new(package.clone());
    let mut measure = Budget::default();
    DeclarationSnapshot::new(package)
        .package_context(&mut measure.steps)
        .unwrap();
    let mut theory_only = Budget {
        steps: Budget::default().steps - measure.steps,
        ..Budget::default()
    };
    assert!(matches!(
        source.closed(&mut theory_only),
        Err(nir::NominalAdmissionError::Budget)
    ));
    assert_eq!(theory_only.steps, 0);
    assert!(source.package_context(&mut 1).is_ok());

    let closed = source.closed(&mut Budget::default()).unwrap();
    let queries = SourceInvocationQueries::in_closed(&closed);
    assert_eq!(
        queries.check(&predicate, Budget::default(), &mut 100_000),
        Ok(comptime_interpretor::ContractOutcome::Satisfied)
    );
    assert_eq!(queries.resolve(&method, &mut 100_000).unwrap().body, id(6));
    assert!(std::ptr::eq(
        closed.capabilities().index,
        queries.world.borrow().as_ref().unwrap().index()
    ));
    let sibling = source
        .closed(&mut Budget {
            steps: 2,
            ..Default::default()
        })
        .unwrap();
    assert!(std::ptr::eq(
        closed.capabilities().index,
        sibling.capabilities().index
    ));
    assert!(sibling.context().assumptions().is_empty());
}

#[test]
fn retained_snapshots_share_metadata_across_fresh_invocation_scopes() {
    use comptime_interpretor::ContractOutcome;
    use std::sync::Arc;
    let (mut package, method, access, predicate) = mixed_fixture();
    package.newtypes[0].name = "retained\n\"source\"".into();
    let package = Arc::new(package);
    let weak = Arc::downgrade(&package);
    let context = Context::from_package(&package).unwrap();
    let snapshot = DeclarationSnapshot::new(package);
    let pending_dump = snapshot.to_string();
    assert!(pending_dump.starts_with("(declaration-snapshot (source ("));
    assert!(pending_dump.contains(r#"retained\n\"source\""#));
    assert!(
        snapshot
            .preparation()
            .to_string()
            .ends_with("(metadata pending))")
    );
    let sibling = snapshot.clone();
    let first =
        SourceInvocationQueries::in_source(DeclarationSource::Snapshot(&snapshot), &context, &[]);
    let mut cold = 100_000;
    assert_eq!(
        first.check(&predicate, Budget::default(), &mut cold),
        Ok(ContractOutcome::Satisfied)
    );
    let next =
        SourceInvocationQueries::in_source(DeclarationSource::Snapshot(&sibling), &context, &[]);
    let mut warm = 100_000;
    assert_eq!(
        next.check(&predicate, Budget::default(), &mut warm),
        Ok(ContractOutcome::Satisfied)
    );
    assert!(cold < warm && warm < 100_000);
    {
        let first = first.world.borrow();
        let next = next.world.borrow();
        assert!(std::ptr::eq(
            first.as_ref().unwrap().index(),
            next.as_ref().unwrap().index()
        ));
    }
    assert_eq!(next.resolve(&method, &mut warm).unwrap().body, id(6));
    assert!(next.associated(&access, &mut warm).is_ok());
    assert_eq!(snapshot.to_string(), pending_dump);
    let completed_dump = snapshot.preparation().to_string();
    assert!(completed_dump.contains("(metadata (implementation-index"));
    assert_eq!(completed_dump, sibling.preparation().to_string());
    drop(first);
    drop(next);
    drop(snapshot);
    assert!(weak.upgrade().is_some());
    // Only the directory survives. A new invocation still needs its own
    // allowance and cannot inherit a prior successful method decision.
    let next =
        SourceInvocationQueries::in_source(DeclarationSource::Snapshot(&sibling), &context, &[]);
    assert_eq!(
        next.resolve(&method, &mut 0).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(next.resolve(&method, &mut 100_000).unwrap().body, id(6));
    drop(next);
    drop(sibling);
    assert!(weak.upgrade().is_none());
}

#[test]
fn retained_metadata_cannot_share_premises_or_changed_source_authority() {
    use std::sync::Arc;
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
    let snapshot = DeclarationSnapshot::new(Arc::new(package));
    let assumptions = [n::TraitRequirement {
        interface: id(40),
        interface_arguments: vec![],
        subject: ty(Primitive::Bool),
    }];
    let with_premise = SourceInvocationQueries::in_source(
        DeclarationSource::Snapshot(&snapshot),
        &context,
        &assumptions,
    );
    let without_premise =
        SourceInvocationQueries::in_source(DeclarationSource::Snapshot(&snapshot), &context, &[]);
    for _ in 0..2 {
        assert_eq!(
            with_premise.resolve(&first, &mut 100_000).unwrap().body,
            id(6)
        );
        assert_eq!(
            without_premise
                .resolve(&first, &mut 100_000)
                .unwrap_err()
                .kind,
            ErrorKind::TypeContractUnresolved
        );
    }
    let mut changed = Arc::clone(snapshot.package());
    Arc::make_mut(&mut changed).implementations[0].methods[0].1 = id(99);
    let changed = DeclarationSnapshot::new(changed);
    let other = SourceInvocationQueries::in_source(
        DeclarationSource::Snapshot(&changed),
        &context,
        &assumptions,
    );
    assert_eq!(other.resolve(&first, &mut 100_000).unwrap().body, id(99));
    assert_eq!(
        with_premise.resolve(&first, &mut 100_000).unwrap().body,
        id(6)
    );
    assert!(!std::ptr::eq(
        with_premise.world.borrow().as_ref().unwrap().index(),
        other.world.borrow().as_ref().unwrap().index(),
    ));
}

#[test]
fn failed_retained_construction_is_retryable_without_publishing_partial_metadata() {
    use std::sync::Arc;
    let (package, method, _, predicate) = mixed_fixture();
    let context = Context::from_package(&package).unwrap();
    let snapshot = DeclarationSnapshot::new(Arc::new(package));
    let queries =
        SourceInvocationQueries::in_source(DeclarationSource::Snapshot(&snapshot), &context, &[]);
    for allowance in [0, 1, 128] {
        assert_eq!(
            queries.check(&predicate, Budget::default(), &mut { allowance }),
            Err(comptime_interpretor::ErrorKind::StepsExhausted)
        );
        assert!(queries.world.borrow().is_none());
    }
    let mut recovered = 100_000;
    assert_eq!(
        queries.resolve(&method, &mut recovered).unwrap().body,
        id(6)
    );
    let independent = DeclarationSnapshot::new(snapshot.package().clone());
    let independent_queries = SourceInvocationQueries::in_source(
        DeclarationSource::Snapshot(&independent),
        &context,
        &[],
    );
    let mut cold = 100_000;
    assert_eq!(
        independent_queries
            .resolve(&method, &mut cold)
            .unwrap()
            .body,
        id(6)
    );
    assert_eq!(
        recovered, cold,
        "failed construction cannot leave a discounted partial directory"
    );
    let next =
        SourceInvocationQueries::in_source(DeclarationSource::Snapshot(&snapshot), &context, &[]);
    let mut warm = 100_000;
    assert_eq!(next.resolve(&method, &mut warm).unwrap().body, id(6));
    assert!(warm > cold && warm < 100_000);
}
