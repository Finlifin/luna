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
