//! Persistent, height-balanced payloads. Concatenation shares immutable leaves;
//! mutable slice access explicitly materializes a detached snapshot.
use super::Value;
use std::sync::{Arc, OnceLock};
mod owned;
pub use owned::ListIntoIter;

#[derive(Clone)]
pub(super) struct Storage {
    root: Arc<Node>,
    flat: OnceLock<Arc<Vec<Value>>>,
}

enum Node {
    Leaf(Arc<Vec<Value>>),
    Branch {
        left: Arc<Node>,
        right: Arc<Node>,
        len: usize,
        height: usize,
    },
}

#[derive(Clone)]
struct Cursor<'a> {
    pending: Vec<&'a Node>,
    leaf: std::slice::Iter<'a, Value>,
}
impl<'a> Cursor<'a> {
    fn new(root: &'a Node) -> Self {
        match root {
            Node::Leaf(values) => Self {
                pending: Vec::new(),
                leaf: values.iter(),
            },
            Node::Branch { .. } => Self {
                pending: vec![root],
                leaf: [].iter(),
            },
        }
    }
    fn next(&mut self, reverse: bool) -> Option<&'a Value> {
        loop {
            if let Some(value) = if reverse {
                self.leaf.next_back()
            } else {
                self.leaf.next()
            } {
                return Some(value);
            }
            match self.pending.pop()? {
                Node::Leaf(values) => self.leaf = values.iter(),
                Node::Branch { left, right, .. } => {
                    let (first, second) = if reverse {
                        (right, left)
                    } else {
                        (left, right)
                    };
                    self.pending.push(second);
                    self.pending.push(first);
                }
            }
        }
    }
}

/// Borrowed traversal uses two bounded tree cursors and never copies elements.
/// A shared remaining count stops opposite cursors before they cross.
#[derive(Clone)]
pub struct ListIter<'a> {
    front: Cursor<'a>,
    back: Cursor<'a>,
    remaining: usize,
}
impl<'a> Iterator for ListIter<'a> {
    type Item = &'a Value;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let value = self.front.next(false)?;
        self.remaining -= 1;
        Some(value)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl DoubleEndedIterator for ListIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let value = self.back.next(true)?;
        self.remaining -= 1;
        Some(value)
    }
}
impl ExactSizeIterator for ListIter<'_> {}
impl std::iter::FusedIterator for ListIter<'_> {}

