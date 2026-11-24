//! Deterministic BTree-backed associative maps and sets for keys that implement `Hash + Eq`
//! without `Ord`, eliminating `RandomState` hash iteration non-determinism.
use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;

const FNV_PRIME: u64 = 0x100000001b3;

struct Fnv1aHasher(u64);

impl Default for Fnv1aHasher {
    fn default() -> Self {
        Self(FNV_OFFSET_BASIS)
    }
}

impl Hasher for Fnv1aHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }
}

pub(crate) fn fnv1a_hash<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = Fnv1aHasher::default();
    value.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone)]
pub(crate) struct DeterministicMap<K, V> {
    buckets: BTreeMap<u64, Vec<(K, V)>>,
    len: usize,
}

impl<K, V> Default for DeterministicMap<K, V> {
    fn default() -> Self {
        Self {
            buckets: BTreeMap::new(),
            len: 0,
        }
    }
}

#[allow(dead_code)]
impl<K, V> DeterministicMap<K, V> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn clear(&mut self) {
        self.buckets.clear();
        self.len = 0;
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.buckets
            .values()
            .flat_map(|bucket| bucket.iter().map(|(k, v)| (k, v)))
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(k, _)| k)
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }
}

#[allow(dead_code)]
impl<K: Hash + Eq, V> DeterministicMap<K, V> {
    pub(crate) fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.get(key).is_some()
    }

    pub(crate) fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let hash = fnv1a_hash(key);
        self.buckets
            .get(&hash)?
            .iter()
            .find(|(k, _)| k.borrow() == key)
            .map(|(_, v)| v)
    }

    pub(crate) fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let hash = fnv1a_hash(key);
        self.buckets
            .get_mut(&hash)?
            .iter_mut()
            .find(|(k, _)| k.borrow() == key)
            .map(|(_, v)| v)
    }

    pub(crate) fn get_key_value<Q>(&self, key: &Q) -> Option<(&K, &V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let hash = fnv1a_hash(key);
        self.buckets
            .get(&hash)?
            .iter()
            .find(|(k, _)| k.borrow() == key)
            .map(|(k, v)| (k, v))
    }

    pub(crate) fn insert(&mut self, key: K, value: V) -> Option<V> {
        let hash = fnv1a_hash(&key);
        let bucket = self.buckets.entry(hash).or_default();
        if let Some((_, existing)) = bucket.iter_mut().find(|(k, _)| *k == key) {
            return Some(std::mem::replace(existing, value));
        }
        self.len += 1;
        bucket.push((key, value));
        None
    }

    pub(crate) fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let hash = fnv1a_hash(key);
        let bucket = self.buckets.get_mut(&hash)?;
        let index = bucket.iter().position(|(k, _)| k.borrow() == key)?;
        let (_, value) = bucket.remove(index);
        if bucket.is_empty() {
            self.buckets.remove(&hash);
        }
        self.len -= 1;
        Some(value)
    }

    pub(crate) fn entry(&mut self, key: K) -> Entry<'_, K, V> {
        let hash = fnv1a_hash(&key);
        let bucket = self.buckets.entry(hash).or_default();
        if let Some(index) = bucket.iter().position(|(k, _)| *k == key) {
            Entry::Occupied { bucket, index }
        } else {
            Entry::Vacant {
                bucket,
                key,
                len: &mut self.len,
            }
        }
    }
}

pub(crate) enum Entry<'a, K, V> {
    Occupied {
        bucket: &'a mut Vec<(K, V)>,
        index: usize,
    },
    Vacant {
        bucket: &'a mut Vec<(K, V)>,
        key: K,
        len: &'a mut usize,
    },
}

impl<'a, K, V> Entry<'a, K, V> {
    pub(crate) fn or_insert_with(self, default: impl FnOnce() -> V) -> &'a mut V {
        match self {
            Self::Occupied { bucket, index } => &mut bucket[index].1,
            Self::Vacant { bucket, key, len } => {
                *len += 1;
                let index = bucket.len();
                bucket.push((key, default()));
                &mut bucket[index].1
            }
        }
    }

    pub(crate) fn or_insert(self, default: V) -> &'a mut V {
        self.or_insert_with(|| default)
    }

    pub(crate) fn or_default(self) -> &'a mut V
    where
        V: Default,
    {
        self.or_insert_with(V::default)
    }
}

