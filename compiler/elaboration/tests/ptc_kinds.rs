use elaboration::ptc::{Budget, Context, Formula as F, Outcome, TypeKind as K};
use nir::{DeclarationId, Primitive as P, Sort, Term};

fn variable(local: u32) -> Term {
    Term::Rigid {
        declaration: DeclarationId { package: 9, local },
        sort: Sort::Type,
    }
}
fn prove(context: &Context, formula: F) {
    let outcome = context.prove(formula, Budget::default());
    let Outcome::Entailed(proof) = outcome else {
        panic!("expected proof: {outcome}")
    };
    assert_eq!(context.verify_proof(&proof, Budget::default()), Ok(()));
}

#[test]
fn closed_descriptors_have_overlapping_integer_classifications() {
    let fixtures = [
        (Term::Primitive(P::Bool), vec![K::Boolean]),
        (
            Term::Primitive(P::U32),
            vec![K::Integer, K::UnsignedInteger],
        ),
        (Term::Primitive(P::I64), vec![K::Integer, K::SignedInteger]),
        (Term::Primitive(P::I128), vec![K::Integer, K::SignedInteger]),
        (
            Term::Primitive(P::U128),
            vec![K::Integer, K::UnsignedInteger],
        ),
        (Term::Primitive(P::F64), vec![K::FloatingPoint]),
        (Term::Primitive(P::Char), vec![]),
        (Term::Tuple(vec![Term::Primitive(P::U32)]), vec![K::Tuple]),
        (
            Term::Pointer(Box::new(Term::Primitive(P::Bool))),
            vec![K::Pointer],
        ),
        (
            Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(P::Void)),
            },
            vec![K::Function],
        ),
        (
            Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(P::Void)),
            }
            .with_comptime_phase(true),
            vec![K::Function],
        ),
    ];
    for (ty, expected) in fixtures {
        for kind in [
            K::Boolean,
            K::Integer,
            K::UnsignedInteger,
            K::SignedInteger,
            K::FloatingPoint,
            K::Tuple,
            K::Pointer,
            K::Function,
        ] {
            let query = F::kind(ty.clone(), kind);
            prove(
                &Context::default(),
                if expected.contains(&kind) {
                    query
                } else {
                    query.negate()
                },
            );
        }
    }
}

#[test]
fn rigid_classification_is_propagated_through_equality_and_structure() {
    let (t, u) = (variable(0), variable(1));
    let mut context = Context::default();
    context.assume(F::equal(t.clone(), u.clone()));
    context.assume(F::kind(t.clone(), K::UnsignedInteger));
    prove(&context, F::kind(u.clone(), K::Integer));
    prove(&context, F::kind(u.clone(), K::SignedInteger).negate());
    context.assume(F::equal(
        Term::Pointer(Box::new(u)),
        Term::Pointer(Box::new(Term::Primitive(P::Bool))),
    ));
    prove(&context, F::False);
}

#[test]
fn closed_description_can_refute_an_incompatible_symbolic_classification() {
    let mut context = Context::default();
    let t = variable(1);
    context.assume(F::kind(t.clone(), K::Tuple));
    prove(&context, F::equal(t, Term::Primitive(P::U32)).negate());
}

#[test]
fn concrete_type_witnesses_do_not_turn_not_entailed_into_negation() {
    let context = Context::default();
    let t = variable(0);
    for query in [
        F::kind(t.clone(), K::Integer),
        F::kind(t.clone(), K::Integer).negate(),
    ] {
        assert!(matches!(
            context.prove(query, Budget::default()),
            Outcome::NotEntailed(_)
        ));
    }
    let mut integers = context.fork();
    integers.assume(F::kind(t.clone(), K::UnsignedInteger));
    let Outcome::NotEntailed(witness) = integers.prove(F::kind(t, K::Tuple), Budget::default())
    else {
        panic!("expected actual unsigned type witness")
    };
    assert_eq!(
        witness.types.values().next(),
        Some(&Term::Primitive(P::U32))
    );
    assert!(format!("{}", Outcome::NotEntailed(witness)).contains("(type-kind"));
}

#[test]
fn nominal_and_factory_descriptors_remain_unknown() {
    let id = DeclarationId {
        package: 1,
        local: 2,
    };
    for ty in [
        Term::Nominal {
            declaration: id,
            arguments: vec![],
        },
        Term::NormalForm {
            head: id,
            sort: Sort::Type,
            arguments: vec![Term::Primitive(P::U32)],
        },
    ] {
        for query in [
            F::kind(ty.clone(), K::Integer),
            F::kind(ty.clone(), K::Integer).negate(),
        ] {
            assert!(matches!(
                Context::default().prove(query, Budget::default()),
                Outcome::Unknown { .. }
            ));
        }
    }
}

#[test]
fn contradictory_kinds_and_branch_facts_are_scoped() {
    let t = variable(0);
    let mut context = Context::default();
    context.assume(F::Or(vec![
        F::kind(t.clone(), K::Tuple),
        F::kind(t.clone(), K::UnsignedInteger),
    ]));
    context.assume(F::kind(t.clone(), K::Integer).negate());
    prove(&context, F::kind(t.clone(), K::Tuple));
    let mut child = context.fork();
    child.assume(F::kind(t.clone(), K::Pointer));
    prove(&child, F::False);
    assert!(matches!(
        context.prove(F::False, Budget::default()),
        Outcome::NotEntailed(_)
    ));
    prove(&context, F::kind(t, K::Tuple));
}

#[test]
fn malformed_classification_inputs_are_rejected_before_tautology() {
    for ty in [
        Term::Bool(true),
        Term::Hole {
            index: 0,
            sort: Sort::Type,
        },
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
    ] {
        assert!(matches!(
            Context::default().prove(
                F::Or(vec![F::True, F::kind(ty, K::Integer)]),
                Budget::default()
            ),
            Outcome::IllFormed(_)
        ));
    }
}
