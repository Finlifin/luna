//! Reversible propagation trail within one immutable root clause set.
//! Retraction preserves the valid prefix; proof replay never consults it.
use super::{Cnf, Literal, Propagation, Status, Watches, charge};
use std::{collections::BTreeSet, fmt};

pub(crate) struct Session {
    values: Vec<Option<bool>>,
    atoms: usize,
    watches: Watches,
    auxiliary_trace: Vec<usize>,
    yielded: Option<(usize, Literal)>,
    conflict: Option<usize>,
    trail: Vec<Entry>,
    atom_positions: Vec<Option<usize>>,
}
struct Entry {
    literal: Literal,
    reason: Option<usize>,
}
impl Session {
    pub(crate) fn new(
        cnf: &Cnf,
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Self, ()> {
        let values = cnf.initial(assignment, fuel)?;
        let mut trail = Vec::new();
        let mut atom_positions = vec![None; assignment.len()];
        for (variable, value) in assignment.iter().enumerate() {
            charge(fuel)?;
            if let Some(positive) = value {
                atom_positions[variable] = Some(trail.len());
                trail.push(Entry {
                    literal: Literal {
                        variable,
                        positive: *positive,
                    },
                    reason: None,
                });
            }
        }
        Ok(Self {
            watches: Watches::new(cnf, values.len(), fuel)?,
            values,
            atoms: assignment.len(),
            auxiliary_trace: Vec::new(),
            yielded: None,
            conflict: None,
            trail,
            atom_positions,
        })
    }

    fn synchronize_changes(
        &mut self,
        assignment: &[Option<bool>],
        changed: &BTreeSet<usize>,
        fuel: &mut usize,
    ) -> Result<(), ()> {
        if assignment.len() != self.atoms {
            return Err(());
        }
        *fuel = fuel.checked_sub(changed.len()).ok_or(())?;
        let mut readmit = changed.clone();
        let mut cut = self.trail.len();
        for &index in changed {
            charge(fuel)?;
            let value = assignment.get(index).ok_or(())?;
            if self.values[index].is_some() && self.values[index] != *value {
                cut = cut.min(self.atom_positions[index].ok_or(())?);
            }
        }
        if cut < self.trail.len() {
            // Every implication depends only on assignments preceding it. Undo
            // the suffix, including unchanged caller atoms that followed the
            // invalid premise, then admit those caller atoms again below.
            while self.trail.len() > cut {
                charge(fuel)?;
                let entry = self.trail.pop().unwrap();
                let variable = entry.literal.variable;
                self.values[variable] = None;
                if let Some(reason) = entry.reason {
                    let removed = self.auxiliary_trace.pop();
                    debug_assert_eq!(removed, Some(reason));
                    self.watches.schedule(reason);
                } else {
                    self.atom_positions[variable] = None;
                    readmit.insert(variable);
                }
                self.watches.assigned(variable, fuel)?;
            }
            if let Some((clause, _)) = self.yielded.take() {
                self.watches.schedule(clause);
            }
            if let Some(clause) = self.conflict.take() {
                self.watches.schedule(clause);
            }
        }
        for variable in readmit {
            charge(fuel)?;
            if let Some(positive) = assignment[variable].filter(|_| self.values[variable].is_none())
            {
                self.atom_positions[variable] = Some(self.trail.len());
                self.trail.push(Entry {
                    literal: Literal { variable, positive },
                    reason: None,
                });
                self.values[variable] = Some(positive);
                self.watches.assigned(variable, fuel)?;
            }
        }
        Ok(())
    }

    fn certificate(&self, clause: usize, fuel: &mut usize) -> Result<Vec<usize>, ()> {
        *fuel = fuel.checked_sub(self.auxiliary_trace.len() + 1).ok_or(())?;
        let mut trace = self.auxiliary_trace.clone();
        trace.push(clause);
        Ok(trace)
    }

    #[cfg(test)]
    pub(crate) fn propagate(
        &mut self,
        cnf: &Cnf,
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Propagation, ()> {
        *fuel = fuel.checked_sub(assignment.len()).ok_or(())?;
        let changed = assignment
            .iter()
            .enumerate()
            .filter_map(|(index, value)| (self.values[index] != *value).then_some(index))
            .collect();
        self.propagate_changes(cnf, assignment, &changed, fuel)
    }

    pub(crate) fn propagate_changes(
        &mut self,
        cnf: &Cnf,
        assignment: &[Option<bool>],
        changed: &BTreeSet<usize>,
        fuel: &mut usize,
    ) -> Result<Propagation, ()> {
        self.synchronize_changes(assignment, changed, fuel)?;
        if let Some(clause) = self.conflict {
            return Ok(Propagation::Conflict(self.certificate(clause, fuel)?));
        }
        if let Some((clause, literal)) = self.yielded {
            match self.values[literal.variable] {
                None => {
                    return Ok(Propagation::Unit {
                        index: literal.variable,
                        value: literal.positive,
                        trace: self.certificate(clause, fuel)?,
                    });
                }
                Some(value) if value != literal.positive => {
                    return Ok(Propagation::Conflict(self.certificate(clause, fuel)?));
                }
                Some(_) => self.yielded = None,
            }
        }
        while let Some(clause) = self.watches.next() {
            match self.watches.inspect(cnf, clause, &self.values, fuel)? {
                Status::Open => {}
                Status::Conflict => {
                    self.conflict = Some(clause);
                    return Ok(Propagation::Conflict(self.certificate(clause, fuel)?));
                }
                Status::Unit(literal) if literal.variable < self.atoms => {
                    self.yielded = Some((clause, literal));
                    return Ok(Propagation::Unit {
                        index: literal.variable,
                        value: literal.positive,
                        trace: self.certificate(clause, fuel)?,
                    });
                }
                Status::Unit(literal) => {
                    charge(fuel)?;
                    self.trail.push(Entry {
                        literal,
                        reason: Some(clause),
                    });
                    self.auxiliary_trace.push(clause);
                    self.values[literal.variable] = Some(literal.positive);
                    self.watches.assigned(literal.variable, fuel)?;
                }
            }
        }
        Ok(Propagation::None)
    }
}
impl fmt::Display for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(root-propagation (theory-atoms {}) (values", self.atoms)?;
        for value in &self.values {
            match value {
                Some(value) => write!(f, " {value}")?,
                None => write!(f, " unknown")?,
            }
        }
        write!(f, ") (trail")?;
        for entry in &self.trail {
            write!(
                f,
                " ({} {} {}",
                if entry.reason.is_some() {
                    "auxiliary"
                } else {
                    "atom"
                },
                entry.literal.variable,
                entry.literal.positive
            )?;
            if let Some(reason) = entry.reason {
                write!(f, " (reason {reason})")?;
            }
            write!(f, ")")?;
        }
        write!(f, ") (conflict")?;
        if let Some(clause) = self.conflict {
            write!(f, " {clause}")?;
        }
        write!(f, ") (auxiliary-reasons")?;
        for clause in &self.auxiliary_trace {
            write!(f, " {clause}")?;
        }
        write!(f, ") (yielded")?;
        if let Some((clause, literal)) = self.yielded {
            write!(
                f,
                " (clause {clause}) (literal {} {})",
                literal.variable, literal.positive
            )?;
        }
        write!(f, "))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_retains_independent_auxiliaries_and_replays_both_branches() {
        let literal = |variable, positive| Literal { variable, positive };
        let prefix = 64;
        let last = 3 + prefix - 1;
        let branch = last + 1;
        let mut clauses = Vec::new();
        for output in 3..=branch {
            let input = if output == 3 {
                0
            } else if output == branch {
                1
            } else {
                output - 1
            };
            clauses.push(vec![literal(input, false), literal(output, true)]);
            clauses.push(vec![literal(input, true), literal(output, false)]);
        }
        clauses.push(vec![
            literal(last, false),
            literal(branch, false),
            literal(2, true),
        ]);
        let cnf = Cnf {
            atoms: 3,
            clauses,
            definitions: vec![(false, vec![]); prefix + 1],
        };
        let initial = [Some(true), None, None];
        let mut session = Session::new(&cnf, &initial, &mut 100_000).unwrap();
        assert!(matches!(
            session.propagate(&cnf, &initial, &mut 100_000),
            Ok(Propagation::None)
        ));
        let reasons = session.auxiliary_trace.clone();
        assert_eq!(reasons.len(), prefix);
        let mut warm = 1_000_000;
        let mut cold = 1_000_000;
        for iteration in 0..64 {
            let enabled = iteration % 2 == 0;
            let assignment = [Some(true), Some(enabled), None];
            let result = session.propagate(&cnf, &assignment, &mut warm).unwrap();
            let fresh = cnf.propagate(&assignment, &mut cold).unwrap();
            assert_eq!(&session.auxiliary_trace[..prefix], reasons.as_slice());
            assert_eq!(session.values[last], Some(true));
            assert_eq!(session.values[branch], Some(enabled));
            match (result, fresh) {
                (
                    Propagation::Unit {
                        index: 2,
                        value: true,
                        trace,
                    },
                    Propagation::Unit {
                        index: 2,
                        value: true,
                        ..
                    },
                ) if enabled => {
                    cnf.replay(&trace, &[Some(true), Some(true), Some(false)], &mut 100_000)
                        .unwrap();
                }
                (Propagation::None, Propagation::None) if !enabled => {}
                _ => panic!("retained and fresh worlds disagree"),
            }
        }
        assert!(
            warm > cold,
            "retaining the prefix must avoid recomputing it"
        );
        // A retained conflict is stable, but must be reconsidered after rollback.
        let conflict = [Some(true), Some(true), Some(false)];
        for _ in 0..2 {
            let Propagation::Conflict(trace) =
                session.propagate(&cnf, &conflict, &mut 100_000).unwrap()
            else {
                panic!("expected conflict");
            };
            cnf.replay(&trace, &conflict, &mut 100_000).unwrap();
        }
        assert!(matches!(
            session.propagate(&cnf, &[Some(true), Some(false), Some(false)], &mut 100_000),
            Ok(Propagation::None)
        ));
        assert!(
            session
                .to_string()
                .contains("(auxiliary 3 true (reason 0))")
        );
        // Atom 2 remains a caller premise when undoing the earlier atom 1
        // removes its local entry. It is absent from the notification set.
        let changed = BTreeSet::from([1]);
        let Propagation::Conflict(trace) = session
            .propagate_changes(&cnf, &conflict, &changed, &mut 100_000)
            .unwrap()
        else {
            panic!("unchanged caller premise was lost during rollback");
        };
        cnf.replay(&trace, &conflict, &mut 100_000).unwrap();
        session
            .propagate_changes(
                &cnf,
                &[Some(true), Some(false), Some(false)],
                &changed,
                &mut 100_000,
            )
            .unwrap();
        assert_eq!(session.values[2], Some(false));
    }

    #[test]
    fn unchanged_root_session_does_not_scan_the_atom_universe() {
        let cnf = Cnf {
            atoms: 4096,
            clauses: vec![vec![
                Literal {
                    variable: 0,
                    positive: true,
                },
                Literal {
                    variable: 1,
                    positive: true,
                },
            ]],
            definitions: vec![],
        };
        let mut assignment = vec![None; 4096];
        assignment[0] = Some(true);
        let mut session = Session::new(&cnf, &assignment, &mut 100_000).unwrap();
        assert!(matches!(
            session.propagate_changes(&cnf, &assignment, &BTreeSet::new(), &mut 100_000),
            Ok(Propagation::None)
        ));
        assert!(matches!(
            session.propagate_changes(&cnf, &assignment, &BTreeSet::new(), &mut 0),
            Ok(Propagation::None)
        ));
        assignment[4095] = Some(false);
        assert!(matches!(
            session.propagate_changes(&cnf, &assignment, &BTreeSet::from([4095]), &mut 3),
            Ok(Propagation::None)
        ));
    }
}
