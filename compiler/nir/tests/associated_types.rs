use nir::{DeclarationId, Parameter, Primitive, RuntimeId, Sort, Term, TypeInterner, TypeRef};
use std::collections::BTreeSet;

fn id(local: u32) -> DeclarationId { DeclarationId { package: 7, local } }
fn ty() -> Term { Term::Primitive(Primitive::Bool) }
fn projection(subject: Term, arguments: Vec<Term>) -> Term {
    Term::AssociatedType { origin: Box::new(nir::AssociatedTypeOrigin { interface: id(1), member: id(2) }), subject: Box::new(subject), arguments }
}

#[test]
fn associated_substitution_preserves_origin_and_visits_subject_and_arguments() {
    let rigid = Term::Rigid { declaration: id(3), sort: Sort::Type };
    let original = projection(rigid.clone(), vec![rigid]);
    assert!(original.mentions_rigid(id(3)));
    let completed = original.instantiate_types(&[id(3)], &[ty()]).unwrap();
    assert_eq!(completed, projection(ty(), vec![ty()]));
    assert!(!completed.mentions_rigid(id(3)));
    assert_ne!(completed, ty());
    assert_eq!(completed.sort(), Sort::Type);
}

#[test]
fn associated_substitution_avoids_capture_under_function_binders() {
    let outer = Term::Bound { depth: 1, index: 0, sort: Sort::Type };
    let inner = Term::Bound { depth: 0, index: 0, sort: Sort::Type };
    let signature = Term::Function {
        parameters: vec![Parameter { sort: Sort::Type, ty: Term::Primitive(Primitive::Type) }],
        result: Box::new(projection(outer, vec![inner.clone()])),
    };
    let free = Term::Bound { depth: 0, index: 1, sort: Sort::Type };
    let replaced = signature.substitute(&[free]).unwrap();
    let Term::Function { result, .. } = replaced else { panic!() };
    assert_eq!(*result, projection(Term::Bound { depth: 1, index: 1, sort: Sort::Type }, vec![inner]));
}

#[test]
fn associated_subject_sort_and_nested_scope_are_validated() {
    assert!(projection(Term::Bool(true), vec![]).check_scope(&[]).is_err());
    assert!(projection(ty(), vec![Term::Bound { depth: 0, index: 0, sort: Sort::Type }]).check_scope(&[]).is_err());
    assert!(projection(Term::Bound { depth: 0, index: 0, sort: Sort::Type }, vec![]).check_scope(&[]).is_err());
}

#[test]
fn associated_runtime_dependencies_remain_reference_side() {
    let mut interner = TypeInterner::default();
    let hole = Term::Hole { index: 0, sort: Sort::Runtime };
    let subject = Term::Nominal { declaration: id(4), arguments: vec![hole.clone()] };
    let core = interner.intern(vec![Parameter { sort: Sort::Runtime, ty: ty() }], projection(subject, vec![hole])).unwrap();
    let runtime = RuntimeId { instance: 9, local: 1 };
    let reference = TypeRef::bind(core, vec![Term::Runtime(runtime)]).unwrap();
    assert_eq!(reference.runtime_dependencies().unwrap(), BTreeSet::from([runtime]));
    assert!(reference.check_runtime_bindings(&BTreeSet::new()).is_err());
    reference.check_runtime_bindings(&BTreeSet::from([runtime])).unwrap();
}

#[test]
fn associated_dump_names_all_identity_fields() {
    let value = projection(ty(), vec![Term::Primitive(Primitive::Char)]);
    assert_eq!(value.to_string(), format!("(associated-type (interface {}) (arguments ((primitive char))) (subject (primitive bool)) (member {}))", id(1), id(2)));
}
