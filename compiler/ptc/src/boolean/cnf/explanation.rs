//! Slice a root-clause contradiction by its unit dependencies. The resulting
//! trace is independently replayable with only the retained theory premises.
use super::propagation::Status;
use super::{Cnf, charge};
use std::collections::BTreeSet;

impl Cnf {
    pub(crate) fn explain(
        &self,
        trace: &[usize],
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<(Vec<usize>, BTreeSet<usize>), ()> {
        let mut values = self.initial(assignment, fuel)?;
        *fuel = fuel.checked_sub(values.len()).ok_or(())?;
        let mut reasons = vec![None; values.len()];
        let mut conflict = None;
        for (position, &clause) in trace.iter().enumerate() {
            if clause >= self.clauses.len() {
                return Err(());
            }
            match self.status(clause, &values, fuel)? {
                Status::Open => return Err(()),
                Status::Unit(literal) => {
                    values[literal.variable] = Some(literal.positive);
                    reasons[literal.variable] = Some(position);
                }
                Status::Conflict => {
                    conflict = Some(position);
                    break;
                }
            }
        }
        let mut pending = vec![(conflict.ok_or(())?, None)];
        let mut retained = BTreeSet::new();
        let mut support = BTreeSet::new();
        while let Some((position, consequent)) = pending.pop() {
            charge(fuel)?;
            if !retained.insert(position) {
                continue;
            }
            for literal in &self.clauses[trace[position]] {
                charge(fuel)?;
                let variable = literal.variable;
                if Some(variable) == consequent {
                    continue;
                }
                if variable < self.atoms && assignment[variable].is_some() {
                    support.insert(variable);
                } else {
                    let parent = reasons[variable].ok_or(())?;
                    if parent >= position {
                        return Err(());
                    }
                    pending.push((parent, Some(variable)));
                }
            }
        }
        *fuel = fuel.checked_sub(retained.len()).ok_or(())?;
        Ok((
            retained
                .into_iter()
                .map(|position| trace[position])
                .collect(),
            support,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Atom, Equality, Formula, boolean::Circuit};
    use nir::{Primitive, Term};

    #[test]
    fn slicing_drops_unrelated_units_and_theory_premises() {
        let atoms = [Primitive::U8, Primitive::U16].map(|p| {
            Atom::TypeEqual(Equality(
                Term::Primitive(p),
                Term::Primitive(Primitive::Bool),
            ))
        });
        let leaves: Vec<_> = atoms
            .iter()
            .map(|atom| {
                let Atom::TypeEqual(e) = atom else {
                    unreachable!()
                };
                Formula::Equal(e.clone())
            })
            .collect();
        let formula = Formula::And(vec![
            Formula::Or(vec![leaves[0].clone(), leaves[0].clone()]),
            Formula::Or(vec![leaves[1].clone(), leaves[1].clone().negate()]),
        ]);
        let circuit = Circuit::compile(&formula, &atoms, &mut 10000).unwrap();
        let cnf = Cnf::compile(&circuit, 2, &mut 10000).unwrap();
        let assignment = [Some(false), Some(true)];
        let super::super::Propagation::Conflict(trace) =
            cnf.propagate(&assignment, &mut 10000).unwrap()
        else {
            panic!("contradictory root");
        };
        let mut remaining = 10000;
        let (trimmed, support) = cnf.explain(&trace, &assignment, &mut remaining).unwrap();
        assert_eq!(support, BTreeSet::from([0]));
        assert!(trimmed.len() < trace.len());
        cnf.replay(&trimmed, &[Some(false), None], &mut 10000)
            .unwrap();
        assert!(cnf.replay(&trimmed, &[None, None], &mut 10000).is_err());
        let cost = 10000 - remaining;
        for budget in 0..cost {
            assert!(cnf.explain(&trace, &assignment, &mut { budget }).is_err());
        }
        assert_eq!(
            cnf.explain(&trace, &assignment, &mut { cost }).unwrap(),
            (trimmed, support)
        );
    }
}
