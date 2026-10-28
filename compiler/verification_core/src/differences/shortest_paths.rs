//! Exact sparse closure. A virtual zero-cost source reaches every vertex for
//! negative-cycle detection; potentials then make all Dijkstra edges nonnegative.
use super::*;

// A private heap keeps every coefficient comparison in the caller's budget.
fn push(
    heap: &mut Vec<(Integer, usize)>,
    value: (Integer, usize),
    limits: &mut Limits,
) -> Result<(), Unknown> {
    limits.step(1)?;
    heap.push(value);
    let mut child = heap.len() - 1;
    while child > 0 {
        let parent = (child - 1) / 2;
        if !limits.compare(&heap[child].0, &heap[parent].0)?.is_lt() {
            break;
        }
        heap.swap(child, parent);
        child = parent;
    }
    Ok(())
}
fn pop(
    heap: &mut Vec<(Integer, usize)>,
    limits: &mut Limits,
) -> Result<Option<(Integer, usize)>, Unknown> {
    limits.step(1)?;
    if heap.is_empty() {
        return Ok(None);
    }
    let result = heap.swap_remove(0);
    let mut parent = 0;
    while parent * 2 + 1 < heap.len() {
        let mut child = parent * 2 + 1;
        if child + 1 < heap.len() && limits.compare(&heap[child + 1].0, &heap[child].0)?.is_lt() {
            child += 1;
        }
        if !limits.compare(&heap[child].0, &heap[parent].0)?.is_lt() {
            break;
        }
        heap.swap(parent, child);
        parent = child;
    }
    Ok(Some(result))
}

impl DifferenceBounds {
    pub(super) fn close_sparse(&mut self, limits: &mut Limits) -> Result<(), Unknown> {
        let size = self.nodes.len();
        let mut edges = vec![Vec::new(); size];
        for (a, row) in self.bounds.iter().enumerate() {
            for (b, weight) in row.iter().enumerate() {
                limits.step(1)?;
                if let Some(weight) = weight {
                    if a != b || weight.is_negative() {
                        edges[a].push((b, limits.integer(weight)?));
                    }
                }
            }
        }
        // Bellman-Ford from a virtual source. A strict improvement in round
        // |V| proves a negative cycle, including in disconnected components.
        let mut potential = vec![Integer::zero(); size];
        for round in 0..size {
            let mut changed = false;
            for (a, outgoing) in edges.iter().enumerate() {
                for (b, weight) in outgoing {
                    limits.step(1)?;
                    let candidate = potential[a].add(weight, &mut limits.arithmetic)?;
                    if limits.compare(&candidate, &potential[*b])?.is_lt() {
                        potential[*b] = candidate;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
            if round + 1 == size {
                self.inconsistent = true;
                return Ok(());
            }
        }
        for (a, outgoing) in edges.iter_mut().enumerate() {
            for (b, weight) in outgoing {
                *weight = weight
                    .add(&potential[a], &mut limits.arithmetic)?
                    .subtract(&potential[*b], &mut limits.arithmetic)?;
                debug_assert!(!weight.is_negative());
            }
        }
        for source in 0..size {
            limits.step(size)?;
            let mut distances = vec![None; size];
            let mut settled = vec![false; size];
            distances[source] = Some(Integer::zero());
            let mut heap = Vec::new();
            push(&mut heap, (Integer::zero(), source), limits)?;
            while let Some((distance, a)) = pop(&mut heap, limits)? {
                if settled[a] {
                    continue;
                }
                settled[a] = true;
                for (b, weight) in &edges[a] {
                    limits.step(1)?;
                    if settled[*b] {
                        continue;
                    }
                    let candidate = distance.add(weight, &mut limits.arithmetic)?;
                    if distances[*b]
                        .as_ref()
                        .map(|old| limits.compare(&candidate, old).map(|c| c.is_lt()))
                        .transpose()?
                        .unwrap_or(true)
                    {
                        distances[*b] = Some(limits.integer(&candidate)?);
                        push(&mut heap, (candidate, *b), limits)?;
                    }
                }
            }
            for (target, distance) in distances.into_iter().enumerate() {
                self.bounds[source][target] = distance
                    .map(|d| {
                        d.subtract(&potential[source], &mut limits.arithmetic)?
                            .add(&potential[target], &mut limits.arithmetic)
                    })
                    .transpose()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_closure_matches_reference_with_negative_edges_cycles_and_disconnected_nodes() {
        for seed in 0..128usize {
            let n = 7;
            let mut reference = vec![vec![None::<i128>; n]; n];
            for (a, row) in reference.iter_mut().enumerate() {
                row[a] = Some(0);
                for (b, cell) in row.iter_mut().enumerate() {
                    let hash = seed.wrapping_mul(1103515245).wrapping_add(a * 97 + b * 173);
                    if a != b && hash % 5 == 0 {
                        *cell = Some((hash % 19) as i128 - 7);
                    }
                }
            }
            // Include a negative self-loop and a disconnected negative cycle.
            if seed % 11 == 0 {
                reference[3][3] = Some(-1);
            }
            if seed % 13 == 0 {
                for a in 0..n {
                    for b in 0..n {
                        if (a >= 5) != (b >= 5) {
                            reference[a][b] = None;
                        }
                    }
                }
                reference[5][6] = Some(-2);
                reference[6][5] = Some(1);
            }
            let mut graph = DifferenceBounds {
                nodes: (0..n)
                    .map(|local| {
                        Some(ValueId {
                            place: PlaceId {
                                owner: 0,
                                local: local as u32,
                            },
                            version: 0,
                        })
                    })
                    .collect(),
                bounds: reference
                    .iter()
                    .map(|row| row.iter().map(|w| w.map(Integer::from_i128)).collect())
                    .collect(),
                inconsistent: false,
            };
            for k in 0..n {
                for a in 0..n {
                    for b in 0..n {
                        if let (Some(x), Some(y)) = (reference[a][k], reference[k][b]) {
                            let candidate = x + y;
                            if reference[a][b].is_none_or(|old| candidate < old) {
                                reference[a][b] = Some(candidate);
                            }
                        }
                    }
                }
            }
            let inconsistent = (0..n).any(|a| reference[a][a].is_some_and(|v| v < 0));
            graph.close_sparse(&mut Limits::default()).unwrap();
            assert_eq!(graph.inconsistent, inconsistent, "seed {seed}");
            if !inconsistent {
                for a in 0..n {
                    for b in 0..n {
                        assert_eq!(
                            graph.bounds[a][b],
                            reference[a][b].map(Integer::from_i128),
                            "seed {seed}, {a}->{b}"
                        );
                    }
                }
            }
        }
    }
}
