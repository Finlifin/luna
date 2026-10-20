//! Established Boolean facts retain disjunctions that interval joins forget.
//! Every referenced scalar identity must still denote its original value.
use super::*;

impl Condition {
    // These are pure mathematical conditions over already evaluated values;
    // commutativity here never changes executable short-circuit ordering.
    fn equivalent(&self, other: &Self, limits: &mut Limits, depth: usize) -> Result<bool, Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        Ok(match (self, other) {
            (Self::Not(a), Self::Not(b)) => a.equivalent(b, limits, depth + 1)?,
            (Self::And(..), Self::And(..)) | (Self::Or(..), Self::Or(..)) => {
                let conjunction = matches!(self, Self::And(..));
                let mut left = Vec::new();
                let mut right = Vec::new();
                self.associative_terms(conjunction, &mut left, limits, depth)?;
                other.associative_terms(conjunction, &mut right, limits, depth)?;
                if left.len() != right.len() {
                    return Ok(false);
                }
                let mut matched = vec![false; right.len()];
                for a in left {
                    let mut found = false;
                    for (index, b) in right.iter().enumerate() {
                        limits.step(1)?;
                        if !matched[index] && a.equivalent(b, limits, depth + 1)? {
                            matched[index] = true;
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return Ok(false);
                    }
                }
                true
            }
            (
                Self::PolynomialCompare {
                    left: a,
                    relation: ar,
                    right: b,
                },
                Self::PolynomialCompare {
                    left: c,
                    relation: br,
                    right: d,
                },
            ) if ar == br && matches!(ar, Comparison::Equal | Comparison::NotEqual) => {
                (a == c && b == d) || (a == d && b == c)
            }
            _ => self == other,
        })
    }
    fn associative_terms<'a>(
        &'a self,
        conjunction: bool,
        terms: &mut Vec<&'a Condition>,
        limits: &mut Limits,
        depth: usize,
    ) -> Result<(), Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        match self {
            Self::And(a, b) if conjunction => {
                a.associative_terms(conjunction, terms, limits, depth + 1)?;
                b.associative_terms(conjunction, terms, limits, depth + 1)?;
            }
            Self::Or(a, b) if !conjunction => {
                a.associative_terms(conjunction, terms, limits, depth + 1)?;
                b.associative_terms(conjunction, terms, limits, depth + 1)?;
            }
            _ => {
                if terms.len() >= limits.max_terms {
                    return Err(Unknown::TermLimit);
                }
                terms.push(self);
            }
        }
        Ok(())
    }
    pub(super) fn is_current(
        &self,
        facts: &Facts,
        limits: &mut Limits,
        depth: usize,
    ) -> Result<bool, Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        Ok(match self {
            Self::Bool(_) => true,
            Self::Compare { value, .. } => facts.value(value.place) == Some(*value),
            Self::PolynomialCompare { left, right, .. } => {
                facts.relation_is_current(left, limits)?
                    && facts.relation_is_current(right, limits)?
            }
            Self::Not(inner) => inner.is_current(facts, limits, depth + 1)?,
            Self::And(a, b) | Self::Or(a, b) => {
                a.is_current(facts, limits, depth + 1)? && b.is_current(facts, limits, depth + 1)?
            }
        })
    }
}
impl Facts {
    pub(super) fn known_condition(
        &self,
        condition: &Condition,
        limits: &mut Limits,
    ) -> Result<Option<bool>, Unknown> {
        if !condition.is_current(self, limits, 0)? {
            return Ok(None);
        }
        for (known, truth) in &self.conditions {
            // Charge tree and coefficient comparison work before structural equality.
            let known = known.copy_bounded(limits, 0)?;
            let query = condition.copy_bounded(limits, 0)?;
            if !known.is_current(self, limits, 0)? {
                continue;
            }
            if known.equivalent(&query, limits, 0)? {
                return Ok(Some(*truth));
            }
            if let (
                Condition::Compare {
                    value: a,
                    relation: ar,
                    constant: ac,
                },
                Condition::Compare {
                    value: b,
                    relation: br,
                    constant: bc,
                },
            ) = (&known, &query)
            {
                if a == b {
                    let ar = if *truth { *ar } else { ar.negated() };
                    if comparison_entails(ar, ac, *br, bc, limits)? {
                        return Ok(Some(true));
                    }
                    if comparison_entails(ar, ac, br.negated(), bc, limits)? {
                        return Ok(Some(false));
                    }
                }
            }
        }
        Ok(None)
    }
    pub(super) fn remember_condition(
        &mut self,
        condition: &Condition,
        truth: bool,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        if !self.reachable {
            return Ok(());
        }
        if let Condition::Compare { value, .. } = condition {
            match self.bound(*value, limits) {
                Ok(_) => return Ok(()),
                Err(Unknown::MissingBound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        if let Condition::PolynomialCompare { left, right, .. } = condition {
            match (
                left.interval_enclosure(self, limits),
                right.interval_enclosure(self, limits),
            ) {
                (Ok(_), Ok(_)) => return Ok(()),
                (Err(Unknown::MissingBound(_)), Ok(_) | Err(Unknown::MissingBound(_)))
                | (Ok(_), Err(Unknown::MissingBound(_))) => {}
                (Err(e), _) | (_, Err(e)) => return Err(e),
            }
        }
        if matches!(condition, Condition::Bool(_)) {
            return Ok(());
        }
        if !self.reachable || self.known_condition(condition, limits)?.is_some() {
            return Ok(());
        }
        if !condition.is_current(self, limits, 0)? {
            return Ok(());
        }
        let mut retained = self.copy_conditions(limits)?;
        if retained.len() >= limits.max_terms {
            return Err(Unknown::TermLimit);
        }
        retained.push((condition.copy_bounded(limits, 0)?, truth));
        self.conditions = retained;
        Ok(())
    }
    pub(super) fn copy_conditions(
        &self,
        limits: &mut Limits,
    ) -> Result<Vec<(Condition, bool)>, Unknown> {
        let mut result = Vec::new();
        for (condition, truth) in &self.conditions {
            if condition.is_current(self, limits, 0)? {
                result.push((condition.copy_bounded(limits, 0)?, *truth));
            }
        }
        Ok(result)
    }
    pub(super) fn common_conditions(
        &self,
        other: &Self,
        limits: &mut Limits,
    ) -> Result<Vec<(Condition, bool)>, Unknown> {
        let mut result: Vec<(Condition, bool)> = Vec::new();
        // Implication is directional: the weaker common fact may be stored
        // only on the right branch. Consider both sets of candidates.
        for (condition, truth) in self
            .copy_conditions(limits)?
            .into_iter()
            .chain(other.copy_conditions(limits)?)
        {
            if self.known_condition(&condition, limits)? == Some(truth)
                && other.known_condition(&condition, limits)? == Some(truth)
            {
                let mut duplicate = false;
                for (saved, value) in &result {
                    if *value == truth && saved.equivalent(&condition, limits, 0)? {
                        duplicate = true;
                        break;
                    }
                }
                if !duplicate {
                    if result.len() >= limits.max_terms {
                        return Err(Unknown::TermLimit);
                    }
                    result.push((condition, truth));
                }
            }
        }
        Ok(result)
    }
}

impl Condition {
    /// Transport an established predicate only when every value is mapped to
    /// a copied destination, or is still current in the same analysis lineage.
    pub(super) fn renamed(
        &self,
        mapping: &BTreeMap<ValueId, ValueId>,
        target: &Facts,
        allow_current: bool,
        limits: &mut Limits,
        depth: usize,
    ) -> Result<Option<Self>, Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        let value =
            |id: ValueId, limits: &mut Limits| -> Result<Option<ValueId>, Unknown> {
                let Some(id) = mapping.get(&id).copied().or_else(|| {
                    (allow_current && target.value(id.place) == Some(id)).then_some(id)
                }) else {
                    return Ok(None);
                };
                limits.step(1)?;
                if target.value(id.place) != Some(id) {
                    return Err(Unknown::StaleValue(id));
                }
                Ok(Some(id))
            };
        let polynomial =
            |p: &Polynomial, limits: &mut Limits| -> Result<Option<Polynomial>, Unknown> {
                let mut result = Polynomial::default();
                for (factors, coefficient) in &p.terms {
                    let mut renamed = Vec::new();
                    for id in factors {
                        let Some(id) = value(*id, limits)? else {
                            return Ok(None);
                        };
                        renamed.push(id);
                    }
                    renamed.sort_unstable();
                    result.insert(renamed, coefficient, limits)?;
                }
                Ok(Some(result))
            };
        Ok(Some(match self {
            Self::Bool(b) => Self::Bool(*b),
            Self::Compare {
                value: id,
                relation,
                constant,
            } => {
                let Some(id) = value(*id, limits)? else {
                    return Ok(None);
                };
                Self::Compare {
                    value: id,
                    relation: *relation,
                    constant: limits.integer(constant)?,
                }
            }
            Self::PolynomialCompare {
                left,
                relation,
                right,
            } => {
                let Some(left) = polynomial(left, limits)? else {
                    return Ok(None);
                };
                let Some(right) = polynomial(right, limits)? else {
                    return Ok(None);
                };
                Self::PolynomialCompare {
                    left,
                    relation: *relation,
                    right,
                }
            }
            Self::Not(inner) => {
                let Some(inner) =
                    inner.renamed(mapping, target, allow_current, limits, depth + 1)?
                else {
                    return Ok(None);
                };
                Self::Not(Box::new(inner))
            }
            Self::And(a, b) | Self::Or(a, b) => {
                let Some(a) = a.renamed(mapping, target, allow_current, limits, depth + 1)? else {
                    return Ok(None);
                };
                let Some(b) = b.renamed(mapping, target, allow_current, limits, depth + 1)? else {
                    return Ok(None);
                };
                if matches!(self, Self::And(..)) {
                    Self::And(Box::new(a), Box::new(b))
                } else {
                    Self::Or(Box::new(a), Box::new(b))
                }
            }
        }))
    }
}

/// Inclusion between integer half-lines, singleton values and excluded points.
/// Strict bounds are normalized in mathematical Integer, never machine arithmetic.
fn comparison_entails(
    premise: Comparison,
    a: &Integer,
    conclusion: Comparison,
    b: &Integer,
    limits: &mut Limits,
) -> Result<bool, Unknown> {
    let (lower, upper, excluded) = comparison_bounds(premise, a, limits)?;
    let (required_lower, required_upper, required_excluded) =
        comparison_bounds(conclusion, b, limits)?;
    if let Some(required) = required_lower {
        let Some(lower) = &lower else {
            return Ok(false);
        };
        if limits.compare(lower, &required)?.is_lt() {
            return Ok(false);
        }
    }
    if let Some(required) = required_upper {
        let Some(upper) = &upper else {
            return Ok(false);
        };
        if limits.compare(upper, &required)?.is_gt() {
            return Ok(false);
        }
    }
    if let Some(required) = required_excluded {
        if let Some(excluded) = excluded {
            if limits.compare(&excluded, &required)?.is_eq() {
                return Ok(true);
            }
        }
        if let Some(lower) = lower {
            if limits.compare(&lower, &required)?.is_gt() {
                return Ok(true);
            }
        }
        if let Some(upper) = upper {
            if limits.compare(&upper, &required)?.is_lt() {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    Ok(true)
}

fn comparison_bounds(
    relation: Comparison,
    value: &Integer,
    limits: &mut Limits,
) -> Result<(Option<Integer>, Option<Integer>, Option<Integer>), Unknown> {
    let value = limits.integer(value)?;
    Ok(match relation {
        Comparison::Equal => (Some(limits.integer(&value)?), Some(value), None),
        Comparison::NotEqual => (None, None, Some(value)),
        Comparison::GreaterEqual => (Some(value), None, None),
        Comparison::LessEqual => (None, Some(value), None),
        Comparison::Greater => (
            Some(value.add(&Integer::from_u128(1), &mut limits.arithmetic)?),
            None,
            None,
        ),
        Comparison::Less => (
            None,
            Some(value.subtract(&Integer::from_u128(1), &mut limits.arithmetic)?),
            None,
        ),
    })
}

impl Facts {
    /// Combine one-sided premises only after both finite endpoints are known.
    /// Keeping the same version makes all retained evidence refer to this value.
    pub(super) fn bound_from_retained_comparisons(
        &mut self,
        id: ValueId,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        let mut lower: Option<Integer> = None;
        let mut upper: Option<Integer> = None;
        let mut excluded = Vec::new();
        for (condition, truth) in &self.conditions {
            limits.step(1)?;
            let Condition::Compare {
                value,
                relation,
                constant,
            } = condition
            else {
                continue;
            };
            if *value != id {
                continue;
            }
            let relation = if *truth {
                *relation
            } else {
                relation.negated()
            };
            let (lo, hi, hole) = comparison_bounds(relation, constant, limits)?;
            if let Some(lo) = lo {
                if match &lower {
                    Some(old) => limits.compare(&lo, old)?.is_gt(),
                    None => true,
                } {
                    lower = Some(lo);
                }
            }
            if let Some(hi) = hi {
                if match &upper {
                    Some(old) => limits.compare(&hi, old)?.is_lt(),
                    None => true,
                } {
                    upper = Some(hi);
                }
            }
            if let Some(hole) = hole {
                excluded.push(hole);
            }
        }
        let (Some(lower), Some(upper)) = (lower, upper) else {
            return Ok(());
        };
        let order = limits.compare(&lower, &upper)?;
        if order.is_gt() {
            self.reachable = false;
            return Ok(());
        }
        if order.is_eq() {
            for hole in excluded {
                if limits.compare(&lower, &hole)?.is_eq() {
                    self.reachable = false;
                    return Ok(());
                }
            }
        }
        let bound = Interval::new(lower, upper, limits)?;
        self.current.insert(id.place, (id.version, Some(bound)));
        Ok(())
    }
}