impl<K: Hash + Eq, V: PartialEq> PartialEq for DeterministicMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}

impl<K: Hash + Eq, V: Eq> Eq for DeterministicMap<K, V> {}

impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for DeterministicMap<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K: Hash + Eq, V, const N: usize> From<[(K, V); N]> for DeterministicMap<K, V> {
    fn from(items: [(K, V); N]) -> Self {
        let mut map = Self::new();
        for (k, v) in items {
            map.insert(k, v);
        }
        map
    }
}

impl<K: Hash + Eq, V> FromIterator<(K, V)> for DeterministicMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut map = Self::new();
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

impl<K, V> IntoIterator for DeterministicMap<K, V> {
    type Item = (K, V);

    type IntoIter = std::iter::Flatten<std::collections::btree_map::IntoValues<u64, Vec<(K, V)>>>;

    fn into_iter(self) -> Self::IntoIter {
        self.buckets.into_values().flatten()
    }
}

#[derive(Clone)]
pub(crate) struct DeterministicSet<T> {
    inner: DeterministicMap<T, ()>,
}

impl<T> Default for DeterministicSet<T> {
    fn default() -> Self {
        Self {
            inner: DeterministicMap::default(),
        }
    }
}

#[allow(dead_code)]
impl<T> DeterministicSet<T> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn len(&self) -> usize {
        self.inner.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.inner.clear();
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        self.inner.keys()
    }
}

impl<T: Hash + Eq> DeterministicSet<T> {
    pub(crate) fn contains<Q>(&self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.inner.contains_key(value)
    }

    pub(crate) fn insert(&mut self, value: T) -> bool {
        self.inner.insert(value, ()).is_none()
    }

    pub(crate) fn remove<Q>(&mut self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.inner.remove(value).is_some()
    }
}

impl<T: Hash + Eq> PartialEq for DeterministicSet<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<T: Hash + Eq> Eq for DeterministicSet<T> {}

impl<T: fmt::Debug> fmt::Debug for DeterministicSet<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl<T: Hash + Eq> FromIterator<T> for DeterministicSet<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut set = Self::new();
        for item in iter {
            set.insert(item);
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{Hash, Hasher};

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CollidingKey {
        id: u32,
        fixed_hash: u64,
    }

    impl Hash for CollidingKey {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.fixed_hash.hash(state);
        }
    }

    #[test]
    fn deterministic_map_preserves_order_and_handles_hash_collisions() {
        let mut first = DeterministicMap::new();
        let mut second = DeterministicMap::new();
        let pairs = [
            (
                CollidingKey {
                    id: 1,
                    fixed_hash: 0xfeed_beef,
                },
                "one",
            ),
            (
                CollidingKey {
                    id: 2,
                    fixed_hash: 0xfeed_beef,
                },
                "two",
            ),
            (
                CollidingKey {
                    id: 3,
                    fixed_hash: 42,
                },
                "three",
            ),
        ];
        for (key, value) in &pairs {
            first.insert(key.clone(), *value);
            second.insert(key.clone(), *value);
        }
        assert_eq!(first.len(), 3);
        assert_eq!(first, second);
        let first_order: Vec<_> = first.iter().map(|(k, v)| (k.id, *v)).collect();
        let second_order: Vec<_> = second.iter().map(|(k, v)| (k.id, *v)).collect();
        assert_eq!(first_order, second_order);
        assert_eq!(
            first.get(&CollidingKey {
                id: 2,
                fixed_hash: 0xfeed_beef
            }),
            Some(&"two")
        );
        assert_eq!(
            first.remove(&CollidingKey {
                id: 1,
                fixed_hash: 0xfeed_beef
            }),
            Some("one")
        );
        assert_eq!(first.len(), 2);
    }

    #[test]
    fn deterministic_set_matches_across_instances() {
        let a: DeterministicSet<u64> = [9, 2, 7, 2, 5].into_iter().collect();
        let b: DeterministicSet<u64> = [9, 2, 7, 5].into_iter().collect();
        assert_eq!(a, b);
        assert_eq!(
            a.iter().copied().collect::<Vec<_>>(),
            b.iter().copied().collect::<Vec<_>>()
        );
    }
}
