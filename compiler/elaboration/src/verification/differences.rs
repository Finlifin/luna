//! Bounded transitive closure for integer difference constraints. Node zero is
//! the mathematical constant 0; each other node is a versioned program value.
//! Matrix entry [a][b] means a - b <= entry, including inferred paths.
use super::*;
use std::collections::BTreeSet;

pub(super) struct DifferenceBounds {
    nodes: Vec<Option<ValueId>>,
    bounds: Vec<Vec<Option<Integer>>>,
    inconsistent: bool,
}
impl DifferenceBounds {
    pub(super) fn build(facts: &Facts, limits: &mut Limits) -> Result<Option<Self>, Unknown> {
        Self::build_with_nodes(facts, &BTreeSet::new(), limits)
    }
    fn build_with_nodes(
        facts: &Facts,
        extra: &BTreeSet<ValueId>,
        limits: &mut Limits,
    ) -> Result<Option<Self>, Unknown> {
        let mut constraints = Vec::new();
        let mut ids = BTreeSet::new();
        let mut has_pair = false;
        for row in &facts.inequalities {
            if !facts.relation_is_current(row, limits)? {
                continue;
            }
            let Some((left, right, bound)) = difference(row, limits)? else {
                continue;
            };
            has_pair |= left.is_some() && right.is_some();
            ids.extend(left);
            ids.extend(right);
            constraints.push((left, right, bound));
        }
        if !has_pair && extra.len() < 2 {
            return Ok(None);
        }
        limits.step(extra.len())?;
        ids.extend(extra.iter().copied());
        let size = ids.len().saturating_add(1);
        // Matrix storage and closure are optional precision. Oversized graphs
        // fall back to ordinary polynomial/interval reasoning, not allocation.
        let cells = size.checked_mul(size).ok_or(Unknown::TermLimit)?;
        if cells > limits.max_terms {
            return Ok(None);
        }
        limits.step(cells)?;
        let mut nodes = vec![None];
        nodes.extend(ids.into_iter().map(Some));
        let mut graph = Self {
            nodes,
            bounds: vec![vec![None; size]; size],
            inconsistent: false,
        };
        for i in 0..size {
            graph.bounds[i][i] = Some(Integer::zero());
        }
        for (left, right, bound) in constraints {
            let a = graph.nodes.binary_search(&left).unwrap();
            let b = graph.nodes.binary_search(&right).unwrap();
            graph.tighten(a, b, bound, limits)?;
        }
        for i in 1..size {
            let bound = facts.bound(graph.nodes[i].unwrap(), limits)?;
            graph.tighten(i, 0, limits.integer(&bound.upper)?, limits)?;
            graph.tighten(0, i, bound.lower.negate(&mut limits.arithmetic)?, limits)?;
        }
        // Each step composes a-b <= u and b-c <= v into a-c <= u+v.
        for k in 0..size {
            for a in 0..size {
                for b in 0..size {
                    limits.step(1)?;
                    let (Some(left), Some(right)) = (&graph.bounds[a][k], &graph.bounds[k][b])
                    else {
                        continue;
                    };
                    let bound = left.add(right, &mut limits.arithmetic)?;
                    graph.tighten(a, b, bound, limits)?;
                }
            }
            if (0..size).any(|i| {
                graph.bounds[i][i]
                    .as_ref()
                    .is_some_and(Integer::is_negative)
            }) {
                graph.inconsistent = true;
                break;
            }
        }
        Ok(Some(graph))
    }
    fn tighten(
        &mut self,
        a: usize,
        b: usize,
        bound: Integer,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        if self.bounds[a][b]
            .as_ref()
            .map(|old| limits.compare(&bound, old).map(|o| o.is_lt()))
            .transpose()?
            .unwrap_or(true)
        {
            self.bounds[a][b] = Some(bound);
        }
        Ok(())
    }
    pub(super) fn contradiction(&self) -> bool {
        self.inconsistent
    }
    pub(super) fn refine(&self, facts: &mut Facts, limits: &mut Limits) -> Result<bool, Unknown> {
        let mut changed = false;
        for (i, node) in self.nodes.iter().enumerate().skip(1) {
            limits.step(1)?;
            let id = node.unwrap();
            let old = facts.bound(id, limits)?.copy(limits)?;
            let lower = self.bounds[0][i]
                .as_ref()
                .map(|n| n.negate(&mut limits.arithmetic))
                .transpose()?;
            facts.refine(id, lower.as_ref(), self.bounds[i][0].as_ref(), limits)?;
            if !facts.is_reachable() {
                return Ok(true);
            }
            changed |= facts.bound(id, limits)? != &old;
        }
        Ok(changed)
    }
    pub(super) fn enclose(
        &self,
        p: &Polynomial,
        facts: &Facts,
        result: &mut Interval,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        for (a, positive) in &p.terms {
            if a.len() != 1 || positive.is_negative() {
                continue;
            }
            let Ok(i) = self.nodes.binary_search(&Some(a[0])) else {
                continue;
            };
            for (b, negative) in &p.terms {
                limits.step(1)?;
                if b.len() != 1 || !negative.is_negative() {
                    continue;
                }
                let Ok(j) = self.nodes.binary_search(&Some(b[0])) else {
                    continue;
                };
                let magnitude = negative.negate(&mut limits.arithmetic)?;
                let coefficient = if limits.compare(positive, &magnitude)?.is_lt() {
                    limits.integer(positive)?
                } else {
                    magnitude
                };
                let mut paired = Polynomial::default();
                paired.insert(a.clone(), &coefficient, limits)?;
                paired.insert(
                    b.clone(),
                    &coefficient.negate(&mut limits.arithmetic)?,
                    limits,
                )?;
                let remainder = p.subtract(&paired, limits)?;
                let (rest, _) = remainder.interval_enclosure(facts, limits)?;
                if let Some(bound) = &self.bounds[i][j] {
                    let high = bound
                        .multiply(&coefficient, &mut limits.arithmetic)?
                        .add(&rest.upper, &mut limits.arithmetic)?;
                    if limits.compare(&high, &result.upper)?.is_lt() {
                        result.upper = high;
                    }
                }
                if let Some(bound) = &self.bounds[j][i] {
                    let low = rest.lower.subtract(
                        &bound.multiply(&coefficient, &mut limits.arithmetic)?,
                        &mut limits.arithmetic,
                    )?;
                    if limits.compare(&low, &result.lower)?.is_gt() {
                        result.lower = low;
                    }
                }
            }
        }
        // A query can need several independent or shared-coefficient edges.
        // Keep the single-edge candidates above, then compose a disjoint
        // coefficient decomposition. Each coefficient is consumed at most once.
        for upper in [true, false] {
            let bound = self.composed_bound(p, facts, upper, limits)?;
            if upper && limits.compare(&bound, &result.upper)?.is_lt() {
                result.upper = bound;
            } else if !upper && limits.compare(&bound, &result.lower)?.is_gt() {
                result.lower = bound;
            }
        }
        Ok(())
    }

