use elaboration::{
    InferenceTerm,
    obligations::{Engine, Requirement},
    ptc::{Budget, TypeKind},
};
use nir::{Primitive, Sort, Term};

fn main() {
    let mut engine = Engine::default();
    let ty = engine.fresh(Sort::Type);
    engine.require(Requirement::Kind(
        InferenceTerm::Meta(ty),
        TypeKind::UnsignedInteger,
    ));
    engine.advance(10, Budget::default());
    let waiting = engine.to_string();
    let mut candidate = engine.trial();
    candidate
        .assign(ty, InferenceTerm::Stable(Term::Primitive(Primitive::U32)))
        .unwrap();
    candidate.advance(10, Budget::default());
    engine.commit(candidate).unwrap();
    let stable = engine.freeze(&InferenceTerm::Meta(ty)).unwrap();
    println!("(elaboration-session (before {waiting}) (after {engine}) (export {stable}))");
}
