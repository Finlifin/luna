//! Lexical versions retained by source regions. Forks share values and all
//! untouched branches. The most significant differing bit orders LocalIds
//! exactly as (owner, index); a path examines at most 64 discriminating bits.
use super::n::LocalId;
use std::{ops::Index, sync::Arc};

pub(super) struct LocalBindings<T> {
    root: Option<Node<T>>,
    len: usize,
}

enum Node<T> {
    Binding(LocalId, Arc<T>),
    Fork(Arc<Fork<T>>),
}

struct Fork<T> {
    prefix: u64,
    bit: u64,
    children: [Node<T>; 2],
}

fn key(id: LocalId) -> u64 {
    u64::from(id.owner) << 32 | u64::from(id.index)
}

fn side(key: u64, bit: u64) -> usize {
    usize::from(key & bit != 0)
}

impl<T> Clone for Node<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Binding(id, value) => Self::Binding(*id, value.clone()),
            Self::Fork(fork) => Self::Fork(fork.clone()),
        }
    }
}

impl<T> Clone for Fork<T> {
    fn clone(&self) -> Self {
        Self {
            children: self.children.clone(),
            ..*self
        }
    }
}

impl<T> Clone for LocalBindings<T> {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            len: self.len,
        }
    }
}

impl<T> Default for LocalBindings<T> {
    fn default() -> Self {
        Self { root: None, len: 0 }
    }
}

impl<T> Node<T> {
    fn insert(&mut self, id: LocalId, value: Arc<T>) -> Option<Arc<T>> {
        let incoming = key(id);
        let existing = match self {
            Self::Binding(previous, old) if *previous == id => {
                return Some(std::mem::replace(old, value));
            }
            Self::Binding(previous, _) => key(*previous),
            Self::Fork(fork) if incoming & !(fork.bit | (fork.bit - 1)) == fork.prefix => {
                let index = side(incoming, fork.bit);
                return Arc::make_mut(fork).children[index].insert(id, value);
            }
            Self::Fork(fork) => fork.prefix,
        };
        let bit = 1 << (63 - (incoming ^ existing).leading_zeros());
        let mut children = [self.clone(), Self::Binding(id, value)];
        if incoming & bit == 0 {
            children.swap(0, 1);
        }
        *self = Self::Fork(Arc::new(Fork {
            prefix: incoming & !(bit | (bit - 1)),
            bit,
            children,
        }));
        None
    }

    fn representative(&self) -> u64 {
        match self {
            Self::Binding(id, _) => key(*id),
            Self::Fork(fork) => fork.prefix,
        }
    }

    // Walk only the intersection. Shared forks have exactly the same values.
    fn changed_bindings(&self, inner: &Self, update: &mut impl FnMut(LocalId, &Arc<T>)) {
        match (self, inner) {
            (Self::Binding(a, before), Self::Binding(b, after)) => {
                if a == b && !Arc::ptr_eq(before, after) {
                    update(*a, after);
                }
            }
            (Self::Fork(a), Self::Fork(b)) if a.bit == b.bit => {
                if a.prefix == b.prefix && !Arc::ptr_eq(a, b) {
                    for (outer, inner) in a.children.iter().zip(&b.children) {
                        outer.changed_bindings(inner, update);
                    }
                }
            }
            (Self::Fork(a), Self::Fork(b)) if b.bit > a.bit => {
                self.changed_bindings(&b.children[side(a.prefix, b.bit)], update);
            }
            (Self::Fork(a), inner) => {
                a.children[side(inner.representative(), a.bit)].changed_bindings(inner, update);
            }
            (outer, Self::Fork(b)) => {
                outer.changed_bindings(&b.children[side(outer.representative(), b.bit)], update);
            }
        }
    }

    // Construct a new path only when removal succeeds. A fork with one
    // remaining child collapses directly to that child's existing handle.
    fn removing(&self, id: LocalId) -> Option<(Option<Self>, Arc<T>)> {
        match self {
            Self::Binding(found, value) => (*found == id).then(|| (None, value.clone())),
            Self::Fork(fork) => {
                let index = side(key(id), fork.bit);
                let (child, value) = fork.children[index].removing(id)?;
                let replacement = match child {
                    Some(child) => {
                        let mut changed = (**fork).clone();
                        changed.children[index] = child;
                        Self::Fork(Arc::new(changed))
                    }
                    None => fork.children[1 - index].clone(),
                };
                Some((Some(replacement), value))
            }
        }
    }
}

impl<T> LocalBindings<T> {
    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn shared(&self, id: &LocalId) -> Option<&Arc<T>> {
        let mut node = self.root.as_ref()?;
        loop {
            match node {
                Node::Binding(found, value) => return (found == id).then_some(value),
                Node::Fork(fork) => node = &fork.children[side(key(*id), fork.bit)],
            }
        }
    }

    pub(super) fn get(&self, id: &LocalId) -> Option<&T> {
        self.shared(id).map(Arc::as_ref)
    }

    pub(super) fn contains_key(&self, id: &LocalId) -> bool {
        self.shared(id).is_some()
    }

    pub(super) fn insert(&mut self, id: LocalId, value: impl Into<Arc<T>>) -> Option<Arc<T>> {
        let value = value.into();
        if let Some(node) = &mut self.root {
            if let Some(old) = node.insert(id, value) {
                return Some(old);
            }
        } else {
            self.root = Some(Node::Binding(id, value));
        }
        self.len += 1;
        None
    }

