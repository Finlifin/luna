//! Persistent lexical bindings indexed by the full local or declaration ID.
//! Each branch discriminates one bit. Forking shares a root; updating copies
//! only the changed path and never copies the bound type or code.
use super::{DeclarationId, n::LocalId};
use std::{ops::Index, sync::Arc};

pub(super) trait BindingKey: Copy + Eq {
    fn bits(self) -> u128;
}
impl BindingKey for LocalId {
    fn bits(self) -> u128 {
        u128::from(self.owner) << 32 | u128::from(self.index)
    }
}
impl BindingKey for DeclarationId {
    fn bits(self) -> u128 {
        u128::from(self.package) << 32 | u128::from(self.local)
    }
}

pub(super) type LocalBindings<T> = Bindings<LocalId, T>;
pub(super) type DeclarationBindings<T> = Bindings<DeclarationId, T>;

pub(super) struct Bindings<K, T> {
    root: Option<Node<K, T>>,
    len: usize,
}

enum Node<K, T> {
    Binding(K, Arc<T>),
    Fork(Arc<Fork<K, T>>),
}

struct Fork<K, T> {
    sample: K,
    bit: u32,
    children: [Node<K, T>; 2],
}

fn side(bits: u128, bit: u32) -> usize {
    ((bits >> bit) & 1) as usize
}

impl<K: BindingKey, T> Fork<K, T> {
    fn covers(&self, bits: u128) -> bool {
        // Only bits above this fork must agree with its representative.
        (bits ^ self.sample.bits()) >> self.bit <= 1
    }
}

impl<K: BindingKey, T> Clone for Node<K, T> {
    fn clone(&self) -> Self {
        match self {
            Self::Binding(id, value) => Self::Binding(*id, value.clone()),
            Self::Fork(fork) => Self::Fork(fork.clone()),
        }
    }
}

impl<K: BindingKey, T> Clone for Fork<K, T> {
    fn clone(&self) -> Self {
        Self {
            children: self.children.clone(),
            ..*self
        }
    }
}

impl<K: BindingKey, T> Clone for Bindings<K, T> {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            len: self.len,
        }
    }
}

impl<K: BindingKey + std::fmt::Debug, T: std::fmt::Debug> std::fmt::Debug for Bindings<K, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K: BindingKey, T> Default for Bindings<K, T> {
    fn default() -> Self {
        Self { root: None, len: 0 }
    }
}

impl<K: BindingKey, T> Node<K, T> {
    fn insert(&mut self, id: K, value: Arc<T>) -> Option<Arc<T>> {
        let incoming = id.bits();
        let existing = match self {
            Self::Binding(previous, old) if *previous == id => {
                return Some(std::mem::replace(old, value));
            }
            Self::Binding(previous, _) => previous.bits(),
            Self::Fork(fork) if fork.covers(incoming) => {
                let index = side(incoming, fork.bit);
                return Arc::make_mut(fork).children[index].insert(id, value);
            }
            Self::Fork(fork) => fork.sample.bits(),
        };
        let bit = (incoming ^ existing).ilog2();
        let mut children = [self.clone(), Self::Binding(id, value)];
        if side(incoming, bit) == 0 {
            children.swap(0, 1);
        }
        *self = Self::Fork(Arc::new(Fork {
            sample: id,
            bit,
            children,
        }));
        None
    }

    fn representative(&self) -> u128 {
        match self {
            Self::Binding(id, _) => id.bits(),
            Self::Fork(fork) => fork.sample.bits(),
        }
    }

