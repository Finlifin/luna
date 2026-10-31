//! Unit propagation certificates contain only root clause indices. Replay
//! reconstructs clauses from the source formula, without search-state trust.
use super::{Cnf, Literal, charge};
use crate::ProofError;
mod watches;
use watches::Watches;

pub(crate) enum Propagation {
    Conflict(Vec<usize>),
    Unit {
        index: usize,
        value: bool,
        trace: Vec<usize>,
    },
    None,
}
pub(super) enum Status {
    Open,
    Unit(Literal),
    Conflict,
}
impl Cnf {
    pub(super) fn initial(
        &self,
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Vec<Option<bool>>, ()> {
        if assignment.len() != self.atoms {
            return Err(());
        }
        let size = self.atoms + self.definitions.len();
        *fuel = fuel.checked_sub(size).ok_or(())?;
        let mut values = assignment.to_vec();
        values.resize(size, None);
        Ok(values)
    }

    pub(super) fn status(
        &self,
        clause: usize,
        values: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Status, ()> {
        charge(fuel)?;
        let mut unit: Option<Literal> = None;
        let mut multiple = false;
        for literal in &self.clauses[clause] {
            charge(fuel)?;
            match values[literal.variable] {
                Some(value) if value == literal.positive => return Ok(Status::Open),
                Some(_) => {}
                None => {
                    if let Some(previous) = unit {
                        // Repeated literals represent the same unknown, not
                        // independent possibilities (e.g. x or x is unit).
                        multiple |= previous.variable != literal.variable
                            || previous.positive != literal.positive;
                    } else {
                        unit = Some(*literal);
                    }
                }
            }
        }
        Ok(if multiple {
            Status::Open
        } else if let Some(unit) = unit {
            Status::Unit(unit)
        } else {
            Status::Conflict
        })
    }

    pub(crate) fn propagate(
        &self,
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Propagation, ()> {
        let mut values = self.initial(assignment, fuel)?;
        let mut trace = Vec::new();
        let mut watches = Watches::new(self, values.len(), fuel)?;
        while let Some(clause) = watches.next() {
            match watches.inspect(self, clause, &values, fuel)? {
                Status::Open => {}
                Status::Conflict => {
                    trace.push(clause);
                    return Ok(Propagation::Conflict(trace));
                }
                Status::Unit(literal) => {
                    trace.push(clause);
                    if literal.variable < self.atoms {
                        return Ok(Propagation::Unit {
                            index: literal.variable,
                            value: literal.positive,
                            trace,
                        });
                    }
                    values[literal.variable] = Some(literal.positive);
                    watches.assigned(literal.variable, fuel)?;
                }
            }
        }
        Ok(Propagation::None)
    }

    pub(crate) fn replay(
        &self,
        trace: &[usize],
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<(), ProofError> {
        let mut values = self
            .initial(assignment, fuel)
            .map_err(|()| ProofError::BudgetExhausted)?;
        for &clause in trace {
            if clause >= self.clauses.len() {
                return Err(ProofError::Invalid);
            }
            match self
                .status(clause, &values, fuel)
                .map_err(|()| ProofError::BudgetExhausted)?
            {
                // Strengthening the premises can cause an earlier conflict;
                // the remaining trace is unnecessary once falsity is proved.
                Status::Conflict => return Ok(()),
                Status::Unit(literal) => values[literal.variable] = Some(literal.positive),
                Status::Open => return Err(ProofError::Invalid),
            }
        }
        Err(ProofError::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Atom, Equality, Formula, boolean::Circuit, evaluate};
    use nir::{Primitive, Term};

    fn check_explanation(cnf: &Cnf, trace: &[usize], assignment: &[Option<bool>]) {
        let (trace, support) = cnf.explain(trace, assignment, &mut 10000).unwrap();
        let reduced: Vec<_> = assignment
            .iter()
            .enumerate()
            .map(|(i, value)| if support.contains(&i) { *value } else { None })
            .collect();
        cnf.replay(&trace, &reduced, &mut 10000).unwrap();
    }

    #[test]
    fn wide_clause_ignores_unwatched_assignments_and_recovers_after_exhaustion() {
        let width = 128;
        let positive = |variable| Literal {
            variable,
            positive: true,
        };
        // The wide clause watches 1 and 2. Most later units cannot affect its
        // status; only the final two assignments force the remaining theory atom.
        let mut clauses = vec![(1..=width).chain([0]).map(positive).collect()];
        clauses.extend((3..=width).chain([1, 2]).map(|variable| {
            vec![Literal {
                variable,
                positive: false,
            }]
        }));
        let cnf = Cnf {
            atoms: 1,
            clauses,
            definitions: vec![(false, vec![]); width],
        };
        assert!(cnf.propagate(&[None], &mut 10).is_err());
        // Bound the total work of preparation, watch events and implication.
        // Proof replay has its own allowance and still scans complete clauses.
        let mut fuel = 20 * width;
        let Propagation::Unit {
            index,
            value,
            trace,
        } = cnf.propagate(&[None], &mut fuel).unwrap()
        else {
            panic!("all auxiliary alternatives are false, so atom zero is forced");
        };
        assert_eq!((index, value), (0, true));
        assert_eq!(cnf.replay(&trace, &[Some(false)], &mut 100_000), Ok(()));
        check_explanation(&cnf, &trace, &[Some(false)]);
        // A new pass may flip assumptions; no auxiliary assignment leaks from
        // the earlier propagation or its budget-interrupted predecessor.
        assert!(matches!(
            cnf.propagate(&[Some(true)], &mut 100_000),
            Ok(Propagation::None)
        ));
        let Propagation::Conflict(trace) = cnf.propagate(&[Some(false)], &mut 100_000).unwrap()
        else {
            panic!("all alternatives are false");
        };
        assert_eq!(cnf.replay(&trace, &[Some(false)], &mut 100_000), Ok(()));
    }

    #[test]
    fn partial_propagations_agree_with_all_source_completions() {
        let atoms = [Primitive::Bool, Primitive::U8, Primitive::Char].map(|p| {
            Atom::TypeEqual(Equality(
                Term::Primitive(p),
                Term::Primitive(Primitive::Bool),
            ))
        });
        let leaves: Vec<_> = atoms
            .iter()
            .map(|a| {
                let Atom::TypeEqual(e) = a else {
                    unreachable!()
                };
                Formula::Equal(e.clone())
            })
            .collect();
        let mut formulas = vec![Formula::True, Formula::False];
        for a in &leaves {
            for b in &leaves {
                formulas.push(Formula::Or(vec![a.clone(), a.clone()]));
                formulas.push(Formula::And(vec![
                    Formula::Or(vec![a.clone(), b.clone()]),
                    a.clone().negate(),
                ]));
                formulas.push(Formula::Or(vec![
                    a.clone(),
                    Formula::And(vec![b.clone(), b.clone().negate()]),
                ]));
                formulas.push(Formula::And(vec![a.clone(), a.clone().negate()]));
                for c in &leaves {
                    formulas.push(Formula::And(vec![
                        Formula::Or(vec![a.clone(), b.clone(), c.clone()]),
                        Formula::Or(vec![a.clone().negate(), b.clone()]),
                        Formula::Or(vec![b.clone().negate(), c.clone().negate()]),
                    ]));
                }
            }
        }
        let mut units = 0;
        let mut conflicts = 0;
        for formula in formulas {
            let circuit = Circuit::compile(&formula, &atoms, &mut 10000).unwrap();
            let cnf = Cnf::compile(&circuit, atoms.len(), &mut 10000).unwrap();
            for code in 0..27 {
                let assignment = [
                    [None, Some(false), Some(true)][code % 3],
                    [None, Some(false), Some(true)][(code / 3) % 3],
                    [None, Some(false), Some(true)][code / 9],
                ];
                let result = cnf.propagate(&assignment, &mut 10000).unwrap();
                let completion_is_model = |mask: usize| {
                    let full = [mask & 1 != 0, mask & 2 != 0, mask & 4 != 0];
                    assignment
                        .iter()
                        .zip(full)
                        .all(|(a, b)| a.is_none_or(|a| a == b))
                        && evaluate(&formula, &|a| {
                            Some(full[atoms.iter().position(|b| a == b).unwrap()])
                        }) == Some(true)
                };
                match result {
                    Propagation::Conflict(trace) => {
                        conflicts += 1;
                        check_explanation(&cnf, &trace, &assignment);
                        assert!(!(0..8).any(completion_is_model));
                        assert_eq!(cnf.replay(&trace, &assignment, &mut 10000), Ok(()));
                    }
                    Propagation::Unit {
                        index,
                        value,
                        trace,
                    } => {
                        units += 1;
                        assert_eq!(assignment[index], None);
                        for mask in 0..8 {
                            if completion_is_model(mask) {
                                assert_eq!(mask & (1 << index) != 0, value);
                            }
                        }
                        let mut opposite = assignment;
                        opposite[index] = Some(!value);
                        check_explanation(&cnf, &trace, &opposite);
                        assert_eq!(cnf.replay(&trace, &opposite, &mut 10000), Ok(()));
                        assert_eq!(
                            cnf.replay(&trace, &assignment, &mut 10000),
                            Err(ProofError::Invalid)
                        );
                    }
                    Propagation::None => {}
                }
                assert_eq!(
                    cnf.replay(&[usize::MAX], &assignment, &mut 10000),
                    Err(ProofError::Invalid)
                );
                assert!(cnf.propagate(&assignment, &mut 0).is_err());
            }
        }
        assert!(units > 0 && conflicts > 0);
    }
}
