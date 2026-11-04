//! Preserve type-query failure categories through the invocation boundary.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_executor::{Context, Error};
use comptime_interpretor::Value;
use nir::{Primitive, Term};

#[test]
fn invocation_contract_exhaustion_is_not_a_protocol_error_or_a_cached_result() {
    let package = sysroot::compile(
        "trait Marker {}
        impl Marker for bool {}
        pure comptime fn accept(comptime T: Type) -> bool
        where requires T :- Marker
        {
            true
        }",
    )
    .unwrap();
    let declaration = package
        .functions
        .iter()
        .find(|f| f.name == "accept")
        .unwrap()
        .declaration;
    let mut call = luna::memoization::Invocation::prepare(
        &package,
        &intrinsic::LangItems::new(),
        declaration,
        vec![],
        8,
        Default::default(),
    )
    .unwrap();
    for steps in [0, 1, 8, 16, 32] {
        let mut budget = Context::new(steps);
        let result = call.invoke(
            vec![Value::Type(Term::Primitive(Primitive::Bool))],
            &mut budget,
        );
        let error = result
            .err()
            .expect("insufficient contract budget must reject");
        assert!(
            matches!(error, Error::BudgetExhausted),
            "steps {steps}: {error:?}"
        );
        assert_eq!(call.last_execution_stats().bytecode_invocations, 0);
        assert_eq!(call.cached_results(), 0);
        assert!(budget.remaining() <= steps);
    }
    let rejected = call.invoke(
        vec![Value::Type(Term::Primitive(Primitive::Char))],
        &mut Context::new(1_000_000),
    );
    let error = rejected.err().expect("unimplemented Marker must reject");
    assert!(matches!(error, Error::Protocol(_)), "{error:?}");
    assert_eq!(call.last_execution_stats().bytecode_invocations, 0);
    assert_eq!(call.cached_results(), 0);
    for _ in 0..2 {
        let value = call
            .invoke(
                vec![Value::Type(Term::Primitive(Primitive::Bool))],
                &mut Context::new(1_000_000),
            )
            .unwrap();
        assert_eq!(*value.value, Value::Bool(true));
        assert!(!value.reused);
        assert!(call.last_execution_stats().bytecode_invocations > 0);
        assert_eq!(call.cached_results(), 0);
    }
}

#[test]
fn associated_selection_returns_actual_fuel_and_typed_failures_in_the_sysroot_world() {
    use comptime_interpretor::ErrorKind;
    use nir::{NominalAdmissionError as Failure, program as n};
    use type_resolution::{capabilities::World, dispatch::DeclarationWorld, ptc::Budget};
    let package = sysroot::compile(
        "trait Flag { assoc enabled: bool; }
        impl Flag for bool { assoc enabled: bool = true; }
        pure comptime fn enabled(comptime T: Type) -> bool
        where requires T :- Flag
        {
            T.as(Flag).enabled
        }",
    )
    .unwrap();
    let contract = package.traits.iter().find(|t| t.name == "Flag").unwrap();
    let interface = n::InterfaceInstance {
        declaration: contract.declaration,
        arguments: vec![],
    };
    let member = contract.associated[0].declaration;
    let subject = Term::Primitive(Primitive::Bool);
    let access = n::Expr {
        ty: subject.clone(),
        source: n::SourceRange { start: 7, end: 11 },
        kind: n::ExprKind::AssociatedValue {
            interface: interface.clone(),
            member,
            subject: subject.clone(),
        },
    };
    let (declarations, _) = DeclarationWorld::new(
        &package,
        Budget {
            steps: 1_000_000,
            ..Budget::default()
        },
        None,
    )
    .unwrap();
    let world = World {
        index: declarations.index(),
        implementations: &package.implementations,
        traits: &package.traits,
        assumptions: declarations.assumptions(),
        context: declarations.context(),
    };
    let mut successes = 0;
    let mut exhausted = 0;
    for steps in [0, 1, 8, 16, 32, 64, 128, 256, 1024, 65_536] {
        let mut budget = Budget {
            steps,
            ..Budget::default()
        };
        let expected = world
            .resolve_associated_value_plan_metered(&interface, member, &subject, &mut budget)
            .and_then(|selected| {
                nir::associated_initializers::AssociatedInitializer::new(
                    selected.value,
                    &mut budget.steps,
                )
            });
        let mut remaining = steps as u64;
        let actual = elaboration::materialization::resolve_associated_value_for_invocation(
            &world,
            &access,
            &mut remaining,
        );
        assert_eq!(remaining, budget.steps as u64, "steps {steps}");
        match (expected, actual) {
            (Ok(expected), Ok(actual)) => {
                successes += 1;
                assert_eq!(actual.initializer, expected);
                assert!(!actual.requires_invocation);
            }
            (Err(Failure::Budget), Err(error)) => {
                exhausted += 1;
                assert_eq!(error.kind, ErrorKind::StepsExhausted);
                assert_eq!(error.source, Some(access.source));
            }
            (expected, actual) => panic!("steps {steps}: {expected:?}, {:?}", actual.err()),
        }
    }
    assert!(successes > 0 && exhausted > 0);
    for (bad_member, bad_subject, kind) in [
        (
            nir::DeclarationId {
                package: u64::MAX,
                local: u32::MAX,
            },
            subject.clone(),
            ErrorKind::UnmaterializedDispatch,
        ),
        (
            member,
            Term::Primitive(Primitive::Char),
            ErrorKind::TypeContractUnresolved,
        ),
    ] {
        let invalid = n::Expr {
            kind: n::ExprKind::AssociatedValue {
                interface: interface.clone(),
                member: bad_member,
                subject: bad_subject,
            },
            ..access.clone()
        };
        let mut remaining = 100_000;
        let error = elaboration::materialization::resolve_associated_value_for_invocation(
            &world,
            &invalid,
            &mut remaining,
        )
        .err()
        .expect("invalid or unresolved access must fail");
        assert_eq!(error.kind, kind);
        assert!(remaining > 0 && remaining < 100_000);
        assert!(
            elaboration::materialization::resolve_associated_value_for_invocation(
                &world,
                &access,
                &mut remaining
            )
            .is_ok()
        );
    }
    let declaration = package
        .functions
        .iter()
        .find(|f| f.name == "enabled")
        .unwrap()
        .declaration;
    let mut invocation = luna::memoization::Invocation::prepare(
        &package,
        &intrinsic::LangItems::new(),
        declaration,
        vec![],
        8,
        Default::default(),
    )
    .unwrap();
    for _ in 0..2 {
        let outcome = invocation
            .invoke(
                vec![Value::Type(subject.clone())],
                &mut Context::new(1_000_000),
            )
            .unwrap();
        assert_eq!(*outcome.value, Value::Bool(true));
        assert!(!outcome.reused);
        assert_eq!(invocation.cached_results(), 0);
        assert!(invocation.last_execution_stats().bytecode_invocations > 0);
    }
}

