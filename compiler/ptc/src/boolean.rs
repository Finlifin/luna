//! Query-local Boolean skeleton. Atom identities are borrowed from the theory
//! catalog; evaluation and propagation never copy type terms.
pub(crate) mod cnf;

use super::{Atom, Formula};
use std::collections::BTreeSet;

pub(super) struct Propagation {
    pub truth: Option<bool>,
    pub literals: Vec<(usize, bool)>,
    pub conflict_support: Option<BTreeSet<usize>>,
}

pub(super) struct Circuit {
    nodes: Vec<Node>,
    clauses: Option<cnf::Cnf>,
    propagation: Option<cnf::Session>,
}
enum Node {
    Constant(bool),
    Atom(usize),
    Not(usize),
    And(Vec<usize>),
    Or(Vec<usize>),
}
fn charge(fuel: &mut usize) -> Result<(), ()> {
    *fuel = fuel.checked_sub(1).ok_or(())?;
    Ok(())
}
impl Circuit {
    pub(super) fn compile(formula: &Formula, atoms: &[Atom], fuel: &mut usize) -> Result<Self, ()> {
        fn visit(
            f: &Formula,
            atoms: &[Atom],
            nodes: &mut Vec<Node>,
            fuel: &mut usize,
        ) -> Result<usize, ()> {
            charge(fuel)?;
            let node = match f {
                Formula::True => Node::Constant(true),
                Formula::False => Node::Constant(false),
                Formula::Not(f) => Node::Not(visit(f, atoms, nodes, fuel)?),
                Formula::And(parts) | Formula::Or(parts) => {
                    let children = parts
                        .iter()
                        .map(|f| visit(f, atoms, nodes, fuel))
                        .collect::<Result<Vec<_>, _>>()?;
                    if matches!(f, Formula::And(_)) {
                        Node::And(children)
                    } else {
                        Node::Or(children)
                    }
                }
                _ => {
                    let mut found = None;
                    for (i, atom) in atoms.iter().enumerate() {
                        charge(fuel)?;
                        let equal = match (f, atom) {
                            (Formula::Equal(a), Atom::TypeEqual(b)) => a == b,
                            (Formula::Kind(a), Atom::Kind(b)) => a == b,
                            (Formula::Trait(a), Atom::Trait(b)) => a == b,
                            (Formula::IntLe(a), Atom::IntLe(b)) => a == b,
                            _ => false,
                        };
                        if equal {
                            found = Some(i);
                            break;
                        }
                    }
                    Node::Atom(found.ok_or(())?)
                }
            };
            let id = nodes.len();
            nodes.push(node);
            Ok(id)
        }
        let mut nodes = Vec::new();
        visit(formula, atoms, &mut nodes, fuel)?;
        Ok(Self {
            nodes,
            clauses: None,
            propagation: None,
        })
    }

    pub(crate) fn clauses(&mut self, atoms: usize, fuel: &mut usize) -> Result<&cnf::Cnf, ()> {
        if self.clauses.is_none() {
            self.clauses = Some(cnf::Cnf::compile(self, atoms, fuel)?);
        }
        Ok(self.clauses.as_ref().unwrap())
    }

