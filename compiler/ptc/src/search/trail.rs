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
    changes: BTreeSet<usize>,
    synchronized: bool,
}
impl<'a> Trail<'a> {
    pub(super) fn new(values: &'a mut [Option<bool>]) -> Self {
        Self {
            levels: vec![0; values.len()],
            values,
            entries: Vec::new(),
            changes: BTreeSet::new(),
            synchronized: false,
        }
    }
    // None starts a new consumer epoch; otherwise only these atoms changed.
    pub(super) fn changes(&self) -> Option<&BTreeSet<usize>> {
        self.synchronized.then_some(&self.changes)
    }
    pub(super) fn acknowledge_changes(&mut self) {
        self.changes.clear();
        self.synchronized = true;
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
        self.changes.insert(index);
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
        self.changes.insert(index);
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
            assert!(trail.changes().is_none());
            trail.acknowledge_changes();
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                trail.probe(2, false, |_| panic!("interrupted proof probe"));
            }));
            assert!(panic.is_err());
            assert_eq!(trail.to_string(), before);
            assert!(trail.changes().unwrap().is_empty());
            assert_eq!(
                trail.values(),
                &[Some(true), Some(false), Some(true), Some(false)]
            );
            trail.backjump(0);
            assert_eq!(trail.changes(), Some(&BTreeSet::from([2, 3])));
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
}