#[test]
fn nested_source_capabilities_preserve_exhaustion_through_invocation() {
    use nir::NominalAdmissionError as Failure;
    use type_resolution::{capabilities::World, dispatch::DeclarationWorld, ptc::Budget};
    let source = "trait First {} trait Second {} trait Third {} trait Fourth {} trait Last {}
        impl First for T where T :- Second {}
        impl Second for T where T :- Third {}
        impl Third for T where T :- Fourth {}
        impl Fourth for T where T :- Last {}
        impl Last for bool {}
        pure comptime fn accept(comptime T: Type) -> bool
        where requires T :- First
        {
            true
        }";
    let package = sysroot::compile(source).unwrap();
    let first = package
        .traits
        .iter()
        .find(|t| t.name == "First")
        .unwrap()
        .declaration;
    let implementation = package
        .implementations
        .iter()
        .find(|i| i.interface == Some(first))
        .unwrap();
    assert!(
        !implementation.capabilities.is_empty(),
        "source must exercise capability qualification"
    );
    let (declarations, _) = DeclarationWorld::new(&package, Budget::default(), None).unwrap();
    let world = World {
        index: declarations.index(),
        implementations: &package.implementations,
        traits: &package.traits,
        assumptions: declarations.assumptions(),
        context: declarations.context(),
    };
    let subject = Term::Primitive(Primitive::Bool);
    let mut shallow = Budget {
        max_depth: 4,
        ..Budget::default()
    };
    assert!(matches!(
        world.resolve_instance_metered(&first.into(), &subject, &mut shallow),
        Err(Failure::Budget)
    ));
    assert!(shallow.steps > 0 && shallow.steps < Budget::default().steps);
    world.resolve(first, &subject, Budget::default()).unwrap();
    let declaration = package
        .functions
        .iter()
        .find(|f| f.name == "accept")
        .unwrap()
        .declaration;
    let mut call = luna::memoization::Invocation::prepare(
        &package,
        &intrinsic::LangItems::new(),
        declaration,
        vec![],
        8,
        Default::default(),
    )
    .unwrap();
    for steps in [32, 128, 512] {
        let mut budget = Context::new(steps);
        match call.invoke(vec![Value::Type(subject.clone())], &mut budget) {
            Err(Error::BudgetExhausted) => {
                assert_eq!(call.last_execution_stats().bytecode_invocations, 0)
            }
            Err(error) => panic!("steps {steps}: {error:?}"),
            Ok(value) => assert_eq!(*value.value, Value::Bool(true)),
        }
        assert!(budget.remaining() <= steps);
        assert_eq!(call.cached_results(), 0);
    }
    let rejected = call.invoke(
        vec![Value::Type(Term::Primitive(Primitive::Char))],
        &mut Context::new(1_000_000),
    );
    assert!(matches!(rejected, Err(Error::Protocol(_))));
    assert_eq!(call.last_execution_stats().bytecode_invocations, 0);
    for _ in 0..2 {
        let value = call
            .invoke(
                vec![Value::Type(subject.clone())],
                &mut Context::new(1_000_000),
            )
            .unwrap();
        assert_eq!(*value.value, Value::Bool(true));
        assert!(!value.reused);
        assert_eq!(call.cached_results(), 0);
    }
}

