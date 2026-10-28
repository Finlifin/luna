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
    /// Discharge an obligation in this exact immutable state. No evidence is
    /// exported: callers retaining a proof across state changes must use
    /// prove_condition and replay instead. Keep the same bounded admission and
    /// refutation procedure as proof construction.
    pub(super) fn check_condition(
        &self,
        condition: &Condition,
        truth: bool,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        if !self.is_reachable() {
            return Err(Unknown::UnreachablePath);
        }
        let condition = condition.copy_bounded(limits, 0)?;
        self.refute_opposite(&condition, truth, limits)
    }

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
        alternative.remember_condition(condition, !truth, limits)?;
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
                || !facts.inequalities.contains(row, limits)?
            {
                return Err(Unknown::InsufficientBounds);
            }
        }
        facts.refute_opposite(&self.condition, self.truth, limits)
    }
}

impl Condition {
    /// Cheap interval evidence only: never searches retained predicates or
    /// aliases, so callers may use this before their relational lookup.
    pub(super) fn truth_by_interval(
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
                    .truth_by_interval(facts, limits, depth + 1)?
                    .map(|v| !v));
            }
            Self::And(a, b) | Self::Or(a, b) => {
                let identity = matches!(self, Self::And(..));
                let a = a.truth_by_interval(facts, limits, depth + 1)?;
                if a == Some(!identity) {
                    return Ok(a);
                }
                let b = b.truth_by_interval(facts, limits, depth + 1)?;
                return Ok(if b == Some(!identity) || a == Some(identity) {
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
                let bound = match facts.bound(*value, limits) {
                    Ok(bound) => bound,
                    Err(Unknown::MissingBound(_)) => return Ok(None),
                    Err(error) => return Err(error),
                };
                let range = Interval::new(
                    bound.lower.subtract(constant, &mut limits.arithmetic)?,
                    bound.upper.subtract(constant, &mut limits.arithmetic)?,
                    limits,
                )?;
                return enclosed_comparison(&range, *relation, limits);
            }
            Self::PolynomialCompare {
                left,
                relation,
                right,
            } => {
                if !facts.relation_is_current(left, limits)?
                    || !facts.relation_is_current(right, limits)?
                {
                    return Ok(None);
                }
                (left.subtract(right, limits)?, *relation)
            }
        };
        match difference.interval_enclosure(facts, limits) {
            Ok((range, _)) => enclosed_comparison(&range, relation, limits),
            Err(Unknown::MissingBound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

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
        if let Some(truth) = difference.sign_comparison(facts, relation, limits)? {
            return Ok(Some(truth));
        }
        if matches!(relation, Comparison::Equal | Comparison::NotEqual)
            && difference.excludes_integer_zero(facts, limits)?
        {
            return Ok(Some(relation == Comparison::NotEqual));
        }
        let (mut range, premises) = match difference.enclosure(facts, limits) {
            Ok(enclosed) => enclosed,
            Err(Unknown::MissingBound(_)) => {
                return difference.global_quadratic_comparison(relation, limits);
            }
            Err(failure) => return Err(failure),
        };
        if let Some(truth) = enclosed_comparison(&range, relation, limits)? {
            return Ok(Some(truth));
        }
        difference.refine_monotone_enclosure(&mut range, &premises, facts, limits)?;
        if let Some(truth) = enclosed_comparison(&range, relation, limits)? {
            return Ok(Some(truth));
        }
        difference.refine_finite_enclosure(&mut range, facts, limits)?;
        enclosed_comparison(&range, relation, limits)
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

fn enclosed_comparison(
    range: &Interval,
    relation: Comparison,
    limits: &mut Limits,
) -> Result<Option<bool>, Unknown> {
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

#[cfg(test)]
mod interval_fast_path_tests {
    use super::*;

    #[test]
    fn settled_comparisons_do_not_search_unrelated_retained_conditions() {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        for local in 0..32 {
            let id = facts
                .assign(
                    PlaceId { owner: 5, local },
                    Interval::new(Integer::from_u128(1), Integer::from_u128(255), &mut limits)
                        .unwrap(),
                )
                .unwrap();
            // Valid but unrelated disequalities retained for future joins.
            facts.conditions.push((
                Condition::Compare {
                    value: id,
                    relation: Comparison::NotEqual,
                    constant: Integer::zero(),
                },
                true,
            ));
        }
        let place = PlaceId {
            owner: 5,
            local: 32,
        };
        let id = facts
            .assign(
                place,
                Interval::singleton(&Integer::from_u128(1), &mut limits).unwrap(),
            )
            .unwrap();
        for (relation, expected) in [
            (Comparison::Equal, true),
            (Comparison::NotEqual, false),
            (Comparison::Less, false),
            (Comparison::LessEqual, true),
            (Comparison::Greater, false),
            (Comparison::GreaterEqual, true),
        ] {
            let condition = Condition::Compare {
                value: id,
                relation,
                constant: Integer::from_u128(1),
            };
            let mut limited = Limits::default();
            limited.arithmetic.steps = 256;
            assert_eq!(
                facts.known_condition(&condition, &mut limited).unwrap(),
                Some(expected)
            );
        }
        let value = Polynomial::variable(id, &mut limits).unwrap();
        facts.invalidate(place).unwrap();
        let stale = Condition::PolynomialCompare {
            left: value.clone(),
            relation: Comparison::Equal,
            right: value,
        };
        assert_eq!(
            stale.truth_by_enclosure(&facts, &mut limits, 0),
            Err(Unknown::StaleValue(id))
        );
    }
}

#[cfg(test)]
mod in_place_tests {
    use super::*;

    #[test]
    fn immediate_checks_preserve_proof_admission_and_stale_value_rejection() {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let place = PlaceId { owner: 1, local: 0 };
        let id = facts
            .assign(place, Interval::machine(8, false, &mut limits).unwrap())
            .unwrap();
        let bounded = Condition::Compare {
            value: id,
            relation: Comparison::LessEqual,
            constant: Integer::from_u128(255),
        };
        let unknown = Condition::Compare {
            value: id,
            relation: Comparison::Equal,
            constant: Integer::from_u128(7),
        };
        for condition in [&bounded, &unknown] {
            for truth in [false, true] {
                let checked = facts.check_condition(condition, truth, &mut Limits::default());
                let replayed = facts
                    .prove_condition(condition, truth, &mut Limits::default())
                    .and_then(|proof| proof.replay(&facts, &mut Limits::default()));
                assert_eq!(checked, replayed);
            }
        }
        let proof = facts.prove_condition(&bounded, true, &mut limits).unwrap();
        let mut empty = Limits::default();
        empty.arithmetic.steps = 0;
        assert!(facts.check_condition(&bounded, true, &mut empty).is_err());
        facts.check_condition(&bounded, true, &mut limits).unwrap();
        facts.invalidate(place).unwrap();
        assert!(facts.check_condition(&bounded, true, &mut limits).is_err());
        assert!(proof.replay(&facts, &mut limits).is_err());
        facts
            .assume(&Condition::Bool(false), true, &mut limits)
            .unwrap();
        assert_eq!(
            facts.check_condition(&Condition::Bool(true), true, &mut limits),
            Err(Unknown::UnreachablePath)
        );
    }
}