    // Walk only the intersection. Shared forks have exactly the same values.
    fn changed_bindings(&self, inner: &Self, update: &mut impl FnMut(K, &Arc<T>)) {
        match (self, inner) {
            (Self::Binding(a, before), Self::Binding(b, after)) => {
                if a == b && !Arc::ptr_eq(before, after) {
                    update(*a, after);
                }
            }
            (Self::Fork(a), Self::Fork(b)) if a.bit == b.bit => {
                if a.covers(b.sample.bits()) && !Arc::ptr_eq(a, b) {
                    for (outer, inner) in a.children.iter().zip(&b.children) {
                        outer.changed_bindings(inner, update);
                    }
                }
            }
            (Self::Fork(a), Self::Fork(b)) if b.bit > a.bit => {
                self.changed_bindings(&b.children[side(a.sample.bits(), b.bit)], update);
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
    fn removing(&self, id: K) -> Option<(Option<Self>, Arc<T>)> {
        match self {
            Self::Binding(found, value) => (*found == id).then(|| (None, value.clone())),
            Self::Fork(fork) => {
                let index = side(id.bits(), fork.bit);
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

impl<K: BindingKey, T> Bindings<K, T> {
    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn shared(&self, id: &K) -> Option<&Arc<T>> {
        let mut node = self.root.as_ref()?;
        loop {
            match node {
                Node::Binding(found, value) => return (found == id).then_some(value),
                Node::Fork(fork) => node = &fork.children[side(id.bits(), fork.bit)],
            }
        }
    }

    pub(super) fn get(&self, id: &K) -> Option<&T> {
        self.shared(id).map(Arc::as_ref)
    }

    pub(super) fn contains_key(&self, id: &K) -> bool {
        self.shared(id).is_some()
    }

    pub(super) fn insert(&mut self, id: K, value: impl Into<Arc<T>>) -> Option<Arc<T>> {
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

    pub(super) fn remove(&mut self, id: &K) -> Option<Arc<T>> {
        let (root, value) = self.root.as_ref()?.removing(*id)?;
        self.root = root;
        self.len -= 1;
        Some(value)
    }

    pub(super) fn iter(&self) -> Iter<'_, K, T> {
        self.into_iter()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &T> {
        self.iter().map(|(_, value)| value)
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(id, _)| id)
    }
}

impl<K: BindingKey, T> Index<&K> for Bindings<K, T> {
    type Output = T;
    fn index(&self, id: &K) -> &T {
        self.get(id).expect("missing lexical binding")
    }
}

impl<K: BindingKey, T, V: Into<Arc<T>>> Extend<(K, V)> for Bindings<K, T> {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, values: I) {
        for (id, value) in values {
            self.insert(id, value);
        }
    }
}

#[cfg(test)]
impl<K: BindingKey, T, const N: usize> From<[(K, T); N]> for Bindings<K, T> {
    fn from(values: [(K, T); N]) -> Self {
        let mut bindings = Self::default();
        bindings.extend(values);
        bindings
    }
}

pub(super) struct Iter<'a, K, T> {
    pending: Vec<&'a Node<K, T>>,
}

impl<'a, K: BindingKey, T> IntoIterator for &'a Bindings<K, T> {
    type Item = (&'a K, &'a T);
    type IntoIter = Iter<'a, K, T>;
    fn into_iter(self) -> Self::IntoIter {
        Iter {
            pending: self.root.iter().collect(),
        }
    }
}

impl<'a, K: BindingKey, T> Iterator for Iter<'a, K, T> {
    type Item = (&'a K, &'a T);
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
        fn right_root(bindings: &LocalBindings<Value>) -> &Arc<Fork<LocalId, Value>> {
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
    fn declaration_versions_preserve_high_package_bits_and_local_order() {
        let mut parent = DeclarationBindings::default();
        let mut expected = BTreeMap::new();
        for package in [u64::MAX, 1 << 63, 1 << 32, 1, 0] {
            for local in [u32::MAX, 0] {
                let key = DeclarationId { package, local };
                expected.insert(key, package);
                parent.insert(key, package);
            }
        }
        let mut inner = parent.clone();
        let key = DeclarationId {
            package: 1 << 63,
            local: 0,
        };
        assert_eq!(inner.remove(&key).as_deref(), Some(&(1 << 63)));
        assert_eq!(parent.get(&key), Some(&(1 << 63)));
        expected.remove(&key);
        assert!(inner.iter().eq(expected.iter()));
        for id in expected.keys() {
            assert!(Arc::ptr_eq(
                parent.shared(id).unwrap(),
                inner.shared(id).unwrap()
            ));
        }
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
