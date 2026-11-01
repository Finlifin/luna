//! Query-local Boolean skeleton. Atom identities are borrowed from the theory
//! catalog; evaluation and propagation never copy type terms.
pub(crate) mod cnf;

use super::{Atom, Formula};
use std::{collections::BTreeSet, sync::Arc};

pub(super) struct Propagation {
    pub truth: Option<bool>,
    pub literals: Vec<Unit>,
    pub conflict_support: Option<BTreeSet<usize>>,
}

pub(super) struct Unit {
    pub index: usize,
    pub value: bool,
    // These are antecedents only; negating the propagated literal completes
    // the independently replayable Boolean conflict. Conjuncts share support.
    pub support: Arc<BTreeSet<usize>>,
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

    #[cfg(test)]
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
            Some(state) => state,
            _ => cnf::Session::new(cnf, assignment, fuel)?,
        };
        let result = state.propagate(cnf, assignment, fuel)?;
        self.propagation = Some(state);
        Ok(result)
    }

    pub(crate) fn propagate_clause_changes(
        &mut self,
        assignment: &[Option<bool>],
        changed: Option<&std::collections::BTreeSet<usize>>,
        fuel: &mut usize,
    ) -> Result<cnf::Propagation, ()> {
        self.clauses(assignment.len(), fuel)?;
        let cnf = self.clauses.as_ref().unwrap();
        // A new epoch invalidates a previous search's assignment history.
        // Failed updates drop the session; the next call rebuilds from the
        // complete current snapshot, regardless of its pending event set.
        let mut state = match (self.propagation.take(), changed) {
            (Some(state), Some(_)) => state,
            _ => cnf::Session::new(cnf, assignment, fuel)?,
        };
        let empty = std::collections::BTreeSet::new();
        let result = state.propagate_changes(cnf, assignment, changed.unwrap_or(&empty), fuel)?;
        self.propagation = Some(state);
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
        let mut pending = vec![(root, true, Arc::new(BTreeSet::new()))];
        while let Some((id, truth, support)) = pending.pop() {
            charge(fuel)?;
            if values[id].is_some() {
                continue;
            }
            match &self.nodes[id] {
                Node::Atom(i) => forced.push(Unit {
                    index: *i,
                    value: truth,
                    support,
                }),
                Node::Not(i) => pending.push((*i, !truth, support)),
                node @ (Node::And(children) | Node::Or(children)) => {
                    let all_required = matches!(node, Node::And(_)) == truth;
                    if all_required {
                        // Charge the frontier before allocating it.
                        *fuel = fuel.checked_sub(children.len()).ok_or(())?;
                        pending.extend(children.iter().rev().map(|i| (*i, truth, support.clone())));
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
                            // Other children establish the guard for descending
                            // through this disjunction (or negated conjunction).
                            *fuel = fuel.checked_sub(children.len() + support.len()).ok_or(())?;
                            let siblings = children.iter().copied().filter(|id| *id != i).collect();
                            let mut antecedents = self.explain_nodes(&values, siblings, fuel)?;
                            antecedents.extend(support.iter().copied());
                            pending.push((i, truth, Arc::new(antecedents)));
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
        self.explain_nodes(values, vec![self.nodes.len() - 1], fuel)
    }
    fn explain_nodes(
        &self,
        values: &[Option<bool>],
        mut pending: Vec<usize>,
        fuel: &mut usize,
    ) -> Result<BTreeSet<usize>, ()> {
        let mut support = BTreeSet::new();
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
                for Unit {
                    index: i,
                    value: truth,
                    support,
                } in result.literals
                {
                    let previous = assignment[i];
                    assert!(previous.is_none());
                    assignment[i] = Some(!truth);
                    let reduced: Vec<_> = assignment
                        .iter()
                        .enumerate()
                        .map(|(index, value)| {
                            if index == i || support.contains(&index) {
                                *value
                            } else {
                                None
                            }
                        })
                        .collect();
                    assert_eq!(
                        evaluate(&formula, &|a| reduced
                            [atoms.iter().position(|b| a == b).unwrap()]),
                        Some(false)
                    );
                    assert!(!support.contains(&i));
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
    fn conjunction_units_share_only_the_guard_antecedents() {
        let atoms: Vec<_> = (0..5)
            .map(|local| {
                Atom::TypeEqual(Equality(
                    Term::Rigid {
                        declaration: nir::DeclarationId { package: 97, local },
                        sort: nir::Sort::Type,
                    },
                    Term::Primitive(Primitive::Bool),
                ))
            })
            .collect();
        let leaf = |i: usize| match &atoms[i] {
            Atom::TypeEqual(e) => Formula::Equal(e.clone()),
            _ => unreachable!(),
        };
        let formula = Formula::And(vec![
            Formula::Or(vec![
                Formula::And(vec![leaf(0), leaf(1)]),
                Formula::And(vec![leaf(2), leaf(3)]),
            ]),
            Formula::Or(vec![leaf(4), leaf(4).negate()]),
        ]);
        let circuit = Circuit::compile(&formula, &atoms, &mut 10000).unwrap();
        let assignment = [Some(false), Some(true), None, None, Some(true)];
        let mut fuel = 10000;
        let result = circuit.propagate(&assignment, &mut fuel).unwrap();
        assert_eq!(
            result
                .literals
                .iter()
                .map(|u| (u.index, u.value))
                .collect::<Vec<_>>(),
            [(2, true), (3, true)]
        );
        assert_eq!(*result.literals[0].support, BTreeSet::from([0]));
        assert!(Arc::ptr_eq(
            &result.literals[0].support,
            &result.literals[1].support
        ));
        let cost = 10000 - fuel;
        for budget in 0..cost {
            assert!(circuit.propagate(&assignment, &mut { budget }).is_err());
            let retried = circuit.propagate(&assignment, &mut 10000).unwrap();
            for unit in retried.literals {
                assert_eq!(*unit.support, BTreeSet::from([0]));
                let mut premises = vec![None; atoms.len()];
                for &index in unit.support.iter() {
                    premises[index] = assignment[index];
                }
                premises[unit.index] = Some(!unit.value);
                assert_eq!(
                    evaluate(&formula, &|a| premises
                        [atoms.iter().position(|b| a == b).unwrap()]),
                    Some(false)
                );
                premises[0] = None;
                assert_eq!(
                    evaluate(&formula, &|a| premises
                        [atoms.iter().position(|b| a == b).unwrap()]),
                    None
                );
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
                (
                    result.truth,
                    result
                        .literals
                        .into_iter()
                        .map(|u| (u.index, u.value))
                        .collect::<Vec<_>>(),
                )
            },
            (None, vec![(0, false)])
        );
        assert_eq!(
            {
                let result = circuit.propagate(&[Some(true)], &mut 1000).unwrap();
                (
                    result.truth,
                    result
                        .literals
                        .into_iter()
                        .map(|u| (u.index, u.value))
                        .collect::<Vec<_>>(),
                )
            },
            (Some(false), vec![])
        );
        assert_eq!(
            {
                let result = circuit.propagate(&[None], &mut 1000).unwrap();
                (
                    result.truth,
                    result
                        .literals
                        .into_iter()
                        .map(|u| (u.index, u.value))
                        .collect::<Vec<_>>(),
                )
            },
            (None, vec![(0, false)])
        );
    }
}
