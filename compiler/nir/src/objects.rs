//! Stable Object value encoding. The tagged aggregate is never a language Tuple.
//! Each entry packet stores its optional key, original domain, and frozen value.
//! Domain admission must still validate each payload against its stored Type.
use crate::{Primitive, Sort, Term};

pub fn value(entries: Vec<Term>) -> Term {
    Term::NominalValue {
        ty: Box::new(Term::Primitive(Primitive::Object)),
        variant: None,
        fields: entries,
    }
}

pub fn entry(key: Option<&str>, domain: Term, value: Term) -> Term {
    Term::TupleValue(vec![
        key.map_or(Term::Unit, |key| Term::Symbol(key.into())),
        domain,
        value,
    ])
}

/// Decode only the representation. This does not establish the payload's type.
pub fn entry_parts(entry: &Term) -> Option<(Option<&str>, &Term, &Term)> {
    let Term::TupleValue(fields) = entry else {
        return None;
    };
    let [key, domain, value] = fields.as_slice() else {
        return None;
    };
    if domain.sort() != Sort::Type || value.sort() == Sort::Runtime {
        return None;
    }
    let key = match key {
        Term::Unit => None,
        Term::Symbol(key) => Some(key.as_ref()),
        _ => return None,
    };
    Some((key, domain, value))
}
