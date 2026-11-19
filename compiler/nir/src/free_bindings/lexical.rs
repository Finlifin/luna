//! One lexical path per analysis. Undo only successful changes to membership
//! when returning to a sibling. Errors discard the entire private walk.
use super::{BTreeMap, BTreeSet, Error, LocalId, Scope, charge};

#[derive(Default)]
pub(super) struct LexicalScope {
    visible: Scope,
    edits: Vec<Edit>,
}

#[derive(Clone, Copy)]
pub(super) struct Edit {
    binding: LocalId,
    present: bool,
}

impl LexicalScope {
    pub(super) fn with_parameters(parameters: &[LocalId]) -> Self {
        Self {
            visible: parameters.iter().copied().collect(),
            edits: Vec::new(),
        }
    }

    pub(super) fn mark(&self) -> usize {
        self.edits.len()
    }

    pub(super) fn contains(&self, binding: &LocalId) -> bool {
        self.visible.contains(binding)
    }

    fn set(&mut self, binding: LocalId, present: bool) -> bool {
        let changed = if present {
            self.visible.insert(binding)
        } else {
            self.visible.remove(&binding)
        };
        if changed {
            self.edits.push(Edit { binding, present });
        }
        changed
    }

    pub(super) fn insert(&mut self, binding: LocalId) -> bool {
        self.set(binding, true)
    }

    pub(super) fn remove(&mut self, binding: &LocalId) {
        self.set(*binding, false);
    }

    pub(super) fn extend(&mut self, bindings: impl IntoIterator<Item = LocalId>) {
        for binding in bindings {
            self.insert(binding);
        }
    }

    pub(super) fn restore(&mut self, mark: usize) {
        for edit in self.edits.drain(mark..).rev() {
            if edit.present {
                self.visible.remove(&edit.binding);
            } else {
                self.visible.insert(edit.binding);
            }
        }
    }

    // A let-pattern's failure arm sees its entry scope. Save the successful
    // edits, check that arm, then commit success for the following statements.
    pub(super) fn take_success(
        &mut self,
        mark: usize,
        fuel: &mut usize,
    ) -> Result<Vec<Edit>, Error> {
        charge(fuel, self.edits.len() - mark)?;
        let success = self.edits[mark..].to_vec();
        self.restore(mark);
        Ok(success)
    }

    pub(super) fn apply(&mut self, edits: Vec<Edit>) {
        for edit in edits {
            self.set(edit.binding, edit.present);
        }
    }
}

/// A name absent on entry must be added by every alternative. A name that
/// was present on entry disappears if any alternative removes it.
#[derive(Default)]
pub(super) struct Alternatives {
    count: usize,
    additions: BTreeMap<LocalId, usize>,
    removals: BTreeSet<LocalId>,
}

impl Alternatives {
    pub(super) fn accept(
        &mut self,
        scope: &mut LexicalScope,
        mark: usize,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        charge(fuel, scope.edits.len() - mark)?;
        let changed: BTreeMap<_, _> = scope.edits[mark..]
            .iter()
            .map(|edit| (edit.binding, edit.present))
            .collect();
        scope.restore(mark);
        self.count += 1;
        for (binding, present) in changed {
            if present && !scope.contains(&binding) {
                *self.additions.entry(binding).or_default() += 1;
            } else if !present && scope.contains(&binding) {
                self.removals.insert(binding);
            }
        }
        Ok(())
    }

    pub(super) fn finish(self, scope: &mut LexicalScope) {
        for binding in self.removals {
            scope.remove(&binding);
        }
        scope.extend(
            self.additions
                .into_iter()
                .filter_map(|(binding, arms)| (arms == self.count).then_some(binding)),
        );
    }
}