impl Node {
    fn len(&self) -> usize {
        match self {
            Self::Leaf(values) => values.len(),
            Self::Branch { len, .. } => *len,
        }
    }
    fn height(&self) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Branch { height, .. } => *height,
        }
    }
    fn children(&self) -> (&Arc<Node>, &Arc<Node>) {
        match self {
            Self::Branch { left, right, .. } => (left, right),
            Self::Leaf(_) => unreachable!("only a tall tree is rotated"),
        }
    }
    fn branch(left: Arc<Self>, right: Arc<Self>, work: &mut usize) -> Arc<Self> {
        *work += 1;
        Arc::new(Self::Branch {
            len: left
                .len()
                .checked_add(right.len())
                .expect("list length overflow"),
            height: 1 + left.height().max(right.height()),
            left,
            right,
        })
    }
    fn balance(left: Arc<Self>, right: Arc<Self>, work: &mut usize) -> Arc<Self> {
        if left.height() > right.height() + 1 {
            let (ll, lr) = left.children();
            if ll.height() >= lr.height() {
                let right = Self::branch(lr.clone(), right, work);
                Self::branch(ll.clone(), right, work)
            } else {
                let (lrl, lrr) = lr.children();
                let left = Self::branch(ll.clone(), lrl.clone(), work);
                let right = Self::branch(lrr.clone(), right, work);
                Self::branch(left, right, work)
            }
        } else if right.height() > left.height() + 1 {
            let (rl, rr) = right.children();
            if rr.height() >= rl.height() {
                let left = Self::branch(left, rl.clone(), work);
                Self::branch(left, rr.clone(), work)
            } else {
                let (rll, rlr) = rl.children();
                let left = Self::branch(left, rll.clone(), work);
                let right = Self::branch(rlr.clone(), rr.clone(), work);
                Self::branch(left, right, work)
            }
        } else {
            Self::branch(left, right, work)
        }
    }
    fn join(left: Arc<Self>, right: Arc<Self>, work: &mut usize) -> Arc<Self> {
        if left.len() == 0 {
            return right;
        }
        if right.len() == 0 {
            return left;
        }
        if left.height() > right.height() + 1 {
            let (ll, lr) = left.children();
            let tail = Self::join(lr.clone(), right, work);
            Self::balance(ll.clone(), tail, work)
        } else if right.height() > left.height() + 1 {
            let (rl, rr) = right.children();
            let head = Self::join(left, rl.clone(), work);
            Self::balance(head, rr.clone(), work)
        } else {
            Self::branch(left, right, work)
        }
    }
    fn first(&self) -> Option<&Value> {
        match self {
            Self::Leaf(values) => values.first(),
            Self::Branch { left, .. } => left.first(),
        }
    }
    fn collect(&self, output: &mut Vec<Value>) {
        match self {
            Self::Leaf(values) => output.extend(values.iter().cloned()),
            Self::Branch { left, right, .. } => {
                left.collect(output);
                right.collect(output);
            }
        }
    }
    fn prefix(root: &Arc<Self>, len: usize) -> Arc<Self> {
        if len >= root.len() {
            return root.clone();
        }
        match root.as_ref() {
            Self::Leaf(values) => Arc::new(Self::Leaf(Arc::new(values[..len].to_vec()))),
            Self::Branch { left, right, .. } => {
                if len <= left.len() {
                    Self::prefix(left, len)
                } else {
                    Self::join(left.clone(), Self::prefix(right, len - left.len()), &mut 0)
                }
            }
        }
    }
}

