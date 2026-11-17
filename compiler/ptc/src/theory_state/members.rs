//! Query-local class membership, reversible with the same union checkpoint.
//! Observers retain catalog order rather than depending on union history.
use super::{Classes, UnknownReason, charge};

impl Classes {
    pub fn ordered_members(
        &self,
        id: usize,
        fuel: &mut usize,
    ) -> Result<Vec<usize>, UnknownReason> {
        let mut root = id;
        loop {
            charge(fuel, 1)?;
            if self.parent[root] == root {
                break;
            }
            root = self.parent[root];
        }
        let count = self.size[root];
        // Reserve enumeration and sorting before allocating. Copy cost and
        // comparison work depend on this class, never on the whole catalog.
        let bits = (usize::BITS - count.leading_zeros()) as usize;
        charge(fuel, count.saturating_mul(bits.saturating_add(1)))?;
        let mut members = Vec::with_capacity(count);
        let mut current = id;
        for _ in 0..count {
            members.push(current);
            current = self.next[current];
        }
        debug_assert_eq!(current, id);
        members.sort_unstable();
        Ok(members)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verify(classes: &Classes) {
        for id in 0..classes.parent.len() {
            let expected: Vec<_> = (0..classes.parent.len())
                .filter(|&other| classes.root(id) == classes.root(other))
                .collect();
            assert_eq!(classes.ordered_members(id, &mut 10000).unwrap(), expected);
        }
    }

    #[test]
    fn indexed_members_restore_each_branch_and_snapshot() {
        for seed in 0..128 {
            let mut classes = Classes::new(12);
            let mut checkpoints = Vec::new();
            let mut snapshots = Vec::new();
            for step in 0..16 {
                checkpoints.push(classes.checkpoint());
                snapshots.push(classes.snapshot());
                classes.merge((seed + step * 7) % 12, (seed * 5 + step * 3) % 12);
                verify(&classes);
            }
            for (checkpoint, snapshot) in checkpoints.into_iter().zip(snapshots).rev() {
                verify(&snapshot);
                classes.rollback(checkpoint);
                verify(&classes);
                for id in 0..12 {
                    assert_eq!(
                        classes.ordered_members(id, &mut 10000).unwrap(),
                        snapshot.ordered_members(id, &mut 10000).unwrap()
                    );
                }
            }
            assert_eq!(classes.next, (0..12).collect::<Vec<_>>());
        }
    }

    #[test]
    fn interrupted_observation_preserves_members_and_union_history() {
        let mut classes = Classes::new(16);
        for id in 1..10 {
            classes.merge(id, id - 1);
        }
        let before = classes.to_string();
        for limit in 0..64 {
            let result = classes.ordered_members(7, &mut { limit });
            if let Ok(members) = result {
                assert_eq!(members, (0..10).collect::<Vec<_>>());
            }
            assert_eq!(classes.to_string(), before);
        }
        assert_eq!(
            classes.ordered_members(7, &mut 1000).unwrap(),
            (0..10).collect::<Vec<_>>()
        );
        classes.rollback(0);
        verify(&classes);
    }
}
