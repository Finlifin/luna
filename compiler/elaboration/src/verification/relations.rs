//! Bounded inequality propagation, independent of type propositions. A row is
//! a mathematical polynomial <= 0 established by a checked source condition.
//! This is an incomplete procedure: failure to derive a bound is Unknown.
use super::*;

/// Ordered evidence with a membership index. Hashes only select candidates;
/// collisions are resolved by the same budgeted structural comparison used
/// for proof replay. Rows are never exposed mutably through this index.
#[derive(Default)]
pub(super) struct RelationStore {
    rows: Vec<Polynomial>,
    buckets: BTreeMap<u64, Vec<usize>>,
}
impl PartialEq for RelationStore {
    fn eq(&self, other: &Self) -> bool {
        self.rows == other.rows
    }
}
impl RelationStore {
    fn fingerprint(row: &Polynomial, limits: &mut Limits) -> Result<u64, Unknown> {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        limits.step(1)?;
        for (factors, coefficient) in &row.terms {
            let limbs = coefficient.bit_len().div_ceil(32);
            if limbs > limits.arithmetic.max_limbs {
                return Err(ArithmeticError::SizeLimit.into());
            }
            limits.step(factors.len().saturating_add(limbs).saturating_add(1))?;
            factors.hash(&mut hash);
            coefficient.hash(&mut hash);
        }
        Ok(hash.finish())
    }
    pub(super) fn contains(&self, row: &Polynomial, limits: &mut Limits) -> Result<bool, Unknown> {
        let key = Self::fingerprint(row, limits)?;
        self.contains_key(key, row, limits)
    }
    fn contains_key(
        &self,
        key: u64,
        row: &Polynomial,
        limits: &mut Limits,
    ) -> Result<bool, Unknown> {
        limits.step(self.buckets.len().max(1).ilog2() as usize + 1)?;
        if let Some(indices) = self.buckets.get(&key) {
            for &index in indices {
                if contains_relation(std::slice::from_ref(&self.rows[index]), row, limits)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    pub(super) fn insert(&mut self, row: Polynomial, limits: &mut Limits) -> Result<bool, Unknown> {
        let key = Self::fingerprint(&row, limits)?;
        if self.contains_key(key, &row, limits)? {
            return Ok(false);
        }
        if self.rows.len() >= limits.max_terms {
            return Err(Unknown::TermLimit);
        }
        self.buckets.entry(key).or_default().push(self.rows.len());
        self.rows.push(row);
        Ok(true)
    }
    pub(super) fn from_rows(rows: Vec<Polynomial>, limits: &mut Limits) -> Result<Self, Unknown> {
        let mut store = Self::default();
        for row in rows {
            store.insert(row, limits)?;
        }
        Ok(store)
    }
    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub(super) fn clear(&mut self) {
        self.rows.clear();
        self.buckets.clear();
    }
    #[cfg(test)]
    pub(super) fn push(&mut self, row: Polynomial) {
        self.insert(row, &mut Limits::default()).unwrap();
    }
}
impl<'a> IntoIterator for &'a RelationStore {
    type Item = &'a Polynomial;
    type IntoIter = std::slice::Iter<'a, Polynomial>;
    fn into_iter(self) -> Self::IntoIter {
        self.rows.iter()
    }
}
#[cfg(test)]
impl From<Vec<Polynomial>> for RelationStore {
    fn from(rows: Vec<Polynomial>) -> Self {
        Self::from_rows(rows, &mut Limits::default()).unwrap()
    }
}

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
                && other.inequalities.contains(row, limits)?
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
        // Validate all identities before deciding whether finite enclosure is
        // available; cancellation must never hide a stale input.
        if !self.relation_is_current(left, limits)? || !self.relation_is_current(right, limits)? {
            return Err(Unknown::InsufficientBounds);
        }
        let unbounded = match (
            left.interval_enclosure(self, limits),
            right.interval_enclosure(self, limits),
        ) {
            (Ok(_), Ok(_)) => false,
            (Err(Unknown::MissingBound(_)), Ok(_) | Err(Unknown::MissingBound(_)))
            | (Ok(_), Err(Unknown::MissingBound(_))) => true,
            (Err(e), _) | (_, Err(e)) => return Err(e),
        };
        if unbounded {
            let condition = Condition::PolynomialCompare {
                left: left.copy(limits)?,
                relation,
                right: right.copy(limits)?,
            };
            match condition.truth_by_enclosure(self, limits, 0) {
                Ok(Some(actual)) => {
                    self.reachable &= actual == truth;
                    return Ok(());
                }
                Ok(None) | Err(Unknown::MissingBound(_)) => {}
                Err(e) => return Err(e),
            }
            return self.remember_condition(&condition, truth, limits);
        }

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
        self.inequalities = RelationStore::from_rows(self.copy_relations(limits)?, limits)?;
        for row in rows {
            if !self.inequalities.contains(&row, limits)? {
                if self.inequalities.len() >= limits.max_terms {
                    return Err(Unknown::TermLimit);
                }
                self.inequalities.insert(row, limits)?;
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
                // The difference graph is already transitively closed. Its
                // interval consequences alone cannot improve a rebuilt graph.
                // Only subsequent polynomial contractions require a new pass.
                graph.refine(self, limits)?;
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
    /// Refine only failed range queries. A derivative enclosed above/below zero
    /// proves coordinate monotonicity throughout the original interval box.
    /// Substituting the corresponding endpoints preserves shared occurrences.
    pub(super) fn enclosure_for_target(
        &self,
        target: &Interval,
        facts: &Facts,
        limits: &mut Limits,
    ) -> Result<(Interval, BTreeMap<ValueId, Interval>), Unknown> {
        let (mut enclosure, premises) = self.enclosure(facts, limits)?;
        if enclosure.within(target, limits)? {
            return Ok((enclosure, premises));
        }
        self.refine_monotone_enclosure(&mut enclosure, &premises, facts, limits)?;
        if !enclosure.within(target, limits)? {
            self.refine_finite_enclosure(&mut enclosure, facts, limits)?;
        }
        if !enclosure.within(target, limits)? {
            self.refine_semidefinite_target(&mut enclosure, target, limits)?;
        }
        Ok((enclosure, premises))
    }

    /// The caller keeps the complete original premises for replay, including
    /// variables eliminated by endpoint substitution.
    pub(super) fn refine_monotone_enclosure(
        &self,
        enclosure: &mut Interval,
        premises: &BTreeMap<ValueId, Interval>,
        facts: &Facts,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        limits.step(self.terms.len())?;
        // Plain interval arithmetic is already exact for a single monomial
        // and for a normalized affine sum over the interval box.
        if self.terms.len() < 2 || self.terms.keys().all(|factors| factors.len() < 2) {
            return Ok(());
        }
        let mut lower = self.copy(limits)?;
        let mut upper = self.copy(limits)?;
        let zero = Integer::zero();
        let mut changed = false;
        for (id, bound) in premises {
            let derivative = self.derivative(*id, limits)?;
            if derivative.terms.is_empty() {
                continue;
            }
            let (slope, _) = derivative.interval_enclosure(facts, limits)?;
            let increasing = if !limits.compare(&slope.lower, &zero)?.is_lt() {
                true
            } else if !limits.compare(&slope.upper, &zero)?.is_gt() {
                false
            } else if limits.compare(&bound.lower, &bound.upper)?.is_eq() {
                true
            } else {
                // Program polynomial inputs are integers. A nonnegative
                // p(x+1)-p(x) proves monotonicity on the integer grid even
                // when the real derivative changes sign between grid points.
                let difference = self.forward_difference(*id, limits)?;
                // The last step is upper-1 -> upper. Do not require the
                // difference at upper, whose destination is outside the box.
                let step_upper = bound
                    .upper
                    .subtract(&Integer::from_u128(1), &mut limits.arithmetic)?;
                let step_domain = Interval::new(limits.integer(&bound.lower)?, step_upper, limits)?;
                let (mut step, _) = difference.interval_enclosure_with_override(
                    facts,
                    Some((*id, &step_domain)),
                    limits,
                )?;
                if limits.compare(&step.lower, &zero)?.is_lt()
                    && limits.compare(&step.upper, &zero)?.is_gt()
                {
                    // Refine the finite difference, not the real derivative:
                    // a quadratic nonnegative at integers may be negative
                    // between them. The original domain is a conservative
                    // superset of the step domain, so intersecting is safe.
                    if !difference.refine_quadratic_enclosure(&mut step, facts, limits)? {
                        difference.refine_separable_polynomials(&mut step, facts, limits)?;
                    }
                }
                if !limits.compare(&step.lower, &zero)?.is_lt() {
                    true
                } else if !limits.compare(&step.upper, &zero)?.is_gt() {
                    false
                } else {
                    continue;
                }
            };
            lower = lower.at_endpoint(
                *id,
                if increasing {
                    &bound.lower
                } else {
                    &bound.upper
                },
                limits,
            )?;
            upper = upper.at_endpoint(
                *id,
                if increasing {
                    &bound.upper
                } else {
                    &bound.lower
                },
                limits,
            )?;
            changed = true;
        }
        if changed {
            let (mut low, _) = lower.interval_enclosure(facts, limits)?;
            let (mut high, _) = upper.interval_enclosure(facts, limits)?;
            // Eliminating a monotone coordinate can leave a non-monotone
            // quadratic. Its interior extremum remains part of the bound.
            if !lower.refine_quadratic_enclosure(&mut low, facts, limits)? {
                lower.refine_separable_polynomials(&mut low, facts, limits)?;
            }
            if !upper.refine_quadratic_enclosure(&mut high, facts, limits)? {
                upper.refine_separable_polynomials(&mut high, facts, limits)?;
            }
            if limits.compare(&low.lower, &enclosure.lower)?.is_gt() {
                enclosure.lower = low.lower;
            }
            if limits.compare(&high.upper, &enclosure.upper)?.is_lt() {
                enclosure.upper = high.upper;
            }
        }
        Ok(())
    }

    /// Exhaust a small integer box when interval/monotonic reasoning cannot
    /// decide the query. Never use a prefix of the box as proof. The volume
    /// cap avoids exponential work; all arithmetic also uses the shared budget.
    pub(super) fn refine_finite_enclosure(
        &self,
        enclosure: &mut Interval,
        facts: &Facts,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        const MAX_POINTS: usize = 256;
        // These forms already have an exact box enclosure. Re-enumerating
        // unresolved affine contract queries wastes the caller's shared budget
        // without adding relational evidence.
        if self.terms.len() < 2 || self.terms.keys().all(|factors| factors.len() < 2) {
            return Ok(());
        }
        if self.refine_univariate_cubic(enclosure, facts, limits)?
            || self.refine_quadratic_enclosure(enclosure, facts, limits)?
            || self.refine_separable_polynomials(enclosure, facts, limits)?
        {
            return Ok(());
        }
        self.refine_diagonally_dominant_quadratic(enclosure, limits)?;
        self.refine_semidefinite_enclosure(enclosure, limits)?;
        let mut domains = BTreeMap::new();
        let mut volume = 1usize;
        for factors in self.terms.keys() {
            limits.step(1 + factors.len())?;
            for id in factors {
                if domains.contains_key(id) {
                    continue;
                }
                let bound = facts.bound(*id, limits)?;
                let width = bound.upper.subtract(&bound.lower, &mut limits.arithmetic)?;
                // The cap is checked before conversion or allocation. Huge and
                // negative domains cannot wrap into a small host-sized count.
                if width.is_negative()
                    || limits
                        .compare(&width, &Integer::from_u128(MAX_POINTS as u128))?
                        .is_ge()
                {
                    return Ok(());
                }
                let width = width.to_u128()? as usize + 1;
                if width > MAX_POINTS / volume {
                    return Ok(());
                }
                volume *= width;
                domains.insert(*id, (bound.copy(limits)?, width));
            }
        }
        let one = Integer::from_u128(1);
        let mut values = domains
            .iter()
            .map(|(id, (bound, _))| Ok((*id, limits.integer(&bound.lower)?)))
            .collect::<Result<BTreeMap<_, _>, Unknown>>()?;
        let mut exact: Option<Interval> = None;
        for point in 0..volume {
            limits.step(1)?;
            let mut sum = Integer::zero();
            for (factors, coefficient) in &self.terms {
                limits.step(1 + factors.len())?;
                let mut term = limits.integer(coefficient)?;
                for id in factors {
                    term = term.multiply(&values[id], &mut limits.arithmetic)?;
                }
                sum = sum.add(&term, &mut limits.arithmetic)?;
            }
            match &mut exact {
                None => exact = Some(Interval::new(limits.integer(&sum)?, sum, limits)?),
                Some(range) => {
                    if limits.compare(&sum, &range.lower)?.is_lt() {
                        range.lower = limits.integer(&sum)?;
                    }
                    if limits.compare(&sum, &range.upper)?.is_gt() {
                        range.upper = sum;
                    }
                }
            }
            if point + 1 < volume {
                for (id, (bound, _)) in &domains {
                    let value = values.get_mut(id).unwrap();
                    if limits.compare(value, &bound.upper)?.is_lt() {
                        *value = value.add(&one, &mut limits.arithmetic)?;
                        break;
                    }
                    *value = limits.integer(&bound.lower)?;
                }
            }
        }
        if let Some(exact) = exact {
            if limits.compare(&exact.lower, &enclosure.lower)?.is_gt() {
                enclosure.lower = exact.lower;
            }
            if limits.compare(&exact.upper, &enclosure.upper)?.is_lt() {
                enclosure.upper = exact.upper;
            }
        }
        Ok(())
    }

    /// Certify a sum of squares without constructing a matrix or dividing
    /// coefficients. For each coordinate, 2*a_i >= sum_j |b_ij| proves
    /// a_i*x_i^2 + sum_{i<j} b_ij*x_i*x_j >= 0. Applying the same rule to
    /// -p proves an upper bound. Linear and cubic terms need other rules.
    fn refine_diagonally_dominant_quadratic(
        &self,
        enclosure: &mut Interval,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        let mut diagonal = BTreeMap::new();
        let mut demand = BTreeMap::new();
        let mut constant = Integer::zero();
        for (factors, coefficient) in &self.terms {
            limits.step(1 + factors.len())?;
            match factors.as_slice() {
                [] => constant = limits.integer(coefficient)?,
                [left, right] if left == right => {
                    diagonal.insert(*left, limits.integer(coefficient)?);
                    demand.entry(*left).or_insert_with(Integer::zero);
                }
                [left, right] => {
                    let magnitude = if coefficient.is_negative() {
                        coefficient.negate(&mut limits.arithmetic)?
                    } else {
                        limits.integer(coefficient)?
                    };
                    for id in [left, right] {
                        let total = demand.entry(*id).or_insert_with(Integer::zero);
                        *total = total.add(&magnitude, &mut limits.arithmetic)?;
                    }
                }
                _ => return Ok(()),
            }
        }
        let mut nonnegative = true;
        let mut nonpositive = true;
        for (id, demand) in &demand {
            limits.step(1)?;
            let zero = Integer::zero();
            let coefficient = diagonal.get(id).unwrap_or(&zero);
            let twice = coefficient.multiply(&Integer::from_u128(2), &mut limits.arithmetic)?;
            nonnegative &= !limits.compare(&twice, demand)?.is_lt();
            let opposite = twice.negate(&mut limits.arithmetic)?;
            nonpositive &= !limits.compare(&opposite, demand)?.is_lt();
        }
        if nonnegative && limits.compare(&constant, &enclosure.lower)?.is_gt() {
            enclosure.lower = limits.integer(&constant)?;
        }
        if nonpositive && limits.compare(&constant, &enclosure.upper)?.is_lt() {
            enclosure.upper = constant;
        }
        Ok(())
    }

    /// Independent coordinate polynomials can be bounded separately, without
    /// enumerating their Cartesian product. Reject cross terms before making
    /// any change: treating x*y as independent squares would be unsound.
    fn refine_separable_polynomials(
        &self,
        enclosure: &mut Interval,
        facts: &Facts,
        limits: &mut Limits,
    ) -> Result<bool, Unknown> {
        let mut components: BTreeMap<ValueId, Polynomial> = BTreeMap::new();
        let mut exact = Interval::singleton(&Integer::zero(), limits)?;
        for (factors, coefficient) in &self.terms {
            limits.step(1 + factors.len())?;
            let Some(id) = factors.first() else {
                exact = exact.add(&Interval::singleton(coefficient, limits)?, limits)?;
                continue;
            };
            if factors.len() > 3 || factors.iter().any(|other| other != id) {
                return Ok(false);
            }
            components
                .entry(*id)
                .or_default()
                .insert(factors.clone(), coefficient, limits)?;
        }
        for component in components.values() {
            let (mut range, _) = component.interval_enclosure(facts, limits)?;
            // Each component keeps its own input interval and version. No
            // Cartesian enumeration or recursive multivariate refinement.
            if !component.refine_univariate_cubic(&mut range, facts, limits)? {
                component.refine_quadratic_enclosure(&mut range, facts, limits)?;
            }
            exact = exact.add(&range, limits)?;
        }
        if limits.compare(&exact.lower, &enclosure.lower)?.is_gt() {
            enclosure.lower = exact.lower;
        }
        if limits.compare(&exact.upper, &enclosure.upper)?.is_lt() {
            enclosure.upper = exact.upper;
        }
        Ok(true)
    }

    /// A univariate quadratic has no extrema away from its endpoints and
    /// vertex. On integers, only floor/ceil of the rational vertex are needed.
    /// Use exact integer division, never floating-point root approximations.
    fn refine_quadratic_enclosure(
        &self,
        enclosure: &mut Interval,
        facts: &Facts,
        limits: &mut Limits,
    ) -> Result<bool, Unknown> {
        let mut variable = None;
        let mut coefficients = [Integer::zero(), Integer::zero(), Integer::zero()];
        for (factors, coefficient) in &self.terms {
            limits.step(1 + factors.len())?;
            if factors.len() > 2 {
                return Ok(false);
            }
            for id in factors {
                if variable.is_some_and(|previous| previous != *id) {
                    return Ok(false);
                }
                variable = Some(*id);
            }
            coefficients[factors.len()] = limits.integer(coefficient)?;
        }
        let Some(id) = variable else { return Ok(false) };
        let [c, b, a] = coefficients;
        if a.is_zero() {
            return Ok(false);
        }
        let bound = facts.bound(id, limits)?.copy(limits)?;
        let mut numerator = b.negate(&mut limits.arithmetic)?;
        let mut denominator = a.multiply(&Integer::from_u128(2), &mut limits.arithmetic)?;
        if denominator.is_negative() {
            numerator = numerator.negate(&mut limits.arithmetic)?;
            denominator = denominator.negate(&mut limits.arithmetic)?;
        }
        let floor = divide_floor(&numerator, &denominator, limits)?;
        let ceil = divide_ceil(&numerator, &denominator, limits)?;
        let candidates = [
            limits.integer(&bound.lower)?,
            limits.integer(&bound.upper)?,
            floor,
            ceil,
        ];
        let mut exact: Option<Interval> = None;
        for value in candidates {
            limits.step(1)?;
            if limits.compare(&value, &bound.lower)?.is_lt()
                || limits.compare(&value, &bound.upper)?.is_gt()
            {
                continue;
            }
            let result = a
                .multiply(&value, &mut limits.arithmetic)?
                .add(&b, &mut limits.arithmetic)?
                .multiply(&value, &mut limits.arithmetic)?
                .add(&c, &mut limits.arithmetic)?;
            match &mut exact {
                None => exact = Some(Interval::new(limits.integer(&result)?, result, limits)?),
                Some(range) => {
                    if limits.compare(&result, &range.lower)?.is_lt() {
                        range.lower = limits.integer(&result)?;
                    }
                    if limits.compare(&result, &range.upper)?.is_gt() {
                        range.upper = result;
                    }
                }
            }
        }
        if let Some(exact) = exact {
            if limits.compare(&exact.lower, &enclosure.lower)?.is_gt() {
                enclosure.lower = exact.lower;
            }
            if limits.compare(&exact.upper, &enclosure.upper)?.is_lt() {
                enclosure.upper = exact.upper;
            }
        }
        Ok(true)
    }

    fn derivative(&self, id: ValueId, limits: &mut Limits) -> Result<Self, Unknown> {
        let mut result = Self::default();
        for (factors, coefficient) in &self.terms {
            limits.step(1 + factors.len())?;
            let count = factors.iter().filter(|factor| **factor == id).count();
            if count == 0 {
                continue;
            }
            let mut remaining = factors.clone();
            remaining.remove(remaining.iter().position(|factor| *factor == id).unwrap());
            let coefficient =
                coefficient.multiply(&Integer::from_u128(count as u128), &mut limits.arithmetic)?;
            result.insert(remaining, &coefficient, limits)?;
        }
        Ok(result)
    }

    /// Expand only the non-cancelling terms of p(x+1)-p(x). Each monomial
    /// has total degree at most three, so the binomial coefficients are tiny
    /// and the difference cannot increase the degree or recurse into itself.
    fn forward_difference(&self, id: ValueId, limits: &mut Limits) -> Result<Self, Unknown> {
        let mut result = Self::default();
        for (factors, coefficient) in &self.terms {
            limits.step(1 + factors.len())?;
            let count = factors.iter().filter(|factor| **factor == id).count();
            for removed in 1..=count {
                limits.step(1 + factors.len())?;
                let multiplier = match (count, removed) {
                    (2, 1) => 2,
                    (3, 1 | 2) => 3,
                    _ => 1,
                };
                let mut remaining = factors.clone();
                for _ in 0..removed {
                    remaining.remove(remaining.iter().position(|factor| *factor == id).unwrap());
                }
                let scaled = coefficient
                    .multiply(&Integer::from_u128(multiplier), &mut limits.arithmetic)?;
                result.insert(remaining, &scaled, limits)?;
            }
        }
        Ok(result)
    }

    fn at_endpoint(
        &self,
        id: ValueId,
        value: &Integer,
        limits: &mut Limits,
    ) -> Result<Self, Unknown> {
        let mut result = Self::default();
        for (factors, coefficient) in &self.terms {
            limits.step(1 + factors.len())?;
            let mut coefficient = limits.integer(coefficient)?;
            let mut remaining = Vec::new();
            for factor in factors {
                if *factor == id {
                    coefficient = coefficient.multiply(value, &mut limits.arithmetic)?;
                } else {
                    remaining.push(*factor);
                }
            }
            result.insert(remaining, &coefficient, limits)?;
        }
        Ok(result)
    }

    /// Combine interval enclosure with checked polynomial inequalities. For
    /// q <= 0: p <= upper(p-q), and p >= lower(p+q). These rewrites preserve
    /// dependencies cancelled by plain interval arithmetic (e.g. end-start).
    pub(super) fn enclosure(
        &self,
        facts: &Facts,
        limits: &mut Limits,
    ) -> Result<(Interval, BTreeMap<ValueId, Interval>), Unknown> {
        let (mut result, mut premises) = self.interval_enclosure(facts, limits)?;
        // A singleton already fixes the value on every state admitted by these
        // interval premises. Relational closure cannot tighten it further; do
        // not rebuild the whole difference graph for a constant-valued query.
        if limits.compare(&result.lower, &result.upper)?.is_eq() {
            return Ok((result, premises));
        }
        for row in &facts.inequalities {
            // With no shared nonconstant monomial, interval evaluation of
            // q +/- row cannot improve q using row <= 0. Keep the complete
            // relation set for the transitive closure below.
            let mut overlaps = false;
            for factors in self.terms.keys().filter(|factors| !factors.is_empty()) {
                limits.step(
                    factors
                        .len()
                        .saturating_add(row.terms.len().max(1).ilog2() as usize)
                        .saturating_add(1),
                )?;
                if row.terms.contains_key(factors) {
                    overlaps = true;
                    break;
                }
            }
            if !overlaps {
                continue;
            }

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
                // Relational evidence can establish a singleton too. The
                // consulted rows and their cancelled variables are already in
                // premises; no additional closure can tighten this result.
                if limits.compare(&result.lower, &result.upper)?.is_eq() {
                    return Ok((result, premises));
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
                result.inequalities.insert(renamed, limits)?;
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod singleton_enclosure_tests {
    use super::*;

    #[test]
    fn relational_singleton_stops_before_unrelated_graph_and_tracks_cancelled_values() {
        let mut setup = Limits::default();
        let mut facts = Facts::default();
        let a = facts
            .assign(
                PlaceId {
                    owner: 92,
                    local: 0,
                },
                Interval::machine(8, false, &mut setup).unwrap(),
            )
            .unwrap();
        let b = facts
            .assign(
                PlaceId {
                    owner: 92,
                    local: 1,
                },
                Interval::machine(8, false, &mut setup).unwrap(),
            )
            .unwrap();
        let query = Polynomial::variable(a, &mut setup)
            .unwrap()
            .subtract(&Polynomial::variable(b, &mut setup).unwrap(), &mut setup)
            .unwrap();
        facts.inequalities.push(query.copy(&mut setup).unwrap());
        facts.inequalities.push(
            Polynomial::constant(Integer::zero(), &mut setup)
                .unwrap()
                .subtract(&query, &mut setup)
                .unwrap(),
        );
        let mut previous = b;
        for local in 2..66 {
            let current = facts
                .assign(
                    PlaceId { owner: 92, local },
                    Interval::machine(8, false, &mut setup).unwrap(),
                )
                .unwrap();
            let row = Polynomial::variable(previous, &mut setup)
                .unwrap()
                .subtract(
                    &Polynomial::variable(current, &mut setup).unwrap(),
                    &mut setup,
                )
                .unwrap();
            facts.inequalities.push(row);
            previous = current;
        }
        let mut limits = Limits::default();
        limits.arithmetic.steps = 1000;
        let (interval, premises) = query.enclosure(&facts, &mut limits).unwrap();
        assert_eq!(interval.lower, Integer::zero());
        assert_eq!(interval.upper, Integer::zero());
        assert_eq!(premises.keys().copied().collect::<Vec<_>>(), vec![a, b]);
        facts
            .assign(b.place, Interval::machine(8, false, &mut setup).unwrap())
            .unwrap();
        assert!(matches!(query.enclosure(&facts, &mut Limits::default()),
            Err(Unknown::StaleValue(id)) if id == b));
    }

    #[test]
    fn exact_interval_avoids_unrelated_closure_and_keeps_version_dependencies() {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let place = PlaceId {
            owner: 91,
            local: 0,
        };
        let seven = Integer::from_u128(7);
        let id = facts
            .assign(place, Interval::singleton(&seven, &mut limits).unwrap())
            .unwrap();
        let query = Polynomial::variable(id, &mut limits).unwrap();
        let mut previous = id;
        for local in 1..17 {
            let current = facts
                .assign(
                    PlaceId { owner: 91, local },
                    Interval::machine(8, false, &mut limits).unwrap(),
                )
                .unwrap();
            let left = Polynomial::variable(previous, &mut limits).unwrap();
            let right = Polynomial::variable(current, &mut limits).unwrap();
            facts
                .inequalities
                .push(left.subtract(&right, &mut limits).unwrap());
            previous = current;
        }
        limits.arithmetic.steps = 100;
        let (interval, premises) = query.enclosure(&facts, &mut limits).unwrap();
        assert_eq!(interval.lower, seven);
        assert_eq!(interval.upper, seven);
        assert_eq!(premises.keys().copied().collect::<Vec<_>>(), vec![id]);
        facts
            .assign(
                place,
                Interval::singleton(&seven, &mut Limits::default()).unwrap(),
            )
            .unwrap();
        assert!(
            matches!(query.enclosure(&facts, &mut Limits::default()), Err(Unknown::StaleValue(stale)) if stale == id)
        );
    }
}

// Exact current-value equalities, independent of interval coincidence. Index
// the two directions of c*(a-b) <= 0 once, instead of searching all rows for
// each reverse inequality. Positive scaling does not change the direction.
pub(super) fn exact_equal_values(
    facts: &Facts,
    limits: &mut Limits,
) -> Result<Vec<(ValueId, ValueId)>, Unknown> {
    fn pair(
        row: &Polynomial,
        facts: &Facts,
        limits: &mut Limits,
    ) -> Result<Option<((ValueId, ValueId), u8)>, Unknown> {
        limits.step(1)?;
        if row.terms.len() != 2 {
            return Ok(None);
        }
        let mut terms = row.terms.iter();
        let (a, ac) = terms.next().unwrap();
        let (b, bc) = terms.next().unwrap();
        if a.len() != 1 || b.len() != 1 {
            return Ok(None);
        }
        let (a, b) = (a[0], b[0]);
        if facts.value(a.place) != Some(a) || facts.value(b.place) != Some(b) {
            return Ok(None);
        }
        let negative = bc.negate(&mut limits.arithmetic).map_err(Unknown::from)?;
        if ac.is_zero() || !limits.compare(ac, &negative)?.is_eq() {
            return Ok(None);
        }
        Ok(Some(((a, b), if ac.is_negative() { 2 } else { 1 })))
    }
    fn insert(
        directions: &mut BTreeMap<(ValueId, ValueId), u8>,
        key: (ValueId, ValueId),
        direction: u8,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        limits.step(1 + directions.len().max(1).ilog2() as usize)?;
        if !directions.contains_key(&key) && directions.len() >= limits.max_terms {
            return Err(Unknown::TermLimit);
        }
        *directions.entry(key).or_default() |= direction;
        Ok(())
    }
    let mut directions = BTreeMap::new();
    for row in &facts.inequalities {
        if let Some((key, direction)) = pair(row, facts, limits)? {
            insert(&mut directions, key, direction, limits)?;
        }
    }
    for (condition, truth) in facts.copy_conditions(limits)? {
        let Condition::PolynomialCompare {
            left,
            relation,
            right,
        } = condition
        else {
            continue;
        };
        if !matches!(
            (relation, truth),
            (Comparison::Equal, true) | (Comparison::NotEqual, false)
        ) {
            continue;
        }
        let row = left.subtract(&right, limits)?;
        if let Some((key, _)) = pair(&row, facts, limits)? {
            insert(&mut directions, key, 3, limits)?;
        }
    }
    limits.step(directions.len())?;
    Ok(directions
        .into_iter()
        .filter_map(|(pair, directions)| (directions == 3).then_some(pair))
        .collect())
}

#[cfg(test)]
mod equality_index_tests {
    use super::*;

    fn row(a: ValueId, ac: i128, b: ValueId, bc: i128) -> Polynomial {
        Polynomial {
            terms: BTreeMap::from([
                (vec![a], Integer::from_i128(ac)),
                (vec![b], Integer::from_i128(bc)),
            ]),
        }
    }

    #[test]
    fn indexed_equalities_require_both_directions_and_current_values() {
        for (ac, bc, reverse_a, reverse_b, equal) in [
            (3, -3, -5, 5, true),
            (-3, 3, 5, -5, true),
            (3, -3, 5, -5, false),
            (2, -3, -2, 3, false),
        ] {
            let mut limits = Limits::default();
            let mut facts = Facts::default();
            let a = facts.invalidate(PlaceId { owner: 1, local: 0 }).unwrap();
            let b = facts.invalidate(PlaceId { owner: 1, local: 1 }).unwrap();
            facts.inequalities = vec![row(a, ac, b, bc), row(a, reverse_a, b, reverse_b)].into();
            let found = exact_equal_values(&facts, &mut limits).unwrap();
            assert_eq!(found, if equal { vec![(a, b)] } else { vec![] });
            // The coefficients have an independent small integer interpretation.
            if equal {
                for x in -5..=5 {
                    for y in -5..=5 {
                        if ac * x + bc * y <= 0 && reverse_a * x + reverse_b * y <= 0 {
                            assert_eq!(x, y);
                        }
                    }
                }
            }
            facts.invalidate(a.place).unwrap();
            assert!(exact_equal_values(&facts, &mut limits).unwrap().is_empty());
        }
    }

    #[test]
    fn indexed_equality_chain_has_bounded_work_and_recovers_after_exhaustion() {
        let mut facts = Facts::default();
        let ids = (0..64)
            .map(|local| facts.invalidate(PlaceId { owner: 1, local }).unwrap())
            .collect::<Vec<_>>();
        for pair in ids.windows(2) {
            facts.inequalities.push(row(pair[0], 1, pair[1], -1));
            facts.inequalities.push(row(pair[0], -1, pair[1], 1));
        }
        let mut empty = Limits::default();
        empty.arithmetic.steps = 0;
        assert!(exact_equal_values(&facts, &mut empty).is_err());
        let mut bounded = Limits::default();
        bounded.arithmetic.steps = 5000;
        let equal = exact_equal_values(&facts, &mut bounded).unwrap();
        assert_eq!(
            equal,
            ids.windows(2)
                .map(|pair| (pair[0], pair[1]))
                .collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod propagation_feedback_tests {
    use super::*;

    #[test]
    fn polynomial_contraction_feeds_back_into_transitive_difference_bounds() {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let mut ids = Vec::new();
        for local in 0..4 {
            let range = Interval::machine(8, false, &mut limits).unwrap();
            ids.push(facts.assign(PlaceId { owner: 9, local }, range).unwrap());
        }
        facts
            .refine(ids[3], None, Some(&Integer::from_u128(2)), &mut limits)
            .unwrap();
        let variable = |id| Polynomial {
            terms: BTreeMap::from([(vec![id], Integer::from_i128(1))]),
        };
        // a <= b <= c <= x*x, with 0 <= x <= 2. The nonlinear row is
        // deliberately last: its new c bound must reach a and b next pass.
        facts.inequalities = vec![
            variable(ids[0])
                .subtract(&variable(ids[1]), &mut limits)
                .unwrap(),
            variable(ids[1])
                .subtract(&variable(ids[2]), &mut limits)
                .unwrap(),
            Polynomial {
                terms: BTreeMap::from([
                    (vec![ids[2]], Integer::from_i128(1)),
                    (vec![ids[3], ids[3]], Integer::from_i128(-1)),
                ]),
            },
        ]
        .into();
        let mut exhausted = Limits::default();
        exhausted.arithmetic.steps = 0;
        assert!(facts.propagate_relations(&mut exhausted).is_err());
        facts.propagate_relations(&mut limits).unwrap();
        assert!(facts.is_reachable());
        for id in &ids[..3] {
            assert_eq!(
                facts.bound(*id, &mut limits).unwrap().upper,
                Integer::from_u128(4)
            );
        }
        // A new externally established bound must still trigger closure.
        facts
            .refine(ids[3], None, Some(&Integer::from_u128(1)), &mut limits)
            .unwrap();
        facts.propagate_relations(&mut limits).unwrap();
        for id in &ids[..3] {
            assert_eq!(
                facts.bound(*id, &mut limits).unwrap().upper,
                Integer::from_u128(1)
            );
        }
    }
}

#[cfg(test)]
mod relation_store_tests {
    use super::*;

    fn row(local: u32) -> Polynomial {
        Polynomial {
            terms: BTreeMap::from([(
                vec![ValueId {
                    place: PlaceId { owner: 32, local },
                    version: 1,
                }],
                Integer::from_u128(1),
            )]),
        }
    }

    #[test]
    fn indexed_membership_is_bounded_collision_checked_and_recovers() {
        let mut limits = Limits::default();
        let mut rows = RelationStore::default();
        for local in 0..512 {
            assert!(rows.insert(row(local), &mut limits).unwrap());
        }
        let mut small = Limits::default();
        small.arithmetic.steps = 128;
        assert!(rows.contains(&row(511), &mut small).unwrap());
        let missing = row(900);
        let key = RelationStore::fingerprint(&missing, &mut limits).unwrap();
        rows.buckets.insert(key, vec![0]); // Force a candidate hash collision.
        assert!(!rows.contains(&missing, &mut limits).unwrap());
        let before = rows.len();
        small.arithmetic.steps = 0;
        assert!(rows.insert(missing.clone(), &mut small).is_err());
        assert_eq!(rows.len(), before);
        assert!(rows.insert(missing.clone(), &mut limits).unwrap());
        assert!(!rows.insert(missing.clone(), &mut limits).unwrap());
        assert_eq!(rows.rows.last(), Some(&missing));
        rows.clear();
        assert!(!rows.contains(&missing, &mut limits).unwrap());
        assert!(rows.insert(missing, &mut limits).unwrap());
    }

    #[test]
    fn unrelated_rows_do_not_expand_query_interval_work() {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let range = Interval::machine(8, false, &mut limits).unwrap();
        let x = facts
            .assign(
                PlaceId {
                    owner: 40,
                    local: 0,
                },
                range.copy(&mut limits).unwrap(),
            )
            .unwrap();
        for local in 1..65 {
            let y = facts
                .assign(
                    PlaceId { owner: 40, local },
                    range.copy(&mut limits).unwrap(),
                )
                .unwrap();
            let row = Polynomial::variable(y, &mut limits)
                .unwrap()
                .subtract(
                    &Polynomial::constant(Integer::from_u128(255), &mut limits).unwrap(),
                    &mut limits,
                )
                .unwrap();
            facts.inequalities.insert(row, &mut limits).unwrap();
        }
        let query = Polynomial::variable(x, &mut limits).unwrap();
        limits.arithmetic.steps = 5_000;
        let (actual, premises) = query.enclosure(&facts, &mut limits).unwrap();
        assert_eq!(actual, range);
        assert_eq!(premises.keys().copied().collect::<Vec<_>>(), vec![x]);
    }
}