impl From<Vec<Value>> for Storage {
    fn from(values: Vec<Value>) -> Self {
        Self {
            root: Arc::new(Node::Leaf(Arc::new(values))),
            flat: OnceLock::new(),
        }
    }
}
impl Storage {
    pub fn iter(&self) -> ListIter<'_> {
        ListIter {
            front: Cursor::new(&self.root),
            back: Cursor::new(&self.root),
            remaining: self.len(),
        }
    }
    pub fn len(&self) -> usize {
        self.root.len()
    }
    pub fn first(&self) -> Option<&Value> {
        self.root.first()
    }
    pub fn element(&self, mut index: usize) -> Option<(&Value, usize)> {
        if index >= self.len() {
            return None;
        }
        let mut node = self.root.as_ref();
        let mut work = 1;
        loop {
            match node {
                Node::Leaf(values) => return Some((&values[index], work)),
                Node::Branch { left, right, .. } => {
                    node = if index < left.len() {
                        left
                    } else {
                        index -= left.len();
                        right
                    };
                    work += 1;
                }
            }
        }
    }
    pub fn append(&mut self, other: Self) -> usize {
        // Preserve the contiguous fast path when no snapshot shares either
        // leaf. Shared prefixes use persistent joins instead of copy-on-write.
        if let Some(Node::Leaf(left)) = Arc::get_mut(&mut self.root) {
            if let Some(left) = Arc::get_mut(left) {
                if let Node::Leaf(right) = other.root.as_ref() {
                    if Arc::strong_count(&other.root) == 1 && Arc::strong_count(right) == 1 {
                        let moved = right.len()
                            + if right.len() > left.capacity() - left.len() {
                                left.len()
                            } else {
                                0
                            };
                        left.extend(other.into_vec());
                        self.flat.take();
                        return moved;
                    }
                }
            }
        }
        let mut work = 0;
        self.root = Node::join(self.root.clone(), other.root, &mut work);
        self.flat.take();
        work
    }
    pub fn truncate(&mut self, len: usize) {
        if let Some(Node::Leaf(values)) = Arc::get_mut(&mut self.root) {
            if let Some(values) = Arc::get_mut(values) {
                values.truncate(len);
                self.flat.take();
                return;
            }
        }
        self.root = Node::prefix(&self.root, len);
        self.flat.take();
    }
    pub fn flat(&self) -> &Vec<Value> {
        if let Node::Leaf(values) = self.root.as_ref() {
            return values;
        }
        self.flat.get_or_init(|| {
            let mut values = Vec::with_capacity(self.len());
            self.root.collect(&mut values);
            Arc::new(values)
        })
    }
    pub fn flat_mut(&mut self) -> &mut Vec<Value> {
        // Moving a unique leaf needs no copy. A shared leaf or a branch must
        // detach before mutation, including when a flattened view was cached.
        if Arc::get_mut(&mut self.root).is_none() || !matches!(&*self.root, Node::Leaf(_)) {
            let values = match self.root.as_ref() {
                Node::Leaf(values) => values.clone(),
                Node::Branch { .. } => {
                    self.flat();
                    self.flat.take().unwrap()
                }
            };
            self.root = Arc::new(Node::Leaf(values));
        }
        self.flat.take();
        match Arc::get_mut(&mut self.root).unwrap() {
            Node::Leaf(values) => Arc::make_mut(values),
            Node::Branch { .. } => unreachable!(),
        }
    }
    pub fn into_vec(self) -> Vec<Value> {
        if let Some(values) = self.flat.into_inner() {
            return Arc::unwrap_or_clone(values);
        }
        match Arc::try_unwrap(self.root) {
            Ok(Node::Leaf(values)) => Arc::unwrap_or_clone(values),
            Ok(node) => ListIntoIter::new(Arc::new(node)).collect(),
            Err(node) => ListIntoIter::new(node).collect(),
        }
    }
    pub fn into_iter(self) -> ListIntoIter {
        // A borrowed flat view is only a cache. Release it so it cannot keep
        // otherwise unique element allocations shared during consumption.
        let Self { root, flat } = self;
        drop(flat);
        ListIntoIter::new(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(node: &Node) -> (usize, usize) {
        match node {
            Node::Leaf(values) => (values.len(), 1),
            Node::Branch {
                left,
                right,
                len,
                height,
            } => {
                let (ln, lh) = check(left);
                let (rn, rh) = check(right);
                assert!(lh.abs_diff(rh) <= 1);
                assert_eq!(*len, ln + rn);
                assert_eq!(*height, 1 + lh.max(rh));
                (*len, *height)
            }
        }
    }

    #[test]
    fn repeated_concatenation_shares_snapshots_and_balances_both_sides() {
        for prepend in [false, true] {
            let mut list = Storage::from(vec![]);
            let mut snapshots = Vec::new();
            let mut work = 0;
            for i in 0..4096 {
                if i % 127 == 0 {
                    snapshots.push((i, list.clone()));
                }
                let singleton = Storage::from(vec![Value::UnsignedInteger(i)]);
                if prepend {
                    let mut next = singleton;
                    work += next.append(list);
                    list = next;
                } else {
                    work += list.append(singleton);
                }
                check(&list.root);
            }
            assert!(work < 4096 * 32, "path allocation work: {work}");
            assert!(list.flat.get().is_none());
            for (len, snapshot) in snapshots {
                assert_eq!(snapshot.len(), len as usize);
                let expected: Vec<_> = if prepend {
                    (0..len).rev().collect()
                } else {
                    (0..len).collect()
                };
                assert_eq!(
                    snapshot.flat(),
                    &expected
                        .into_iter()
                        .map(Value::UnsignedInteger)
                        .collect::<Vec<_>>()
                );
            }
            let original = list.clone();
            list.flat_mut()[0] = Value::Bool(false);
            assert!(!matches!(original.first(), Some(Value::Bool(_))));
            list.truncate(123);
            assert_eq!(list.len(), 123);
            assert_eq!(original.len(), 4096);
        }
    }

    #[test]
    fn arbitrary_joins_and_prefixes_preserve_order() {
        let mut chunks: Vec<_> = (0..200)
            .map(|i| Storage::from(vec![Value::UnsignedInteger(i)]))
            .collect();
        let mut seed = 17usize;
        while chunks.len() > 1 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let i = seed % (chunks.len() - 1);
            let right = chunks.remove(i + 1);
            chunks[i].append(right);
            check(&chunks[i].root);
        }
        let original = chunks.pop().unwrap();
        for n in 0..=200 {
            let mut prefix = original.clone();
            prefix.truncate(n);
            check(&prefix.root);
            assert_eq!(
                prefix.flat(),
                &(0..n as u128)
                    .map(Value::UnsignedInteger)
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(original.len(), 200);
    }

    #[test]
    fn element_reads_visit_only_a_balanced_path_without_flattening() {
        let mut list = Storage::from(vec![]);
        for i in 0..4096 {
            // Retain each prefix during concatenation, as ordinary calls do.
            let snapshot = list.clone();
            list.append(Storage::from(vec![Value::UnsignedInteger(i)]));
            assert_eq!(snapshot.len(), i as usize);
        }
        for i in 0..4096 {
            let (value, work) = list.element(i).unwrap();
            assert_eq!(*value, Value::UnsignedInteger(i as u128));
            assert!(work <= 16, "index {i}: {work}");
        }
        assert!(list.element(4096).is_none());
        assert!(list.element(usize::MAX).is_none());
        assert!(list.flat.get().is_none());
    }

    #[test]
    fn borrowed_iteration_preserves_order_and_mixed_end_exhaustion() {
        for len in [0, 1, 2, 19, 1024] {
            let expected: Vec<_> = (0..len).map(Value::UnsignedInteger).collect();
            for chunked in [false, true] {
                let mut list = Storage::from(if chunked { vec![] } else { expected.clone() });
                if chunked {
                    for chunk in expected.chunks(3) {
                        let snapshot = list.clone();
                        list.append(Storage::from(chunk.to_vec()));
                        assert!(snapshot.len() < list.len());
                    }
                }
                assert!(list.iter().eq(expected.iter()));
                assert!(list.iter().rev().eq(expected.iter().rev()));
                for seed in [1usize, 19, 271] {
                    let mut seed = seed;
                    let mut iter = list.iter();
                    let mut reference = expected.iter();
                    while reference.len() > 0 {
                        assert_eq!(iter.size_hint(), reference.size_hint());
                        assert!(iter.clone().eq(reference.clone()));
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        let (actual, expected) = if seed & 4 == 0 {
                            (iter.next(), reference.next())
                        } else {
                            (iter.next_back(), reference.next_back())
                        };
                        assert_eq!(actual, expected);
                        assert!(iter.front.pending.len() <= list.root.height());
                        assert!(iter.back.pending.len() <= list.root.height());
                    }
                    assert_eq!(iter.len(), 0);
                    for _ in 0..3 {
                        assert_eq!(iter.next(), None);
                        assert_eq!(iter.next_back(), None);
                    }
                }
                assert!(list.flat.get().is_none());
            }
        }
    }

    #[test]
    fn list_observation_equality_and_debug_do_not_materialize_payloads() {
        use super::super::ListValues;
        let mut values = ListValues::default();
        for i in 0..256 {
            let snapshot = values.clone();
            values.append_payload(vec![Value::UnsignedInteger(i)].into());
            assert_eq!(snapshot.len(), i as usize);
        }
        let expected: Vec<_> = (0..256).map(Value::UnsignedInteger).collect();
        assert_eq!(values, expected);
        assert_eq!(format!("{values:?}"), format!("{expected:?}"));
        let value = Value::List {
            ty: crate::Term::Primitive(crate::Primitive::Bool),
            values,
        };
        // Structural inspection does not establish the intentionally mismatched
        // element domain, but still visits every machine-integer payload.
        assert!(value.immutable_copy_work().is_some());
        let Value::List { values, .. } = value else {
            unreachable!()
        };
        assert!(values.values.flat.get().is_none());
    }
}
