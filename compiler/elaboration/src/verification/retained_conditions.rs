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
            if known.equivalent(&query, limits, 0)? {
                return Ok(Some(*truth));
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
        if matches!(
            condition,
            Condition::Bool(_) | Condition::Compare { .. } | Condition::PolynomialCompare { .. }
        ) {
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
        let mut result = Vec::new();
        for (condition, truth) in self.copy_conditions(limits)? {
            if other.known_condition(&condition, limits)? == Some(truth) {
                result.push((condition, truth));
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
                target.bound(id, limits)?;
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
