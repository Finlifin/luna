//! Revisit structural pairs whose representatives or child representatives
//! changed. Row nodes remain active because their rules also read explicit
//! disequalities and class aliases; unrelated structural pairs stay dormant.
use super::{Classes, Terms, UnknownReason};
#[cfg(test)]
mod row_tests;

pub(super) struct Agenda {
    parents: Vec<Vec<usize>>,
    roots: Vec<usize>,
    dirty: Vec<bool>,
    rows: Vec<usize>,
}
fn charge(fuel: &mut usize, work: usize) -> Result<(), UnknownReason> {
    *fuel = fuel
        .checked_sub(work)
        .ok_or(UnknownReason::BudgetExhausted)?;
    Ok(())
}
impl Agenda {
    pub fn new(terms: &Terms, roots: Vec<usize>, fuel: &mut usize) -> Result<Self, UnknownReason> {
        charge(fuel, roots.len())?;
        let mut parents = vec![Vec::new(); roots.len()];
        for (parent, children) in terms.children.iter().enumerate() {
            for &child in children {
                charge(fuel, 1)?;
                parents[child].push(parent);
            }
        }
        charge(fuel, terms.nodes.len())?;
        let rows = terms
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(id, term)| {
                matches!(
                    term,
                    nir::Term::Computation { .. } | nir::Term::ErrorResult { .. }
                )
                .then_some(id)
            })
            .collect();
        Ok(Self {
            dirty: vec![false; roots.len()],
            parents,
            roots,
            rows,
        })
    }
    pub fn refresh(&mut self, classes: &Classes, fuel: &mut usize) -> Result<(), UnknownReason> {
        charge(fuel, self.roots.len())?;
        self.dirty.fill(false);
        let mut pending = Vec::new();
        // A new label disequality may permit row commutation without merging
        // any classes. Keep every row and its structural parents in the agenda
        // until row observations carry their own finer dependency evidence.
        charge(fuel, self.rows.len())?;
        for &id in &self.rows {
            self.dirty[id] = true;
            pending.push(id);
        }
        for (id, root) in self.roots.iter_mut().enumerate() {
            let current = classes.root(id);
            if *root != current {
                *root = current;
                if !self.dirty[id] {
                    self.dirty[id] = true;
                    pending.push(id);
                }
            }
        }
        while let Some(child) = pending.pop() {
            for &parent in &self.parents[child] {
                charge(fuel, 1)?;
                if !self.dirty[parent] {
                    self.dirty[parent] = true;
                    pending.push(parent);
                }
            }
        }
        Ok(())
    }
    pub fn pairs(&self) -> Pairs<'_> {
        Pairs {
            count: self.roots.len(),
            dirty: Some(&self.dirty),
            a: 0,
            b: 0,
        }
    }
}
pub(super) struct Pairs<'a> {
    count: usize,
    dirty: Option<&'a [bool]>,
    a: usize,
    b: usize,
}
impl Pairs<'_> {
    pub fn all(count: usize) -> Self {
        Self {
            count,
            dirty: None,
            a: 0,
            b: 0,
        }
    }
}
impl Iterator for Pairs<'_> {
    type Item = (usize, usize);
    fn next(&mut self) -> Option<Self::Item> {
        while self.a < self.count {
            if self.dirty.is_some_and(|dirty| !dirty[self.a]) {
                self.a += 1;
                self.b = 0;
                continue;
            }
            let bound = if self.dirty.is_some() {
                self.count
            } else {
                self.a
            };
            while self.b < bound {
                let b = self.b;
                self.b += 1;
                if b == self.a || self.dirty.is_some_and(|dirty| dirty[b] && b > self.a) {
                    continue;
                }
                return Some((self.a.max(b), self.a.min(b)));
            }
            self.a += 1;
            self.b = 0;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    #[test]
    fn dirty_pairs_cover_each_affected_pair_once() {
        for mask in 0..64 {
            let dirty: Vec<_> = (0..6).map(|i| mask & (1 << i) != 0).collect();
            let pairs: Vec<_> = Pairs {
                count: 6,
                dirty: Some(&dirty),
                a: 0,
                b: 0,
            }
            .collect();
            let expected: BTreeSet<_> = Pairs::all(6)
                .filter(|&(a, b)| dirty[a] || dirty[b])
                .collect();
            assert_eq!(pairs.len(), expected.len());
            assert_eq!(pairs.into_iter().collect::<BTreeSet<_>>(), expected);
        }
    }
}