    fn composed_bound(
        &self,
        p: &Polynomial,
        facts: &Facts,
        upper: bool,
        limits: &mut Limits,
    ) -> Result<Integer, Unknown> {
        let mut remainder = p.copy(limits)?;
        let mut total = Integer::zero();
        loop {
            // Prefer the largest per-unit improvement over marginal bounds.
            // This is a bounded heuristic, not an optimal transport solver.
            let mut best = None;
            for (a, positive) in &remainder.terms {
                limits.step(1)?;
                if a.len() != 1 || positive.is_negative() {
                    continue;
                }
                let Ok(i) = self.nodes.binary_search(&Some(a[0])) else {
                    continue;
                };
                for (b, negative) in &remainder.terms {
                    limits.step(1)?;
                    if b.len() != 1 || !negative.is_negative() {
                        continue;
                    }
                    let Ok(j) = self.nodes.binary_search(&Some(b[0])) else {
                        continue;
                    };
                    let (x, y) = if upper { (i, j) } else { (j, i) };
                    let Some(edge) = &self.bounds[x][y] else {
                        continue;
                    };
                    let independent = facts
                        .bound(self.nodes[x].unwrap(), limits)?
                        .upper
                        .subtract(
                            &facts.bound(self.nodes[y].unwrap(), limits)?.lower,
                            &mut limits.arithmetic,
                        )?;
                    let saving = independent.subtract(edge, &mut limits.arithmetic)?;
                    if saving.is_negative() || saving.is_zero() {
                        continue;
                    }
                    if let Some((_, _, _, _, previous)) = &best {
                        if !limits.compare(&saving, previous)?.is_gt() {
                            continue;
                        }
                    }
                    let magnitude = negative.negate(&mut limits.arithmetic)?;
                    let coefficient = if limits.compare(positive, &magnitude)?.is_lt() {
                        limits.integer(positive)?
                    } else {
                        magnitude
                    };
                    best = Some((a[0], b[0], coefficient, limits.integer(edge)?, saving));
                }
            }
            let Some((a, b, coefficient, edge, _)) = best else {
                break;
            };
            let negative = coefficient.negate(&mut limits.arithmetic)?;
            remainder.insert(vec![a], &negative, limits)?;
            remainder.insert(vec![b], &coefficient, limits)?;
            let contribution = coefficient.multiply(&edge, &mut limits.arithmetic)?;
            total = if upper {
                total.add(&contribution, &mut limits.arithmetic)?
            } else {
                total.subtract(&contribution, &mut limits.arithmetic)?
            };
        }
        // Nonlinear terms and unpaired linear coefficients retain the sound
        // ordinary interval enclosure. Nothing is dropped from the query.
        let (rest, _) = remainder.interval_enclosure(facts, limits)?;
        total
            .add(
                if upper { &rest.upper } else { &rest.lower },
                &mut limits.arithmetic,
            )
            .map_err(Into::into)
    }
}
// Recognize k*x-k*y+c <= 0 with integer k>0, including one absent endpoint.
fn difference(
    row: &Polynomial,
    limits: &mut Limits,
) -> Result<Option<(Option<ValueId>, Option<ValueId>, Integer)>, Unknown> {
    let mut constant = Integer::zero();
    let mut positive = None;
    let mut negative = None;
    for (factors, coefficient) in &row.terms {
        limits.step(1)?;
        match factors.as_slice() {
            [] => constant = limits.integer(coefficient)?,
            [id] => {
                let slot = if coefficient.is_negative() {
                    &mut negative
                } else {
                    &mut positive
                };
                if slot.is_some() {
                    return Ok(None);
                }
                *slot = Some((*id, limits.integer(coefficient)?));
            }
            _ => return Ok(None),
        }
    }
    let divisor = match (&positive, &negative) {
        (Some((_, a)), Some((_, b))) => {
            let b = b.negate(&mut limits.arithmetic)?;
            if !limits.compare(a, &b)?.is_eq() {
                return Ok(None);
            }
            limits.integer(a)?
        }
        (Some((_, a)), None) => limits.integer(a)?,
        (None, Some((_, b))) => b.negate(&mut limits.arithmetic)?,
        _ => return Ok(None),
    };
    let bound = super::relations::divide_floor(
        &constant.negate(&mut limits.arithmetic)?,
        &divisor,
        limits,
    )?;
    Ok(Some((
        positive.map(|(id, _)| id),
        negative.map(|(id, _)| id),
        bound,
    )))
}
impl fmt::Display for DifferenceBounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(difference-bounds (inconsistent {}) (nodes",
            self.inconsistent
        )?;
        for node in &self.nodes {
            match node {
                Some(id) => write!(f, " {id}")?,
                None => f.write_str(" zero")?,
            }
        }
        f.write_str(") (edges")?;
        for (a, row) in self.bounds.iter().enumerate() {
            for (b, bound) in row.iter().enumerate() {
                if let Some(bound) = bound {
                    write!(f, " (upper-difference {a} {b} {bound})")?;
                }
            }
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn difference_graph_dump_and_storage_cap_are_explicit() {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let bound = Interval::machine(8, false, &mut limits).unwrap();
        let a = facts
            .assign(
                PlaceId { owner: 1, local: 0 },
                bound.copy(&mut limits).unwrap(),
            )
            .unwrap();
        let b = facts.assign(PlaceId { owner: 1, local: 1 }, bound).unwrap();
        facts
            .assume(
                &Condition::PolynomialCompare {
                    left: Polynomial::variable(a, &mut limits).unwrap(),
                    relation: Comparison::Less,
                    right: Polynomial::variable(b, &mut limits).unwrap(),
                },
                true,
                &mut limits,
            )
            .unwrap();
        let graph = DifferenceBounds::build(&facts, &mut limits)
            .unwrap()
            .unwrap();
        let dump = graph.to_string();
        assert!(dump.starts_with("(difference-bounds (inconsistent false) (nodes zero "));
        assert!(dump.contains("(upper-difference 1 2 (integer (sign negative) (hex \"1\")))"));
        println!("{graph}");
        let mut small = Limits::default();
        small.max_terms = 8;
        assert!(
            DifferenceBounds::build(&facts, &mut small)
                .unwrap()
                .is_none()
        );
        // The cap disables only optional closure precision, never the baseline.
        let p = Polynomial::variable(a, &mut limits).unwrap();
        assert!(matches!(
            p.prove_within(
                &Interval::machine(8, false, &mut limits).unwrap(),
                &facts,
                &mut small
            ),
            Outcome::Proven(_)
        ));
    }
}

/// Keep common implied differences even when alternatives used different
/// intermediates. Endpoints must retain the same value identity on both paths.
pub(super) fn join_guarantees(
    left: &Facts,
    right: &Facts,
    joined: &mut Facts,
    limits: &mut Limits,
) -> Result<(), Unknown> {
    limits.step(joined.current.len())?;
    let mut selected = BTreeSet::new();
    for (place, (version, bound)) in &joined.current {
        let id = ValueId {
            place: *place,
            version: *version,
        };
        if bound.is_none() || left.value(*place) != Some(id) || right.value(*place) != Some(id) {
            continue;
        }
        let a = left.bound(id, limits)?;
        let b = right.bound(id, limits)?;
        // Equal marginal ranges alone cannot improve a joined difference.
        // Relation-bearing nodes are already included by the graph builder.
        if !limits.compare(&a.lower, &b.lower)?.is_eq()
            || !limits.compare(&a.upper, &b.upper)?.is_eq()
        {
            selected.insert(id);
        }
    }
    let Some(a) = DifferenceBounds::build_with_nodes(left, &selected, limits)? else {
        return Ok(());
    };
    let Some(b) = DifferenceBounds::build_with_nodes(right, &selected, limits)? else {
        return Ok(());
    };
    if a.contradiction() || b.contradiction() {
        return Ok(());
    }
    for (i, x) in a.nodes.iter().enumerate().skip(1) {
        let x = x.unwrap();
        if joined.value(x.place) != Some(x) {
            continue;
        }
        let Ok(bi) = b.nodes.binary_search(&Some(x)) else {
            continue;
        };
        for (j, y) in a.nodes.iter().enumerate().skip(1) {
            limits.step(1)?;
            let y = y.unwrap();
            if x == y || joined.value(y.place) != Some(y) {
                continue;
            }
            let Ok(bj) = b.nodes.binary_search(&Some(y)) else {
                continue;
            };
            let (Some(u), Some(v)) = (&a.bounds[i][j], &b.bounds[bi][bj]) else {
                continue;
            };
            let upper = if limits.compare(u, v)?.is_gt() { u } else { v };
            let independent = joined
                .bound(x, limits)?
                .upper
                .subtract(&joined.bound(y, limits)?.lower, &mut limits.arithmetic)?;
            if !limits.compare(upper, &independent)?.is_lt() {
                continue;
            }
            let mut row = Polynomial::variable(x, limits)?
                .subtract(&Polynomial::variable(y, limits)?, limits)?;
            row.insert(vec![], &upper.negate(&mut limits.arithmetic)?, limits)?;
            if !super::relations::contains_relation(&joined.inequalities, &row, limits)? {
                if joined.inequalities.len() >= limits.max_terms {
                    return Ok(());
                }
                joined.inequalities.push(row);
            }
        }
    }
    Ok(())
}

/// Existentially forget intermediate nodes while keeping implied differences
/// between copied values. No source value identity is exported as a target ID.
pub(super) fn project_guarantees(
    facts: &Facts,
    selected: &BTreeSet<ValueId>,
    limits: &mut Limits,
) -> Result<Vec<Polynomial>, Unknown> {
    if selected.len() < 2 {
        return Ok(Vec::new());
    }
    let Some(graph) = DifferenceBounds::build(facts, limits)? else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for (i, x) in graph.nodes.iter().enumerate().skip(1) {
        let x = x.unwrap();
        if !selected.contains(&x) {
            continue;
        }
        for (j, y) in graph.nodes.iter().enumerate().skip(1) {
            limits.step(1)?;
            let y = y.unwrap();
            if x == y || !selected.contains(&y) {
                continue;
            }
            let Some(upper) = &graph.bounds[i][j] else {
                continue;
            };
            let independent = facts
                .bound(x, limits)?
                .upper
                .subtract(&facts.bound(y, limits)?.lower, &mut limits.arithmetic)?;
            if !limits.compare(upper, &independent)?.is_lt() {
                continue;
            }
            let mut row = Polynomial::variable(x, limits)?
                .subtract(&Polynomial::variable(y, limits)?, limits)?;
            row.insert(vec![], &upper.negate(&mut limits.arithmetic)?, limits)?;
            if rows.len() >= limits.max_terms {
                return Ok(rows);
            }
            rows.push(row);
        }
    }
    Ok(rows)
}
