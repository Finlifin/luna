//! Linear Tseitin encoding. Auxiliary variables are private Boolean identities,
//! never theory atoms or source-level inference variables.
mod explanation;
mod propagation;
pub(crate) use propagation::Propagation;

use super::{Circuit, Node, charge};
use std::fmt;

#[derive(Clone, Copy)]
struct Literal {
    variable: usize,
    positive: bool,
}
impl Literal {
    fn negate(self) -> Self {
        Self {
            positive: !self.positive,
            ..self
        }
    }
    fn value(self, values: &[bool]) -> bool {
        values[self.variable] == self.positive
    }
}

pub(crate) struct Cnf {
    atoms: usize,
    clauses: Vec<Vec<Literal>>,
    // Definitions are topologically ordered. Each auxiliary variable is the
    // conjunction/disjunction of these literals; empty gates encode constants.
    definitions: Vec<(bool, Vec<Literal>)>,
}
impl Cnf {
    pub(crate) fn compile(circuit: &Circuit, atoms: usize, fuel: &mut usize) -> Result<Self, ()> {
        let mut result = Self {
            atoms,
            clauses: Vec::new(),
            definitions: Vec::new(),
        };
        let mut outputs: Vec<Literal> = Vec::new();
        for node in &circuit.nodes {
            charge(fuel)?;
            let (disjunction, children) = match node {
                Node::Atom(i) => {
                    if *i >= atoms {
                        return Err(());
                    }
                    outputs.push(Literal {
                        variable: *i,
                        positive: true,
                    });
                    continue;
                }
                Node::Not(i) => {
                    outputs.push(outputs[*i].negate());
                    continue;
                }
                Node::Constant(value) => (!value, &[][..]),
                Node::And(children) => (false, children.as_slice()),
                Node::Or(children) => (true, children.as_slice()),
            };
            let gate = Literal {
                variable: atoms + result.definitions.len(),
                positive: true,
            };
            let mut inputs = Vec::new();
            let mut reverse = vec![if disjunction { gate.negate() } else { gate }];
            for child in children {
                // Charge before allocating the two binary literals, reverse
                // literal and definition input. Encoding size is linear.
                for _ in 0..4 {
                    charge(fuel)?;
                }
                let input = outputs[*child];
                inputs.push(input);
                result.clauses.push(if disjunction {
                    vec![gate, input.negate()]
                } else {
                    vec![gate.negate(), input]
                });
                reverse.push(if disjunction { input } else { input.negate() });
            }
            result.clauses.push(reverse);
            result.definitions.push((disjunction, inputs));
            outputs.push(gate);
        }
        charge(fuel)?;
        result.clauses.push(vec![*outputs.last().ok_or(())?]);
        Ok(result)
    }

    /// Extend interpreted theory atoms deterministically, then check every
    /// clause. An abstract SAT assignment is not a source-model certificate.
    pub(crate) fn accepts(&self, atoms: &[bool], fuel: &mut usize) -> Result<bool, ()> {
        if atoms.len() != self.atoms {
            return Err(());
        }
        let mut values = Vec::new();
        for value in atoms {
            charge(fuel)?;
            values.push(*value);
        }
        for (disjunction, inputs) in &self.definitions {
            charge(fuel)?;
            let mut value = !disjunction;
            for input in inputs {
                charge(fuel)?;
                if input.value(&values) == *disjunction {
                    value = *disjunction;
                }
            }
            values.push(value);
        }
        self.satisfied(&values, fuel)
    }

    fn satisfied(&self, values: &[bool], fuel: &mut usize) -> Result<bool, ()> {
        for clause in &self.clauses {
            charge(fuel)?;
            let mut satisfied = false;
            for literal in clause {
                charge(fuel)?;
                satisfied |= literal.value(values);
            }
            if !satisfied {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
impl fmt::Display for Cnf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(cnf (theory-atoms {}) (auxiliaries {}) (clauses",
            self.atoms,
            self.definitions.len()
        )?;
        for clause in &self.clauses {
            write!(f, " (clause")?;
            for literal in clause {
                write!(
                    f,
                    " ({} {})",
                    if literal.positive {
                        "positive"
                    } else {
                        "negative"
                    },
                    literal.variable
                )?;
            }
            write!(f, ")")?;
        }
        write!(f, "))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Atom, Equality, Formula, evaluate};
    use nir::{Primitive, Term};

    #[test]
    fn clauses_have_exactly_the_source_models() {
        let atoms = [Primitive::Bool, Primitive::U8].map(|p| {
            Atom::TypeEqual(Equality(
                Term::Primitive(p),
                Term::Primitive(Primitive::Bool),
            ))
        });
        let leaves = atoms
            .iter()
            .map(|a| {
                let Atom::TypeEqual(e) = a else {
                    unreachable!()
                };
                Formula::Equal(e.clone())
            })
            .collect::<Vec<_>>();
        let mut formulas = vec![
            Formula::True,
            Formula::False,
            Formula::And(vec![]),
            Formula::Or(vec![]),
        ];
        formulas.extend(leaves.clone());
        for a in &leaves {
            for b in &leaves {
                formulas.push(Formula::And(vec![
                    a.clone(),
                    Formula::Or(vec![a.clone().negate(), b.clone()]),
                ]));
                formulas.push(Formula::Or(vec![a.clone(), b.clone()]).negate());
            }
        }
        for formula in formulas {
            let circuit = Circuit::compile(&formula, &atoms, &mut 10000).unwrap();
            let cnf = Cnf::compile(&circuit, atoms.len(), &mut 10000).unwrap();
            for mask in 0..4 {
                let values = [mask & 1 != 0, mask & 2 != 0];
                let expected = evaluate(&formula, &|a| {
                    Some(values[atoms.iter().position(|b| a == b).unwrap()])
                })
                .unwrap();
                assert_eq!(cnf.accepts(&values, &mut 10000), Ok(expected));
                // Check all auxiliary valuations, independently of extension:
                // false source formulas have no satisfying extension; true
                // ones have exactly one (definitional equivalence).
                let count = (0..1usize << cnf.definitions.len())
                    .filter(|mask| {
                        let mut full = values.to_vec();
                        full.extend((0..cnf.definitions.len()).map(|i| mask & (1 << i) != 0));
                        cnf.satisfied(&full, &mut 10000).unwrap()
                    })
                    .count();
                assert_eq!(count, usize::from(expected));
            }
        }
    }

    #[test]
    fn encoding_is_linear_and_budget_interruptions_are_retryable() {
        let formula = Formula::Or(
            (0..100)
                .map(|_| Formula::And(vec![Formula::True, Formula::False]))
                .collect(),
        );
        let circuit = Circuit::compile(&formula, &[], &mut 10000).unwrap();
        let cnf = Cnf::compile(&circuit, 0, &mut 10000).unwrap();
        assert_eq!(cnf.definitions.len(), 301);
        assert_eq!(cnf.clauses.len(), 602);
        assert_eq!(cnf.clauses.iter().map(Vec::len).sum::<usize>(), 1202);
        for budget in 0..80 {
            assert!(Cnf::compile(&circuit, 0, &mut { budget }).is_err());
            assert!(cnf.accepts(&[], &mut { budget }).is_err());
        }
        assert_eq!(cnf.accepts(&[], &mut 10000), Ok(false));
        let simple = Circuit::compile(&Formula::True, &[], &mut 10).unwrap();
        assert_eq!(
            Cnf::compile(&simple, 0, &mut 10).unwrap().to_string(),
            "(cnf (theory-atoms 0) (auxiliaries 1) (clauses (clause (positive 0)) (clause (positive 0))))"
        );
    }
}
