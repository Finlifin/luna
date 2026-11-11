//! Query-owned difference graph and feasible potentials. Checkpoints belong to
//! the enclosing theory layers; independent proof replay never trusts this state.
use super::{IntLe, UnknownReason, tick};
use crate::Atom;
use nir::DeclarationId;
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Copy, Default)]
pub(crate) struct Checkpoint {
    edges: usize,
    updates: usize,
}

pub(crate) struct State {
    variables: BTreeMap<DeclarationId, usize>,
    edges: Vec<(usize, usize, i128)>,
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
            variables,
            edges: Vec::new(),
            outgoing: vec![Vec::new(); count],
            potential: vec![0; count],
            undo: Vec::new(),
        })
    }

    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            edges: self.edges.len(),
            updates: self.undo.len(),
        }
    }

    pub fn rollback_work(&self, to: Checkpoint) -> usize {
        (self.edges.len() - to.edges).saturating_add(self.undo.len() - to.updates)
    }

    // Caller either reserves rollback before revoking admitted layers or has
    // already paid for these writes during an uncommitted attempt.
    pub fn rollback(&mut self, to: Checkpoint) {
        while self.undo.len() > to.updates {
            let (vertex, old) = self.undo.pop().unwrap();
            self.potential[vertex] = old;
        }
        while self.edges.len() > to.edges {
            let (from, _, _) = self.edges.pop().unwrap();
            let removed = self.outgoing[from].pop();
            debug_assert_eq!(removed, Some(self.edges.len()));
        }
    }

    pub fn admit(
        &mut self,
        atom: &IntLe,
        value: bool,
        fuel: &mut usize,
    ) -> Result<(), UnknownReason> {
        tick(fuel)?;
        let (from, to, bound) = atom.directed(value)?;
        let from = from.variable.map_or(0, |id| self.variables[&id]);
        let to = to.variable.map_or(0, |id| self.variables[&id]);
        self.outgoing[from].push(self.edges.len());
        self.edges.push((from, to, bound));
        Ok(())
    }

    /// Previously admitted edges already satisfy the retained potential. Start
    /// Bellman-Ford only at sources of added edges, then visit changed vertices.
    /// An improving walk still present after |V| rounds implies a negative cycle.
    pub fn stabilize(&mut self, from: Checkpoint, fuel: &mut usize) -> Result<bool, UnknownReason> {
        if from.edges == self.edges.len() {
            return Ok(true);
        }
        let count = self.potential.len();
        *fuel = fuel
            .checked_sub(count)
            .ok_or(UnknownReason::BudgetExhausted)?;
        let mut queued = vec![false; count];
        let mut frontier = Vec::new();
        for &(vertex, _, _) in &self.edges[from.edges..] {
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
                    let (_, to, bound) = self.edges[edge];
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

    pub fn model(&self, fuel: &mut usize) -> Result<BTreeMap<DeclarationId, i128>, UnknownReason> {
        self.variables
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
        write!(f, "(integer-state (zero 0) (variables")?;
        for (id, index) in &self.variables {
            write!(f, " ({id} {index})")?;
        }
        write!(f, ") (edges")?;
        for (from, to, bound) in &self.edges {
            write!(f, " (edge {from} {to} {bound})")?;
        }
        write!(f, ") (potentials")?;
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
        state.admit(&bounds[1], true, &mut 1000).unwrap();
        state.admit(&bounds[2], true, &mut 1000).unwrap();
        assert!(state.stabilize(Checkpoint::default(), &mut 1000).unwrap());
        let prefix = state.checkpoint();
        let before = state.to_string();
        let mut interrupted = false;
        let mut completed = false;
        for budget in 0..64 {
            let mut fuel = budget;
            let result = state
                .admit(&bounds[0], true, &mut fuel)
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
                    state.admit(&bounds[3], true, &mut 1000).unwrap();
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
        state.admit(&bounds[0], true, &mut 1000).unwrap();
        assert!(state.stabilize(Checkpoint::default(), &mut 1000).unwrap());
        let prefix = state.checkpoint();
        let before = state.to_string();
        state.admit(&bounds[1], true, &mut 1000).unwrap();
        assert_eq!(
            state.stabilize(prefix, &mut 1000),
            Err(UnknownReason::ArithmeticRange)
        );
        state.rollback(prefix);
        assert_eq!(state.to_string(), before);
        assert_eq!(
            state.admit(&bounds[2], true, &mut 1000),
            Err(UnknownReason::ArithmeticRange)
        );
        assert_eq!(state.to_string(), before);
        // Negating the self-bound is consistent and must not inherit failure.
        state.admit(&bounds[1], false, &mut 1000).unwrap();
        assert!(state.stabilize(prefix, &mut 1000).unwrap());
    }
}
