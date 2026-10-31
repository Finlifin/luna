//! Owned list payloads invalidate structural summaries before mutable access.
use super::Value;
use std::{
    fmt,
    ops::{Deref, DerefMut},
    sync::OnceLock,
};

#[derive(Clone, Copy, Debug)]
pub(super) struct Summary {
    pub work: usize,
    /// Maximum element subtree height; zero for an empty payload.
    pub height: usize,
    /// A uniform, content-independent scalar carrier. This records value
    /// discriminants only, not an expected type or a successful admission.
    pub uniform_scalar: Option<std::mem::Discriminant<Value>>,
}

/// Only these leaf carriers have type membership independent of their payload.
/// Machine integers, BitVec, Type values and aggregates require their ordinary
/// value-specific checks and cannot use a representative scalar observation.
pub(super) fn scalar_tag(value: &Value) -> Option<std::mem::Discriminant<Value>> {
    matches!(
        value,
        Value::Float(_)
            | Value::Integer(_)
            | Value::Float16(_)
            | Value::BFloat16(_)
            | Value::Float32(_)
            | Value::Float64(_)
            | Value::Unit
            | Value::Bool(_)
            | Value::Char(_)
            | Value::String(_)
            | Value::Symbol(_)
    )
    .then(|| std::mem::discriminant(value))
}

