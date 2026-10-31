//! Preserve nonzero evidence through multiplication over mathematical integers.
use super::*;

pub(super) fn nonzero_factor_entailment(
    known: &Condition,
    truth: bool,
    query: &Condition,
    limits: &mut Limits,
) -> Result<Option<bool>, Unknown> {
    let Condition::PolynomialCompare {
        left,
        relation,
        right,
    } = known
    else {
        return Ok(None);
    };
    let relation = if truth { *relation } else { relation.negated() };
    if !matches!(
        relation,
        Comparison::NotEqual | Comparison::Less | Comparison::Greater
    ) {
        return Ok(None);
    }
    let (value, relation) = match query {
        Condition::Compare {
            value,
            relation,
            constant,
        } if constant.is_zero() && matches!(relation, Comparison::Equal | Comparison::NotEqual) => {
            (*value, *relation)
        }
        Condition::PolynomialCompare {
            left,
            relation,
            right,
        } if matches!(relation, Comparison::Equal | Comparison::NotEqual) => {
            let difference = left.subtract(right, limits)?;
            let Some((factors, _)) = difference.terms.first_key_value() else {
                return Ok(None);
            };
            if difference.terms.len() != 1 || factors.len() != 1 {
                return Ok(None);
            }
            (factors[0], *relation)
        }
        _ => return Ok(None),
    };
    let difference = left.subtract(right, limits)?;
    if difference.terms.is_empty() {
        return Ok(None);
    }
    // The caller has checked every original value version before any
    // cancellation. A sum containing the variable is not enough: it must
    // divide every surviving monomial of the known nonzero difference.
    for factors in difference.terms.keys() {
        limits.step(factors.len().saturating_add(1))?;
        if !factors.contains(&value) {
            return Ok(None);
        }
    }
    Ok(Some(relation == Comparison::NotEqual))
}

impl Polynomial {
    pub(super) fn excludes_zero_by_common_factor(
        &self,
        facts: &Facts,
        limits: &mut Limits,
    ) -> Result<bool, Unknown> {
        let Some((first, coefficient)) = self.terms.first_key_value() else {
            return Ok(false);
        };
        if first.is_empty() {
            return Ok(self.terms.len() == 1 && !coefficient.is_zero());
        }
        // Removing a common variable never introduces fractions or increases
        // degree. Each recursive query has strictly smaller degree.
        for factor in first {
            limits.step(self.terms.len())?;
            if !self.terms.keys().all(|term| term.contains(factor)) {
                continue;
            }
            let scalar = Condition::Compare {
                value: *factor,
                relation: Comparison::NotEqual,
                constant: Integer::zero(),
            };
            let known = facts.known_condition(&scalar, limits)? == Some(true);
            let scalar = Condition::PolynomialCompare {
                left: Polynomial::variable(*factor, limits)?,
                relation: Comparison::NotEqual,
                right: Polynomial::constant(Integer::zero(), limits)?,
            };
            let known = known || facts.known_condition(&scalar, limits)? == Some(true);
            let nonzero = if known {
                true
            } else {
                match facts.bound(*factor, limits) {
                    Ok(bound) => {
                        limits.compare(bound.lower(), &Integer::zero())?.is_gt()
                            || limits.compare(bound.upper(), &Integer::zero())?.is_lt()
                    }
                    Err(Unknown::MissingBound(_)) => false,
                    Err(failure) => return Err(failure),
                }
            };
            if !nonzero {
                continue;
            }
            let mut terms = BTreeMap::new();
            for (factors, coefficient) in &self.terms {
                limits.step(factors.len().saturating_add(1))?;
                let mut remaining = factors.clone();
                let index = remaining
                    .iter()
                    .position(|value| value == factor)
                    .ok_or(Unknown::InsufficientBounds)?;
                remaining.remove(index);
                terms.insert(remaining, limits.integer(coefficient)?);
            }
            let remainder = Condition::PolynomialCompare {
                left: Polynomial { terms },
                relation: Comparison::NotEqual,
                right: Polynomial::constant(Integer::zero(), limits)?,
            };
            if facts.known_condition(&remainder, limits)? == Some(true)
                || remainder.truth_by_enclosure(facts, limits, 0)? == Some(true)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
