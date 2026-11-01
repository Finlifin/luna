//! Mathematical integer difference logic. i128 is an implementation capacity,
//! never a source overflow rule: any exhausted arithmetic capacity is Unknown.
use super::UnknownReason;
use nir::DeclarationId;
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IntegerTerm {
    pub variable: Option<DeclarationId>,
    pub offset: i128,
}
impl IntegerTerm {
    pub fn constant(value: i128) -> Self {
        Self {
            variable: None,
            offset: value,
        }
    }
    pub fn variable(variable: DeclarationId, offset: i128) -> Self {
        Self {
            variable: Some(variable),
            offset,
        }
    }
    pub fn evaluate(&self, values: &BTreeMap<DeclarationId, i128>) -> Option<i128> {
        let base = match self.variable {
            Some(id) => *values.get(&id)?,
            None => 0,
        };
        base.checked_add(self.offset)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IntLe {
    pub left: IntegerTerm,
    pub right: IntegerTerm,
}
impl IntLe {
    pub fn evaluate(&self, values: &BTreeMap<DeclarationId, i128>) -> Option<bool> {
        Some(self.left.evaluate(values)? <= self.right.evaluate(values)?)
    }
}

fn tick(steps: &mut usize) -> Result<(), UnknownReason> {
    *steps = steps.checked_sub(1).ok_or(UnknownReason::BudgetExhausted)?;
    Ok(())
}

struct Graph {
    variables: BTreeMap<DeclarationId, usize>,
    edges: Vec<(usize, usize, i128)>,
}
impl IntLe {
    fn directed(&self, value: bool) -> Result<(&IntegerTerm, &IntegerTerm, i128), UnknownReason> {
        // not (x+a <= y+b) means y-x <= a-b-1 over mathematical integers.
        let (from, to, bound) = if value {
            (
                &self.right,
                &self.left,
                self.right.offset.checked_sub(self.left.offset),
            )
        } else {
            (
                &self.left,
                &self.right,
                self.left
                    .offset
                    .checked_sub(self.right.offset)
                    .and_then(|n| n.checked_sub(1)),
            )
        };
        Ok((from, to, bound.ok_or(UnknownReason::ArithmeticRange)?))
    }
}
impl Graph {
    fn new<'a>(
        literals: impl Iterator<Item = (&'a IntLe, Option<bool>)>,
        steps: &mut usize,
    ) -> Result<Self, UnknownReason> {
        let literals: Vec<_> = literals.collect();
        let mut variables = BTreeMap::new();
        for (atom, _) in &literals {
            tick(steps)?;
            for term in [&atom.left, &atom.right] {
                if let Some(id) = term.variable {
                    let next = variables.len() + 1;
                    variables.entry(id).or_insert(next);
                }
            }
        }
        let index = |term: &IntegerTerm| term.variable.map_or(0, |id| variables[&id]);
        let mut edges = Vec::new();
        for (atom, value) in literals {
            let Some(value) = value else { continue };
            let (from, to, bound) = atom.directed(value)?;
            edges.push((index(from), index(to), bound));
        }

        Ok(Self { variables, edges })
    }
    fn index(&self, term: &IntegerTerm) -> usize {
        term.variable.map_or(0, |id| self.variables[&id])
    }
    fn shortest(
        &self,
        source: usize,
        steps: &mut usize,
    ) -> Result<Vec<Option<i128>>, UnknownReason> {
        let count = self.variables.len() + 1;
        *steps = steps
            .checked_sub(count)
            .ok_or(UnknownReason::BudgetExhausted)?;
        let mut distances: Vec<Option<i128>> = vec![None; count];
        distances[source] = Some(0);
        for _ in 1..count {
            let mut changed = false;
            for &(from, to, bound) in &self.edges {
                tick(steps)?;
                let Some(distance) = distances[from] else {
                    continue;
                };
                let candidate = distance
                    .checked_add(bound)
                    .ok_or(UnknownReason::ArithmeticRange)?;
                if distances[to].is_none_or(|previous| candidate < previous) {
                    distances[to] = Some(candidate);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Ok(distances)
    }
}
impl fmt::Display for Graph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(difference-graph (zero 0) (variables")?;
        for (id, index) in &self.variables {
            write!(f, " (variable {index} {id})")?;
        }
        write!(f, ") (edges")?;
        for (from, to, bound) in &self.edges {
            write!(f, " (edge {from} {to} {bound})")?;
        }
        write!(f, "))")
    }
}

/// Propagate only source-path bounds from a consistent assignment. Distances
/// from the model's super-source are deliberately not used as consequences.
pub(super) fn propagate(
    literals: &[(usize, &IntLe, Option<bool>)],
    steps: &mut usize,
) -> Result<Option<(usize, bool)>, UnknownReason> {
    let graph = Graph::new(
        literals.iter().map(|(_, atom, value)| (*atom, *value)),
        steps,
    )?;
    let mut paths = BTreeMap::new();
    for &(index, atom, value) in literals {
        tick(steps)?;
        if value.is_some() {
            continue;
        }
        for truth in [true, false] {
            let (from, to, bound) = atom.directed(truth)?;
            let (from, to) = (graph.index(from), graph.index(to));
            if !paths.contains_key(&from) {
                paths.insert(from, graph.shortest(from, steps)?);
            }
            if paths[&from][to].is_some_and(|distance| distance <= bound) {
                // Ensure this explanation also fits the current independent
                // checker's arithmetic capacity. Overflow is not a source fact.
                let opposite = solve(
                    literals.iter().map(|(i, atom, assigned)| {
                        (*atom, if *i == index { Some(!truth) } else { *assigned })
                    }),
                    steps,
                )?;
                return Ok(opposite.is_none().then_some((index, truth)));
            }
        }
    }
    Ok(None)
}

/// All vertices start at distance zero, equivalently a fresh super-source has
/// zero-cost edges to every vertex. The distinguished zero vertex is shifted
/// back to zero when producing the concrete model.
pub(super) fn solve<'a>(
    literals: impl Iterator<Item = (&'a IntLe, Option<bool>)>,
    steps: &mut usize,
) -> Result<Option<BTreeMap<DeclarationId, i128>>, UnknownReason> {
    let Graph { variables, edges } = Graph::new(literals, steps)?;
    let mut distance = vec![0i128; variables.len() + 1];
    for round in 0..distance.len() {
        let mut changed = false;
        for &(from, to, bound) in &edges {
            tick(steps)?;
            let candidate = distance[from]
                .checked_add(bound)
                .ok_or(UnknownReason::ArithmeticRange)?;
            if distance[to] > candidate {
                distance[to] = candidate;
                changed = true;
                if round + 1 == distance.len() {
                    return Ok(None);
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut values = BTreeMap::new();
    for (variable, index) in variables {
        let value = distance[index]
            .checked_sub(distance[0])
            .ok_or(UnknownReason::ArithmeticRange)?;
        values.insert(variable, value);
    }
    Ok(Some(values))
}

impl fmt::Display for IntegerTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.variable {
            Some(id) => write!(f, "(int-offset {id} {})", self.offset),
            None => write!(f, "(integer {})", self.offset),
        }
    }
}
impl fmt::Display for IntLe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(int-le {} {})", self.left, self.right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn variable(local: u32, offset: i128) -> IntegerTerm {
        IntegerTerm::variable(
            DeclarationId {
                package: 104,
                local,
            },
            offset,
        )
    }
    #[test]
    fn path_propagation_agrees_with_integer_valuations_and_retries() {
        let atoms = [
            IntLe {
                left: variable(0, 1),
                right: variable(1, 0),
            },
            IntLe {
                left: variable(1, 0),
                right: variable(2, 0),
            },
            IntLe {
                left: variable(0, 1),
                right: variable(2, 0),
            },
            IntLe {
                left: variable(2, 0),
                right: variable(0, 0),
            },
        ];
        let mut seen = [false; 2];
        for code in 0..81 {
            let mut code = code;
            let assignment: Vec<_> = (0..4)
                .map(|_| {
                    let value = [None, Some(false), Some(true)][code % 3];
                    code /= 3;
                    value
                })
                .collect();
            if solve(atoms.iter().zip(assignment.iter().copied()), &mut 100000)
                .unwrap()
                .is_none()
            {
                continue;
            }
            let inputs: Vec<_> = atoms
                .iter()
                .enumerate()
                .map(|(i, atom)| (i, atom, assignment[i]))
                .collect();
            if let Some((index, truth)) = propagate(&inputs, &mut 100000).unwrap() {
                seen[truth as usize] = true;
                let mut models = 0;
                for x in -3..=3 {
                    for y in -3..=3 {
                        for z in -3..=3 {
                            let values: BTreeMap<_, _> = [x, y, z]
                                .into_iter()
                                .enumerate()
                                .map(|(local, value)| {
                                    (
                                        DeclarationId {
                                            package: 104,
                                            local: local as u32,
                                        },
                                        value,
                                    )
                                })
                                .collect();
                            if atoms.iter().zip(&assignment).all(|(atom, assigned)| {
                                assigned.is_none_or(|value| atom.evaluate(&values) == Some(value))
                            }) {
                                models += 1;
                                assert_eq!(atoms[index].evaluate(&values), Some(truth));
                            }
                        }
                    }
                }
                assert!(models > 0);
            }
        }
        assert_eq!(seen, [true, true]);
        let inputs: Vec<_> = atoms
            .iter()
            .enumerate()
            .map(|(i, atom)| (i, atom, (i < 2).then_some(true)))
            .collect();
        let mut fuel = 100000;
        assert_eq!(propagate(&inputs, &mut fuel).unwrap(), Some((2, true)));
        for budget in 0..100000 - fuel {
            assert_eq!(
                propagate(&inputs, &mut { budget }),
                Err(UnknownReason::BudgetExhausted)
            );
            assert_eq!(propagate(&inputs, &mut 100000).unwrap(), Some((2, true)));
        }
        let empty: Vec<_> = atoms
            .iter()
            .enumerate()
            .map(|(i, atom)| (i, atom, None))
            .collect();
        assert_eq!(propagate(&empty, &mut 100000).unwrap(), None);
        let graph = Graph::new(
            inputs.iter().map(|(_, atom, value)| (*atom, *value)),
            &mut 1000,
        )
        .unwrap();
        assert!(
            graph
                .to_string()
                .starts_with("(difference-graph (zero 0) (variables")
        );
        assert!(
            graph
                .to_string()
                .contains("(edges (edge 2 1 -1) (edge 3 2 0))")
        );
    }
    #[test]
    fn constant_bounds_and_arithmetic_capacity_do_not_use_machine_overflow() {
        for (left, right, expected) in [(2, 3, true), (3, 2, false), (2, 2, true)] {
            let atom = IntLe {
                left: IntegerTerm::constant(left),
                right: IntegerTerm::constant(right),
            };
            assert_eq!(
                propagate(&[(7, &atom, None)], &mut 1000).unwrap(),
                Some((7, expected))
            );
        }
        let atom = IntLe {
            left: IntegerTerm::constant(i128::MIN),
            right: IntegerTerm::constant(i128::MAX),
        };
        assert_eq!(
            propagate(&[(0, &atom, None)], &mut 1000),
            Err(UnknownReason::ArithmeticRange)
        );
        let premise = IntLe {
            left: variable(0, 0),
            right: variable(1, i128::MIN),
        };
        let implied = IntLe {
            left: variable(0, -1),
            right: variable(1, i128::MIN),
        };
        assert!(
            solve(
                [(&premise, Some(true)), (&implied, None)].into_iter(),
                &mut 1000
            )
            .unwrap()
            .is_some()
        );
        // The path itself fits i128, but replay of the opposite literal runs
        // out of arithmetic capacity before discovering its negative cycle.
        assert_eq!(
            propagate(&[(0, &premise, Some(true)), (1, &implied, None)], &mut 1000),
            Err(UnknownReason::ArithmeticRange)
        );
        let atom = IntLe {
            left: variable(0, 0),
            right: variable(1, 0),
        };
        assert_eq!(propagate(&[(0, &atom, None)], &mut 1000).unwrap(), None);
    }
}