/// List payload storage. The summary describes closed representation work and
/// scalar discriminants, never nominal admission, assumptions or type equality.
#[derive(Clone, Default)]
pub struct ListValues {
    values: Vec<Value>,
    summary: OnceLock<Summary>,
}
impl ListValues {
    /// Bound slot relocation by the existing capacity. An in-place extension
    /// moves only the suffix; growth may also relocate every prefix slot.
    /// Nested payloads are moved with their owners, not recursively copied.
    pub(crate) fn append_payload(&mut self, other: Self) -> usize {
        let moved = other.len().saturating_add(
            if other.len() > self.values.capacity() - self.values.len() {
                self.values.len()
            } else {
                0
            },
        );
        let combined = self
            .summary()
            .zip(other.summary())
            .and_then(|(left, right)| {
                Some(Summary {
                    work: left.work.checked_add(right.work)?,
                    height: left.height.max(right.height),
                    uniform_scalar: if self.is_empty() {
                        right.uniform_scalar
                    } else if other.is_empty() || left.uniform_scalar == right.uniform_scalar {
                        left.uniform_scalar
                    } else {
                        None
                    },
                })
            });
        self.summary.take();
        self.values.extend(other.values);
        if let Some(summary) = combined {
            self.remember(summary);
        }
        moved
    }
    pub fn into_vec(self) -> Vec<Value> {
        self.values
    }
    pub(super) fn summary(&self) -> Option<Summary> {
        self.summary.get().copied()
    }
    pub(super) fn remember(&self, summary: Summary) {
        let _ = self.summary.set(summary);
    }
}
impl From<Vec<Value>> for ListValues {
    fn from(values: Vec<Value>) -> Self {
        Self {
            values,
            summary: OnceLock::new(),
        }
    }
}
impl FromIterator<Value> for ListValues {
    fn from_iter<T: IntoIterator<Item = Value>>(iter: T) -> Self {
        Vec::from_iter(iter).into()
    }
}
impl Deref for ListValues {
    type Target = Vec<Value>;
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
impl DerefMut for ListValues {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.summary.take();
        &mut self.values
    }
}
impl IntoIterator for ListValues {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}
impl<'a> IntoIterator for &'a ListValues {
    type Item = &'a Value;
    type IntoIter = std::slice::Iter<'a, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}
impl<'a> IntoIterator for &'a mut ListValues {
    type Item = &'a mut Value;
    type IntoIter = std::slice::IterMut<'a, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.deref_mut().iter_mut()
    }
}
impl PartialEq for ListValues {
    fn eq(&self, other: &Self) -> bool {
        self.values == other.values
    }
}
impl PartialEq<Vec<Value>> for ListValues {
    fn eq(&self, other: &Vec<Value>) -> bool {
        &self.values == other
    }
}
impl fmt::Debug for ListValues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.values.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Primitive, Term, test_type_list};

    fn payload(value: &Value) -> &ListValues {
        let Value::List { values, .. } = value else {
            panic!("list expected")
        };
        values
    }
    #[test]
    fn scalar_observations_compose_and_invalidate_without_type_verdicts() {
        let make = |values: Vec<Value>| {
            let mut value = test_type_list(vec![]);
            let Value::List { values: stored, .. } = &mut value else {
                unreachable!()
            };
            *stored = values.into();
            // The intentionally mismatched declared Type domain must not
            // influence a purely structural observation of Bool carriers.
            value.immutable_copy_work().unwrap();
            value
        };
        let mut value = make(vec![]);
        let suffix = make(vec![Value::Bool(true), Value::Bool(false)]);
        crate::memory::append_list_checked(&mut value, suffix, |_, _| Ok(())).unwrap();
        assert_eq!(
            payload(&value).summary().unwrap().uniform_scalar,
            scalar_tag(&Value::Bool(false))
        );
        crate::memory::append_list_checked(&mut value, make(vec![]), |_, _| Ok(())).unwrap();
        assert!(payload(&value).summary().unwrap().uniform_scalar.is_some());
        let saved = value.clone();
        crate::memory::append_list_checked(&mut value, make(vec![Value::Char('x')]), |_, _| Ok(()))
            .unwrap();
        assert!(payload(&value).summary().unwrap().uniform_scalar.is_none());
        assert!(payload(&saved).summary().unwrap().uniform_scalar.is_some());
        let Value::List { values, .. } = &mut value else {
            unreachable!()
        };
        values[0] = Value::UnsignedInteger(256);
        assert!(values.summary().is_none());
        for value in [
            Value::UnsignedInteger(1),
            Value::SignedInteger(1),
            Value::Type(Term::Primitive(Primitive::Bool)),
        ] {
            assert!(scalar_tag(&value).is_none());
        }
    }

    #[test]
    fn cached_inspection_cost_does_not_discount_owned_copies() {
        let value = test_type_list(vec![Term::Primitive(Primitive::Bool); 512]);
        let inspect = |value: &Value| {
            value.scoped_representation_work(&Default::default(), &Default::default())
        };
        let (copy, cold) = inspect(&value).unwrap();
        let (same_copy, warm) = inspect(&value).unwrap();
        assert_eq!(same_copy, copy);
        assert_eq!(cold, copy);
        assert!(warm < cold / 16);
        assert_eq!(value.immutable_copy_work(), Some(copy));

        let mut changed = value.clone();
        let Value::List { values, .. } = &mut changed else {
            unreachable!()
        };
        values[0] = Value::Storage(crate::Storage::new(Value::Bool(true)));
        assert!(inspect(&changed).is_none());
        assert_eq!(inspect(&value), Some((copy, warm)));
    }

    #[test]
    fn summaries_survive_reads_and_clones_but_not_mutable_access() {
        let original = test_type_list(vec![Term::Primitive(Primitive::Bool)]);
        let work = original.immutable_copy_work().unwrap();
        assert!(payload(&original).summary().is_some());
        let mut changed = original.clone();
        let Value::List { values, .. } = &mut changed else {
            unreachable!()
        };
        assert!(values.summary().is_some());
        values[0] = Value::Type(Term::Pointer(Box::new(Term::Primitive(Primitive::Bool))));
        assert!(values.summary().is_none());
        assert!(changed.immutable_copy_work().unwrap() > work);
        assert_eq!(original.immutable_copy_work(), Some(work));
        let Value::List { values, .. } = &mut changed else {
            unreachable!()
        };
        for value in values {
            *value = Value::Storage(crate::Storage::new(Value::Bool(true)));
        }
        assert!(changed.immutable_copy_work().is_none());
        assert!(payload(&changed).summary().is_none());
    }
    #[test]
    fn append_composes_summaries_and_rollback_restores_the_prefix_summary() {
        let mut left = test_type_list(vec![Term::Primitive(Primitive::Bool)]);
        let right = test_type_list(vec![Term::Primitive(Primitive::Char)]);
        left.immutable_copy_work().unwrap();
        right.immutable_copy_work().unwrap();
        crate::memory::append_list_checked(&mut left, right.clone(), |combined, _| {
            assert!(payload(combined).summary().is_some());
            Err(crate::ErrorKind::StepsExhausted)
        })
        .unwrap_err();
        assert_eq!(left, test_type_list(vec![Term::Primitive(Primitive::Bool)]));
        assert!(payload(&left).summary().is_some());
        left.immutable_copy_work().unwrap();
        crate::memory::append_list_checked(&mut left, right, |_, _| Ok(())).unwrap();
        assert!(payload(&left).summary().is_some());
        let fresh = test_type_list(vec![
            Term::Primitive(Primitive::Bool),
            Term::Primitive(Primitive::Char),
        ]);
        assert_eq!(left.immutable_copy_work(), fresh.immutable_copy_work());
    }
    #[test]
    fn warmed_summaries_respect_enclosing_depth_size_and_scope() {
        let list = test_type_list(vec![Term::Primitive(Primitive::Bool)]);
        list.immutable_copy_work().unwrap();
        let mut nested = list;
        for _ in 0..126 {
            nested = Value::Tuple(vec![nested]);
        }
        assert!(nested.immutable_copy_work().is_some());
        nested = Value::Tuple(vec![nested]);
        assert!(nested.immutable_copy_work().is_none());

        let large = Value::List {
            ty: Term::Primitive(Primitive::Bool),
            values: vec![Value::String("x".repeat(50000).into())].into(),
        };
        assert!(large.immutable_copy_work().is_some());
        assert!(
            Value::Tuple(vec![large.clone(), large])
                .immutable_copy_work()
                .is_none()
        );

        let id = nir::DeclarationId {
            package: 1,
            local: 1,
        };
        let scoped = test_type_list(vec![Term::Rigid {
            declaration: id,
            sort: nir::Sort::Type,
        }]);
        assert!(
            scoped
                .scoped_copy_work(&std::collections::BTreeSet::from([id]))
                .is_some()
        );
        assert!(payload(&scoped).summary().is_none());
        assert!(scoped.immutable_copy_work().is_none());
    }
}

