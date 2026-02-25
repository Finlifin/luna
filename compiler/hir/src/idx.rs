//! Typed index collections.
//!
//! Provides [`Idx`] (a trait for newtype indices) and [`IndexVec`] (a vector
//! indexed by a typed index instead of `usize`).  This is a minimal,
//! dependency-free version of rustc's `rustc_index::IndexVec`.

use std::fmt;
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

// ── Idx trait ────────────────────────────────────────────────────────────────

/// A newtype index that can be used with [`IndexVec`].
///
/// Implementors must be a thin wrapper around `u32` and provide
/// conversion to/from `usize` for indexing into vectors.
pub trait Idx: Copy + Eq + fmt::Debug {
    fn new(raw: u32) -> Self;
    fn index(self) -> usize;
}

// ── IndexVec ─────────────────────────────────────────────────────────────────

/// A `Vec<T>` indexed by a typed index `I` rather than `usize`.
///
/// This provides compile-time safety: you cannot accidentally index a
/// `IndexVec<LocalDefId, _>` with an `ItemLocalId`, or vice versa.
///
/// ```ignore
/// let mut vec: IndexVec<LocalDefId, String> = IndexVec::new();
/// let id: LocalDefId = vec.push("hello".into());
/// assert_eq!(vec[id], "hello");
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct IndexVec<I: Idx, T> {
    raw: Vec<T>,
    _marker: PhantomData<fn(&I)>,
}

impl<I: Idx, T: fmt::Debug> fmt::Debug for IndexVec<I, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.raw.iter()).finish()
    }
}

impl<I: Idx, T> IndexVec<I, T> {
    /// Create an empty `IndexVec`.
    #[inline]
    pub fn new() -> Self {
        IndexVec {
            raw: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Create an `IndexVec` with the given capacity.
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        IndexVec {
            raw: Vec::with_capacity(cap),
            _marker: PhantomData,
        }
    }

    /// Push a value and return its index.
    #[inline]
    pub fn push(&mut self, val: T) -> I {
        let idx = self.raw.len() as u32;
        self.raw.push(val);
        I::new(idx)
    }

    /// Get a reference to the value at `idx`, or `None` if out of bounds.
    #[inline]
    pub fn get(&self, idx: I) -> Option<&T> {
        self.raw.get(idx.index())
    }

    /// Get a mutable reference to the value at `idx`.
    #[inline]
    pub fn get_mut(&mut self, idx: I) -> Option<&mut T> {
        self.raw.get_mut(idx.index())
    }

    /// Number of elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.raw.len()
    }

    /// Whether the vec is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// The next index that would be returned by [`push`].
    #[inline]
    pub fn next_index(&self) -> I {
        I::new(self.raw.len() as u32)
    }

    /// Iterate over `(index, &value)` pairs.
    pub fn iter_enumerated(&self) -> impl Iterator<Item = (I, &T)> {
        self.raw
            .iter()
            .enumerate()
            .map(|(i, v)| (I::new(i as u32), v))
    }

    /// Iterate over `(index, &mut value)` pairs.
    pub fn iter_enumerated_mut(&mut self) -> impl Iterator<Item = (I, &mut T)> {
        self.raw
            .iter_mut()
            .enumerate()
            .map(|(i, v)| (I::new(i as u32), v))
    }

    /// Iterate over values.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.raw.iter()
    }

    /// Iterate over mutable values.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.raw.iter_mut()
    }

    /// Access the underlying raw `Vec`.
    #[inline]
    pub fn raw(&self) -> &Vec<T> {
        &self.raw
    }

    /// Access the underlying raw `Vec` mutably.
    #[inline]
    pub fn raw_mut(&mut self) -> &mut Vec<T> {
        &mut self.raw
    }

    /// Truncate to `len` elements.
    #[inline]
    pub fn truncate(&mut self, len: usize) {
        self.raw.truncate(len);
    }
}

impl<I: Idx, T> IndexVec<I, Option<T>> {
    /// Ensure the vec is large enough for `idx`, filling with `None`.
    pub fn ensure_contains(&mut self, idx: I) {
        let needed = idx.index() + 1;
        if needed > self.raw.len() {
            self.raw.resize_with(needed, || None);
        }
    }

    /// Insert a value at `idx`, growing the vec if needed.
    pub fn insert(&mut self, idx: I, val: T) {
        self.ensure_contains(idx);
        self.raw[idx.index()] = Some(val);
    }
}

impl<I: Idx, T> Default for IndexVec<I, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: Idx, T> Index<I> for IndexVec<I, T> {
    type Output = T;

    #[inline]
    fn index(&self, idx: I) -> &T {
        &self.raw[idx.index()]
    }
}

impl<I: Idx, T> IndexMut<I> for IndexVec<I, T> {
    #[inline]
    fn index_mut(&mut self, idx: I) -> &mut T {
        &mut self.raw[idx.index()]
    }
}

impl<I: Idx, T> FromIterator<T> for IndexVec<I, T> {
    fn from_iter<It: IntoIterator<Item = T>>(iter: It) -> Self {
        IndexVec {
            raw: iter.into_iter().collect(),
            _marker: PhantomData,
        }
    }
}

impl<I: Idx, T> IntoIterator for IndexVec<I, T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.raw.into_iter()
    }
}

impl<'a, I: Idx, T> IntoIterator for &'a IndexVec<I, T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.raw.iter()
    }
}
