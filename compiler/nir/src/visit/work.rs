//! Work-stack allocation follows the caller's remaining node budget. A pending
//! node costs at least one visit even if its own descendants will be pruned.
use super::Error;

/// Budget the suffix appended to a caller-owned stack. A lexical caller may
/// already have paid for outer continuations below this boundary.
pub(super) struct Work<'s, T> {
    nodes: &'s mut Vec<T>,
    start: usize,
}

impl<'s, T> Work<'s, T> {
    pub(super) fn new(nodes: &'s mut Vec<T>) -> Self {
        Self {
            start: nodes.len(),
            nodes,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.nodes.len() - self.start
    }

    pub(super) fn push(&mut self, node: impl Into<T>, remaining: usize) -> Result<(), Error> {
        if self.len() >= remaining {
            return Err(Error::Budget("IR traversal work-stack budget exhausted"));
        }
        self.nodes.push(node.into());
        Ok(())
    }

    pub(super) fn extend<N: Into<T>>(
        &mut self,
        nodes: impl IntoIterator<Item = N>,
        remaining: usize,
    ) -> Result<(), Error> {
        let nodes = nodes.into_iter();
        if nodes.size_hint().0 > remaining.saturating_sub(self.len()) {
            return Err(Error::Budget("IR traversal work-stack budget exhausted"));
        }
        // Do not hand an unchecked stream to Vec::extend. Push individually
        // so unknown-width streams stop at the same bound without reserving
        // their upper hint. A failed traversal discards its private frontier.
        for node in nodes {
            self.push(node, remaining)?;
        }
        Ok(())
    }

    pub(super) fn pop(&mut self) -> Option<T> {
        (self.len() > 0).then(|| self.nodes.pop().unwrap())
    }
}

pub(super) fn charge_entries(fuel: &mut usize, count: usize) -> Result<(), Error> {
    *fuel = fuel.checked_sub(count).ok_or(Error::Budget(
        "IR traversal structural-entry budget exhausted",
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn wide_streams_stop_before_unbounded_enumeration() {
        let reads = Cell::new(0);
        let stream = (0..usize::MAX).inspect(|_| reads.set(reads.get() + 1));
        let mut nodes = Vec::<usize>::new();
        let mut work = Work::new(&mut nodes);
        assert!(matches!(work.extend(stream, 4), Err(Error::Budget(_))));
        assert_eq!(reads.get(), 0, "known width must fail before enumeration");

        // Filtering removes the useful lower bound; the upper bound is still
        // enormous and must never drive a Vec allocation.
        let stream = (0..usize::MAX)
            .filter(|_| true)
            .inspect(|_| reads.set(reads.get() + 1));
        assert!(matches!(work.extend(stream, 4), Err(Error::Budget(_))));
        assert_eq!(reads.get(), 5);
        assert_eq!(work.nodes.len(), 4);
        assert_eq!(work.pop(), Some(3));
        assert_eq!(work.pop(), Some(2));
        assert_eq!(work.pop(), Some(1));
        assert_eq!(work.pop(), Some(0));
        assert_eq!(work.pop(), None);
    }

    #[test]
    fn pending_siblings_share_the_remaining_allowance() {
        let mut nodes = Vec::<i32>::new();
        let mut work = Work::new(&mut nodes);
        work.extend(0..3, 4).unwrap();
        let reads = Cell::new(0);
        let stream = (3..5).inspect(|_| reads.set(reads.get() + 1));
        assert!(matches!(work.extend(stream, 4), Err(Error::Budget(_))));
        assert_eq!(reads.get(), 0);
        assert_eq!(work.pop(), Some(2));
        // A callback can consume fuel while siblings are pending.
        assert!(matches!(work.push(9, 2), Err(Error::Budget(_))));
        assert_eq!(work.pop(), Some(1));
        work.push(9, 2).unwrap();
        assert_eq!(work.pop(), Some(9));
        assert_eq!(work.pop(), Some(0));
    }
}
