//! Query-owned difference graph and feasible potentials. Checkpoints belong to
//! the enclosing theory layers; independent proof replay never trusts this state.
use super::{Graph, IntLe, UnknownReason, propagation::Explained, tick};
use crate::Atom;
use nir::DeclarationId;
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Copy, Default)]
pub(crate) struct Checkpoint {
    edges: usize,
    updates: usize,
}

pub(crate) struct State {
    graph: Graph,
    outgoing: Vec<Vec<usize>>,
    potential: Vec<i128>,
    undo: Vec<(usize, i128)>,
}

impl State {
    pub fn new(atoms: &[Atom], fuel: &mut usize) -> Result<Self, UnknownReason> {
        let mut variables = BTreeMap::new();
        for atom in atoms {
            tick(fuel)?;
            if let Atom::IntLe(atom) = atom {
                for term in [&atom.left, &atom.right] {
                    if let Some(id) = term.variable {
                        let next = variables.len() + 1;
                        variables.entry(id).or_insert(next);
                    }
                }
            }
        }
        let count = variables.len() + 1;
        *fuel = fuel
            .checked_sub(count.saturating_mul(2))
            .ok_or(UnknownReason::BudgetExhausted)?;
        Ok(Self {
            graph: Graph {
                variables,
                edges: Vec::new(),
                sources: Vec::new(),
            },
            outgoing: vec![Vec::new(); count],
            potential: vec![0; count],
            undo: Vec::new(),
        })
    }

    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            edges: self.graph.edges.len(),
            updates: self.undo.len(),
        }
    }

    pub fn rollback_work(&self, to: Checkpoint) -> usize {
        (self.graph.edges.len() - to.edges).saturating_add(self.undo.len() - to.updates)
    }

    // Caller either reserves rollback before revoking admitted layers or has
    // already paid for these writes during an uncommitted attempt.
    pub fn rollback(&mut self, to: Checkpoint) {
        while self.undo.len() > to.updates {
            let (vertex, old) = self.undo.pop().unwrap();
            self.potential[vertex] = old;
        }
        while self.graph.edges.len() > to.edges {
            let (from, _, _) = self.graph.edges.pop().unwrap();
            self.graph.sources.pop();
            let removed = self.outgoing[from].pop();
            debug_assert_eq!(removed, Some(self.graph.edges.len()));
        }
    }

    pub fn admit(
        &mut self,
        index: usize,
        atom: &IntLe,
        value: bool,
        fuel: &mut usize,
    ) -> Result<(), UnknownReason> {
        tick(fuel)?;
        let (from, to, bound) = atom.directed(value)?;
        let from = from.variable.map_or(0, |id| self.graph.variables[&id]);
        let to = to.variable.map_or(0, |id| self.graph.variables[&id]);
        self.outgoing[from].push(self.graph.edges.len());
        self.graph.edges.push((from, to, bound));
        self.graph.sources.push((index, value));
        Ok(())
    }

    /// Previously admitted edges already satisfy the retained potential. Start
    /// Bellman-Ford only at sources of added edges, then visit changed vertices.
    /// An improving walk still present after |V| rounds implies a negative cycle.
    pub fn stabilize(&mut self, from: Checkpoint, fuel: &mut usize) -> Result<bool, UnknownReason> {
        if from.edges == self.graph.edges.len() {
            return Ok(true);
        }
        let count = self.potential.len();
        *fuel = fuel
            .checked_sub(count)
            .ok_or(UnknownReason::BudgetExhausted)?;
        let mut queued = vec![false; count];
        let mut frontier = Vec::new();
        for &(vertex, _, _) in &self.graph.edges[from.edges..] {
            tick(fuel)?;
            if !queued[vertex] {
                queued[vertex] = true;
                frontier.push(vertex);
            }
        }
        for round in 0..count {
            for &vertex in &frontier {
                tick(fuel)?;
                queued[vertex] = false;
            }
            let mut next = Vec::new();
            for vertex in frontier {
                tick(fuel)?;
                for &edge in &self.outgoing[vertex] {
                    tick(fuel)?;
                    let (_, to, bound) = self.graph.edges[edge];
                    let candidate = self.potential[vertex]
                        .checked_add(bound)
                        .ok_or(UnknownReason::ArithmeticRange)?;
                    if candidate < self.potential[to] {
                        tick(fuel)?;
                        self.undo.push((to, self.potential[to]));
                        self.potential[to] = candidate;
                        if !queued[to] {
                            queued[to] = true;
                            next.push(to);
                        }
                    }
                }
            }
            if next.is_empty() {
                return Ok(true);
            }
            if round + 1 == count {
                return Ok(false);
            }
            frontier = next;
        }
        unreachable!()
    }

    pub fn explain<'a>(
        &self,
        candidates: &[(usize, &'a IntLe)],
        atoms: &'a [Atom],
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<Option<Explained>, UnknownReason> {
        self.graph.explain(
            candidates.iter().copied(),
            |index, truth| match (atoms.get(index), assignment.get(index)) {
                (Some(Atom::IntLe(atom)), Some(Some(value))) if *value == truth => {
                    Some((index, atom))
                }
                _ => None,
            },
            fuel,
        )
    }

    pub fn model(&self, fuel: &mut usize) -> Result<BTreeMap<DeclarationId, i128>, UnknownReason> {
        self.graph
            .variables
            .iter()
            .map(|(&id, &index)| {
                tick(fuel)?;
                let value = self.potential[index]
                    .checked_sub(self.potential[0])
                    .ok_or(UnknownReason::ArithmeticRange)?;
                Ok((id, value))
            })
            .collect()
    }
}

impl fmt::Display for Checkpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(integer-checkpoint (edges {}) (updates {}))",
            self.edges, self.updates
        )
    }
}
impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(integer-state {} (potentials", self.graph)?;
        for value in &self.potential {
            write!(f, " {value}")?;
        }
        write!(f, ") (undo")?;
        for (vertex, value) in &self.undo {
            write!(f, " (restore {vertex} {value})")?;
        }
        write!(f, "))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IntegerTerm;

    #[test]
    fn retained_paths_keep_signed_sources_across_rollback_and_edge_reuse() {
        use crate::{Equality, ProofError, integer::propagate_explained};
        use nir::{Primitive, Term};
        let variable = |local, offset| {
            IntegerTerm::variable(
                DeclarationId {
                    package: 108,
                    local,
                },
                offset,
            )
        };
        let a = IntLe {
            left: variable(0, 1),
            right: variable(1, 0),
        };
        let b = IntLe {
            left: variable(1, 0),
            right: variable(2, 0),
        };
        let c = IntLe {
            left: variable(0, 1),
            right: variable(2, 0),
        };
        let d = IntLe {
            left: variable(2, 0),
            right: variable(0, 0),
        };
        let reverse = IntLe {
            left: variable(2, 1),
            right: variable(0, 0),
        };
        let ty = Atom::TypeEqual(Equality(
            Term::Primitive(Primitive::Bool),
            Term::Primitive(Primitive::Bool),
        ));
        let atoms = [
            ty.clone(),
            Atom::IntLe(a.clone()),
            Atom::IntLe(c.clone()),
            ty,
            Atom::IntLe(b.clone()),
            Atom::IntLe(d.clone()),
            Atom::IntLe(reverse.clone()),
        ];
        let mut assignment = [None, Some(true), None, None, Some(true), None, None];
        let mut state = State::new(&atoms, &mut 1000).unwrap();
        state.admit(4, &b, true, &mut 1000).unwrap();
        assert!(state.stabilize(Checkpoint::default(), &mut 1000).unwrap());
        let prefix = state.checkpoint();
        state.admit(1, &a, true, &mut 1000).unwrap();
        assert!(state.stabilize(prefix, &mut 1000).unwrap());
        let stable = state.to_string();
        let mut fuel = 10000;
        let (index, proof, support) = state
            .explain(&[(2, &c)], &atoms, &assignment, &mut fuel)
            .unwrap()
            .unwrap();
        assert_eq!((index, proof.value), (2, true));
        assert_eq!(support, [1, 2, 4].into_iter().collect());
        let work = 10000 - fuel;
        for budget in 0..work {
            assert!(matches!(
                state.explain(&[(2, &c)], &atoms, &assignment, &mut { budget }),
                Err(UnknownReason::BudgetExhausted)
            ));
            assert_eq!(state.to_string(), stable);
        }
        let mut cold_fuel = 10000;
        propagate_explained(
            &[
                (4, &b, Some(true)),
                (1, &a, Some(true)),
                (2, &c, None),
                (5, &d, None),
                (6, &reverse, None),
            ],
            &mut cold_fuel,
        )
        .unwrap()
        .unwrap();
        assert!(work < 10000 - cold_fuel);
        let (_, denied, _) = state
            .explain(&[(5, &d)], &atoms, &assignment, &mut 10000)
            .unwrap()
            .unwrap();
        assert!(!denied.value);
        let mut original = assignment;
        original[2] = Some(false);
        proof.verify(&atoms, &original, &mut 10000).unwrap();
        let mut negative = assignment;
        negative[5] = Some(true);
        denied.verify(&atoms, &negative, &mut 10000).unwrap();
        // Even an unsynchronized caller cannot publish a path using a stale
        // polarity. This extra check is not a replacement for layer rollback.
        assignment[1] = Some(false);
        assert!(
            state
                .explain(&[(2, &c)], &atoms, &assignment, &mut 10000)
                .unwrap()
                .is_none()
        );
        state.rollback(prefix);
        assignment[1] = None;
        assert!(
            state
                .explain(&[(2, &c)], &atoms, &assignment, &mut 10000)
                .unwrap()
                .is_none()
        );
        state.admit(6, &reverse, true, &mut 1000).unwrap();
        assert!(state.stabilize(prefix, &mut 1000).unwrap());
        assignment[6] = Some(true);
        let (_, opposite, support) = state
            .explain(&[(2, &c)], &atoms, &assignment, &mut 10000)
            .unwrap()
            .unwrap();
        assert!(!opposite.value);
        assert_eq!(support, [2, 6].into_iter().collect());
        assignment[2] = Some(true);
        opposite.verify(&atoms, &assignment, &mut 10000).unwrap();
        assert_eq!(
            proof.verify(&atoms, &assignment, &mut 10000),
            Err(ProofError::Invalid)
        );
        // The old certificate has no live graph handles and can still be checked
        // under its original assumptions after that graph slot has been reused.
        proof.verify(&atoms, &original, &mut 10000).unwrap();

        state.rollback(prefix);
        assignment[6] = None;
        assignment[2] = Some(false);
        state.admit(2, &c, false, &mut 1000).unwrap();
        assert!(state.stabilize(prefix, &mut 1000).unwrap());
        let (_, from_negative, support) = state
            .explain(&[(5, &d)], &atoms, &assignment, &mut 10000)
            .unwrap()
            .unwrap();
        assert!(from_negative.value);
        assert_eq!(support, [2, 5].into_iter().collect());
        assert_eq!(from_negative.path.as_ref(), &[(c, false)]);
        assignment[5] = Some(false);
        from_negative
            .verify(&atoms, &assignment, &mut 10000)
            .unwrap();
    }

    #[test]
    fn interrupted_extensions_restore_potentials_edges_and_adjacency() {
        let variable = |local| {
            IntegerTerm::variable(
                DeclarationId {
                    package: 105,
                    local,
                },
                0,
            )
        };
        let bounds = [
            IntLe {
                left: variable(0),
                right: IntegerTerm::constant(-1),
            },
            IntLe {
                left: variable(1),
                right: variable(0),
            },
            IntLe {
                left: variable(2),
                right: variable(1),
            },
            IntLe {
                left: IntegerTerm::constant(0),
                right: variable(2),
            },
        ];
        let atoms: Vec<_> = bounds.iter().cloned().map(Atom::IntLe).collect();
        let mut state = State::new(&atoms, &mut 1000).unwrap();
        // Admit the disconnected tail first; the next extension must propagate
        // into previously admitted edges, not just validate its new edge.
        state.admit(1, &bounds[1], true, &mut 1000).unwrap();
        state.admit(2, &bounds[2], true, &mut 1000).unwrap();
        assert!(state.stabilize(Checkpoint::default(), &mut 1000).unwrap());
        let prefix = state.checkpoint();
        let before = state.to_string();
        let mut interrupted = false;
        let mut completed = false;
        for budget in 0..64 {
            let mut fuel = budget;
            let result = state
                .admit(0, &bounds[0], true, &mut fuel)
                .and_then(|()| state.stabilize(prefix, &mut fuel));
            match result {
                Err(UnknownReason::BudgetExhausted) => interrupted = true,
                Ok(true) => {
                    completed = true;
                    let model = state.model(&mut 1000).unwrap();
                    assert!(
                        bounds[..3]
                            .iter()
                            .all(|bound| bound.evaluate(&model) == Some(true))
                    );
                    let consistent = state.checkpoint();
                    state.admit(3, &bounds[3], true, &mut 1000).unwrap();
                    assert!(!state.stabilize(consistent, &mut 1000).unwrap());
                    state.rollback(consistent);
                    assert_eq!(state.model(&mut 1000).unwrap(), model);
                }
                _ => panic!("unexpected extension result: {result:?}"),
            }
            state.rollback(prefix);
            assert_eq!(state.to_string(), before);
        }
        assert!(interrupted && completed);
        assert!(state.stabilize(prefix, &mut 0).unwrap());
        state.rollback(Checkpoint::default());
        assert!(state.outgoing.iter().all(Vec::is_empty));
        assert!(state.potential.iter().all(|value| *value == 0));
    }

    #[test]
    fn arithmetic_capacity_failure_can_be_retracted() {
        let x = IntegerTerm::variable(
            DeclarationId {
                package: 106,
                local: 0,
            },
            0,
        );
        let bounds = [
            IntLe {
                left: x.clone(),
                right: IntegerTerm::constant(i128::MIN),
            },
            IntLe {
                left: x.clone(),
                right: IntegerTerm::variable(x.variable.unwrap(), -1),
            },
            IntLe {
                left: IntegerTerm::constant(i128::MIN),
                right: IntegerTerm::constant(i128::MAX),
            },
        ];
        let atoms: Vec<_> = bounds.iter().cloned().map(Atom::IntLe).collect();
        let mut state = State::new(&atoms, &mut 1000).unwrap();
        state.admit(0, &bounds[0], true, &mut 1000).unwrap();
        assert!(state.stabilize(Checkpoint::default(), &mut 1000).unwrap());
        let prefix = state.checkpoint();
        let before = state.to_string();
        state.admit(1, &bounds[1], true, &mut 1000).unwrap();
        assert_eq!(
            state.stabilize(prefix, &mut 1000),
            Err(UnknownReason::ArithmeticRange)
        );
        state.rollback(prefix);
        assert_eq!(state.to_string(), before);
        assert_eq!(
            state.admit(2, &bounds[2], true, &mut 1000),
            Err(UnknownReason::ArithmeticRange)
        );
        assert_eq!(state.to_string(), before);
        // Negating the self-bound is consistent and must not inherit failure.
        state.admit(1, &bounds[1], false, &mut 1000).unwrap();
        assert!(state.stabilize(prefix, &mut 1000).unwrap());
    }
}
