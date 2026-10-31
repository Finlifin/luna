use nir::{DeclarationId, Primitive, Sort, Term};
use ptc::{Budget, Context, Formula, Outcome};

#[test]
fn bound_lists_are_distinct_from_structural_types_without_unfolding() {
    let head = DeclarationId {
        package: 1,
        local: 1,
    };
    let list = Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    let mut context = Context::default();
    context.admit_list_inference_head(head);
    for other in [
        Term::Primitive(Primitive::Object),
        Term::Primitive(Primitive::Bool),
        Term::Optional(Box::new(Term::Primitive(Primitive::Bool))),
        Term::Tuple(vec![]),
    ] {
        let goal = Formula::equal(list.clone(), other).negate();
        let Outcome::Entailed(proof) = context.prove(goal.clone(), Budget::default()) else {
            panic!("{goal:?}")
        };
        context.verify_proof(&proof, Budget::default()).unwrap();
        assert!(!matches!(
            Context::default().prove(goal, Budget::default()),
            Outcome::Entailed(_)
        ));
    }
    let error_wrapped = Term::ErrorResult {
        errors: vec![Term::Primitive(Primitive::Bool)],
        result: Box::new(list.clone()),
    };
    let goal = Formula::equal(list.clone(), error_wrapped).negate();
    let Outcome::Entailed(proof) = context.prove(goal.clone(), Budget::default()) else {
        panic!("{goal:?}")
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
    assert!(!matches!(
        Context::default().prove(goal, Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(matches!(
        context.prove(
            Formula::equal(
                list.clone(),
                Term::ErrorResult {
                    errors: vec![],
                    result: Box::new(list.clone()),
                }
            ),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
    let opaque = Term::NormalForm {
        head: DeclarationId {
            package: 2,
            local: 1,
        },
        sort: Sort::Type,
        arguments: vec![],
    };
    assert!(!matches!(
        context.prove(Formula::equal(list, opaque).negate(), Budget::default()),
        Outcome::Entailed(_)
    ));
}
