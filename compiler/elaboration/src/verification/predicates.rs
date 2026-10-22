//! Entailment of program conditions. Refuting the opposite condition may prove
//! an obligation, but an unsuccessful search is Unknown, never a countermodel.
use super::*;

/// Evidence belongs to one program analysis lineage and its current values.
/// It cannot authorize type propositions, memory access or ownership transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PredicateProof {
    lineage: Lineage,
    condition: Condition,
    truth: bool,
    premises: BTreeMap<ValueId, Interval>,
    inequalities: Vec<Polynomial>,
}

impl Facts {
    pub fn prove_condition(
        &self,
        condition: &Condition,
        truth: bool,
        limits: &mut Limits,
    ) -> Result<PredicateProof, Unknown> {
        if !self.is_reachable() {
            return Err(Unknown::UnreachablePath);
        }
        let condition = condition.copy_bounded(limits, 0)?;
        self.refute_opposite(&condition, truth, limits)?;
        // Conservatively retain every bounded premise and current relation.
        // Dependency minimization is separate from validity of the evidence.
        let mut premises = BTreeMap::new();
        for (place, (version, bound)) in &self.current {
            limits.step(1)?;
            if let Some(bound) = bound {
                premises.insert(
                    ValueId {
                        place: *place,
                        version: *version,
                    },
                    bound.copy(limits)?,
                );
            }
        }
        Ok(PredicateProof {
            lineage: Lineage(self.versions.clone()),
            condition,
            truth,
            premises,
            inequalities: self.copy_relations(limits)?,
        })
    }

    fn refute_opposite(
        &self,
        condition: &Condition,
        truth: bool,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        if let Some(actual) = self.known_condition(condition, limits)? {
            return if actual == truth {
                Ok(())
            } else {
                Err(Unknown::InsufficientBounds)
            };
        }
        let enclosed = match condition.truth_by_enclosure(self, limits, 0) {
            Ok(value) => value,
            Err(Unknown::MissingBound(_)) if condition.is_current(self, limits, 0)? => None,
            Err(e) => return Err(e),
        };
        if let Some(actual) = enclosed {
            return if actual == truth {
                Ok(())
            } else {
                Err(Unknown::InsufficientBounds)
            };
        }
        let mut alternative = self.fork(limits)?;
        alternative.assume_inner(condition, !truth, 0, limits)?;
        alternative.propagate_retained_conditions(limits)?;
        if alternative.is_reachable() {
            Err(Unknown::InsufficientBounds)
        } else {
            Ok(())
        }
    }
}

impl PredicateProof {
    pub fn condition(&self) -> &Condition {
        &self.condition
    }
    pub fn truth(&self) -> bool {
        self.truth
    }

    pub fn replay(&self, facts: &Facts, limits: &mut Limits) -> Result<(), Unknown> {
        limits.step(1)?;
        if !Arc::ptr_eq(&self.lineage.0, &facts.versions) {
            return Err(Unknown::UnrelatedFacts);
        }
        if !facts.is_reachable() {
            return Err(Unknown::UnreachablePath);
        }
        for (id, expected) in &self.premises {
            let actual = facts.bound(*id, limits)?;
            if !limits.compare(&actual.lower, &expected.lower)?.is_eq()
                || !limits.compare(&actual.upper, &expected.upper)?.is_eq()
            {
                return Err(Unknown::StaleValue(*id));
            }
        }
        for row in &self.inequalities {
            if !facts.relation_is_current(row, limits)?
                || !relations::contains_relation(&facts.inequalities, row, limits)?
            {
                return Err(Unknown::InsufficientBounds);
            }
        }
        facts.refute_opposite(&self.condition, self.truth, limits)
    }
}

