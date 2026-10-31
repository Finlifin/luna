//! Invocation registers. Ordinary locals of one owner use direct indexed
//! slots; mixed-owner captures and sparse identities use the ordered fallback.
//! Local identities are never truncated or used for unbounded allocation.
use super::{LocalId, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(super) enum Environment {
    Slots {
        entries: Vec<(LocalId, Option<Value>)>,
        live: usize,
    },
    Sparse(BTreeMap<LocalId, Value>),
}
impl Default for Environment {
    fn default() -> Self {
        Self::Slots {
            entries: Vec::new(),
            live: 0,
        }
    }
}
impl PartialEq for Environment {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}
impl Environment {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_capacity(capacity: usize) -> Self {
        Self::Slots {
            entries: Vec::with_capacity(capacity),
            live: 0,
        }
    }
    pub fn snapshot(&self) -> ScopeSnapshot {
        match self {
            Self::Slots { entries, .. } if entries.len() <= 64 => {
                let bits = entries
                    .iter()
                    .enumerate()
                    .fold(0u64, |bits, (i, (_, value))| {
                        bits | (u64::from(value.is_some()) << i)
                    });
                ScopeSnapshot::Registers {
                    base: entries.first().map(|e| e.0),
                    bits,
                }
            }
            _ => ScopeSnapshot::Sparse(self.keys().copied().collect()),
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Self::Slots { live, .. } => *live,
            Self::Sparse(map) => map.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn offset(entries: &[(LocalId, Option<Value>)], id: &LocalId) -> Option<usize> {
        let first = entries.first()?.0;
        (id.owner == first.owner).then_some(())?;
        Some(id.index.checked_sub(first.index)? as usize)
    }
    pub fn get(&self, id: &LocalId) -> Option<&Value> {
        match self {
            Self::Slots { entries, .. } => entries.get(Self::offset(entries, id)?)?.1.as_ref(),
            Self::Sparse(map) => map.get(id),
        }
    }
    pub fn get_mut(&mut self, id: &LocalId) -> Option<&mut Value> {
        match self {
            Self::Slots { entries, .. } => {
                let offset = Self::offset(entries, id)?;
                entries.get_mut(offset)?.1.as_mut()
            }
            Self::Sparse(map) => map.get_mut(id),
        }
    }
    pub fn contains_key(&self, id: &LocalId) -> bool {
        self.get(id).is_some()
    }
    pub fn insert(&mut self, id: LocalId, value: Value) -> Option<Value> {
        if let Self::Slots { entries, live } = self {
            if entries.is_empty() {
                entries.push((id, Some(value)));
                *live = 1;
                return None;
            }
            if let Some(offset) = Self::offset(entries, &id) {
                // Limit holes by the live population, independently of the
                // numeric magnitude of a caller-supplied LocalId.
                if offset < entries.len() || offset <= live.saturating_mul(2).saturating_add(16) {
                    let first = entries[0].0;
                    while entries.len() <= offset {
                        entries.push((
                            LocalId {
                                owner: first.owner,
                                index: first.index + entries.len() as u32,
                            },
                            None,
                        ));
                    }
                    let previous = entries[offset].1.replace(value);
                    if previous.is_none() {
                        *live += 1;
                    }
                    return previous;
                }
            }
            let map = std::mem::take(entries)
                .into_iter()
                .filter_map(|(id, value)| value.map(|v| (id, v)))
                .collect();
            *self = Self::Sparse(map);
        }
        let Self::Sparse(map) = self else {
            unreachable!()
        };
        map.insert(id, value)
    }
    pub fn remove(&mut self, id: &LocalId) -> Option<Value> {
        let value = match self {
            Self::Slots { entries, live } => {
                let offset = Self::offset(entries, id)?;
                let value = entries.get_mut(offset)?.1.take();
                if value.is_some() {
                    *live -= 1;
                }
                value
            }
            Self::Sparse(map) => map.remove(id),
        };
        self.compact();
        value
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&LocalId, &mut Value) -> bool) {
        match self {
            Self::Slots { entries, live } => {
                for (id, slot) in entries {
                    if slot.as_mut().is_some_and(|value| !keep(id, value)) {
                        *slot = None;
                        *live -= 1;
                    }
                }
            }
            Self::Sparse(map) => map.retain(keep),
        }
        self.compact();
    }
    fn compact(&mut self) {
        if let Self::Slots { entries, live } = self {
            // Iteration must remain bounded by live bindings, not by the
            // largest historical frame. Otherwise repeated scope snapshots
            // could scan arbitrarily many dead registers for a tiny budget.
            if *live == 0 {
                *self = Self::new();
            } else if entries.len() > live.saturating_mul(2).saturating_add(16) {
                let map = std::mem::take(entries)
                    .into_iter()
                    .filter_map(|(id, value)| value.map(|value| (id, value)))
                    .collect();
                *self = Self::Sparse(map);
            }
        }
    }
    pub fn iter(&self) -> Iter<'_> {
        match self {
            Self::Slots { entries, .. } => Iter::Slots(entries.iter()),
            Self::Sparse(map) => Iter::Sparse(map.iter()),
        }
    }
    pub fn keys(&self) -> impl Iterator<Item = &LocalId> {
        self.iter().map(|(id, _)| id)
    }
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.iter().map(|(_, value)| value)
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Value> {
        let (slots, sparse) = match self {
            Self::Slots { entries, .. } => (Some(entries.iter_mut()), None),
            Self::Sparse(map) => (None, Some(map.values_mut())),
        };
        slots
            .into_iter()
            .flatten()
            .filter_map(|(_, value)| value.as_mut())
            .chain(sparse.into_iter().flatten())
    }
}
pub(super) enum ScopeSnapshot {
    Registers { base: Option<LocalId>, bits: u64 },
    Sparse(Vec<LocalId>),
}
impl ScopeSnapshot {
    pub fn contains(&self, id: &LocalId) -> bool {
        match self {
            Self::Registers {
                base: Some(base),
                bits,
            } if base.owner == id.owner => id
                .index
                .checked_sub(base.index)
                .is_some_and(|offset| offset < 64 && bits & (1 << offset) != 0),
            Self::Registers { .. } => false,
            Self::Sparse(ids) => ids.binary_search(id).is_ok(),
        }
    }
}
pub(super) enum Iter<'a> {
    Slots(std::slice::Iter<'a, (LocalId, Option<Value>)>),
    Sparse(std::collections::btree_map::Iter<'a, LocalId, Value>),
}
impl<'a> Iterator for Iter<'a> {
    type Item = (&'a LocalId, &'a Value);
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Slots(entries) => {
                entries.find_map(|(id, value)| value.as_ref().map(|value| (id, value)))
            }
            Self::Sparse(entries) => entries.next(),
        }
    }
}
impl<'a> IntoIterator for &'a Environment {
    type Item = (&'a LocalId, &'a Value);
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl FromIterator<(LocalId, Value)> for Environment {
    fn from_iter<T: IntoIterator<Item = (LocalId, Value)>>(iter: T) -> Self {
        let mut environment = Self::new();
        for (id, value) in iter {
            environment.insert(id, value);
        }
        environment
    }
}
impl<const N: usize> From<[(LocalId, Value); N]> for Environment {
    fn from(values: [(LocalId, Value); N]) -> Self {
        values.into_iter().collect()
    }
}
impl std::ops::Index<&LocalId> for Environment {
    type Output = Value;
    fn index(&self, id: &LocalId) -> &Self::Output {
        self.get(id).expect("missing local register")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dead_registers_do_not_accumulate_across_scopes() {
        let id = |index| LocalId { owner: 1, index };
        let mut frame: Environment = (0..1024).map(|i| (id(i), Value::Unit)).collect();
        frame.retain(|binding, _| binding.index == 1023);
        assert_eq!(frame.len(), 1);
        assert!(matches!(frame, Environment::Sparse(_)));
        assert_eq!(frame.remove(&id(1023)), Some(Value::Unit));
        let mut frame: Environment = (0..1024).map(|i| (id(i), Value::Unit)).collect();
        for i in 0..1023 {
            frame.remove(&id(i));
        }
        assert!(matches!(frame, Environment::Sparse(_)));
        assert_eq!(frame.get(&id(1023)), Some(&Value::Unit));
    }
    #[test]
    fn scope_snapshots_cover_empty_dense_wide_and_sparse_frames() {
        for size in [0, 1, 64, 65, 128] {
            for sparse in [false, true] {
                let mut frame = Environment::new();
                let mut expected = BTreeMap::new();
                for index in 0..size {
                    let id = LocalId {
                        owner: if sparse { index % 3 } else { 1 },
                        index: if sparse { index * 1000 } else { index },
                    };
                    frame.insert(id, Value::Bool(true));
                    expected.insert(id, Value::Bool(true));
                }
                let snapshot = frame.snapshot();
                // Replacing an old binding survives a scope exit; new owners,
                // holes and sparse far-away identities must disappear.
                for index in 0..256 {
                    let id = LocalId { owner: 1, index };
                    frame.insert(id, Value::Bool(false));
                    if let Some(value) = expected.get_mut(&id) {
                        *value = Value::Bool(false);
                    }
                }
                frame.insert(
                    LocalId {
                        owner: u32::MAX,
                        index: u32::MAX,
                    },
                    Value::Unit,
                );
                frame.retain(|id, _| snapshot.contains(id));
                assert!(frame.iter().eq(expected.iter()));
                assert_eq!(frame.len(), expected.len());
            }
        }
    }
    #[test]
    fn registers_preserve_order_holes_and_sparse_identity() {
        let mut actual = Environment::new();
        let mut expected = BTreeMap::new();
        for (owner, index) in [
            (3, 10),
            (3, 12),
            (3, 11),
            (3, 12),
            (3, u32::MAX),
            (2, 0),
            (3, 0),
        ] {
            let id = LocalId { owner, index };
            assert_eq!(
                actual.insert(id, Value::Bool(index % 2 == 0)),
                expected.insert(id, Value::Bool(index % 2 == 0))
            );
            assert!(actual.iter().eq(expected.iter()));
        }
        for id in expected.keys().copied().collect::<Vec<_>>() {
            assert_eq!(actual.remove(&id), expected.remove(&id));
            assert_eq!(actual.remove(&id), None);
            assert_eq!(actual.len(), expected.len());
        }
    }
    #[test]
    fn slot_mutation_retention_and_clone_do_not_resurrect_bindings() {
        let id = |index| LocalId { owner: 9, index };
        let mut frame: Environment = (0..16).map(|i| (id(i), Value::Bool(true))).collect();
        frame.remove(&id(4));
        *frame.get_mut(&id(5)).unwrap() = Value::Bool(false);
        let saved = frame.clone();
        let snapshot = frame.snapshot();
        frame.insert(
            LocalId {
                owner: 10,
                index: u32::MAX,
            },
            Value::Unit,
        );
        frame.retain(|id, _| snapshot.contains(id));
        assert_eq!(frame, saved);
        frame.retain(|id, _| id.index % 2 == 0);
        assert_eq!(frame.len(), 7);
        assert!(!frame.contains_key(&id(4)));
        assert_eq!(saved.len(), 15);
        assert_eq!(saved.get(&id(5)), Some(&Value::Bool(false)));
        for value in frame.values_mut() {
            *value = Value::Bool(false);
        }
        assert!(frame.values().all(|v| *v == Value::Bool(false)));
        assert_eq!(
            saved,
            Environment::Sparse(saved.iter().map(|(id, v)| (*id, v.clone())).collect())
        );
    }
}