#[cfg(test)]
mod rollback_tests {
    use super::*;
    use crate::{Primitive, Term, test_type_list};

    #[test]
    fn rollback_restores_only_the_summary_known_before_the_append() {
        for warm in [false, true] {
            for unwind in [false, true] {
                let mut list = test_type_list(vec![Term::Primitive(Primitive::Bool)]);
                if warm {
                    list.immutable_copy_work().unwrap();
                }
                let original = list.clone();
                let suffix = test_type_list(vec![Term::Primitive(Primitive::Char)]);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::memory::append_list_checked(&mut list, suffix, |combined, _| {
                        // Even a cold receiver may acquire a combined summary
                        // while validation is in progress. It is not a prefix summary.
                        combined.immutable_copy_work().unwrap();
                        if unwind {
                            panic!("interrupted validation");
                        }
                        Err(crate::ErrorKind::StepsExhausted)
                    })
                }));
                if unwind {
                    assert!(result.is_err());
                } else {
                    assert_eq!(result.unwrap(), Err(crate::ErrorKind::StepsExhausted));
                }
                assert_eq!(list, original);
                let Value::List { values, .. } = &list else {
                    unreachable!()
                };
                assert_eq!(values.summary().is_some(), warm);
                assert_eq!(list.immutable_copy_work(), original.immutable_copy_work());
                let Value::List { values, .. } = &mut list else {
                    unreachable!()
                };
                values.truncate(0);
                assert!(
                    values.summary().is_none(),
                    "ordinary truncation has no prefix evidence"
                );
            }
        }
    }
}