impl Condition {
    pub(super) fn truth_by_enclosure(
        &self,
        facts: &Facts,
        limits: &mut Limits,
        depth: usize,
    ) -> Result<Option<bool>, Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        let (difference, relation) = match self {
            Self::Bool(value) => return Ok(Some(*value)),
            Self::Not(inner) => {
                return Ok(inner
                    .truth_by_enclosure(facts, limits, depth + 1)?
                    .map(|v| !v));
            }
            Self::And(a, b) | Self::Or(a, b) => {
                let conjunction = matches!(self, Self::And(..));
                let a = a.truth_by_enclosure(facts, limits, depth + 1)?;
                if a == Some(!conjunction) {
                    return Ok(a);
                }
                let b = b.truth_by_enclosure(facts, limits, depth + 1)?;
                return Ok(if b == Some(!conjunction) {
                    b
                } else if a == Some(conjunction) {
                    b
                } else {
                    None
                });
            }
            Self::Compare {
                value,
                relation,
                constant,
            } => {
                facts.bound(*value, limits)?;
                let left = Polynomial::variable(*value, limits)?;
                (
                    left.subtract(
                        &Polynomial::constant(limits.integer(constant)?, limits)?,
                        limits,
                    )?,
                    *relation,
                )
            }
            Self::PolynomialCompare {
                left,
                relation,
                right,
            } => {
                // Check versions before normalization can cancel an old value.
                // Identity validity and a finite interval are different facts:
                // x + 1 > x is decidable even for an unbounded current x.
                for polynomial in [left, right] {
                    for factors in polynomial.terms.keys() {
                        limits.step(factors.len().saturating_add(1))?;
                        for value in factors {
                            if facts.value(value.place) != Some(*value) {
                                return Err(Unknown::StaleValue(*value));
                            }
                        }
                    }
                }
                (left.subtract(right, limits)?, *relation)
            }
        };
        let (range, _) = difference.enclosure(facts, limits)?;
        let low = limits.compare(&range.lower, &Integer::zero())?;
        let high = limits.compare(&range.upper, &Integer::zero())?;
        let equality = if low.is_eq() && high.is_eq() {
            Some(true)
        } else if low.is_gt() || high.is_lt() {
            Some(false)
        } else {
            None
        };
        Ok(match relation {
            Comparison::Equal => equality,
            Comparison::NotEqual => equality.map(|v| !v),
            Comparison::Less => {
                if high.is_lt() {
                    Some(true)
                } else if !low.is_lt() {
                    Some(false)
                } else {
                    None
                }
            }
            Comparison::LessEqual => {
                if !high.is_gt() {
                    Some(true)
                } else if low.is_gt() {
                    Some(false)
                } else {
                    None
                }
            }
            Comparison::Greater => {
                if low.is_gt() {
                    Some(true)
                } else if !high.is_gt() {
                    Some(false)
                } else {
                    None
                }
            }
            Comparison::GreaterEqual => {
                if !low.is_lt() {
                    Some(true)
                } else if high.is_lt() {
                    Some(false)
                } else {
                    None
                }
            }
        })
    }

    pub(super) fn copy_bounded(&self, limits: &mut Limits, depth: usize) -> Result<Self, Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        Ok(match self {
            Self::Bool(value) => Self::Bool(*value),
            Self::Compare {
                value,
                relation,
                constant,
            } => Self::Compare {
                value: *value,
                relation: *relation,
                constant: limits.integer(constant)?,
            },
            Self::PolynomialCompare {
                left,
                relation,
                right,
            } => Self::PolynomialCompare {
                left: left.copy(limits)?,
                relation: *relation,
                right: right.copy(limits)?,
            },
            Self::Not(inner) => Self::Not(Box::new(inner.copy_bounded(limits, depth + 1)?)),
            Self::And(a, b) | Self::Or(a, b) => {
                let a = Box::new(a.copy_bounded(limits, depth + 1)?);
                let b = Box::new(b.copy_bounded(limits, depth + 1)?);
                if matches!(self, Self::And(..)) {
                    Self::And(a, b)
                } else {
                    Self::Or(a, b)
                }
            }
        })
    }
}

impl fmt::Display for PredicateProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(predicate-proof (condition {}) (truth {}) (premises",
            self.condition, self.truth
        )?;
        for (id, bound) in &self.premises {
            write!(f, " (bound {id} {bound})")?;
        }
        f.write_str(") (inequalities")?;
        for row in &self.inequalities {
            write!(f, " (less-equal-zero {row})")?;
        }
        f.write_str("))")
    }
}
