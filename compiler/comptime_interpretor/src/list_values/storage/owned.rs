//! Consume persistent payloads without materializing a second full element array.
use super::{Node, Value};
use std::{collections::VecDeque, sync::Arc};

enum Part {
    Tree(Arc<Node>),
    Owned(std::vec::IntoIter<Value>),
    Shared {
        values: Arc<Vec<Value>>,
        front: usize,
        back: usize,
    },
}

/// Owned traversal moves unique leaves and clones shared values only on demand.
/// The deque holds the unconsumed frontier, in source order, for both ends.
pub struct ListIntoIter {
    pending: VecDeque<Part>,
    remaining: usize,
}
impl ListIntoIter {
    pub(super) fn new(root: Arc<Node>) -> Self {
        Self {
            remaining: root.len(),
            pending: VecDeque::from([Part::Tree(root)]),
        }
    }
    fn put(&mut self, part: Part, reverse: bool) {
        if reverse {
            self.pending.push_back(part);
        } else {
            self.pending.push_front(part);
        }
    }
    fn leaf(&mut self, values: Arc<Vec<Value>>, reverse: bool) {
        let part = match Arc::try_unwrap(values) {
            Ok(values) => Part::Owned(values.into_iter()),
            Err(values) => Part::Shared {
                front: 0,
                back: values.len(),
                values,
            },
        };
        self.put(part, reverse);
    }
    fn branch(&mut self, left: Arc<Node>, right: Arc<Node>, reverse: bool) {
        let (first, second) = if reverse {
            (right, left)
        } else {
            (left, right)
        };
        self.put(Part::Tree(second), reverse);
        self.put(Part::Tree(first), reverse);
    }
    fn take(&mut self, reverse: bool) -> Option<Value> {
        if self.remaining == 0 {
            return None;
        }
        loop {
            let part = if reverse {
                self.pending.pop_back()
            } else {
                self.pending.pop_front()
            }?;
            let value = match part {
                Part::Tree(node) => {
                    match Arc::try_unwrap(node) {
                        Ok(Node::Leaf(values)) => self.leaf(values, reverse),
                        Ok(Node::Branch { left, right, .. }) => self.branch(left, right, reverse),
                        Err(node) => match node.as_ref() {
                            Node::Leaf(values) => self.leaf(values.clone(), reverse),
                            Node::Branch { left, right, .. } => {
                                self.branch(left.clone(), right.clone(), reverse)
                            }
                        },
                    }
                    continue;
                }
                Part::Owned(mut values) => {
                    let value = if reverse {
                        values.next_back()
                    } else {
                        values.next()
                    };
                    if values.len() != 0 {
                        self.put(Part::Owned(values), reverse);
                    }
                    value
                }
                Part::Shared {
                    values,
                    mut front,
                    mut back,
                } => {
                    if front == back {
                        continue;
                    }
                    let index = if reverse {
                        back -= 1;
                        back
                    } else {
                        let index = front;
                        front += 1;
                        index
                    };
                    let value = values[index].clone();
                    if front != back {
                        self.put(
                            Part::Shared {
                                values,
                                front,
                                back,
                            },
                            reverse,
                        );
                    }
                    Some(value)
                }
            };
            if let Some(value) = value {
                self.remaining -= 1;
                return Some(value);
            }
        }
    }
}
impl Iterator for ListIntoIter {
    type Item = Value;
    fn next(&mut self) -> Option<Value> {
        self.take(false)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl DoubleEndedIterator for ListIntoIter {
    fn next_back(&mut self) -> Option<Value> {
        self.take(true)
    }
}
impl ExactSizeIterator for ListIntoIter {}
impl std::iter::FusedIterator for ListIntoIter {}

#[cfg(test)]
mod tests {
    use super::super::Storage;
    use super::*;

    fn tree(len: usize) -> Storage {
        let mut list = Storage::from(vec![]);
        for index in 0..len {
            let snapshot = list.clone();
            list.append(Storage::from(vec![Value::UnsignedInteger(index as u128)]));
            drop(snapshot);
        }
        list
    }

    #[test]
    fn owned_traversal_preserves_order_and_mixed_end_exhaustion() {
        for len in [0, 1, 2, 19, 1024] {
            for chunked in [false, true] {
                for shared in [false, true] {
                    let list = if chunked {
                        tree(len)
                    } else {
                        Storage::from(
                            (0..len)
                                .map(|index| Value::UnsignedInteger(index as u128))
                                .collect::<Vec<_>>(),
                        )
                    };
                    let height = list.root.height();
                    let snapshot = shared.then(|| list.clone());
                    let mut iter = list.into_iter();
                    let mut front = 0;
                    let mut back = len;
                    let mut seed = 19usize;
                    while front < back {
                        assert_eq!(iter.size_hint(), (back - front, Some(back - front)));
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        let (value, index) = if seed & 4 == 0 {
                            back -= 1;
                            (iter.next_back(), back)
                        } else {
                            let index = front;
                            front += 1;
                            (iter.next(), index)
                        };
                        assert_eq!(value, Some(Value::UnsignedInteger(index as u128)));
                        assert!(iter.pending.len() <= 2 * height + 1);
                    }
                    assert_eq!(iter.len(), 0);
                    assert!(iter.next().is_none());
                    assert!(iter.next_back().is_none());
                    if let Some(snapshot) = snapshot {
                        assert_eq!(snapshot.len(), len);
                        assert!(snapshot.flat.get().is_none());
                        assert!(
                            snapshot
                                .iter()
                                .cloned()
                                .eq((0..len).map(|index| Value::UnsignedInteger(index as u128)))
                        );
                    }
                }
            }
            assert!(
                tree(len).into_iter().rev().eq((0..len)
                    .rev()
                    .map(|index| Value::UnsignedInteger(index as u128)))
            );
        }
    }

    #[test]
    fn consuming_unique_branches_moves_enclosed_allocations() {
        let mut list = Storage::from(vec![]);
        let mut allocations = Vec::new();
        for _ in 0..128 {
            let fields = vec![Value::Bool(true)];
            allocations.push(fields.as_ptr());
            let snapshot = list.clone();
            list.append(Storage::from(vec![Value::Tuple(fields)]));
            drop(snapshot);
        }
        assert!(matches!(list.root.as_ref(), Node::Branch { .. }));
        for (value, allocation) in list.into_iter().zip(allocations) {
            let Value::Tuple(fields) = value else {
                panic!()
            };
            assert_eq!(fields.as_ptr(), allocation);
        }
    }

    #[test]
    fn full_collection_moves_unique_leaves_and_shared_snapshots_stay_independent() {
        let fields = vec![Value::Bool(true)];
        let allocation = fields.as_ptr();
        let mut list = tree(10);
        list.append(Storage::from(vec![Value::Tuple(fields)]));
        let values = list.into_vec();
        let Value::Tuple(fields) = values.last().unwrap() else {
            panic!()
        };
        assert_eq!(fields.as_ptr(), allocation);

        let mut list = Storage::from(values);
        let snapshot = list.clone();
        let mut iter = snapshot.into_iter();
        let Value::Tuple(mut fields) = iter.next_back().unwrap() else {
            panic!()
        };
        fields[0] = Value::Bool(false);
        let Value::Tuple(original) = list.element(10).unwrap().0 else {
            panic!()
        };
        assert_eq!(original[0], Value::Bool(true));
        list.flat_mut()[0] = Value::Bool(false);
        assert_eq!(iter.next(), Some(Value::UnsignedInteger(0)));
    }

    #[test]
    fn cached_flat_views_do_not_change_owned_iteration() {
        let list = tree(128);
        list.flat();
        let snapshot = list.clone();
        assert!(list.into_iter().eq((0..128).map(Value::UnsignedInteger)));
        assert!(
            snapshot
                .into_vec()
                .into_iter()
                .eq((0..128).map(Value::UnsignedInteger))
        );
    }
}