    pub(crate) fn propagate_clauses(
        &mut self,
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<cnf::Propagation, ()> {
        self.clauses(assignment.len(), fuel)?;
        let cnf = self.clauses.as_ref().unwrap();
        // Taking ownership makes every interrupted update discard its partial
        // state. Certificates never depend on retaining this optimization.
        let mut state = match self.propagation.take() {
            Some(state) if state.extends(assignment, fuel)? => state,
            _ => cnf::Session::new(cnf, assignment, fuel)?,
        };
        let result = state.propagate(cnf, assignment, fuel)?;
        if !matches!(result, cnf::Propagation::Conflict(_)) {
            self.propagation = Some(state);
        }
        Ok(result)
    }

    /// Return the root truth and forced theory literals. No assignment is
    /// retained across calls, so backtracking cannot leak branch-local facts.
    pub(super) fn propagate(
        &self,
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Propagation, ()> {
        let mut values: Vec<Option<bool>> = Vec::new();
        for node in &self.nodes {
            charge(fuel)?;
            let value = match node {
                Node::Constant(b) => Some(*b),
                Node::Atom(i) => assignment[*i],
                Node::Not(i) => values[*i].map(|b| !b),
                Node::And(children) | Node::Or(children) => {
                    let absorbing = matches!(node, Node::Or(_));
                    let mut result = Some(!absorbing);
                    for i in children {
                        charge(fuel)?;
                        if values[*i] == Some(absorbing) {
                            result = Some(absorbing);
                            break;
                        }
                        if values[*i].is_none() {
                            result = None;
                        }
                    }
                    result
                }
            };
            values.push(value);
        }
        let root = self.nodes.len() - 1;
        if values[root].is_some() {
            let mut support = None;
            if values[root] == Some(false) {
                let allowance = *fuel / 8;
                let mut work = allowance;
                support = self.explain_conflict(&values, &mut work).ok();
                *fuel -= allowance - work;
            }
            return Ok(Propagation {
                truth: values[root],
                literals: Vec::new(),
                conflict_support: support,
            });
        }
        let mut forced = Vec::new();
        let mut pending = vec![(root, true)];
        while let Some((id, truth)) = pending.pop() {
            charge(fuel)?;
            if values[id].is_some() {
                continue;
            }
            match &self.nodes[id] {
                Node::Atom(i) => forced.push((*i, truth)),
                Node::Not(i) => pending.push((*i, !truth)),
                node @ (Node::And(children) | Node::Or(children)) => {
                    let all_required = matches!(node, Node::And(_)) == truth;
                    if all_required {
                        // Charge the frontier before allocating it.
                        *fuel = fuel.checked_sub(children.len()).ok_or(())?;
                        pending.extend(children.iter().rev().map(|i| (*i, truth)));
                    } else {
                        let mut unique = None;
                        for i in children {
                            charge(fuel)?;
                            if values[*i].is_none() {
                                if unique.is_some() {
                                    unique = None;
                                    break;
                                }
                                unique = Some(*i);
                            }
                        }
                        if let Some(i) = unique {
                            pending.push((i, truth));
                        }
                    }
                }
                Node::Constant(_) => {}
            }
        }
        Ok(Propagation {
            truth: None,
            literals: forced,
            conflict_support: None,
        })
    }
    fn explain_conflict(
        &self,
        values: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<BTreeSet<usize>, ()> {
        let mut support = BTreeSet::new();
        let mut pending = vec![self.nodes.len() - 1];
        while let Some(id) = pending.pop() {
            charge(fuel)?;
            let truth = values[id].ok_or(())?;
            match &self.nodes[id] {
                Node::Constant(_) => {}
                Node::Atom(index) => {
                    support.insert(*index);
                }
                Node::Not(input) => pending.push(*input),
                node @ (Node::And(inputs) | Node::Or(inputs)) => {
                    if matches!(node, Node::And(_)) == truth {
                        *fuel = fuel.checked_sub(inputs.len()).ok_or(())?;
                        pending.extend(inputs.iter().copied());
                    } else {
                        let mut selected = None;
                        for input in inputs {
                            charge(fuel)?;
                            if values[*input] == Some(truth) {
                                selected = Some(*input);
                                break;
                            }
                        }
                        pending.push(selected.ok_or(())?);
                    }
                }
            }
        }
        Ok(support)
    }
}

impl std::fmt::Display for Circuit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(boolean-circuit (root {}) (nodes", self.nodes.len() - 1)?;
        for (id, node) in self.nodes.iter().enumerate() {
            write!(f, " (node {id} ")?;
            match node {
                Node::Constant(value) => write!(f, "(constant {value})")?,
                Node::Atom(index) => write!(f, "(atom {index})")?,
                Node::Not(input) => write!(f, "(not {input})")?,
                Node::And(inputs) | Node::Or(inputs) => {
                    write!(
                        f,
                        "({}",
                        if matches!(node, Node::And(_)) {
                            "and"
                        } else {
                            "or"
                        }
                    )?;
                    for input in inputs {
                        write!(f, " {input}")?;
                    }
                    write!(f, ")")?;
                }
            }
            write!(f, ")")?;
        }
        write!(f, "))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Equality, evaluate};
    use nir::{Primitive, Term};

    #[test]
    fn propagation_agrees_with_boolean_semantics_for_partial_assignments() {
        let atoms: Vec<_> = [Primitive::Bool, Primitive::U8, Primitive::U16]
            .into_iter()
            .map(|p| {
                Atom::TypeEqual(Equality(
                    Term::Primitive(p),
                    Term::Primitive(Primitive::Bool),
                ))
            })
            .collect();
        let leaves: Vec<_> = atoms
            .iter()
            .map(|a| match a {
                Atom::TypeEqual(e) => Formula::Equal(e.clone()),
                _ => unreachable!(),
            })
            .collect();
        let mut formulas = vec![Formula::True, Formula::False];
        formulas.extend(leaves.clone());
        for a in &leaves {
            for b in &leaves {
                for c in &leaves {
                    formulas.push(Formula::And(vec![
                        a.clone(),
                        Formula::Or(vec![b.clone().negate(), c.clone()]),
                    ]));
                    formulas.push(
                        Formula::Or(vec![
                            a.clone().negate(),
                            Formula::And(vec![b.clone(), c.clone().negate()]),
                        ])
                        .negate(),
                    );
                }
            }
        }
        for formula in formulas {
            let circuit = Circuit::compile(&formula, &atoms, &mut 100_000).unwrap();
            for code in 0..27 {
                let mut code = code;
                let mut assignment: Vec<_> = (0..3)
                    .map(|_| {
                        let value = [None, Some(false), Some(true)][code % 3];
                        code /= 3;
                        value
                    })
                    .collect();
                let lookup = |a: &Atom| assignment[atoms.iter().position(|b| a == b).unwrap()];
                let expected = evaluate(&formula, &lookup);
                let result = circuit.propagate(&assignment, &mut 100_000).unwrap();
                assert_eq!(result.truth, expected);
                assert_eq!(result.conflict_support.is_some(), expected == Some(false));
                if let Some(support) = &result.conflict_support {
                    let reduced: Vec<_> = assignment
                        .iter()
                        .enumerate()
                        .map(|(i, b)| if support.contains(&i) { *b } else { None })
                        .collect();
                    assert_eq!(
                        evaluate(&formula, &|a| reduced
                            [atoms.iter().position(|b| a == b).unwrap()]),
                        Some(false)
                    );
                }
                for (i, truth) in result.literals {
                    let previous = assignment[i];
                    assert!(previous.is_none());
                    assignment[i] = Some(!truth);
                    assert_eq!(
                        evaluate(&formula, &|a| assignment
                            [atoms.iter().position(|b| a == b).unwrap()]),
                        Some(false)
                    );
                    assignment[i] = previous;
                }
            }
        }
    }

    #[test]
    fn exhaustion_does_not_retain_assignment_or_partial_compilation() {
        let atom = Atom::TypeEqual(Equality(
            Term::Primitive(Primitive::Bool),
            Term::Primitive(Primitive::Bool),
        ));
        let Atom::TypeEqual(e) = &atom else {
            unreachable!()
        };
        let formula = Formula::And(vec![Formula::Equal(e.clone()).negate(), Formula::True]);
        assert!(Circuit::compile(&formula, std::slice::from_ref(&atom), &mut 0).is_err());
        let circuit = Circuit::compile(&formula, &[atom], &mut 1000).unwrap();
        assert_eq!(
            circuit.to_string(),
            "(boolean-circuit (root 3) (nodes (node 0 (atom 0)) (node 1 (not 0)) (node 2 (constant true)) (node 3 (and 1 2))))"
        );
        assert!(circuit.propagate(&[None], &mut 1).is_err());
        assert_eq!(
            {
                let result = circuit.propagate(&[None], &mut 1000).unwrap();
                (result.truth, result.literals)
            },
            (None, vec![(0, false)])
        );
        assert_eq!(
            {
                let result = circuit.propagate(&[Some(true)], &mut 1000).unwrap();
                (result.truth, result.literals)
            },
            (Some(false), vec![])
        );
        assert_eq!(
            {
                let result = circuit.propagate(&[None], &mut 1000).unwrap();
                (result.truth, result.literals)
            },
            (None, vec![(0, false)])
        );
    }
}