    /// Keep assignments to pre-existing code bindings when a generator scope
    /// succeeds. Introductions and removals remain local to the child.
    pub(super) fn retain_assignments(&mut self, inner: &Self) {
        if let (Some(outer), Some(current)) = (self.root.clone(), &inner.root) {
            outer.changed_bindings(current, &mut |id, value| {
                self.insert(id, value.clone());
            });
        }
    }

    pub(super) fn remove(&mut self, id: &LocalId) -> Option<Arc<T>> {
        let (root, value) = self.root.as_ref()?.removing(*id)?;
        self.root = root;
        self.len -= 1;
        Some(value)
    }

    pub(super) fn iter(&self) -> Iter<'_, T> {
        self.into_iter()
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &LocalId> {
        self.iter().map(|(id, _)| id)
    }
}

impl<T> Index<&LocalId> for LocalBindings<T> {
    type Output = T;
    fn index(&self, id: &LocalId) -> &T {
        self.get(id).expect("missing lexical binding")
    }
}

impl<T, V: Into<Arc<T>>> Extend<(LocalId, V)> for LocalBindings<T> {
    fn extend<I: IntoIterator<Item = (LocalId, V)>>(&mut self, values: I) {
        for (id, value) in values {
            self.insert(id, value);
        }
    }
}

#[cfg(test)]
impl<T, const N: usize> From<[(LocalId, T); N]> for LocalBindings<T> {
    fn from(values: [(LocalId, T); N]) -> Self {
        let mut bindings = Self::default();
        bindings.extend(values);
        bindings
    }
}

pub(super) struct Iter<'a, T> {
    pending: Vec<&'a Node<T>>,
}

impl<'a, T> IntoIterator for &'a LocalBindings<T> {
    type Item = (&'a LocalId, &'a T);
    type IntoIter = Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        Iter {
            pending: self.root.iter().collect(),
        }
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = (&'a LocalId, &'a T);
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(node) = self.pending.pop() {
            match node {
                Node::Binding(id, value) => return Some((id, value.as_ref())),
                Node::Fork(fork) => {
                    self.pending.push(&fork.children[1]);
                    self.pending.push(&fork.children[0]);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn id(bits: u64) -> LocalId {
        LocalId {
            owner: (bits >> 32) as u32,
            index: bits as u32,
        }
    }

    #[test]
    fn retained_versions_share_unmodified_subtrees_and_payloads() {
        // No Clone implementation: retaining or modifying a scope never
        // copies the type or quoted code stored in a binding.
        struct Value(u32);
        let mut original = LocalBindings::default();
        for bits in [0, 1, u64::MAX - 1, u64::MAX] {
            original.insert(id(bits), Value(11));
        }
        let mut child = original.clone();
        let mut sibling = original.clone();
        let old = child.insert(id(0), Value(20)).unwrap();
        assert!(Arc::ptr_eq(original.shared(&id(0)).unwrap(), &old));
        sibling.remove(&id(1));
        sibling.insert(id(17), Value(30));
        assert_eq!(child[&id(0)].0, 20);
        assert_eq!(original[&id(0)].0, 11);
        assert!(!original.contains_key(&id(17)));
        assert!(child.contains_key(&id(1)));
        fn right_root(bindings: &LocalBindings<Value>) -> &Arc<Fork<Value>> {
            let Some(Node::Fork(root)) = &bindings.root else {
                panic!()
            };
            let Node::Fork(right) = &root.children[1] else {
                panic!()
            };
            right
        }
        assert!(Arc::ptr_eq(right_root(&original), right_root(&child)));
        assert!(Arc::ptr_eq(right_root(&original), right_root(&sibling)));
        child.insert(id(0), old);
        sibling.clear();
        assert!(Arc::ptr_eq(
            child.shared(&id(0)).unwrap(),
            original.shared(&id(0)).unwrap()
        ));
        assert_eq!(original.len(), 4);
    }

    #[test]
    fn versions_preserve_key_order_through_splits_collapse_and_scope_exit() {
        let ids: Vec<_> = [0, u64::MAX, 27]
            .into_iter()
            .chain((0..64).flat_map(|bit| [1u64 << bit, !(1u64 << bit)]))
            .map(id)
            .collect();
        let mut seed = 983117u64;
        let mut parent = LocalBindings::default();
        let mut expected = BTreeMap::new();
        for (index, key) in ids.iter().enumerate() {
            expected.insert(*key, index);
            parent.insert(*key, index);
        }
        for generation in 1..65 {
            let mut child = parent.clone();
            let mut child_expected = expected.clone();
            for index in 0..72 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let key = ids[(seed >> 32) as usize % ids.len()];
                if seed % 4 == 0 {
                    assert_eq!(
                        child.remove(&key).as_deref(),
                        child_expected.remove(&key).as_ref()
                    );
                } else {
                    let value = generation * 100 + index;
                    assert_eq!(
                        child.insert(key, value).as_deref(),
                        child_expected.insert(key, value).as_ref()
                    );
                }
            }
            assert!(child.iter().eq(child_expected.iter()));
            assert!(parent.iter().eq(expected.iter()));
            assert_eq!(child.len(), child_expected.len());
            // A generator's successful exit exports only assignments to names
            // present in both scopes, including assignments beneath split forks.
            let mut exported = parent.clone();
            exported.retain_assignments(&child);
            let mut expected_exports = expected.clone();
            for (k, v) in &mut expected_exports {
                if let Some(value) = child_expected.get(k) {
                    *v = *value;
                }
            }
            assert!(exported.iter().eq(expected_exports.iter()));
            parent = child;
            expected = child_expected;
        }
        for key in expected.keys() {
            assert!(parent.remove(key).is_some());
        }
        assert_eq!(parent.len(), 0);
        assert!(parent.iter().next().is_none());
    }
}
