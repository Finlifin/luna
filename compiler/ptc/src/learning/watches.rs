//! Two non-falsified clause literals are watched. Entries store forbidden
//! conjunctions, so a matching assignment falsifies the corresponding clause
//! literal. Any watched-value change schedules a recheck, including rollback.
use super::Clauses;
use crate::UnknownReason;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Watches {
    pairs: Vec<[Option<usize>; 2]>,
    by_atom: BTreeMap<usize, BTreeSet<usize>>,
    pending: BTreeSet<usize>,
    #[cfg(test)]
    previous: Vec<Option<bool>>,
}
fn charge(fuel: &mut usize, count: usize) -> Result<(), UnknownReason> {
    *fuel = fuel
        .checked_sub(count)
        .ok_or(UnknownReason::BudgetExhausted)?;
    Ok(())
}
impl Watches {
    fn replace(&mut self, clause: usize, pair: [Option<usize>; 2]) {
        for atom in self.pairs[clause].into_iter().flatten() {
            if let Some(entries) = self.by_atom.get_mut(&atom) {
                entries.remove(&clause);
                if entries.is_empty() {
                    self.by_atom.remove(&atom);
                }
            }
        }
        for atom in pair.into_iter().flatten() {
            self.by_atom.entry(atom).or_default().insert(clause);
        }
        self.pairs[clause] = pair;
    }
}
impl Clauses {
    pub(super) fn push(&mut self, literals: Vec<(usize, bool)>) {
        let id = self.entries.len();
        self.literals += literals.len();
        let pair = [literals.first().map(|x| x.0), literals.get(1).map(|x| x.0)];
        self.entries.push(literals);
        self.watches.pairs.push([None; 2]);
        self.watches.replace(id, pair);
        self.watches.pending.insert(id);
    }

