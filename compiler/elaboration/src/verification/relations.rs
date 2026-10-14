//! Bounded inequality propagation, independent of type propositions. A row is
//! a mathematical polynomial <= 0 established by a checked source condition.
//! This is an incomplete procedure: failure to derive a bound is Unknown.
use super::*;

impl Facts {
    pub(super) fn relation_is_current(
        &self,
        row: &Polynomial,
        limits: &mut Limits,
    ) -> Result<bool, Unknown> {
        for factors in row.terms.keys() {
            limits.step(factors.len().saturating_add(1))?;
            for id in factors {
                if self.value(id.place) != Some(*id) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    pub(super) fn copy_relations(&self, limits: &mut Limits) -> Result<Vec<Polynomial>, Unknown> {
        let mut rows = Vec::new();
        for row in &self.inequalities {
            limits.step(1)?;
            if self.relation_is_current(row, limits)? {
                rows.push(row.copy(limits)?);
            }
        }
        Ok(rows)
    }
    pub(super) fn common_relations(
        &self,
        other: &Self,
        limits: &mut Limits,
    ) -> Result<Vec<Polynomial>, Unknown> {
        let mut rows = Vec::new();
        for row in &self.inequalities {
            if self.relation_is_current(row, limits)?
                && other.relation_is_current(row, limits)?
                && contains_relation(&other.inequalities, row, limits)?
            {
                rows.push(row.copy(limits)?);
            }
        }
        Ok(rows)
    }
    pub(super) fn establish_relation(
        &mut self,
        left: &Polynomial,
        relation: Comparison,
        right: &Polynomial,
        truth: bool,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        // Validate every input before normalization can cancel stale identities.
        left.interval_enclosure(self, limits)?;
        right.interval_enclosure(self, limits)?;
        let relation = if truth { relation } else { relation.negated() };
        let difference = left.subtract(right, limits)?;
        let negated = Polynomial::default().subtract(&difference, limits)?;
        let one = Polynomial::constant(Integer::from_u128(1), limits)?;
        let rows = match relation {
            Comparison::Less => vec![difference.add(&one, limits)?],
            Comparison::LessEqual => vec![difference],
            Comparison::Greater => vec![negated.add(&one, limits)?],
            Comparison::GreaterEqual => vec![negated],
            Comparison::Equal => vec![difference, negated],
            Comparison::NotEqual => {
                let (bound, _) = difference.enclosure(self, limits)?;
                if bound.lower.is_zero() && bound.upper.is_zero() {
                    self.reachable = false;
                }
                // A hole in a range is not a convex inequality.
                Vec::new()
            }
        };
        self.inequalities = self.copy_relations(limits)?;
        for row in rows {
            if !contains_relation(&self.inequalities, &row, limits)? {
                if self.inequalities.len() >= limits.max_terms {
                    return Err(Unknown::TermLimit);
                }
                self.inequalities.push(row);
            }
        }
        Ok(())
    }
    pub(super) fn propagate_relations(&mut self, limits: &mut Limits) -> Result<(), Unknown> {
        if !self.reachable || self.inequalities.is_empty() {
            return Ok(());
        }
        let rows = self.copy_relations(limits)?;
        // Monotone contraction. No unbounded fixed point: every term, integer
        // operation and pass consumes the caller's shared work budget.
        loop {
            limits.step(1)?;
            let mut changed = false;
            if let Some(graph) = super::differences::DifferenceBounds::build(self, limits)? {
                if graph.contradiction() {
                    self.reachable = false;
                    return Ok(());
                }
                changed |= graph.refine(self, limits)?;
                if !self.reachable {
                    return Ok(());
                }
            }
            for row in &rows {
                let (range, _) = row.interval_enclosure(self, limits)?;
                if limits.compare(&range.lower, &Integer::zero())?.is_gt() {
                    self.reachable = false;
                    return Ok(());
                }
                for (factors, coefficient) in &row.terms {
                    limits.step(1)?;
                    if factors.len() != 1 {
                        continue;
                    }
                    let positive = !coefficient.is_negative();
                    let id = factors[0];
                    let mut isolated = Polynomial::default();
                    isolated.insert(factors.clone(), coefficient, limits)?;
                    let remainder = row.subtract(&isolated, limits)?;
                    let (rest, _) = remainder.interval_enclosure(self, limits)?;
                    let old = self.bound(id, limits)?.copy(limits)?;
                    if positive {
                        let numerator = rest.lower.negate(&mut limits.arithmetic)?;
                        let upper = divide_floor(&numerator, coefficient, limits)?;
                        self.refine(id, None, Some(&upper), limits)?;
                    } else {
                        let divisor = coefficient.negate(&mut limits.arithmetic)?;
                        let lower = divide_ceil(&rest.lower, &divisor, limits)?;
                        self.refine(id, Some(&lower), None, limits)?;
                    }
                    if !self.reachable {
                        return Ok(());
                    }
                    changed |= self.bound(id, limits)? != &old;
                }
            }
            if !changed {
                return Ok(());
            }
        }
    }
}
impl Polynomial {
    /// Combine interval enclosure with checked polynomial inequalities. For
    /// q <= 0: p <= upper(p-q), and p >= lower(p+q). These rewrites preserve
    /// dependencies cancelled by plain interval arithmetic (e.g. end-start).
    pub(super) fn enclosure(
        &self,
        facts: &Facts,
        limits: &mut Limits,
    ) -> Result<(Interval, BTreeMap<ValueId, Interval>), Unknown> {
        let (mut result, mut premises) = self.interval_enclosure(facts, limits)?;
        for row in &facts.inequalities {
            limits.step(1)?;
            if !facts.relation_is_current(row, limits)? {
                continue;
            }
            // Even cancelled variables remain dependencies of this evidence.
            let (_, dependencies) = row.interval_enclosure(facts, limits)?;
            premises.extend(dependencies);
            for subtract in [true, false] {
                let remainder = if subtract {
                    self.subtract(row, limits)
                } else {
                    self.add(row, limits)
                };
                let remainder = match remainder {
                    Ok(p) => p,
                    Err(Unknown::TermLimit) => continue,
                    Err(e) => return Err(e),
                };
                let (bound, dependencies) = remainder.interval_enclosure(facts, limits)?;
                premises.extend(dependencies);
                if subtract && limits.compare(&bound.upper, &result.upper)?.is_lt() {
                    result.upper = bound.upper;
                }
                if !subtract && limits.compare(&bound.lower, &result.lower)?.is_gt() {
                    result.lower = bound.lower;
                }
            }
        }
        if let Some(graph) = super::differences::DifferenceBounds::build(facts, limits)? {
            if graph.contradiction() {
                return Err(Unknown::UnreachablePath);
            }
            graph.enclose(self, facts, &mut result, limits)?;
        }
        if limits.compare(&result.lower, &result.upper)?.is_gt() {
            return Err(Unknown::InsufficientBounds);
        }
        Ok((result, premises))
    }
}

/// Equality is also budgeted: a row can contain many large coefficients.
pub(super) fn contains_relation(
    rows: &[Polynomial],
    expected: &Polynomial,
    limits: &mut Limits,
) -> Result<bool, Unknown> {
    for row in rows {
        limits.step(1)?;
        if row.terms.len() != expected.terms.len() {
            continue;
        }
        let mut equal = true;
        for ((a, x), (b, y)) in row.terms.iter().zip(&expected.terms) {
            limits.step(a.len().saturating_add(b.len()).saturating_add(1))?;
            if a != b || !limits.compare(x, y)?.is_eq() {
                equal = false;
                break;
            }
        }
        if equal {
            return Ok(true);
        }
    }
    Ok(false)
}

// Proof-domain rounding only; this does not select Flurry's source `/` ABI.
// Both helpers require a positive divisor and retain arbitrary precision.
pub(super) fn divide_floor(
    numerator: &Integer,
    divisor: &Integer,
    limits: &mut Limits,
) -> Result<Integer, Unknown> {
    let (quotient, remainder) = numerator.div_rem_trunc(divisor, &mut limits.arithmetic)?;
    if numerator.is_negative() && !remainder.is_zero() {
        Ok(quotient.subtract(&Integer::from_u128(1), &mut limits.arithmetic)?)
    } else {
        Ok(quotient)
    }
}
fn divide_ceil(
    numerator: &Integer,
    divisor: &Integer,
    limits: &mut Limits,
) -> Result<Integer, Unknown> {
    let (quotient, remainder) = numerator.div_rem_trunc(divisor, &mut limits.arithmetic)?;
    if !numerator.is_negative() && !remainder.is_zero() {
        Ok(quotient.add(&Integer::from_u128(1), &mut limits.arithmetic)?)
    } else {
        Ok(quotient)
    }
}

impl Facts {
    /// Internal join preparation: each renamed identity denotes that branch's
    /// value of the same merged field. Callers allocate fresh shared versions.
    pub(super) fn rename_values(
        &self,
        mapping: &BTreeMap<ValueId, ValueId>,
        limits: &mut Limits,
    ) -> Result<Self, Unknown> {
        let mut result = self.fork(limits)?;
        let mut replacements = Vec::new();
        for (old, new) in mapping {
            limits.step(1)?;
            replacements.push((*new, self.bound(*old, limits)?.copy(limits)?));
            result.current.remove(&old.place);
        }
        for (new, bound) in replacements {
            result.current.insert(new.place, (new.version, Some(bound)));
        }
        result.conditions.clear();
        for (condition, truth) in self.copy_conditions(limits)? {
            if let Some(condition) = condition.renamed(mapping, &result, true, limits, 0)? {
                result.remember_condition(&condition, truth, limits)?;
            }
        }
        result.inequalities.clear();
        for row in self.copy_relations(limits)? {
            let mut renamed = Polynomial::default();
            for (factors, coefficient) in row.terms {
                limits.step(factors.len().saturating_add(1))?;
                let mut factors: Vec<_> = factors
                    .into_iter()
                    .map(|id| mapping.get(&id).copied().unwrap_or(id))
                    .collect();
                factors.sort_unstable();
                renamed.insert(factors, &coefficient, limits)?;
            }
            if result.relation_is_current(&renamed, limits)? {
                result.inequalities.push(renamed);
            }
        }
        Ok(result)
    }
}
