//! Search-owned assignments in chronological order. Proof probes are temporary
//! views and never become search entries or change their decision levels.
use std::{collections::BTreeSet, fmt};

#[derive(Clone, Copy)]
pub(super) enum Origin {
    Decision,
    Boolean,
    RootClause,
    Learned,
}
#[derive(Clone, Copy)]
pub(super) enum Consumer {
    Learned,
    Root,
    Theory,
}
#[derive(Default)]
struct Pending {
    atoms: BTreeSet<usize>,
    synchronized: bool,
}
struct Entry {
    index: usize,
    value: bool,
    level: usize,
    origin: Origin,
}
pub(super) struct Trail<'a> {
    values: &'a mut [Option<bool>],
    levels: Vec<usize>,
    entries: Vec<Entry>,
    pending: [Pending; 3],
}
impl<'a> Trail<'a> {
    pub(super) fn new(values: &'a mut [Option<bool>]) -> Self {
        Self {
            levels: vec![0; values.len()],
            values,
            entries: Vec::new(),
            pending: std::array::from_fn(|_| Pending::default()),
        }
    }
    // Consumers run at different times and acknowledge their own event set.
    pub(super) fn changes(&self, consumer: Consumer) -> Option<&BTreeSet<usize>> {
        let pending = &self.pending[consumer as usize];
        pending.synchronized.then_some(&pending.atoms)
    }
    pub(super) fn acknowledge_changes(&mut self, consumer: Consumer) {
        let pending = &mut self.pending[consumer as usize];
        pending.atoms.clear();
        pending.synchronized = true;
    }
    fn notify(&mut self, index: usize) {
        for pending in &mut self.pending {
            // The first call receives a complete snapshot, so it needs no IDs.
            if pending.synchronized {
                pending.atoms.insert(index);
            }
        }
    }
    pub(super) fn values(&self) -> &[Option<bool>] {
        self.values
    }
    pub(super) fn levels(&self) -> &[usize] {
        &self.levels
    }
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(super) fn assign(&mut self, index: usize, value: bool, level: usize, origin: Origin) {
        assert!(
            self.values[index].is_none(),
            "search assignment overwrites a live premise"
        );
        self.values[index] = Some(value);
        self.notify(index);
        self.levels[index] = level;
        self.entries.push(Entry {
            index,
            value,
            level,
            origin,
        });
    }
    pub(super) fn undo(&mut self, index: usize) {
        let entry = self.entries.pop().expect("missing search assignment");
        assert_eq!(entry.index, index, "search unwind must follow the trail");
        self.values[index] = None;
        self.notify(index);
        self.levels[index] = 0;
    }
    pub(super) fn backjump(&mut self, target: usize) {
        while self
            .entries
            .last()
            .is_some_and(|entry| entry.level > target)
        {
            self.undo(self.entries.last().unwrap().index);
        }
    }
    pub(super) fn probe<T>(
        &mut self,
        index: usize,
        value: bool,
        operation: impl FnOnce(&[Option<bool>]) -> T,
    ) -> T {
        struct Restore<'a> {
            values: &'a mut [Option<bool>],
            index: usize,
            previous: Option<bool>,
        }
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.values[self.index] = self.previous;
            }
        }
        let previous = self.values[index];
        self.values[index] = Some(value);
        let restore = Restore {
            values: self.values,
            index,
            previous,
        };
        operation(restore.values)
    }
}
impl Drop for Trail<'_> {
    fn drop(&mut self) {
        // A subsequent search starts a new consumer epoch; cleanup need not
        // allocate notifications that no consumer can observe.
        while let Some(entry) = self.entries.pop() {
            self.values[entry.index] = None;
        }
    }
}
impl fmt::Display for Trail<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(assignment-trail")?;
        for entry in &self.entries {
            let origin = match entry.origin {
                Origin::Decision => "decision",
                Origin::Boolean => "boolean",
                Origin::RootClause => "root-clause",
                Origin::Learned => "learned",
            };
            write!(
                f,
                " (assignment {} {} (level {}) (origin {origin}))",
                entry.index, entry.value, entry.level
            )?;
        }
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backjump_and_failed_probe_preserve_entry_premises() {
        let mut values = [Some(true), None, None, None];
        {
            let mut trail = Trail::new(&mut values);
            trail.assign(1, false, 0, Origin::Boolean);
            trail.assign(2, true, 1, Origin::Decision);
            trail.assign(3, false, 1, Origin::Learned);
            let before = trail.to_string();
            assert!(trail.changes(Consumer::Learned).is_none());
            trail.acknowledge_changes(Consumer::Learned);
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                trail.probe(2, false, |_| panic!("interrupted proof probe"));
            }));
            assert!(panic.is_err());
            assert_eq!(trail.to_string(), before);
            assert!(trail.changes(Consumer::Learned).unwrap().is_empty());
            assert_eq!(
                trail.values(),
                &[Some(true), Some(false), Some(true), Some(false)]
            );
            trail.backjump(0);
            assert_eq!(
                trail.changes(Consumer::Learned),
                Some(&BTreeSet::from([2, 3]))
            );
            assert_eq!(trail.values(), &[Some(true), Some(false), None, None]);
            assert_eq!(trail.levels(), &[0, 0, 0, 0]);
            assert_eq!(
                trail.to_string(),
                "(assignment-trail (assignment 1 false (level 0) (origin boolean)))"
            );
        }
        assert_eq!(values, [Some(true), None, None, None]);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut trail = Trail::new(&mut values);
            trail.assign(1, true, 0, Origin::RootClause);
            trail.assign(2, false, 1, Origin::Decision);
            panic!("interrupted search");
        }));
        assert!(panic.is_err());
        assert_eq!(values, [Some(true), None, None, None]);
    }

    #[test]
    fn consumers_acknowledge_independently_when_root_propagation_is_delayed() {
        let mut values = [None; 3];
        let mut trail = Trail::new(&mut values);
        trail.acknowledge_changes(Consumer::Learned);
        trail.acknowledge_changes(Consumer::Theory);
        trail.assign(0, true, 0, Origin::Learned);
        assert!(trail.changes(Consumer::Root).is_none());
        trail.acknowledge_changes(Consumer::Root);
        trail.assign(1, false, 1, Origin::Decision);
        trail.acknowledge_changes(Consumer::Learned);
        trail.assign(2, true, 1, Origin::Boolean);
        assert_eq!(trail.changes(Consumer::Learned), Some(&BTreeSet::from([2])));
        assert_eq!(trail.changes(Consumer::Root), Some(&BTreeSet::from([1, 2])));
        trail.acknowledge_changes(Consumer::Root);
        assert_eq!(
            trail.changes(Consumer::Theory),
            Some(&BTreeSet::from([0, 1, 2]))
        );
        trail.acknowledge_changes(Consumer::Theory);
        trail.backjump(0);
        assert_eq!(
            trail.changes(Consumer::Theory),
            Some(&BTreeSet::from([1, 2]))
        );
        assert_eq!(
            trail.changes(Consumer::Learned),
            Some(&BTreeSet::from([1, 2]))
        );
        assert_eq!(trail.changes(Consumer::Root), Some(&BTreeSet::from([1, 2])));
    }
}