#[test]
fn source_inherent_queries_and_cached_type_views_keep_failure_boundaries() {
    use nir::{AssociatedTypeOrigin, NominalAdmissionError as Failure};
    use std::collections::HashMap;
    use type_resolution::{
        capabilities::World,
        dispatch::DeclarationWorld,
        implementation::{InherentHead, InherentIndex, Selection},
        ptc::Budget,
    };
    let package = sysroot::compile(
        "trait First {} trait Second {} trait Third {} trait Fourth {} trait Last {}
        impl First for T where T :- Second {}
        impl Second for T where T :- Third {}
        impl Third for T where T :- Fourth {}
        impl Fourth for T where T :- Last {}
        impl Last for bool {}
        trait Element { assoc Item: Type; }
        impl Element for bool { assoc Item: Type = meta.List<bool>; }
        struct Holder where T {}
        impl Holder<T> where T :- First {
            comptime fn enabled() -> bool { true }
        }
        comptime fn run() -> bool { Holder<bool>.enabled() }",
    )
    .unwrap();
    let holder = package
        .structs
        .iter()
        .find(|s| s.name == "Holder")
        .unwrap()
        .declaration;
    let candidate = package
        .implementations
        .iter()
        .find(|i| {
            i.interface.is_none()
                && matches!(&i.subject, Term::Nominal { declaration, .. } if *declaration == holder)
        })
        .unwrap();
    assert!(!candidate.capabilities.is_empty());
    let mut inherent = InherentIndex::for_builtin_packages(
        package.target,
        package.builtin_implementation_packages.iter().copied(),
    );
    inherent.set_nominal_layouts(
        package.structs.clone(),
        package.enums.clone(),
        package.newtypes.clone(),
    );
    inherent.set_normal_form_metadata(package.normal_forms.clone());
    inherent
        .insert(
            InherentHead {
                parameter_types: candidate.parameter_types.clone(),
                declaration: candidate.declaration,
                subject: candidate.subject.clone(),
                type_parameters: candidate.type_parameters.clone(),
                requirements: candidate.requirements.clone(),
                members: candidate
                    .methods
                    .iter()
                    .map(|(name, _)| name.clone())
                    .collect(),
            },
            Budget::default(),
        )
        .unwrap();
    let (declarations, _) = DeclarationWorld::new(&package, Budget::default(), None).unwrap();
    let world = World {
        index: declarations.index(),
        implementations: &package.implementations,
        traits: &package.traits,
        assumptions: declarations.assumptions(),
        context: declarations.context(),
    };
    let subject = Term::Nominal {
        declaration: holder,
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    for budget in [
        Budget {
            steps: 0,
            ..Budget::default()
        },
        Budget {
            max_depth: 4,
            ..Budget::default()
        },
    ] {
        assert!(matches!(
            world.resolve_inherent(&inherent, "enabled", &subject, budget),
            Err(Failure::Budget)
        ));
    }
    let resolution = world
        .resolve_inherent(&inherent, "enabled", &subject, Budget::default())
        .unwrap();
    assert_eq!(
        resolution.types.selection,
        Selection::Selected(candidate.declaration)
    );
    world
        .verify_inherent_evidence(
            &inherent,
            "enabled",
            &subject,
            &resolution,
            Budget::default(),
        )
        .unwrap();
    assert_eq!(
        world
            .resolve_inherent(
                &inherent,
                "enabled",
                &Term::Nominal {
                    declaration: holder,
                    arguments: vec![Term::Primitive(Primitive::Char)],
                },
                Budget::default()
            )
            .unwrap()
            .types
            .selection,
        Selection::NoApplicableCandidate
    );

    let element = package.traits.iter().find(|t| t.name == "Element").unwrap();
    let projection = Term::AssociatedType {
        origin: Box::new(AssociatedTypeOrigin {
            interface: element.declaration,
            member: element.associated[0].declaration,
        }),
        subject: Box::new(Term::Primitive(Primitive::Bool)),
        arguments: vec![],
    };
    let mut cache = HashMap::new();
    let mut budget = Budget::default();
    let view = world
        .associated_type_view_cached_metered(&projection, &mut budget, &mut cache)
        .unwrap();
    assert!(
        matches!(view.value, Term::NormalForm { .. }),
        "the List head remains opaque"
    );
    assert!(budget.steps < Budget::default().steps);
    let before = cache.clone();
    let mut bad = projection.clone();
    if let Term::AssociatedType { origin, .. } = &mut bad {
        origin.member = nir::DeclarationId {
            package: u64::MAX,
            local: u32::MAX,
        };
    }
    let mut failed = Budget::default();
    assert!(matches!(
        world.associated_type_view_cached_metered(&bad, &mut failed, &mut cache),
        Err(Failure::Invalid(_))
    ));
    assert!(failed.steps > 0 && failed.steps < Budget::default().steps);
    assert_eq!(cache, before);
    assert_eq!(
        world
            .associated_type_view_cached_metered(&projection, &mut failed, &mut cache)
            .unwrap()
            .value,
        view.value
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}
