//! Classification constrains candidate domains; it does not pick a type.
use nir::{DeclarationId, Primitive, Sort, Term};
use ptc::{Budget, Context, Formula as F, KindQuery, TypeInputOutcome, TypeKind};

fn id(local: u32) -> DeclarationId {
    DeclarationId {
        package: 984,
        local,
    }
}
fn input(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}
fn unsigned(local: u32) -> F {
    F::Kind(KindQuery {
        ty: input(local),
        kind: TypeKind::UnsignedInteger,
    })
}
fn constraint() -> F {
    F::And(vec![
        unsigned(0),
        unsigned(1),
        F::equal(input(0), Term::Primitive(Primitive::U32)).negate(),
        F::equal(input(0), input(1)).negate(),
        F::equal(input(2), Term::Optional(Box::new(input(0)))),
    ])
}

#[test]
fn classified_type_search_backtracks_across_related_inputs() {
    let context = Context::default();
    let inputs = [id(0), id(1), id(2)];
    let outcome = context.search_type_inputs(&inputs, constraint(), &mut Budget::default());
    let TypeInputOutcome::Ambiguous {
        candidate,
        alternative,
        ..
    } = outcome
    else {
        panic!("classification and exclusions leave multiple types: {outcome}")
    };
    let problem = context
        .type_input_context(&inputs, constraint(), &mut Budget::default())
        .unwrap();
    for witness in [candidate, alternative] {
        problem.verify_witness(&witness, Budget::default()).unwrap();
        assert_eq!(
            TypeKind::UnsignedInteger.evaluate(&witness.types[&id(0)]),
            Some(true)
        );
        assert_ne!(witness.types[&id(0)], Term::Primitive(Primitive::U32));
        assert_ne!(witness.types[&id(0)], witness.types[&id(1)]);
        assert_eq!(
            witness.types[&id(2)],
            Term::Optional(Box::new(witness.types[&id(0)].clone()))
        );
        let mut forged = witness;
        forged.types.insert(id(0), Term::Primitive(Primitive::U32));
        assert!(problem.verify_witness(&forged, Budget::default()).is_err());
    }
    assert!(context.assumptions().is_empty());
}

#[test]
fn classified_search_budget_exhaustion_does_not_publish_a_choice() {
    let context = Context::default();
    let inputs = [id(0), id(1), id(2)];
    for steps in [0, 10, 100] {
        assert!(matches!(
            context.search_type_inputs(
                &inputs,
                constraint(),
                &mut Budget {
                    steps,
                    ..Budget::default()
                }
            ),
            TypeInputOutcome::Unknown(ptc::UnknownReason::BudgetExhausted)
        ));
    }
    assert!(matches!(
        context.search_type_inputs(&inputs, constraint(), &mut Budget::default()),
        TypeInputOutcome::Ambiguous { .. }
    ));
    assert!(context.assumptions().is_empty());
}

#[test]
fn classified_models_also_try_closed_types_mentioned_by_the_problem() {
    let context = Context::default();
    let pointer = |p| Term::Pointer(Box::new(Term::Primitive(p)));
    let constraint = F::And(vec![
        F::Kind(KindQuery {
            ty: input(0),
            kind: TypeKind::Pointer,
        }),
        F::equal(input(0), pointer(Primitive::Bool)).negate(),
        F::equal(input(1), pointer(Primitive::U8)),
    ]);
    let problem = context
        .type_input_context(&[id(0), id(1)], constraint, &mut Budget::default())
        .unwrap();
    let ptc::Outcome::NotEntailed(witness) = problem.prove(F::False, Budget::default()) else {
        panic!("the mentioned pointer type supplies a valid alternative")
    };
    problem.verify_witness(&witness, Budget::default()).unwrap();
    assert_eq!(witness.types[&id(0)], pointer(Primitive::U8));
    // This is only a satisfying model, never proof that the input must be u8*.
    assert!(!matches!(
        problem.prove(
            F::equal(input(0), pointer(Primitive::U8)),
            Budget::default()
        ),
        ptc::Outcome::Entailed(_)
    ));
}

#[test]
fn exhausting_classified_candidates_is_not_an_inconsistency_proof() {
    let mut constraints = vec![unsigned(0)];
    for primitive in [
        Primitive::U8,
        Primitive::U16,
        Primitive::U32,
        Primitive::U64,
        Primitive::U128,
        Primitive::Usize,
    ] {
        constraints.push(F::equal(input(0), Term::Primitive(primitive)).negate());
    }
    let outcome = Context::default().search_type_inputs(
        &[id(0)],
        F::And(constraints),
        &mut Budget::default(),
    );
    assert!(matches!(outcome, TypeInputOutcome::Unknown(_)), "{outcome}");
}
