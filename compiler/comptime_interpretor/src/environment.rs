//! Invocation registers. Ordinary locals of one owner use direct indexed
//! slots; mixed-owner captures and sparse identities use the ordered fallback.
//! Local identities are never truncated or used for unbounded allocation.
use super::{LocalId, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
enum Registers {
    Slots {
        entries: Vec<(LocalId, Option<Value>)>,
        live: usize,
    },
    Sparse(BTreeMap<LocalId, Value>),
}
// Each active lexical scope records net membership changes only. Values stay in
// their registers so mutations and moves of outer bindings survive scope exit.
#[derive(Debug)]
pub(super) struct Environment {
    registers: Registers,
    scopes: Vec<BTreeMap<LocalId, Membership>>,
}
#[derive(Clone, Copy, Debug)]
enum Membership {
    Added,
    Removed,
}

impl Clone for Environment {
    fn clone(&self) -> Self {
        // Captured environments start a new activation with its own scopes.
        Self {
            registers: self.registers.clone(),
            scopes: Vec::new(),
        }
    }
}
impl Default for Environment {
    fn default() -> Self {
        Self::with_capacity(0)
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
        Self {
            registers: Registers::Slots {
                entries: Vec::with_capacity(capacity),
                live: 0,
            },
            scopes: Vec::new(),
        }
    }
    pub fn begin_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
    }
    fn record(&mut self, id: LocalId, change: Membership) {
        use std::collections::btree_map::Entry;
        if let Some(scope) = self.scopes.last_mut() {
            // Membership alternates on insert/remove. Opposite transitions
            // cancel, including when a child scope commits to its parent.
            match scope.entry(id) {
                Entry::Vacant(entry) => {
                    entry.insert(change);
                }
                Entry::Occupied(entry) => {
                    entry.remove();
                }
            }
        }
    }
    pub fn leave_scope(&mut self) {
        for (id, change) in self.scopes.pop().expect("active lexical scope") {
            match change {
                Membership::Added => {
                    self.remove_register(&id);
                }
                Membership::Removed => self.record(id, change),
            }
        }
    }
    // A successful let-pattern delivers its bindings into the enclosing scope.
    pub fn commit_scope(&mut self) {
        let changes = self.scopes.pop().expect("active lexical scope");
        if !self.scopes.is_empty() {
            for (id, change) in changes {
                self.record(id, change);
            }
        }
    }
    pub fn scoped<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
        self.begin_scope();
        let result = run(self);
        self.leave_scope();
        result
    }
    pub fn len(&self) -> usize {
        match &self.registers {
            Registers::Slots { live, .. } => *live,
            Registers::Sparse(map) => map.len(),
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
        match &self.registers {
            Registers::Slots { entries, .. } => entries.get(Self::offset(entries, id)?)?.1.as_ref(),
            Registers::Sparse(map) => map.get(id),
        }
    }
    pub fn get_mut(&mut self, id: &LocalId) -> Option<&mut Value> {
        match &mut self.registers {
            Registers::Slots { entries, .. } => {
                let offset = Self::offset(entries, id)?;
                entries.get_mut(offset)?.1.as_mut()
            }
            Registers::Sparse(map) => map.get_mut(id),
        }
    }
    pub fn contains_key(&self, id: &LocalId) -> bool {
        self.get(id).is_some()
    }
    pub fn insert(&mut self, id: LocalId, value: Value) -> Option<Value> {
        let previous = self.insert_register(id, value);
        if previous.is_none() {
            self.record(id, Membership::Added);
        }
        previous
    }
    fn insert_register(&mut self, id: LocalId, value: Value) -> Option<Value> {
        if let Registers::Slots { entries, live } = &mut self.registers {
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
            self.registers = Registers::Sparse(map);
        }
        let Registers::Sparse(map) = &mut self.registers else {
            unreachable!()
        };
        map.insert(id, value)
    }
    pub fn remove(&mut self, id: &LocalId) -> Option<Value> {
        let value = self.remove_register(id);
        if value.is_some() {
            self.record(*id, Membership::Removed);
        }
        value
    }
    fn remove_register(&mut self, id: &LocalId) -> Option<Value> {
        let value = match &mut self.registers {
            Registers::Slots { entries, live } => {
                let offset = Self::offset(entries, id)?;
                let value = entries.get_mut(offset)?.1.take();
                if value.is_some() {
                    *live -= 1;
                }
                value
            }
            Registers::Sparse(map) => map.remove(id),
        };
        self.compact();
        value
    }
    fn compact(&mut self) {
        if let Registers::Slots { entries, live } = &mut self.registers {
            // Iteration must remain bounded by live bindings, not by the
            // largest historical frame. Otherwise iterating over locals
            // could scan arbitrarily many dead registers for a tiny budget.
            if *live == 0 {
                *entries = Vec::new();
            } else if entries.len() > live.saturating_mul(2).saturating_add(16) {
                let map = std::mem::take(entries)
                    .into_iter()
                    .filter_map(|(id, value)| value.map(|value| (id, value)))
                    .collect();
                self.registers = Registers::Sparse(map);
            }
        }
    }
    pub fn iter(&self) -> Iter<'_> {
        match &self.registers {
            Registers::Slots { entries, .. } => Iter::Slots(entries.iter()),
            Registers::Sparse(map) => Iter::Sparse(map.iter()),
        }
    }
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.iter().map(|(_, value)| value)
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Value> {
        let (slots, sparse) = match &mut self.registers {
            Registers::Slots { entries, .. } => (Some(entries.iter_mut()), None),
            Registers::Sparse(map) => (None, Some(map.values_mut())),
        };
        slots
            .into_iter()
            .flatten()
            .filter_map(|(_, value)| value.as_mut())
            .chain(sparse.into_iter().flatten())
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
        for i in 0..1023 {
            frame.remove(&id(i));
        }
        assert_eq!(frame.len(), 1);
        assert!(matches!(frame.registers, Registers::Sparse(_)));
        assert_eq!(frame.remove(&id(1023)), Some(Value::Unit));
    }
    #[test]
    fn nested_scopes_preserve_moves_and_commit_pattern_bindings_to_the_parent() {
        let id = |index| LocalId { owner: 1, index };
        let mut frame = Environment::from([(id(0), Value::Unit), (id(1), Value::Unit)]);
        frame.begin_scope();
        frame.insert(id(2), Value::Unit);
        frame.scoped(|frame| {
            frame.remove(&id(0));
            frame.remove(&id(1));
            frame.remove(&id(2));
            frame.insert(id(3), Value::Unit);
        });
        frame.insert(id(0), Value::Bool(true));
        frame.begin_scope();
        frame.insert(id(4), Value::Unit);
        frame.commit_scope();
        assert_eq!(frame.get(&id(4)), Some(&Value::Unit));
        let captured = frame.clone();
        frame.leave_scope();
        assert_eq!(frame, Environment::from([(id(0), Value::Bool(true))]));
        assert_eq!(captured.len(), 2);
        assert!(captured.scopes.is_empty());
    }
    #[test]
    fn lexical_scopes_cover_empty_dense_wide_and_sparse_frames() {
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
                frame.begin_scope();
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
                frame.leave_scope();
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
        frame.begin_scope();
        frame.insert(
            LocalId {
                owner: 10,
                index: u32::MAX,
            },
            Value::Unit,
        );
        frame.leave_scope();
        assert_eq!(frame, saved);
        for i in (1..16).step_by(2) {
            frame.remove(&id(i));
        }
        assert_eq!(frame.len(), 7);
        assert!(!frame.contains_key(&id(4)));
        assert_eq!(saved.len(), 15);
        assert_eq!(saved.get(&id(5)), Some(&Value::Bool(false)));
        for value in frame.values_mut() {
            *value = Value::Bool(false);
        }
        assert!(frame.values().all(|v| *v == Value::Bool(false)));
    }
}
