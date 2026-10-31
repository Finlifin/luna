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
}

/// List payload storage. The cached summary describes closed representation
/// work only, never nominal admission, caller assumptions or type equality.
#[derive(Clone, Default)]
pub struct ListValues {
    values: Vec<Value>,
    summary: OnceLock<Summary>,
}
impl ListValues {
    pub(crate) fn append_payload(&mut self, other: Self) {
        let combined = self
            .summary()
            .zip(other.summary())
            .and_then(|(left, right)| {
                Some(Summary {
                    work: left.work.checked_add(right.work)?,
                    height: left.height.max(right.height),
                })
            });
        self.summary.take();
        self.values.extend(other.values);
        if let Some(summary) = combined {
            self.remember(summary);
        }
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
    fn append_composes_summaries_and_rollback_invalidates_them() {
        let mut left = test_type_list(vec![Term::Primitive(Primitive::Bool)]);
        let right = test_type_list(vec![Term::Primitive(Primitive::Char)]);
        left.immutable_copy_work().unwrap();
        right.immutable_copy_work().unwrap();
        crate::memory::append_list_checked(&mut left, right.clone(), |combined| {
            assert!(payload(combined).summary().is_some());
            Err(crate::ErrorKind::StepsExhausted)
        })
        .unwrap_err();
        assert_eq!(left, test_type_list(vec![Term::Primitive(Primitive::Bool)]));
        assert!(payload(&left).summary().is_none());
        left.immutable_copy_work().unwrap();
        crate::memory::append_list_checked(&mut left, right, |_| Ok(())).unwrap();
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
