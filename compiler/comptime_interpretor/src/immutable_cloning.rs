//! Cost of cloning admitted immutable host values, distinct from expanded size.
use super::Value;

impl Value {
    /// Bound inspection and `Value::clone` work without discounting expanded
    /// representation limits. Text and persistent list payloads are shared;
    /// type metadata and enclosing owned aggregates are still cloned. This is
    /// not admission into a type, resource duplication permission or Eq evidence.
    pub fn immutable_clone_work(&self) -> Option<usize> {
        let (expanded, inspected) =
            self.scoped_representation_work(&Default::default(), &Default::default())?;
        // The preceding walk bounds the complete graph and publishes only
        // closed summaries. Stop at shared payloads, including nested lists;
        // subtract each payload once, never its enclosing owned metadata.
        fn shared(value: &Value, visited: &mut usize) -> Option<usize> {
            *visited = visited.checked_add(1)?;
            let mut total = 0usize;
            let mut child = |value: &Value| {
                total = total.checked_add(shared(value, visited)?)?;
                Some(())
            };
            match value {
                Value::String(text) | Value::Symbol(text) => return Some(text.len()),
                Value::List { values, .. } => return Some(values.summary()?.work),
                Value::Tuple(values)
                | Value::Record { fields: values, .. }
                | Value::Variant { fields: values, .. } => {
                    for value in values {
                        child(value)?;
                    }
                }
                Value::AnonymousRecord(fields) => {
                    for (_, value) in fields {
                        child(value)?;
                    }
                }
                Value::Object(object) => {
                    for entry in object.entries() {
                        child(&entry.value().value)?;
                    }
                }
                Value::Optional { value, .. } => {
                    if let Some(value) = value {
                        child(value)?;
                    }
                }
                Value::ErrorResult { value, .. } | Value::Newtype { value, .. } => child(value)?,
                _ => {}
            }
            Some(total)
        }
        let mut visited = 0;
        let shared = shared(self, &mut visited)?;
        inspected
            .checked_add(visited)?
            .checked_add(expanded.checked_sub(shared)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Primitive, Storage, Term};

    #[test]
    fn text_clones_share_bytes_but_keep_expanded_size_limits() {
        let mut costs = Vec::new();
        for len in [32, 4096] {
            let text: std::sync::Arc<str> = "x".repeat(len).into();
            let value = Value::Tuple(vec![Value::String(text.clone()), Value::Symbol(text)]);
            costs.push(value.immutable_clone_work().unwrap());
            assert!(value.immutable_copy_work().unwrap() >= len * 2);
            let copy = value.clone();
            let (Value::Tuple(original), Value::Tuple(copied)) = (&value, &copy) else {
                unreachable!()
            };
            let (Value::String(original), Value::String(copied)) = (&original[0], &copied[0])
            else {
                unreachable!()
            };
            assert!(std::sync::Arc::ptr_eq(original, copied));
        }
        assert_eq!(costs[0], costs[1]);
        let oversized = Value::String("x".repeat(100_000).into());
        assert!(oversized.immutable_clone_work().is_none());
        assert!(
            Value::String("small".into())
                .immutable_clone_work()
                .is_some()
        );
    }

    fn list(len: usize) -> Value {
        Value::List {
            ty: Term::Primitive(Primitive::Bool),
            values: vec![Value::Bool(true); len].into(),
        }
    }

    #[test]
    fn shared_payload_copy_cost_is_separate_from_expanded_size() {
        let mut costs = Vec::new();
        for len in [32, 4096] {
            let value = Value::Tuple(vec![Value::Bool(true), list(len)]);
            let cold = value.immutable_clone_work().unwrap();
            let warm = value.immutable_clone_work().unwrap();
            assert!(cold >= len);
            assert!(value.immutable_copy_work().unwrap() >= len);
            assert!(warm < cold);
            costs.push(warm);
            let mut snapshot = value.clone();
            let Value::Tuple(fields) = &mut snapshot else {
                unreachable!()
            };
            let Value::List { values, .. } = &mut fields[1] else {
                unreachable!()
            };
            values[0] = Value::Bool(false);
            assert_ne!(snapshot, value);
            assert_eq!(value.immutable_clone_work(), Some(warm));
            let Value::Tuple(fields) = &mut snapshot else {
                unreachable!()
            };
            let Value::List { values, .. } = &mut fields[1] else {
                unreachable!()
            };
            values[0] = Value::Storage(Storage::new(Value::Bool(true)));
            assert_eq!(snapshot.immutable_clone_work(), None);
            assert_eq!(value.immutable_clone_work(), Some(warm));
        }
        assert_eq!(costs[0], costs[1]);
    }

    #[test]
    fn shared_payloads_keep_size_and_depth_rejection() {
        let left = list(60_000);
        left.immutable_clone_work().unwrap();
        let Value::List { mut values, ty } = left else {
            unreachable!()
        };
        let Value::List { values: right, .. } = list(60_000) else {
            unreachable!()
        };
        let warmed = Value::List {
            ty: ty.clone(),
            values: right,
        };
        warmed.immutable_clone_work().unwrap();
        let Value::List { values: right, .. } = warmed else {
            unreachable!()
        };
        values.append_payload(right);
        assert!(Value::List { ty, values }.immutable_clone_work().is_none());
        let mut nested = list(1);
        nested.immutable_clone_work().unwrap();
        for _ in 0..128 {
            nested = Value::Tuple(vec![nested]);
        }
        assert!(nested.immutable_clone_work().is_none());
        assert!(list(1).immutable_clone_work().is_some());
    }
}
