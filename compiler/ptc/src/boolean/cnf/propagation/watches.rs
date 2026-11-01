//! Root-clause watches retained across assignments and rollback.
//! Proof replay uses the independent full-clause scanner.
use super::{Cnf, Literal, Status, charge};
use std::collections::{BTreeSet, VecDeque};

pub(super) struct Watches {
    watched: Vec<[Option<usize>; 2]>,
    cursor: Vec<usize>,
    buckets: Vec<BTreeSet<usize>>,
    pending: VecDeque<usize>,
    queued: Vec<bool>,
}
impl Watches {
    pub(super) fn new(cnf: &Cnf, variables: usize, fuel: &mut usize) -> Result<Self, ()> {
        let count = cnf.clauses.len();
        *fuel = fuel.checked_sub(variables).ok_or(())?;
        *fuel = fuel.checked_sub(count).ok_or(())?;
        Ok(Self {
            watched: vec![[None; 2]; count],
            cursor: vec![0; count],
            buckets: vec![BTreeSet::new(); variables],
            pending: (0..count).collect(),
            queued: vec![true; count],
        })
    }
    pub(super) fn next(&mut self) -> Option<usize> {
        let clause = self.pending.pop_front()?;
        self.queued[clause] = false;
        Some(clause)
    }
    pub(super) fn assigned(&mut self, variable: usize, fuel: &mut usize) -> Result<(), ()> {
        // A variable can be assigned, unassigned and assigned again. Inspection
        // reinstalls its subscriptions after each queued change.
        for clause in std::mem::take(&mut self.buckets[variable]) {
            charge(fuel)?;
            if self.watched[clause].contains(&Some(variable)) && !self.queued[clause] {
                self.schedule(clause);
            }
        }
        Ok(())
    }
    pub(super) fn schedule(&mut self, clause: usize) {
        if !self.queued[clause] {
            self.queued[clause] = true;
            self.pending.push_back(clause);
        }
    }
    fn install(&mut self, clause: usize, first: usize, second: Option<usize>) {
        self.watched[clause] = [Some(first), second];
        self.buckets[first].insert(clause);
        if let Some(second) = second.filter(|second| *second != first) {
            self.buckets[second].insert(clause);
        }
    }
    pub(super) fn inspect(
        &mut self,
        cnf: &Cnf,
        clause: usize,
        values: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Status, ()> {
        charge(fuel)?;
        for variable in self.watched[clause].iter().flatten() {
            charge(fuel)?;
            self.buckets[*variable].remove(&clause);
        }
        self.watched[clause] = [None; 2];
        let literals = &cnf.clauses[clause];
        let mut unit: Option<Literal> = None;
        // Resume after the last inspected literal. Replacements do not repeatedly
        // walk a long already-false prefix. Empty clauses never index the cursor.
        for _ in 0..literals.len() {
            charge(fuel)?;
            let position = self.cursor[clause];
            self.cursor[clause] = (position + 1) % literals.len();
            let literal = literals[position];
            match values[literal.variable] {
                Some(value) if value == literal.positive => {
                    self.install(clause, literal.variable, None);
                    return Ok(Status::Open);
                }
                Some(_) => {}
                None => {
                    if let Some(previous) = unit {
                        if previous.variable != literal.variable
                            || previous.positive != literal.positive
                        {
                            self.install(clause, previous.variable, Some(literal.variable));
                            return Ok(Status::Open);
                        }
                    } else {
                        unit = Some(literal);
                    }
                }
            }
        }
        Ok(match unit {
            Some(unit) => {
                self.install(clause, unit.variable, None);
                Status::Unit(unit)
            }
            None => Status::Conflict,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clause(width: usize) -> Cnf {
        Cnf {
            atoms: width,
            clauses: vec![
                (0..width)
                    .map(|variable| Literal {
                        variable,
                        positive: true,
                    })
                    .collect(),
            ],
            definitions: vec![],
        }
    }
    #[test]
    fn unwatched_variables_do_not_wake_a_wide_clause() {
        let cnf = clause(256);
        let mut values = vec![None; 256];
        let mut fuel = 4096;
        let mut watches = Watches::new(&cnf, values.len(), &mut fuel).unwrap();
        assert_eq!(watches.next(), Some(0));
        assert!(matches!(
            watches.inspect(&cnf, 0, &values, &mut fuel),
            Ok(Status::Open)
        ));
        for variable in 2..values.len() {
            values[variable] = Some(false);
            watches.assigned(variable, &mut fuel).unwrap();
            assert_eq!(watches.next(), None);
        }
        values[0] = Some(false);
        watches.assigned(0, &mut fuel).unwrap();
        assert_eq!(watches.next(), Some(0));
        assert!(matches!(
            watches.inspect(&cnf, 0, &values, &mut fuel),
            Ok(Status::Unit(Literal {
                variable: 1,
                positive: true
            }))
        ));
    }
    #[test]
    fn moved_watch_entries_cannot_schedule_unrelated_clauses() {
        let cnf = clause(4);
        let mut values = vec![None; 4];
        let mut fuel = 4096;
        let mut watches = Watches::new(&cnf, values.len(), &mut fuel).unwrap();
        assert_eq!(watches.next(), Some(0));
        assert!(matches!(
            watches.inspect(&cnf, 0, &values, &mut fuel),
            Ok(Status::Open)
        ));
        values[0] = Some(false);
        watches.assigned(0, &mut fuel).unwrap();
        assert_eq!(watches.next(), Some(0));
        assert!(matches!(
            watches.inspect(&cnf, 0, &values, &mut fuel),
            Ok(Status::Open)
        ));
        // Circular scanning moved both watches to 2 and 3. The old bucket for
        // variable 1 may still contain the clause but no longer owns its watch.
        values[1] = Some(false);
        watches.assigned(1, &mut fuel).unwrap();
        assert_eq!(watches.next(), None);
        values[2] = Some(false);
        watches.assigned(2, &mut fuel).unwrap();
        assert_eq!(watches.next(), Some(0));
        assert!(matches!(
            watches.inspect(&cnf, 0, &values, &mut fuel),
            Ok(Status::Unit(Literal {
                variable: 3,
                positive: true
            }))
        ));
    }

    #[test]
    fn repeated_retractions_keep_live_subscriptions_bounded() {
        let cnf = clause(4);
        let mut values = vec![None; 4];
        let mut fuel = 100_000;
        let mut watches = Watches::new(&cnf, 4, &mut fuel).unwrap();
        for iteration in 0..1000 {
            values[0] = if iteration % 2 == 0 { Some(true) } else { None };
            watches.assigned(0, &mut fuel).unwrap();
            while let Some(clause) = watches.next() {
                assert!(matches!(
                    watches.inspect(&cnf, clause, &values, &mut fuel),
                    Ok(Status::Open)
                ));
            }
            assert!(watches.buckets.iter().map(BTreeSet::len).sum::<usize>() <= 2);
        }
        values.fill(Some(false));
        for variable in 0..4 {
            watches.assigned(variable, &mut fuel).unwrap();
        }
        let clause = watches.next().expect("final changes must wake the clause");
        assert!(matches!(
            watches.inspect(&cnf, clause, &values, &mut fuel),
            Ok(Status::Conflict)
        ));
    }
}
