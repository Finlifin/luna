//! Run with `cargo run -p elaboration --example ptc` to inspect mixed proofs,
//! validated counterexamples and arithmetic capacity residuals as Lisp data.
use elaboration::ptc::{Budget, Context, Formula as F, IntegerTerm as I, Outcome, TypeKind};
use nir::{DeclarationId, Primitive, Sort, Term};

fn main() {
    let id = DeclarationId {
        package: 1,
        local: 0,
    };
    let ty = Term::Rigid {
        declaration: id,
        sort: Sort::Type,
    };
    let is_bool = F::equal(ty.clone(), Term::Primitive(Primitive::Bool));
    let mut context = Context::default();
    context.assume(F::Or(vec![
        is_bool.clone(),
        F::int_le(I::variable(id, 0), I::constant(4)),
    ]));
    context.assume(F::int_le(I::constant(5), I::variable(id, 0)));
    let proof = context.prove(is_bool, Budget::default());
    if let Outcome::Entailed(proof) = &proof {
        context
            .verify_proof(proof, Budget::default())
            .expect("proof replay");
    } else {
        panic!("expected mixed entailment: {proof}");
    }
    let witness = context.prove(F::False, Budget::default());
    assert!(matches!(witness, Outcome::NotEntailed(_)));
    let classification = context.prove(F::kind(ty, TypeKind::Boolean), Budget::default());
    let Outcome::Entailed(kind_proof) = &classification else {
        panic!("expected classification proof")
    };
    context
        .verify_proof(kind_proof, Budget::default())
        .expect("classification replay");
    let residual = Context::default().prove(
        F::int_le(I::constant(i128::MIN), I::constant(i128::MAX)),
        Budget::default(),
    );
    println!("(ptc-session\n {proof}\n {classification}\n {witness}\n {residual})");
}
