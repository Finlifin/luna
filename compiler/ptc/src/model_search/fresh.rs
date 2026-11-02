//! Shallow, distinct closed Types for unconstrained model classes. These are
//! search candidates, not canonical types or equations available to inference.
use nir::{Primitive, Term};

pub(crate) fn candidate(index: usize, fuel: &mut usize) -> Option<Term> {
    // Encode the index in a product of two distinct primitive identities. The
    // empty product encodes zero; nonzero products have no leading zero digit.
    // Representation size is logarithmic and depth is constant, independently
    // of how many free classes a query contains.
    let width = (usize::BITS - index.leading_zeros()) as usize;
    let Some(remaining) = fuel.checked_sub(width + 2) else {
        *fuel = 0;
        return None;
    };
    *fuel = remaining;
    let fields = (0..width)
        .map(|bit| {
            Term::Primitive(if index & (1 << bit) == 0 {
                Primitive::Bool
            } else {
                Primitive::U8
            })
        })
        .collect();
    Some(Term::Pointer(Box::new(Term::Tuple(fields))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn candidates_are_distinct_shallow_and_budgeted_before_allocation() {
        let mut seen = HashSet::new();
        for index in (0..1024).chain([usize::MAX]) {
            let mut fuel = 100;
            let value = candidate(index, &mut fuel).unwrap();
            assert!(seen.insert(value.clone()));
            let Term::Pointer(inner) = value else {
                panic!()
            };
            let Term::Tuple(fields) = *inner else {
                panic!()
            };
            assert!(
                fields
                    .iter()
                    .all(|field| matches!(field, Term::Primitive(_)))
            );
            assert_eq!(100 - fuel, fields.len() + 2);
            let mut insufficient = fields.len() + 1;
            assert!(candidate(index, &mut insufficient).is_none());
            assert_eq!(insufficient, 0);
        }
    }
}
