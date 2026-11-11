//! Paths through admitted source constraints, independent of model potentials.
use super::{Graph, IntLe, IntegerBoundProof, UnknownReason, tick};
use std::collections::{BTreeMap, BTreeSet};

pub(super) type Explained = (usize, IntegerBoundProof, BTreeSet<usize>);

struct Paths {
    distances: Vec<Option<i128>>,
    predecessors: Vec<Option<usize>>,
}

impl Graph {
    fn shortest(&self, source: usize, steps: &mut usize) -> Result<Paths, UnknownReason> {
        let count = self.variables.len() + 1;
        *steps = steps
            .checked_sub(count.saturating_mul(2))
            .ok_or(UnknownReason::BudgetExhausted)?;
        let mut distances: Vec<Option<i128>> = vec![None; count];
        let mut predecessors = vec![None; count];
        distances[source] = Some(0);
        for _ in 1..count {
            let mut changed = false;
            for (edge, &(from, to, bound)) in self.edges.iter().enumerate() {
                tick(steps)?;
                let Some(distance) = distances[from] else {
                    continue;
                };
                let candidate = distance
                    .checked_add(bound)
                    .ok_or(UnknownReason::ArithmeticRange)?;
                if distances[to].is_none_or(|previous| candidate < previous) {
                    distances[to] = Some(candidate);
                    predecessors[to] = Some(edge);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Ok(Paths {
            distances,
            predecessors,
        })
    }
    // The caller supplies current source premises; path certificates own their
    // signed literals and remain checkable after the retained graph is rolled back.
    pub(super) fn explain<'a>(
        &self,
        candidates: impl Iterator<Item = (usize, &'a IntLe)>,
        premise: impl Fn(usize, bool) -> Option<(usize, &'a IntLe)>,
        steps: &mut usize,
    ) -> Result<Option<Explained>, UnknownReason> {
        let mut paths = BTreeMap::new();
        for (index, atom) in candidates {
            tick(steps)?;
            for truth in [true, false] {
                let (from, to, bound) = atom.directed(truth)?;
                let (from, to) = (self.index(from), self.index(to));
                if !paths.contains_key(&from) {
                    paths.insert(from, self.shortest(from, steps)?);
                }
                if paths[&from].distances[to].is_some_and(|distance| distance <= bound) {
                    let mut cursor = to;
                    let mut path = Vec::new();
                    let mut support = std::collections::BTreeSet::from([index]);
                    while cursor != from {
                        tick(steps)?;
                        // Defensive cycle guard; consistent shortest paths need
                        // at most one visit per vertex.
                        if path.len() >= self.variables.len() + 1 {
                            return Ok(None);
                        }
                        let Some(edge) = paths[&from].predecessors[cursor] else {
                            return Ok(None);
                        };
                        let (source_index, truth) = self.sources[edge];
                        let Some((source_index, literal)) = premise(source_index, truth) else {
                            return Ok(None);
                        };
                        path.push((literal.clone(), truth));
                        support.insert(source_index);
                        cursor = self.edges[edge].0;
                    }
                    *steps = steps
                        .checked_sub(path.len())
                        .ok_or(UnknownReason::BudgetExhausted)?;
                    path.reverse();
                    let proof = IntegerBoundProof {
                        path: path.into(),
                        conclusion: std::sync::Arc::new(atom.clone()),
                        value: truth,
                    };
                    if proof.check_bound(steps)? {
                        return Ok(Some((index, proof, support)));
                    }
                    return Ok(None);
                }
            }
        }
        Ok(None)
    }
}
