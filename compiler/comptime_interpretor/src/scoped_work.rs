//! Scope and budget boundaries for lazy type-description traversal.
use super::*;

fn work(ty: &Term) -> Option<usize> {
    scoped_type_value_work(ty, &BTreeMap::new(), &BTreeSet::new())
}

fn bound(depth: u32) -> Term {
    Term::Bound {
        depth,
        index: 0,
        sort: nir::Sort::Type,
    }
}

fn function(parameters: Vec<nir::Parameter>, result: Term) -> Term {
    Term::Function {
        parameters,
        result: Box::new(result),
    }
}

#[test]
fn empty_telescope_remains_a_scope_frame() {
    let parameter = nir::Parameter {
        sort: nir::Sort::Type,
        ty: Term::Primitive(Primitive::Type),
    };
    let valid = function(vec![parameter.clone()], function(vec![], bound(1)));
    assert!(work(&valid).is_some());
    let invalid = function(vec![parameter], function(vec![], bound(0)));
    assert!(work(&invalid).is_none());
    assert!(work(&valid).is_some());
}

#[test]
fn sibling_types_do_not_inherit_each_others_binders() {
    let parameter = nir::Parameter {
        sort: nir::Sort::Type,
        ty: Term::Primitive(Primitive::Type),
    };
    let closed = function(vec![parameter], bound(0));
    assert!(work(&closed).is_some());
    for children in [
        vec![closed.clone(), bound(0)],
        vec![bound(0), closed.clone()],
    ] {
        assert!(work(&Term::Tuple(children)).is_none());
    }
    assert!(work(&Term::Tuple(vec![closed.clone(), closed])).is_some());
}

#[test]
fn leaf_work_and_traversal_limits_are_preserved() {
    let boolean = Term::Primitive(Primitive::Bool);
    assert_eq!(work(&boolean), Some(1));
    let mut nested = boolean.clone();
    for _ in 0..128 {
        nested = Term::Pointer(Box::new(nested));
    }
    assert_eq!(work(&nested), Some(129));
    assert_eq!(work(&Term::Pointer(Box::new(nested))), None);
    assert_eq!(
        work(&Term::Tuple(vec![boolean.clone(); 99_999])),
        Some(100_000)
    );
    assert_eq!(work(&Term::Tuple(vec![boolean.clone(); 100_000])), None);
    assert_eq!(work(&boolean), Some(1));
}
