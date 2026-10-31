//! Unit propagation certificates contain only root clause indices. Replay
//! reconstructs clauses from the source formula, without search-state trust.
use super::{Cnf, Literal, charge};
use crate::ProofError;
use std::collections::VecDeque;

pub(crate) enum Propagation {
    Conflict(Vec<usize>),
    Unit {
        index: usize,
        value: bool,
        trace: Vec<usize>,
    },
    None,
}
enum Status {
    Open,
    Unit(Literal),
    Conflict,
}
impl Cnf {
    fn initial(
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

    fn status(
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
        *fuel = fuel.checked_sub(self.clauses.len()).ok_or(())?;
        let mut pending: VecDeque<_> = (0..self.clauses.len()).collect();
        let mut queued = vec![true; self.clauses.len()];
        while let Some(clause) = pending.pop_front() {
            queued[clause] = false;
            match self.status(clause, &values, fuel)? {
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
                    for &affected in &self.occurrences[literal.variable] {
                        charge(fuel)?;
                        if !queued[affected] {
                            queued[affected] = true;
                            pending.push_back(affected);
                        }
                    }
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

    #[test]
    fn partial_propagations_agree_with_all_source_completions() {
        let atoms = [Primitive::Bool, Primitive::U8].map(|p| {
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
            }
        }
        let mut units = 0;
        let mut conflicts = 0;
        for formula in formulas {
            let circuit = Circuit::compile(&formula, &atoms, &mut 10000).unwrap();
            let cnf = Cnf::compile(&circuit, atoms.len(), &mut 10000).unwrap();
            for code in 0..9 {
                let assignment = [
                    [None, Some(false), Some(true)][code % 3],
                    [None, Some(false), Some(true)][code / 3],
                ];
                let result = cnf.propagate(&assignment, &mut 10000).unwrap();
                let completion_is_model = |mask: usize| {
                    let full = [mask & 1 != 0, mask & 2 != 0];
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
                        assert!(!(0..4).any(completion_is_model));
                        assert_eq!(cnf.replay(&trace, &assignment, &mut 10000), Ok(()));
                    }
                    Propagation::Unit {
                        index,
                        value,
                        trace,
                    } => {
                        units += 1;
                        assert_eq!(assignment[index], None);
                        for mask in 0..4 {
                            if completion_is_model(mask) {
                                assert_eq!(mask & (1 << index) != 0, value);
                            }
                        }
                        let mut opposite = assignment;
                        opposite[index] = Some(!value);
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
