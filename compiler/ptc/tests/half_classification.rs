use nir::{DeclarationId, Primitive as P, Sort, Term};
use ptc::{Budget, Context, Formula, Outcome, TypeKind};

fn proved(context: &Context, goal: Formula) {
    let Outcome::Entailed(proof) = context.prove(goal, Budget::default()) else {
        panic!("expected a proved type classification");
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
}

#[test]
fn half_formats_are_floating_types_without_becoming_equal() {
    let context = Context::default();
    for primitive in [P::F16, P::Bf16] {
        let ty = Term::Primitive(primitive);
        proved(&context, Formula::kind(ty.clone(), TypeKind::FloatingPoint));
        for kind in [
            TypeKind::Boolean,
            TypeKind::Symbol,
            TypeKind::Integer,
            TypeKind::SignedInteger,
            TypeKind::UnsignedInteger,
            TypeKind::Tuple,
            TypeKind::Pointer,
            TypeKind::Function,
        ] {
            proved(&context, Formula::kind(ty.clone(), kind).negate());
        }
        for other in [P::F16, P::Bf16, P::F32, P::F64, P::Float] {
            if other != primitive {
                proved(
                    &context,
                    Formula::equal(ty.clone(), Term::Primitive(other)).negate(),
                );
            }
        }
        assert!(!matches!(
            context.prove(
                Formula::kind(ty, TypeKind::FloatingPoint).negate(),
                Budget::default()
            ),
            Outcome::Entailed(_)
        ));
    }
}

#[test]
fn disjunctive_half_domains_imply_floating_classification_without_explosion() {
    let subject = Term::Rigid {
        declaration: DeclarationId {
            package: 0,
            local: 1,
        },
        sort: Sort::Type,
    };
    let mut context = Context::default();
    context.assume(Formula::Or(vec![
        Formula::equal(subject.clone(), Term::Primitive(P::F16)),
        Formula::equal(subject.clone(), Term::Primitive(P::Bf16)),
    ]));
    proved(
        &context,
        Formula::kind(subject.clone(), TypeKind::FloatingPoint),
    );
    proved(
        &context,
        Formula::kind(subject.clone(), TypeKind::Integer).negate(),
    );
    context.assume(Formula::kind(subject.clone(), TypeKind::FloatingPoint));
    assert!(!matches!(
        context.prove(Formula::False, Budget::default()),
        Outcome::Entailed(_)
    ));
    for primitive in [P::F16, P::Bf16] {
        assert!(!matches!(
            context.prove(
                Formula::equal(subject.clone(), Term::Primitive(primitive)),
                Budget::default()
            ),
            Outcome::Entailed(_)
        ));
    }
}

#[test]
fn opaque_type_heads_do_not_inherit_half_classification() {
    let mut context = Context::default();
    context.admit_congruence(DeclarationId {
        package: 0,
        local: 2,
    });
    let opaque = Term::NormalForm {
        head: DeclarationId {
            package: 0,
            local: 2,
        },
        sort: Sort::Type,
        arguments: vec![Term::Primitive(P::F16)],
    };
    for goal in [
        Formula::kind(opaque.clone(), TypeKind::FloatingPoint),
        Formula::kind(opaque, TypeKind::FloatingPoint).negate(),
    ] {
        let outcome = context.prove(goal, Budget::default());
        assert!(
            !matches!(outcome, Outcome::Entailed(_) | Outcome::IllFormed(_)),
            "{outcome:?}"
        );
    }
}
