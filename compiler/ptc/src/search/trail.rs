//! Search-owned assignments in chronological order. Proof probes are temporary
//! views and never become search entries or change their decision levels.
//! Propagation explanations live with their assignments, including across
//! clause eviction; continuations retain only boundaries into this trail.
use super::Conflict;
use std::{collections::BTreeSet, fmt};

pub(super) enum Reason {
    Decision,
    Boolean(std::sync::Arc<BTreeSet<usize>>),
    RootClause(Conflict),
    Learned(Conflict),
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
    reason: Reason,
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
    pub(super) fn assign(&mut self, index: usize, value: bool, level: usize, reason: Reason) {
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
            reason,
        });
    }
    pub(super) fn undo(&mut self, index: usize) -> Reason {
        let entry = self.entries.pop().expect("missing search assignment");
        assert_eq!(entry.index, index, "search unwind must follow the trail");
        self.values[index] = None;
        self.notify(index);
        self.levels[index] = 0;
        entry.reason
    }
    pub(super) fn last_literal(&self) -> Option<(usize, bool)> {
        self.entries.last().map(|entry| (entry.index, entry.value))
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
            let origin = match &entry.reason {
                Reason::Decision => "decision",
                Reason::Boolean(_) => "boolean",
                Reason::RootClause(_) => "root-clause",
                Reason::Learned(_) => "learned",
            };
            write!(
                f,
                " (assignment {} {} (level {}) (origin {origin})",
                entry.index, entry.value, entry.level
            )?;
            match &entry.reason {
                Reason::RootClause(proof) | Reason::Learned(proof) => {
                    write!(f, " (reason {proof})")?
                }
                Reason::Boolean(support) => {
                    write!(f, " (antecedents")?;
                    for atom in support.iter() {
                        write!(f, " {atom}")?;
                    }
                    write!(f, ")")?;
                }
                Reason::Decision => {}
            }
            write!(f, ")")?;
        }
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reasons_follow_assignment_lifetime_across_jump_and_unwind() {
        use crate::ProofTree;
        use std::sync::Arc;
        let retained = Arc::new(ProofTree::BooleanConflict);
        let discarded = Arc::new(ProofTree::BooleanConflict);
        let reason = |proof: &Arc<ProofTree>, index| Conflict {
            tree: ProofTree::Lemma {
                premises: Vec::new(),
                proof: proof.clone(),
            },
            support: BTreeSet::from([index]),
        };
        let mut values = [Some(true), None, None, None];
        {
            let mut trail = Trail::new(&mut values);
            trail.assign(1, true, 0, Reason::RootClause(reason(&retained, 1)));
            trail.assign(2, false, 1, Reason::Decision);
            trail.assign(3, true, 1, Reason::Learned(reason(&discarded, 3)));
            assert_eq!(Arc::strong_count(&retained), 2);
            assert_eq!(Arc::strong_count(&discarded), 2);
            assert!(
                trail
                    .to_string()
                    .contains("(origin root-clause) (reason (conflict")
            );
            trail.backjump(0);
            assert_eq!(Arc::strong_count(&discarded), 1);
            assert_eq!(Arc::strong_count(&retained), 2);
            assert_eq!(trail.last_literal(), Some((1, true)));
            let Reason::RootClause(explanation) = trail.undo(1) else {
                panic!("lost root reason")
            };
            assert_eq!(explanation.support, BTreeSet::from([1]));
            assert_eq!(trail.values(), &[Some(true), None, None, None]);
            assert_eq!(Arc::strong_count(&retained), 2);
            drop(explanation);
            assert_eq!(Arc::strong_count(&retained), 1);
            trail.assign(2, true, 0, Reason::Learned(reason(&discarded, 2)));
        }
        assert_eq!(Arc::strong_count(&discarded), 1);
        assert_eq!(values, [Some(true), None, None, None]);
    }

    #[test]
    fn backjump_and_failed_probe_preserve_entry_premises() {
        let mut values = [Some(true), None, None, None];
        {
            let mut trail = Trail::new(&mut values);
            trail.assign(1, false, 0, Reason::Boolean(Default::default()));
            trail.assign(2, true, 1, Reason::Decision);
            trail.assign(3, false, 1, Reason::Learned(Conflict::boolean(&[])));
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
                "(assignment-trail (assignment 1 false (level 0) (origin boolean) (antecedents)))"
            );
        }
        assert_eq!(values, [Some(true), None, None, None]);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut trail = Trail::new(&mut values);
            trail.assign(1, true, 0, Reason::RootClause(Conflict::boolean(&[])));
            trail.assign(2, false, 1, Reason::Decision);
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
        trail.assign(0, true, 0, Reason::Learned(Conflict::boolean(&[])));
        assert!(trail.changes(Consumer::Root).is_none());
        trail.acknowledge_changes(Consumer::Root);
        trail.assign(1, false, 1, Reason::Decision);
        trail.acknowledge_changes(Consumer::Learned);
        trail.assign(2, true, 1, Reason::Boolean(Default::default()));
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
