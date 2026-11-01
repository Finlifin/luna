//! Incremental three-valued circuit evaluation. Pending atoms come from the
//! assignment trail; independent proof replay never trusts this cache.
use super::{Circuit, Node, Propagation, charge};
use std::{collections::BTreeSet, fmt};

pub(super) struct State {
    values: Vec<Option<bool>>,
    dependencies: Option<Dependencies>,
    evaluated_work: usize,
}
struct Dependencies {
    parents: Vec<Vec<usize>>,
    atoms: Vec<Vec<usize>>,
    counts: Vec<[usize; 3]>,
}
fn truth_index(value: Option<bool>) -> usize {
    value.map_or(2, |value| value as usize)
}
impl Dependencies {
    // The full cost is reserved from a prior evaluation's node/edge counts.
    fn new(circuit: &Circuit, atoms_count: usize, values: &[Option<bool>]) -> Self {
        let mut parents = vec![Vec::new(); circuit.nodes.len()];
        let mut atoms = vec![Vec::new(); atoms_count];
        let mut counts = vec![[0; 3]; circuit.nodes.len()];
        for (id, node) in circuit.nodes.iter().enumerate() {
            match node {
                Node::Atom(index) => atoms[*index].push(id),
                Node::Not(input) => parents[*input].push(id),
                Node::And(inputs) | Node::Or(inputs) => {
                    for &input in inputs {
                        parents[input].push(id);
                        counts[id][truth_index(values[input])] += 1;
                    }
                }
                Node::Constant(_) => {}
            }
        }
        Self {
            parents,
            atoms,
            counts,
        }
    }
}
impl State {
    fn new(circuit: &Circuit, assignment: &[Option<bool>], fuel: &mut usize) -> Result<Self, ()> {
        let before = *fuel;
        let (values, _) = circuit.evaluate(assignment, fuel)?;
        Ok(Self {
            values,
            dependencies: None,
            evaluated_work: before - *fuel,
        })
    }
    fn update(
        &mut self,
        circuit: &Circuit,
        assignment: &[Option<bool>],
        changed: &BTreeSet<usize>,
        fuel: &mut usize,
    ) -> Result<(), ()> {
        let Some(dependencies) = &mut self.dependencies else {
            let before = *fuel;
            let (values, index_cost) = circuit.evaluate(assignment, fuel)?;
            self.values = values;
            self.evaluated_work = self.evaluated_work.saturating_add(before - *fuel);
            // Index only observed reuse, with a bounded fraction of remaining
            // query fuel. Cold and short-lived circuits keep direct evaluation.
            if self.evaluated_work >= index_cost && index_cost <= *fuel / 16 {
                *fuel -= index_cost;
                self.dependencies =
                    Some(Dependencies::new(circuit, assignment.len(), &self.values));
            }
            return Ok(());
        };
        let mut dirty = BTreeSet::new();
        for &atom in changed {
            charge(fuel)?;
            for &node in &dependencies.atoms[atom] {
                charge(fuel)?;
                dirty.insert(node);
            }
        }
        // Node IDs are topological. Every changed child is processed before
        // its parents, including duplicated occurrences of a source atom.
        while let Some(node) = dirty.pop_first() {
            let value = match &circuit.nodes[node] {
                Node::And(_) | Node::Or(_) => {
                    charge(fuel)?;
                    let absorbing = matches!(circuit.nodes[node], Node::Or(_));
                    let counts = dependencies.counts[node];
                    if counts[absorbing as usize] > 0 {
                        Some(absorbing)
                    } else if counts[2] > 0 {
                        None
                    } else {
                        Some(!absorbing)
                    }
                }
                _ => circuit.node_value(node, &self.values, assignment, fuel)?,
            };
            if value != self.values[node] {
                let previous = self.values[node];
                self.values[node] = value;
                for &parent in &dependencies.parents[node] {
                    charge(fuel)?;
                    if matches!(circuit.nodes[parent], Node::And(_) | Node::Or(_)) {
                        charge(fuel)?;
                        let counts = &mut dependencies.counts[parent];
                        counts[truth_index(previous)] -= 1;
                        counts[truth_index(value)] += 1;
                    }
                    dirty.insert(parent);
                }
            }
        }
        Ok(())
    }
}
impl Circuit {
    pub(crate) fn propagate_changes(
        &mut self,
        assignment: &[Option<bool>],
        changed: Option<&BTreeSet<usize>>,
        fuel: &mut usize,
    ) -> Result<Propagation, ()> {
        // On tiny circuits, notification dispatch and reverse-edge indexing
        // cost more than walking the handful of nodes directly.
        if self.nodes.len() <= 8 {
            self.evaluation = None;
            let (values, _) = self.evaluate(assignment, fuel)?;
            return self.propagate_values(&values, fuel);
        }
        // Any interrupted operation discards this session. A retry reconstructs
        // the complete current assignment even if only sparse events are sent.
        let state = match (self.evaluation.take(), changed) {
            (Some(mut state), Some(changed)) => {
                state.update(self, assignment, changed, fuel)?;
                state
            }
            _ => State::new(self, assignment, fuel)?,
        };
        let result = self.propagate_values(&state.values, fuel)?;
        self.evaluation = Some(state);
        Ok(result)
    }
}
impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(circuit-values")?;
        for (id, value) in self.values.iter().enumerate() {
            match value {
                Some(value) => write!(f, " (node {id} {value})")?,
                None => write!(f, " (node {id} unknown)")?,
            }
        }
        if let Some(dependencies) = &self.dependencies {
            write!(f, " (counts")?;
            for (id, counts) in dependencies.counts.iter().enumerate() {
                if *counts != [0; 3] {
                    write!(
                        f,
                        " (node {id} (false {}) (true {}) (unknown {}))",
                        counts[0], counts[1], counts[2]
                    )?;
                }
            }
            write!(f, ")")?;
        }
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Atom, Equality, Formula};
    use nir::{DeclarationId, Primitive, Sort, Term};
    fn warm(circuit: &mut Circuit, assignment: &[Option<bool>]) {
        circuit
            .propagate_changes(assignment, None, &mut 10000000)
            .unwrap();
        for _ in 0..16 {
            circuit
                .propagate_changes(assignment, Some(&BTreeSet::new()), &mut 10000000)
                .unwrap();
            if circuit.evaluation.as_ref().unwrap().dependencies.is_some() {
                return;
            }
        }
        panic!("reused circuit never installed its index");
    }
    fn example() -> (Formula, Vec<Atom>) {
        let atoms: Vec<_> = (0..4)
            .map(|local| {
                Atom::TypeEqual(Equality(
                    Term::Rigid {
                        declaration: DeclarationId {
                            package: 107,
                            local,
                        },
                        sort: Sort::Type,
                    },
                    Term::Primitive(Primitive::Bool),
                ))
            })
            .collect();
        let leaf = |index: usize| match &atoms[index] {
            Atom::TypeEqual(equality) => Formula::Equal(equality.clone()),
            _ => unreachable!(),
        };
        let formula = Formula::And(vec![
            Formula::Or(vec![leaf(0), Formula::And(vec![leaf(1), leaf(2).negate()])]),
            Formula::Or(vec![leaf(0).negate(), leaf(3)]),
            Formula::Or(vec![leaf(2), leaf(3).negate()]),
        ]);
        (formula, atoms)
    }
    fn same(warm: Propagation, fresh: Propagation) {
        assert_eq!(warm.truth, fresh.truth);
        assert_eq!(warm.conflict_support, fresh.conflict_support);
        let units = |p: Propagation| {
            p.literals
                .into_iter()
                .map(|u| (u.index, u.value, u.support))
                .collect::<Vec<_>>()
        };
        assert_eq!(units(warm), units(fresh));
    }
    #[test]
    fn sparse_updates_match_full_evaluation_across_partial_assignments() {
        let (formula, atoms) = example();
        let mut circuit = Circuit::compile(&formula, &atoms, &mut 100000).unwrap();
        let mut previous = vec![None; atoms.len()];
        for code in (0..81).chain((0..81).rev()) {
            let mut code = code;
            let assignment: Vec<_> = (0..4)
                .map(|_| {
                    let value = [None, Some(false), Some(true)][code % 3];
                    code /= 3;
                    value
                })
                .collect();
            let changed = previous
                .iter()
                .zip(&assignment)
                .enumerate()
                .filter_map(|(i, (old, new))| (old != new).then_some(i))
                .collect();
            let warm = circuit
                .propagate_changes(&assignment, Some(&changed), &mut 100000)
                .unwrap();
            same(warm, circuit.propagate(&assignment, &mut 100000).unwrap());
            previous = assignment;
        }
        assert!(circuit.evaluation.as_ref().unwrap().dependencies.is_some());
        assert!(
            circuit
                .evaluation
                .as_ref()
                .unwrap()
                .to_string()
                .starts_with("(circuit-values (node 0")
        );
    }
    #[test]
    fn interrupted_updates_and_new_epochs_rebuild_complete_state() {
        let (formula, atoms) = example();
        let initial = [Some(false), Some(true), Some(false), Some(false)];
        let next = [Some(true), Some(true), None, Some(true)];
        let changed = BTreeSet::from([0, 2, 3]);
        let mut interrupted = false;
        let mut completed = false;
        for budget in 0..128 {
            let mut circuit = Circuit::compile(&formula, &atoms, &mut 100000).unwrap();
            warm(&mut circuit, &initial);
            assert!(circuit.evaluation.as_ref().unwrap().dependencies.is_some());
            match circuit.propagate_changes(&next, Some(&changed), &mut { budget }) {
                Ok(_) => completed = true,
                Err(()) => {
                    interrupted = true;
                    assert!(circuit.evaluation.is_none());
                }
            }
            let retry = circuit
                .propagate_changes(&next, Some(&changed), &mut 100000)
                .unwrap();
            same(retry, circuit.propagate(&next, &mut 100000).unwrap());
            let reset = circuit
                .propagate_changes(&initial, None, &mut 100000)
                .unwrap();
            same(reset, circuit.propagate(&initial, &mut 100000).unwrap());
        }
        assert!(interrupted && completed);
    }
    #[test]
    fn wide_gate_updates_count_occurrences_without_scanning_siblings() {
        for disjunction in [false, true] {
            let mut costs = Vec::new();
            for count in [16, 4096] {
                let mut nodes: Vec<_> = (0..count).map(Node::Atom).collect();
                let mut inputs: Vec<_> = (0..count).collect();
                // A shared child occurs twice; reverse edges and counters
                // must retain multiplicity even though the dirty set dedups.
                inputs.push(0);
                nodes.push(if disjunction {
                    Node::Or(inputs)
                } else {
                    Node::And(inputs)
                });
                let mut circuit = Circuit {
                    nodes,
                    clauses: None,
                    propagation: None,
                    evaluation: None,
                };
                let mut assignment = vec![Some(!disjunction); count];
                warm(&mut circuit, &assignment);
                let mut state = circuit.evaluation.take().unwrap();
                let mut work = Vec::new();
                for value in [
                    None,
                    Some(disjunction),
                    Some(!disjunction),
                    Some(disjunction),
                    None,
                    Some(!disjunction),
                ] {
                    assignment[0] = value;
                    let mut fuel = 1000;
                    state
                        .update(&circuit, &assignment, &BTreeSet::from([0]), &mut fuel)
                        .unwrap();
                    work.push(1000 - fuel);
                    assert_eq!(
                        state.values,
                        circuit.evaluate(&assignment, &mut 1000000).unwrap().0
                    );
                    assert_eq!(state.values[count], value);
                    let counts = state.dependencies.as_ref().unwrap().counts[count];
                    assert_eq!(counts.iter().sum::<usize>(), count + 1);
                    assert_eq!(counts[2], if value.is_none() { 2 } else { 0 });
                }
                assert!(state.to_string().contains("(counts (node"));
                costs.push(work);
            }
            assert_eq!(costs[0], costs[1]);
            assert!(
                costs[0].iter().all(|cost| *cost <= 16),
                "wide gate update work {costs:?}"
            );
        }
    }

    #[test]
    fn settled_subtrees_avoid_visiting_unrelated_nodes() {
        let mut costs = Vec::new();
        for count in [16, 4096] {
            // Independent tautologies: changing a Boolean atom changes its
            // occurrences but not the containing disjunction or wide root.
            let mut nodes = Vec::new();
            let mut children = Vec::new();
            for atom in 0..count {
                let base = nodes.len();
                nodes.extend([
                    Node::Atom(atom),
                    Node::Atom(atom),
                    Node::Not(base + 1),
                    Node::Or(vec![base, base + 2]),
                ]);
                children.push(base + 3);
            }
            nodes.push(Node::And(children));
            let mut circuit = Circuit {
                nodes,
                clauses: None,
                propagation: None,
                evaluation: None,
            };
            let mut assignment = vec![Some(false); count];
            warm(&mut circuit, &assignment);
            assert!(circuit.evaluation.as_ref().unwrap().dependencies.is_some());
            assert_eq!(
                circuit
                    .propagate_changes(&assignment, Some(&BTreeSet::new()), &mut 0)
                    .unwrap()
                    .truth,
                Some(true)
            );
            assignment[count / 2] = Some(true);
            let mut fuel = 1000;
            assert_eq!(
                circuit
                    .propagate_changes(&assignment, Some(&BTreeSet::from([count / 2])), &mut fuel)
                    .unwrap()
                    .truth,
                Some(true)
            );
            costs.push(1000 - fuel);
            same(
                circuit
                    .propagate_changes(&assignment, Some(&BTreeSet::new()), &mut 1000000)
                    .unwrap(),
                circuit.propagate(&assignment, &mut 1000000).unwrap(),
            );
        }
        assert_eq!(costs[0], costs[1]);
        assert!(costs[0] <= 16, "sparse update cost {costs:?}");
    }
}
