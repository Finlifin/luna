//! Query-owned difference graph and feasible potentials. Checkpoints belong to
//! the enclosing theory layers; independent proof replay never trusts this state.
use super::{Graph, IntCongruent, IntLe, UnknownReason, propagation::Explained, tick};
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
    has_congruences: bool,
}

impl State {
    pub fn new(atoms: &[Atom], fuel: &mut usize) -> Result<Self, UnknownReason> {
        let mut variables = BTreeMap::new();
        let mut has_congruences = false;
        for atom in atoms {
            tick(fuel)?;
            match atom {
                Atom::IntLe(atom) => {
                    for term in [&atom.left, &atom.right] {
                        if let Some(id) = term.variable {
                            let next = variables.len() + 1;
                            variables.entry(id).or_insert(next);
                        }
                    }
                }
                Atom::IntCongruent(atom) => {
                    has_congruences = true;
                    if let Some(id) = atom.term.variable {
                        let next = variables.len() + 1;
                        variables.entry(id).or_insert(next);
                    }
                }
                _ => {}
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
            has_congruences,
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

    /// Check consistency of active `IntCongruent` literals against each other
    /// and against `IntLe` bounds / fixed-difference relations.
    pub fn check_congruences(
        &self,
        atoms: &[Atom],
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<bool, UnknownReason> {
        if !self.has_congruences {
            return Ok(true);
        }
        let mut active: Vec<(&IntCongruent, bool)> = Vec::new();
        for (atom, value) in atoms.iter().zip(assignment) {
            tick(fuel)?;
            if let (Atom::IntCongruent(cong), Some(truth)) = (atom, *value) {
                if cong.modulus <= 0 {
                    return Err(UnknownReason::ArithmeticRange);
                }
                if cong.term.variable.is_none() {
                    let holds = cong.term.offset.rem_euclid(cong.modulus)
                        == cong.remainder.rem_euclid(cong.modulus);
                    if holds != truth {
                        return Ok(false);
                    }
                } else {
                    active.push((cong, truth));
                }
            }
        }
        if active.is_empty() {
            return Ok(true);
        }
        let count = self.potential.len();
        let mut positive: Vec<Option<(i128, i128)>> = vec![None; count];
        let mut negative: Vec<Vec<(i128, i128)>> = vec![Vec::new(); count];
        for &(cong, truth) in &active {
            tick(fuel)?;
            let var = cong.term.variable.expect("checked variable presence");
            let idx = self.graph.variables[&var];
            let target = effective_target_residue(cong);
            if !add_congruence(
                idx,
                target,
                cong.modulus,
                truth,
                &mut positive,
                &mut negative,
            )? {
                return Ok(false);
            }
        }
        // Propagate congruences across exact fixed-difference pairs `u - v == c`
        // established by bidirectional shortest paths in the `IntLe` graph.
        let mut constrained_vertices = Vec::new();
        for v in 1..count {
            if positive[v].is_some() || !negative[v].is_empty() {
                constrained_vertices.push(v);
            }
        }
        for v in constrained_vertices {
            let from_v = self.shortest_paths_from(v, fuel)?;
            let to_v = self.shortest_paths_to(v, fuel)?;
            for u in 1..count {
                tick(fuel)?;
                if u == v {
                    continue;
                }
                // `u - v <= from_v[u]` and `v - u <= to_v[u]` => if `from_v[u] == -to_v[u]`,
                // then `u = v + from_v[u]` holds in every feasible integer solution.
                if let (Some(upper_uv), Some(upper_vu)) = (from_v[u], to_v[u])
                    && let Some(neg_vu) = upper_vu.checked_neg()
                    && upper_uv == neg_vu
                {
                    let delta = upper_uv;
                    if let Some((r, m)) = positive[v] {
                        let shifted = shift_residue(r, delta, m);
                        if !add_congruence(u, shifted, m, true, &mut positive, &mut negative)? {
                            return Ok(false);
                        }
                    }
                    let negs = negative[v].clone();
                    for (r, m) in negs {
                        let shifted = shift_residue(r, delta, m);
                        if !add_congruence(u, shifted, m, false, &mut positive, &mut negative)? {
                            return Ok(false);
                        }
                    }
                }
            }
        }
        // Check each constrained variable against its exact `[lo, hi]` constant bounds from `IntLe`.
        let from_zero = self.shortest_paths_from(0, fuel)?;
        let to_zero = self.shortest_paths_to(0, fuel)?;
        for v in 1..count {
            if positive[v].is_none() && negative[v].is_empty() {
                continue;
            }
            let hi = from_zero[v];
            let lo = to_zero[v].and_then(i128::checked_neg);
            if let (Some(lo_val), Some(hi_val)) = (lo, hi) {
                if lo_val > hi_val {
                    return Ok(false);
                }
                if let Some((r0, m0)) = positive[v] {
                    let Some(first) = first_congruent_ge(lo_val, r0, m0) else {
                        return Ok(false);
                    };
                    if first > hi_val {
                        return Ok(false);
                    }
                    if hi_val.saturating_sub(lo_val) <= 64 {
                        let mut cur = first;
                        let mut any_ok = false;
                        while cur <= hi_val {
                            tick(fuel)?;
                            if negative[v].iter().all(|&(nr, nm)| cur.rem_euclid(nm) != nr) {
                                any_ok = true;
                                break;
                            }
                            let Some(next) = cur.checked_add(m0) else {
                                break;
                            };
                            cur = next;
                        }
                        if !any_ok {
                            return Ok(false);
                        }
                    }
                } else if hi_val.saturating_sub(lo_val) <= 64 {
                    let mut any_ok = false;
                    for candidate in lo_val..=hi_val {
                        tick(fuel)?;
                        if negative[v]
                            .iter()
                            .all(|&(nr, nm)| candidate.rem_euclid(nm) != nr)
                        {
                            any_ok = true;
                            break;
                        }
                    }
                    if !any_ok {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    /// Build a concrete integer assignment respecting `IntLe` potentials and active `IntCongruent`s.
    pub fn model_with_congruences(
        &self,
        atoms: &[Atom],
        assignment: &[Option<bool>],
        fuel: &mut usize,
    ) -> Result<BTreeMap<DeclarationId, i128>, UnknownReason> {
        if !self.has_congruences {
            return self.model(fuel);
        }
        let count = self.potential.len();
        let mut values = vec![0i128; count];
        for v in 1..count {
            tick(fuel)?;
            values[v] = self.potential[v]
                .checked_sub(self.potential[0])
                .ok_or(UnknownReason::ArithmeticRange)?;
        }
        let mut positive: Vec<Option<(i128, i128)>> = vec![None; count];
        let mut negative: Vec<Vec<(i128, i128)>> = vec![Vec::new(); count];
        let mut has_congruences = false;
        for (atom, value) in atoms.iter().zip(assignment) {
            tick(fuel)?;
            if let (Atom::IntCongruent(cong), Some(truth)) = (atom, *value)
                && let Some(var) = cong.term.variable
            {
                if cong.modulus <= 0 {
                    return Err(UnknownReason::ArithmeticRange);
                }
                has_congruences = true;
                let idx = self.graph.variables[&var];
                let target = effective_target_residue(cong);
                let _ = add_congruence(
                    idx,
                    target,
                    cong.modulus,
                    truth,
                    &mut positive,
                    &mut negative,
                )?;
            }
        }
        if has_congruences {
            let from_zero = self.shortest_paths_from(0, fuel)?;
            let to_zero = self.shortest_paths_to(0, fuel)?;
            for v in 1..count {
                if positive[v].is_none() && negative[v].is_empty() {
                    continue;
                }
                let satisfies = |x: i128| {
                    positive[v].is_none_or(|(r0, m0)| x.rem_euclid(m0) == r0)
                        && negative[v].iter().all(|&(nr, nm)| x.rem_euclid(nm) != nr)
                };
                if satisfies(values[v]) {
                    continue;
                }
                // Compute neighbor-consistent bounds `[lo, hi]` against current `values`.
                let mut lo = to_zero[v].and_then(i128::checked_neg);
                let mut hi = from_zero[v];
                for &(from, to, bound) in &self.graph.edges {
                    tick(fuel)?;
                    if to == v
                        && from != v
                        && let Some(limit) = values[from].checked_add(bound)
                    {
                        hi = Some(hi.map_or(limit, |prev| prev.min(limit)));
                    }
                    if from == v
                        && to != v
                        && let Some(limit) = values[to].checked_sub(bound)
                    {
                        lo = Some(lo.map_or(limit, |prev| prev.max(limit)));
                    }
                }
                let start = lo.unwrap_or(values[v]);
                let step_base = positive[v].map_or(1, |(_, m0)| m0);
                let first = if let Some((r0, m0)) = positive[v] {
                    first_congruent_ge(start, r0, m0)
                } else {
                    Some(start)
                };
                if let Some(mut cand) = first {
                    for _ in 0..256 {
                        tick(fuel)?;
                        if hi.is_some_and(|h| cand > h) {
                            break;
                        }
                        if satisfies(cand) {
                            values[v] = cand;
                            break;
                        }
                        let Some(next) = cand.checked_add(step_base) else {
                            break;
                        };
                        cand = next;
                    }
                }
            }
        }
        let mut out = BTreeMap::new();
        for (&id, &index) in &self.graph.variables {
            tick(fuel)?;
            out.insert(id, values[index]);
        }
        Ok(out)
    }

    fn shortest_paths_from(
        &self,
        source: usize,
        fuel: &mut usize,
    ) -> Result<Vec<Option<i128>>, UnknownReason> {
        let count = self.potential.len();
        let mut dist: Vec<Option<i128>> = vec![None; count];
        dist[source] = Some(0);
        for _ in 0..count.saturating_sub(1) {
            let mut changed = false;
            for &(from, to, bound) in &self.graph.edges {
                tick(fuel)?;
                let Some(d) = dist[from] else { continue };
                let candidate = d.checked_add(bound).ok_or(UnknownReason::ArithmeticRange)?;
                if dist[to].is_none_or(|prev| candidate < prev) {
                    dist[to] = Some(candidate);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Ok(dist)
    }

    fn shortest_paths_to(
        &self,
        target: usize,
        fuel: &mut usize,
    ) -> Result<Vec<Option<i128>>, UnknownReason> {
        let count = self.potential.len();
        let mut dist: Vec<Option<i128>> = vec![None; count];
        dist[target] = Some(0);
        for _ in 0..count.saturating_sub(1) {
            let mut changed = false;
            for &(from, to, bound) in &self.graph.edges {
                tick(fuel)?;
                let Some(d) = dist[to] else { continue };
                let candidate = d.checked_add(bound).ok_or(UnknownReason::ArithmeticRange)?;
                if dist[from].is_none_or(|prev| candidate < prev) {
                    dist[from] = Some(candidate);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Ok(dist)
    }
}

/// Solve active `IntLe` and `IntCongruent` literals from scratch for
/// independent proof replay without relying on incremental solver state.
pub(crate) fn solve_with_congruences(
    atoms: &[Atom],
    assignment: &[Option<bool>],
    fuel: &mut usize,
) -> Result<Option<BTreeMap<DeclarationId, i128>>, UnknownReason> {
    let mut state = State::new(atoms, fuel)?;
    let checkpoint = Checkpoint::default();
    for (index, (atom, value)) in atoms.iter().zip(assignment).enumerate() {
        tick(fuel)?;
        if let (Atom::IntLe(atom), Some(value)) = (atom, value) {
            state.admit(index, atom, *value, fuel)?;
        }
    }
    if !state.stabilize(checkpoint, fuel)? {
        return Ok(None);
    }
    if !state.check_congruences(atoms, assignment, fuel)? {
        return Ok(None);
    }
    state
        .model_with_congruences(atoms, assignment, fuel)
        .map(Some)
}

fn effective_target_residue(cong: &IntCongruent) -> i128 {
    let rem = cong.remainder.rem_euclid(cong.modulus);
    let off = cong.term.offset.rem_euclid(cong.modulus);
    if rem >= off {
        rem - off
    } else {
        cong.modulus - (off - rem)
    }
}

fn shift_residue(r: i128, delta: i128, m: i128) -> i128 {
    let d = delta.rem_euclid(m);
    let rem = r.rem_euclid(m);
    let sum = rem as u128 + d as u128;
    (sum % m as u128) as i128
}

fn gcd_i128(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.abs()
}

fn mul_mod_u128(a: u128, b: u128, m: u128) -> u128 {
    let mut res = 0u128;
    let mut x = a % m;
    let mut y = b % m;
    while y > 0 {
        if y & 1 == 1 {
            // Avoid overflow: res + x mod m
            res = if res >= m - x { res - (m - x) } else { res + x };
        }
        x = if x >= m - x { x - (m - x) } else { x + x };
        y >>= 1;
    }
    res
}

fn mod_inverse_i128(a: i128, m: i128) -> Option<i128> {
    if m <= 1 {
        return Some(0);
    }
    let mut t: i128 = 0;
    let mut new_t: i128 = 1;
    let mut r: i128 = m;
    let mut new_r: i128 = a.rem_euclid(m);
    while new_r != 0 {
        let q = r / new_r;
        let qt = q.checked_mul(new_t)?;
        let next_t = t.checked_sub(qt)?;
        t = new_t;
        new_t = next_t;
        let qr = q.checked_mul(new_r)?;
        let next_r = r.checked_sub(qr)?;
        r = new_r;
        new_r = next_r;
    }
    if r != 1 {
        return None;
    }
    Some(t.rem_euclid(m))
}

fn crt_combine(
    r1: i128,
    m1: i128,
    r2: i128,
    m2: i128,
) -> Result<Option<(i128, i128)>, UnknownReason> {
    let g = gcd_i128(m1, m2);
    if r1.rem_euclid(g) != r2.rem_euclid(g) {
        return Ok(None);
    }
    let m1_g = m1 / g;
    let m2_g = m2 / g;
    let lcm = m1_g.checked_mul(m2).ok_or(UnknownReason::ArithmeticRange)?;
    let diff = if r2 >= r1 {
        r2 - r1
    } else {
        m2 - ((r1 - r2) % m2)
    };
    let rhs = (diff / g).rem_euclid(m2_g);
    let inv = mod_inverse_i128(m1_g, m2_g).ok_or(UnknownReason::ArithmeticRange)?;
    let k = mul_mod_u128(rhs as u128, inv as u128, m2_g as u128);
    let offset = mul_mod_u128(k, m1 as u128, lcm as u128);
    let combined = ((r1.rem_euclid(lcm) as u128) + offset) % (lcm as u128);
    Ok(Some((combined as i128, lcm)))
}

fn add_congruence(
    idx: usize,
    rem: i128,
    modulus: i128,
    truth: bool,
    positive: &mut [Option<(i128, i128)>],
    negative: &mut [Vec<(i128, i128)>],
) -> Result<bool, UnknownReason> {
    if truth {
        if let Some((r0, m0)) = positive[idx] {
            let Some(merged) = crt_combine(r0, m0, rem, modulus)? else {
                return Ok(false);
            };
            positive[idx] = Some(merged);
        } else {
            positive[idx] = Some((rem, modulus));
        }
    } else {
        if modulus == 1 {
            return Ok(false);
        }
        negative[idx].push((rem, modulus));
    }
    if let Some((r0, m0)) = positive[idx] {
        for &(nr, nm) in &negative[idx] {
            if m0 % nm == 0 && r0.rem_euclid(nm) == nr {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn first_congruent_ge(lo: i128, rem: i128, modulus: i128) -> Option<i128> {
    let rem_lo = lo.rem_euclid(modulus);
    let delta = if rem >= rem_lo {
        rem - rem_lo
    } else {
        modulus - (rem_lo - rem)
    };
    lo.checked_add(delta)
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
