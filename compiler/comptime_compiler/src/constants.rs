//! Owned immutable operands, separate from callable selection and expression trees.
use crate::{Error, copy_error};
use nir::program::{Expr, ExprKind as K, Literal};
use nir::{DeclarationId, Term};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::hash::BuildHasher;
use std::sync::{Arc, Weak};

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Literal(Literal),
    Type(Term),
    Frozen(Term),
    Function(DeclarationId),
}
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Constant {
    pub ty: Term,
    pub kind: Kind,
}
/// Stable table-local identity, independent of allocation addresses or hashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Id(pub usize);
impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
#[derive(Default)]
pub(crate) struct Table {
    ids: HashMap<Arc<Constant>, Id>,
    values: Vec<Arc<Constant>>,
}
impl Table {
    // Pool::get has already charged the complete identity traversal.
    pub(crate) fn insert(&mut self, value: Arc<Constant>) -> Id {
        if let Some(id) = self.ids.get(&value) {
            return *id;
        }
        let id = Id(self.values.len());
        self.ids.insert(value.clone(), id);
        self.values.push(value);
        id
    }
    pub(crate) fn finish(self) -> Vec<Arc<Constant>> {
        self.values
    }
}

// Keys borrow source operands only while comparing. The bounded weak index
// neither owns source bodies nor keeps constants alive after code eviction.
#[derive(PartialEq, Eq, Hash)]
enum Input<'a> {
    Literal(&'a Literal),
    Type(&'a Term),
    Frozen(&'a Term),
    Function(DeclarationId),
}
#[derive(Default)]
pub(crate) struct Pool {
    entries: HashMap<u64, Weak<Constant>>,
    order: VecDeque<u64>,
}
const MAX_ENTRIES: usize = 4096;
impl Pool {
    pub(crate) fn get(&mut self, expr: &Expr, fuel: &mut usize) -> Result<Arc<Constant>, Error> {
        // Hashing and equality inspect payloads even on hits. Preserve bounded
        // traversal and depth admission before inspecting or copying a key.
        nir::copy_budget::charge_substitution(expr, &[], &[], fuel)
            .map_err(|error| copy_error(error, expr.source))?;
        let input = match &expr.kind {
            K::Literal(value) => Input::Literal(value),
            K::TypeValue(value) => Input::Type(value),
            K::ComptimeValue(value) => Input::Frozen(value),
            K::Function(id) => Input::Function(*id),
            _ => return Err(Error::Unsupported(expr.source)),
        };
        let key = (&expr.ty, input);
        let hash = self.entries.hasher().hash_one(&key);
        if let Some(value) = self.entries.get(&hash).and_then(Weak::upgrade) {
            let stored = match &value.kind {
                Kind::Literal(v) => Input::Literal(v),
                Kind::Type(v) => Input::Type(v),
                Kind::Frozen(v) => Input::Frozen(v),
                Kind::Function(v) => Input::Function(*v),
            };
            if (&value.ty, stored) == key {
                return Ok(value);
            }
        }
        let kind = match &key.1 {
            Input::Literal(value) => Kind::Literal((*value).clone()),
            Input::Type(value) => Kind::Type((*value).clone()),
            Input::Frozen(value) => Kind::Frozen((*value).clone()),
            Input::Function(id) => Kind::Function(*id),
        };
        let value = Arc::new(Constant {
            ty: expr.ty.clone(),
            kind,
        });
        // A hash collision only replaces an optimization entry. Exact identity
        // above is mandatory; existing instructions retain their own payload.
        if !self.entries.contains_key(&hash) {
            if self.order.len() == MAX_ENTRIES {
                self.entries.remove(&self.order.pop_front().unwrap());
            }
            self.order.push_back(hash);
        }
        self.entries.insert(hash, Arc::downgrade(&value));
        Ok(value)
    }
}

impl fmt::Display for Constant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(typed-constant (type {}) ", self.ty)?;
        match &self.kind {
            Kind::Literal(value) => write!(f, "(literal {value})")?,
            Kind::Type(value) => write!(f, "(type-value {value})")?,
            Kind::Frozen(value) => write!(f, "(frozen-value {value})")?,
            Kind::Function(id) => write!(f, "(function {id})")?,
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::{Primitive, program::SourceRange};
    fn literal(value: u128) -> Expr {
        Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: Term::Primitive(Primitive::U64),
            kind: K::Literal(Literal::UnsignedInteger(value)),
        }
    }

    #[test]
    fn weak_pool_is_bounded_and_never_retains_evicted_payloads() {
        let mut pool = Pool::default();
        let first = pool.get(&literal(0), &mut 1000).unwrap();
        let weak = Arc::downgrade(&first);
        drop(first);
        assert!(weak.upgrade().is_none());
        for value in 1..MAX_ENTRIES + 8 {
            pool.get(&literal(value as u128), &mut 1000).unwrap();
            assert!(pool.entries.len() <= MAX_ENTRIES);
            assert_eq!(pool.entries.len(), pool.order.len());
        }
        assert_eq!(pool.entries.len(), MAX_ENTRIES);
        assert!(pool.entries.values().all(|entry| entry.upgrade().is_none()));
    }

    #[test]
    fn hash_collisions_never_alias_different_constants() {
        let mut pool = Pool::default();
        let input = literal(1);
        let wrong = pool.get(&literal(2), &mut 1000).unwrap();
        let K::Literal(value) = &input.kind else {
            unreachable!()
        };
        let hash = pool
            .entries
            .hasher()
            .hash_one((&input.ty, Input::Literal(value)));
        pool.entries.insert(hash, Arc::downgrade(&wrong));
        let correct = pool.get(&input, &mut 1000).unwrap();
        assert!(!Arc::ptr_eq(&wrong, &correct));
        assert_eq!(correct.kind, Kind::Literal(Literal::UnsignedInteger(1)));
        assert_eq!(wrong.kind, Kind::Literal(Literal::UnsignedInteger(2)));
    }

    #[test]
    fn failed_compilation_leaves_no_owned_constant_payloads() {
        let mut session = crate::CompilationSession::default();
        let body = Expr {
            kind: K::Block {
                statements: vec![literal(1), literal(2)],
                result: Some(Box::new(Expr {
                    kind: K::Break(nir::program::LocalId { owner: 1, index: 0 }),
                    ..literal(0)
                })),
            },
            ..literal(0)
        };
        assert!(matches!(
            session.compile(&body, &mut 1000),
            Err(Error::InvalidControl(_))
        ));
        assert!(!session.constants.entries.is_empty());
        assert!(
            session
                .constants
                .entries
                .values()
                .all(|entry| entry.upgrade().is_none())
        );
        assert!(session.compile(&literal(1), &mut 1000).is_ok());
    }

    #[test]
    fn table_ids_survive_weak_pool_eviction_and_equal_reallocation() {
        let mut pool = Pool::default();
        let mut table = Table::default();
        let first = pool.get(&literal(0), &mut 1000).unwrap();
        let id = table.insert(first.clone());
        assert_eq!(id.to_string(), "0");
        for value in 1..MAX_ENTRIES + 8 {
            let value = pool.get(&literal(value as u128), &mut 1000).unwrap();
            table.insert(value);
        }
        let recreated = pool.get(&literal(0), &mut 1000).unwrap();
        assert!(!Arc::ptr_eq(&first, &recreated));
        assert_eq!(table.insert(recreated), id);
        let values = table.finish();
        assert_eq!(values.len(), MAX_ENTRIES + 8);
        assert!(Arc::ptr_eq(&values[id.0], &first));
    }
}
