//! Monotone propagation continuation within one immutable root clause set.
//! Retraction rebuilds the continuation; proof replay never consults it.
use super::{Cnf, Literal, Propagation, Status, Watches, charge};
use std::fmt;

pub(crate) struct Session {
    values: Vec<Option<bool>>,
    atoms: usize,
    watches: Watches,
    auxiliary_trace: Vec<usize>,
    yielded: Option<(usize, Literal)>,
}
impl Session {
    pub(crate) fn new(
        cnf: &Cnf,
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Self, ()> {
        let values = cnf.initial(assignment, fuel)?;
        Ok(Self {
            watches: Watches::new(cnf, values.len(), fuel)?,
            values,
            atoms: assignment.len(),
            auxiliary_trace: Vec::new(),
            yielded: None,
        })
    }

    pub(crate) fn extends(
        &self,
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<bool, ()> {
        if assignment.len() != self.atoms {
            return Ok(false);
        }
        for (old, new) in self.values[..self.atoms].iter().zip(assignment) {
            charge(fuel)?;
            if old.is_some() && old != new {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn certificate(&self, clause: usize, fuel: &mut usize) -> Result<Vec<usize>, ()> {
        *fuel = fuel.checked_sub(self.auxiliary_trace.len() + 1).ok_or(())?;
        let mut trace = self.auxiliary_trace.clone();
        trace.push(clause);
        Ok(trace)
    }

    pub(crate) fn propagate(
        &mut self,
        cnf: &Cnf,
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Propagation, ()> {
        for (index, value) in assignment.iter().copied().enumerate() {
            charge(fuel)?;
            if self.values[index] != value {
                self.values[index] = value;
                self.watches.assigned(index, fuel)?;
            }
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
                    // A conflicting pass is terminal. Retain the empty-clause
                    // result through a full restart on the next request.
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
