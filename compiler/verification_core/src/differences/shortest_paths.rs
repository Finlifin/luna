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

fn close_matrix(bounds: &mut [Vec<Option<Integer>>], limits: &mut Limits) -> Result<bool, Unknown> {
    let size = bounds.len();
    let mut edges = vec![Vec::new(); size];
    for (a, row) in bounds.iter().enumerate() {
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
            return Ok(true);
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
            bounds[source][target] = distance
                .map(|d| {
                    d.subtract(&potential[source], &mut limits.arithmetic)?
                        .add(&potential[target], &mut limits.arithmetic)
                })
                .transpose()?;
        }
    }
    Ok(false)
}

impl DifferenceBounds {
    pub(super) fn close_sparse(&mut self, limits: &mut Limits) -> Result<(), Unknown> {
        let size = self.nodes.len();
        if !self.nodes.first().is_some_and(Option::is_none) || size <= 2 {
            if close_matrix(&mut self.bounds, limits)? {
                self.inconsistent = true;
            }
            return Ok(());
        }

        // Hub node 0 connects otherwise independent variable components.
        // Check diagonal negative self-loops, partition 1..size into connected
        // components under variable-to-variable edges, close each `{0} U C_k`,
        // and derive exact cross-component bounds through hub 0.
        for a in 0..size {
            limits.step(1)?;
            if self.bounds[a][a].as_ref().is_some_and(Integer::is_negative) {
                self.inconsistent = true;
                return Ok(());
            }
        }
        let mut adj = vec![Vec::new(); size];
        for a in 1..size {
            for b in (a + 1)..size {
                limits.step(1)?;
                if self.bounds[a][b].is_some() || self.bounds[b][a].is_some() {
                    adj[a].push(b);
                    adj[b].push(a);
                }
            }
        }
        let mut component_of = vec![usize::MAX; size];
        let mut components = Vec::new();
        for start in 1..size {
            if component_of[start] != usize::MAX {
                continue;
            }
            let comp_id = components.len();
            let mut comp = Vec::new();
            let mut stack = vec![start];
            component_of[start] = comp_id;
            while let Some(v) = stack.pop() {
                limits.step(1)?;
                comp.push(v);
                for &next in &adj[v] {
                    if component_of[next] == usize::MAX {
                        component_of[next] = comp_id;
                        stack.push(next);
                    }
                }
            }
            comp.sort_unstable();
            components.push(comp);
        }
        if components.len() <= 1 {
            if close_matrix(&mut self.bounds, limits)? {
                self.inconsistent = true;
            }
            return Ok(());
        }

        self.bounds[0][0] = Some(Integer::zero());
        for comp in &components {
            if comp.len() == 1 {
                let v = comp[0];
                limits.step(1)?;
                if let (Some(v_to_zero), Some(zero_to_v)) = (&self.bounds[v][0], &self.bounds[0][v])
                {
                    let cycle = v_to_zero.add(zero_to_v, &mut limits.arithmetic)?;
                    if cycle.is_negative() {
                        self.inconsistent = true;
                        return Ok(());
                    }
                }
                self.bounds[v][v] = Some(Integer::zero());
                continue;
            }
            let m = comp.len() + 1;
            let mut sub = vec![vec![None; m]; m];
            sub[0][0] = Some(Integer::zero());
            for (k, &global_k) in comp.iter().enumerate() {
                sub[0][k + 1] = self.bounds[0][global_k]
                    .as_ref()
                    .map(|w| limits.integer(w))
                    .transpose()?;
                sub[k + 1][0] = self.bounds[global_k][0]
                    .as_ref()
                    .map(|w| limits.integer(w))
                    .transpose()?;
                for (n, &global_n) in comp.iter().enumerate() {
                    sub[k + 1][n + 1] = self.bounds[global_k][global_n]
                        .as_ref()
                        .map(|w| limits.integer(w))
                        .transpose()?;
                }
            }
            if close_matrix(&mut sub, limits)? {
                self.inconsistent = true;
                return Ok(());
            }
            for (k, &global_k) in comp.iter().enumerate() {
                self.bounds[0][global_k] = sub[0][k + 1].take();
                self.bounds[global_k][0] = sub[k + 1][0].take();
                for (n, &global_n) in comp.iter().enumerate() {
                    self.bounds[global_k][global_n] = sub[k + 1][n + 1].take();
                }
            }
        }

        let (zero_row, rest) = self.bounds.split_at_mut(1);
        let from_zero = &zero_row[0];
        for (a_offset, row) in rest.iter_mut().enumerate() {
            let a = a_offset + 1;
            let (to_zero_slot, tail) = row.split_at_mut(1);
            let Some(a_to_zero) = &to_zero_slot[0] else {
                continue;
            };
            let comp_a = component_of[a];
            for b in 1..size {
                if component_of[b] != comp_a {
                    limits.step(1)?;
                    if let Some(zero_to_b) = &from_zero[b] {
                        tail[b - 1] = Some(a_to_zero.add(zero_to_b, &mut limits.arithmetic)?);
                    }
                }
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

    #[test]
    fn hub_decomposed_closure_matches_floyd_warshall_across_mixed_components() {
        // Node 0 is `None` (constant zero hub); variables split across components of sizes 1, 2, 3, 4.
        let comp_id = |v: usize| match v {
            1 => 0,
            2 | 3 => 1,
            4..=6 => 2,
            _ => 3,
        };
        for seed in 0..256usize {
            let n = 11; // hub 0 + 10 variables
            let mut reference = vec![vec![None::<i128>; n]; n];
            for a in 0..n {
                reference[a][a] = Some(0);
            }
            for a in 1..n {
                let h_out = seed.wrapping_mul(1664525).wrapping_add(a * 131 + 17);
                if h_out % 5 != 0 {
                    reference[a][0] = Some((h_out % 25) as i128 - 4);
                }
                let h_in = seed.wrapping_mul(22695477).wrapping_add(a * 193 + 29);
                if h_in % 5 != 0 {
                    reference[0][a] = Some((h_in % 25) as i128 - 4);
                }
                for b in 1..n {
                    if a != b && comp_id(a) == comp_id(b) {
                        let hash = seed.wrapping_mul(1103515245).wrapping_add(a * 97 + b * 173);
                        if hash % 3 == 0 {
                            reference[a][b] = Some((hash % 17) as i128 - 5);
                        }
                    }
                }
            }
            // Inject occasional singleton/component negative cycles or self-loops.
            if seed % 17 == 0 {
                reference[1][0] = Some(-3);
                reference[0][1] = Some(2);
            }
            if seed % 19 == 0 {
                reference[5][5] = Some(-1);
            }
            let mut nodes = vec![None];
            for local in 0..(n - 1) {
                nodes.push(Some(ValueId {
                    place: PlaceId {
                        owner: 0,
                        local: local as u32,
                    },
                    version: 0,
                }));
            }
            let mut graph = DifferenceBounds {
                nodes,
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