    /// Return a conflicting clause or one unit literal. Interrupted work stays
    /// pending; an unchanged retry cannot accidentally skip an unfinished scan.
    #[cfg(test)]
    pub(super) fn propagate(
        &mut self,
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Option<(usize, Option<(usize, bool)>)>, UnknownReason> {
        if self.entries.is_empty() {
            return Ok(None);
        }
        if self.watches.previous.len() != assignment.len() {
            charge(fuel, assignment.len())?;
            self.watches.previous = vec![None; assignment.len()];
            self.watches.pending.extend(0..self.entries.len());
        }
        for (atom, value) in assignment.iter().enumerate() {
            charge(fuel, 1)?;
            if *value == self.watches.previous[atom] {
                continue;
            }
            if let Some(entries) = self.watches.by_atom.get(&atom) {
                charge(fuel, entries.len())?;
                self.watches.pending.extend(entries);
            }
            self.watches.previous[atom] = *value;
        }
        self.propagate_pending(assignment, fuel)
    }

    /// None opens a new search epoch. Otherwise only notified atoms may have
    /// changed; interrupted callers must resend their unacknowledged changes.
    pub(super) fn propagate_changes(
        &mut self,
        assignment: &[Option<bool>],
        changed: Option<&BTreeSet<usize>>,
        fuel: &mut usize,
    ) -> Result<Option<(usize, Option<(usize, bool)>)>, UnknownReason> {
        #[cfg(test)]
        self.watches.previous.clear();
        if let Some(changed) = changed {
            for atom in changed {
                charge(fuel, 1)?;
                if let Some(entries) = self.watches.by_atom.get(atom) {
                    charge(fuel, entries.len())?;
                    self.watches.pending.extend(entries);
                }
            }
        } else {
            charge(fuel, self.entries.len())?;
            self.watches.pending.extend(0..self.entries.len());
        }
        self.propagate_pending(assignment, fuel)
    }

    fn propagate_pending(
        &mut self,
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Option<(usize, Option<(usize, bool)>)>, UnknownReason> {
        while let Some(&id) = self.watches.pending.first() {
            charge(fuel, 1)?;
            let clause = &self.entries[id];
            let mut live = Vec::new();
            for &(atom, forbidden) in clause {
                charge(fuel, 1)?;
                if assignment[atom] != Some(forbidden) {
                    live.push((atom, forbidden));
                    if live.len() == 2 {
                        break;
                    }
                }
            }
            let first = live
                .first()
                .map(|x| x.0)
                .or_else(|| clause.first().map(|x| x.0));
            let second = live
                .get(1)
                .map(|x| x.0)
                .or_else(|| clause.iter().find(|x| Some(x.0) != first).map(|x| x.0));
            let result = match live.as_slice() {
                [] => Some((id, None)),
                [(atom, forbidden)] if assignment[*atom].is_none() => {
                    Some((id, Some((*atom, !*forbidden))))
                }
                _ => None,
            };
            self.watches.replace(id, [first, second]);
            if result.is_some() {
                // Keep an outstanding unit/conflict visible until it is handled.
                return Ok(result);
            }
            self.watches.pending.remove(&id);
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oracle(
        clause: &[(usize, bool)],
        assignment: &[Option<bool>],
    ) -> Option<Option<(usize, bool)>> {
        let mut open = Vec::new();
        for &(atom, value) in clause {
            match assignment[atom] {
                Some(actual) if actual != value => return None,
                None => open.push((atom, !value)),
                _ => {}
            }
        }
        match open.as_slice() {
            [] => Some(None),
            [literal] => Some(Some(*literal)),
            _ => None,
        }
    }
    fn store() -> Clauses {
        let mut clauses = Clauses::default();
        for clause in [
            vec![(0, true), (1, true), (2, false)],
            vec![(0, false), (3, true)],
            vec![(1, false), (2, true)],
            vec![(3, false)],
        ] {
            clauses.push(clause);
        }
        clauses
    }
    fn check(clauses: &mut Clauses, assignment: &[Option<bool>]) {
        let actual = clauses.propagate(assignment, &mut 100_000).unwrap();
        if let Some((id, result)) = actual {
            assert_eq!(oracle(&clauses.entries[id], assignment), Some(result));
        } else {
            assert!(
                clauses
                    .entries
                    .iter()
                    .all(|c| oracle(c, assignment).is_none())
            );
        }
    }

    #[test]
    fn arbitrary_rollbacks_and_polarity_changes_match_full_clause_evaluation() {
        let mut clauses = store();
        for code in (0..81).chain((0..81).rev()) {
            let mut code = code;
            let mut assignment: Vec<_> = (0..4)
                .map(|_| {
                    let value = [None, Some(false), Some(true)][code % 3];
                    code /= 3;
                    value
                })
                .collect();
            check(&mut clauses, &assignment);
            // Drive every unit to a conflict or a propagation fixpoint.
            for _ in 0..5 {
                match clauses.propagate(&assignment, &mut 100_000).unwrap() {
                    Some((id, Some((atom, value)))) => {
                        assert_eq!(
                            oracle(&clauses.entries[id], &assignment),
                            Some(Some((atom, value)))
                        );
                        assignment[atom] = Some(value);
                    }
                    Some((id, None)) => {
                        assert_eq!(oracle(&clauses.entries[id], &assignment), Some(None));
                        break;
                    }
                    None => {
                        assert!(
                            clauses
                                .entries
                                .iter()
                                .all(|c| oracle(c, &assignment).is_none())
                        );
                        break;
                    }
                }
            }
        }
    }

    #[test]
    fn budget_interruptions_preserve_pending_events_and_new_clauses() {
        for fuel in 0..40 {
            let mut clauses = store();
            let first = [Some(false), Some(false), Some(false), Some(true)];
            check(&mut clauses, &first);
            let next = [Some(true), Some(true), Some(false), None];
            let _ = clauses.propagate(&next, &mut fuel.clone());
            check(&mut clauses, &next);
            check(&mut clauses, &first);
            clauses.push(vec![(0, false)]);
            check(&mut clauses, &first);
            // Clearing resets clause IDs, pending work, and assignment history.
            clauses = Clauses::default();
            clauses.push(vec![(2, false)]);
            assert_eq!(
                clauses.propagate(&first, &mut 1000).unwrap(),
                Some((0, None))
            );
        }
    }

    #[test]
    fn unchanged_assignments_do_not_rescan_inactive_clauses() {
        let mut clauses = Clauses::default();
        for atom in 1..65 {
            clauses.push(vec![(0, true), (atom, true)]);
        }
        let assignment = vec![Some(false); 65];
        assert!(
            clauses
                .propagate(&assignment, &mut 100_000)
                .unwrap()
                .is_none()
        );
        // Only synchronizing the current assignment remains; no clause-body
        // examination is necessary until a watched value changes.
        let mut budget = assignment.len();
        assert!(
            clauses
                .propagate(&assignment, &mut budget)
                .unwrap()
                .is_none()
        );
        assert_eq!(budget, 0);
        assert_eq!(
            clauses
                .watches
                .by_atom
                .values()
                .map(BTreeSet::len)
                .sum::<usize>(),
            128
        );
    }

    fn check_event_result(
        clauses: &Clauses,
        assignment: &[Option<bool>],
        result: Option<(usize, Option<(usize, bool)>)>,
    ) {
        if let Some((id, outcome)) = result {
            assert_eq!(oracle(&clauses.entries[id], assignment), Some(outcome));
        } else {
            assert!(
                clauses
                    .entries
                    .iter()
                    .all(|clause| oracle(clause, assignment).is_none())
            );
        }
    }

    #[test]
    fn trail_notifications_match_all_partial_assignments_and_retry_after_interruptions() {
        let mut clauses = store();
        let mut previous = vec![None; 4];
        for code in (0..81).chain((0..81).rev()) {
            let mut digits = code;
            let assignment: Vec<_> = (0..4)
                .map(|_| {
                    let value = [None, Some(false), Some(true)][digits % 3];
                    digits /= 3;
                    value
                })
                .collect();
            let changed = assignment
                .iter()
                .zip(&previous)
                .enumerate()
                .filter_map(|(atom, (a, b))| (a != b).then_some(atom))
                .collect();
            let result = clauses
                .propagate_changes(&assignment, Some(&changed), &mut 100_000)
                .unwrap();
            check_event_result(&clauses, &assignment, result);
            previous = assignment;
        }
        for fuel in 0..40 {
            let mut clauses = store();
            let first = [Some(false), Some(false), Some(false), Some(true)];
            clauses
                .propagate_changes(&first, None, &mut 100_000)
                .unwrap();
            let next = [Some(true), Some(true), Some(false), None];
            let changes = BTreeSet::from([0, 1, 3]);
            let _ = clauses.propagate_changes(&next, Some(&changes), &mut { fuel });
            let result = clauses
                .propagate_changes(&next, Some(&changes), &mut 100_000)
                .unwrap();
            check_event_result(&clauses, &next, result);
            // A new search may have unrelated premises but no assignment events.
            let result = clauses
                .propagate_changes(&first, None, &mut 100_000)
                .unwrap();
            check_event_result(&clauses, &first, result);
            clauses.push(vec![(0, false)]);
            let result = clauses
                .propagate_changes(&first, Some(&BTreeSet::new()), &mut 100_000)
                .unwrap();
            check_event_result(&clauses, &first, result);
        }
    }

    #[test]
    fn unchanged_trail_does_not_scan_the_atom_universe() {
        let mut clauses = Clauses::default();
        for atom in 1..65 {
            clauses.push(vec![(0, true), (atom, true)]);
        }
        let assignment = vec![Some(false); 4096];
        assert!(
            clauses
                .propagate_changes(&assignment, None, &mut 100_000)
                .unwrap()
                .is_none()
        );
        assert!(
            clauses
                .propagate_changes(&assignment, Some(&BTreeSet::new()), &mut 0)
                .unwrap()
                .is_none()
        );
        let mut changed = assignment;
        changed[4095] = None;
        assert!(
            clauses
                .propagate_changes(&changed, Some(&BTreeSet::from([4095])), &mut 1)
                .unwrap()
                .is_none()
        );
    }
}
