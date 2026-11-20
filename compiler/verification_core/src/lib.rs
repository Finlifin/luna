//! Bounded program-state reasoning, independent of type propositions and code execution.
mod congruence;
mod cubic;
mod differences;
mod division;
mod factors;
pub mod ownership;
mod predicates;
mod quadratic;
mod relations;
mod retained_conditions;
mod signs;

pub use predicates::PredicateProof;

use comptime_runtime::{Budget, Error as ArithmeticError, Integer};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaceId {
    pub owner: u64,
    pub local: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValueId {
    pub place: PlaceId,
    pub version: u64,
}

impl fmt::Display for ValueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(program-value (owner {}) (local {}) (version {}))",
            self.place.owner, self.place.local, self.version
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unknown {
    Arithmetic(ArithmeticError),
    DegreeLimit,
    TermLimit,
    InvalidInterval,
    InvalidMachineWidth,
    MissingBound(ValueId),
    StaleValue(ValueId),
    VersionExhausted,
    InsufficientBounds,
    UnreachablePath,
    UnrelatedFacts,
    ConditionDepthLimit,
}

impl From<ArithmeticError> for Unknown {
    fn from(e: ArithmeticError) -> Self {
        Self::Arithmetic(e)
    }
}

impl fmt::Display for Unknown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arithmetic(e) => write!(f, "(verification-unknown (resource {e:?}))"),
            Self::MissingBound(id) => write!(f, "(verification-unknown (missing-bound {id}))"),
            Self::StaleValue(id) => write!(f, "(verification-unknown (stale-value {id}))"),
            other => write!(
                f,
                "(verification-unknown (reason {}))",
                match other {
                    Self::DegreeLimit => "degree-limit",
                    Self::TermLimit => "term-limit",
                    Self::InvalidInterval => "invalid-interval",
                    Self::InvalidMachineWidth => "invalid-machine-width",
                    Self::VersionExhausted => "version-exhausted",
                    Self::InsufficientBounds => "insufficient-bounds",
                    Self::UnreachablePath => "unreachable-path",
                    Self::UnrelatedFacts => "unrelated-facts",
                    Self::ConditionDepthLimit => "condition-depth-limit",
                    _ => unreachable!(),
                }
            ),
        }
    }
}

/// A shared work budget, coefficient size bound and expansion bound.
pub struct Limits {
    pub arithmetic: Budget,
    pub max_terms: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            arithmetic: Budget::default(),
            max_terms: 1024,
        }
    }
}

impl Limits {
    pub fn step(&mut self, n: usize) -> Result<(), Unknown> {
        self.arithmetic.consume(n).map_err(Into::into)
    }

    pub fn integer(&mut self, x: &Integer) -> Result<Integer, Unknown> {
        let limbs = x.bit_len().div_ceil(32);
        if limbs > self.arithmetic.max_limbs {
            return Err(ArithmeticError::SizeLimit.into());
        }
        self.step(limbs.saturating_add(1))?;
        Ok(x.clone())
    }

    pub fn compare(&mut self, a: &Integer, b: &Integer) -> Result<std::cmp::Ordering, Unknown> {
        if a.bit_len().max(b.bit_len()).div_ceil(32) > self.arithmetic.max_limbs {
            return Err(ArithmeticError::SizeLimit.into());
        }
        self.step(a.bit_len().max(b.bit_len()).div_ceil(32).saturating_add(1))?;
        Ok(a.cmp(b))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interval {
    lower: Integer,
    upper: Integer,
}

impl Interval {
    pub fn new(lower: Integer, upper: Integer, limits: &mut Limits) -> Result<Self, Unknown> {
        limits.integer(&lower)?;
        limits.integer(&upper)?;
        if limits.compare(&lower, &upper)?.is_gt() {
            return Err(Unknown::InvalidInterval);
        }
        Ok(Self { lower, upper })
    }

    /// A value range only; callers still need proofs before a machine operation.
    /// Target pointer width must be supplied explicitly by the caller.
    pub fn machine(width: u16, signed: bool, limits: &mut Limits) -> Result<Self, Unknown> {
        if !(1..=128).contains(&width) {
            return Err(Unknown::InvalidMachineWidth);
        }
        if usize::from(width).div_ceil(32) > limits.arithmetic.max_limbs {
            return Err(ArithmeticError::SizeLimit.into());
        }
        let (lower, upper) = if signed {
            if width == 128 {
                (Integer::from_i128(i128::MIN), Integer::from_i128(i128::MAX))
            } else {
                let high = 1i128 << (width - 1);
                (Integer::from_i128(-high), Integer::from_i128(high - 1))
            }
        } else {
            (
                Integer::zero(),
                Integer::from_u128(if width == 128 {
                    u128::MAX
                } else {
                    (1u128 << width) - 1
                }),
            )
        };
        Self::new(lower, upper, limits)
    }

    pub fn into_bounds(self) -> (Integer, Integer) {
        (self.lower, self.upper)
    }

    pub fn lower(&self) -> &Integer {
        &self.lower
    }

    pub fn upper(&self) -> &Integer {
        &self.upper
    }

    pub fn copy(&self, limits: &mut Limits) -> Result<Self, Unknown> {
        Ok(Self {
            lower: limits.integer(&self.lower)?,
            upper: limits.integer(&self.upper)?,
        })
    }

    pub fn singleton(value: &Integer, limits: &mut Limits) -> Result<Self, Unknown> {
        Ok(Self {
            lower: limits.integer(value)?,
            upper: limits.integer(value)?,
        })
    }

    pub fn add(&self, other: &Self, limits: &mut Limits) -> Result<Self, Unknown> {
        Ok(Self {
            lower: self.lower.add(&other.lower, &mut limits.arithmetic)?,
            upper: self.upper.add(&other.upper, &mut limits.arithmetic)?,
        })
    }

    pub fn multiply(&self, other: &Self, limits: &mut Limits) -> Result<Self, Unknown> {
        let mut products = Vec::with_capacity(4);
        for a in [&self.lower, &self.upper] {
            for b in [&other.lower, &other.upper] {
                products.push(a.multiply(b, &mut limits.arithmetic)?);
            }
        }
        let mut low = 0;
        let mut high = 0;
        for i in 1..4 {
            if limits.compare(&products[i], &products[low])?.is_lt() {
                low = i;
            }
            if limits.compare(&products[i], &products[high])?.is_gt() {
                high = i;
            }
        }
        Ok(Self {
            lower: limits.integer(&products[low])?,
            upper: limits.integer(&products[high])?,
        })
    }

    fn power(&self, exponent: usize, limits: &mut Limits) -> Result<Self, Unknown> {
        if exponent == 1 {
            return self.copy(limits);
        }
        let low_squared = self.lower.multiply(&self.lower, &mut limits.arithmetic)?;
        let high_squared = self.upper.multiply(&self.upper, &mut limits.arithmetic)?;
        if exponent == 3 {
            return Ok(Self {
                lower: low_squared.multiply(&self.lower, &mut limits.arithmetic)?,
                upper: high_squared.multiply(&self.upper, &mut limits.arithmetic)?,
            });
        }
        debug_assert_eq!(exponent, 2);
        let zero = Integer::zero();
        if !limits.compare(&self.lower, &zero)?.is_lt() {
            Ok(Self {
                lower: low_squared,
                upper: high_squared,
            })
        } else if !limits.compare(&self.upper, &zero)?.is_gt() {
            Ok(Self {
                lower: high_squared,
                upper: low_squared,
            })
        } else {
            let upper = if limits.compare(&low_squared, &high_squared)?.is_gt() {
                low_squared
            } else {
                high_squared
            };
            Ok(Self { lower: zero, upper })
        }
    }

    pub fn within(&self, target: &Self, limits: &mut Limits) -> Result<bool, Unknown> {
        Ok(!limits.compare(&self.lower, &target.lower)?.is_lt()
            && !limits.compare(&self.upper, &target.upper)?.is_gt())
    }
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(interval (lower {}) (upper {}))",
            self.lower, self.upper
        )
    }
}

/// Keys are sorted multisets of at most three versioned value identities.
/// Coefficients are canonical and zero terms are absent.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Polynomial {
    terms: BTreeMap<Vec<ValueId>, Integer>,
}

impl Polynomial {
    pub fn terms(&self) -> &BTreeMap<Vec<ValueId>, Integer> {
        &self.terms
    }

    pub fn constant(value: Integer, limits: &mut Limits) -> Result<Self, Unknown> {
        let mut result = Self::default();
        result.insert(vec![], &value, limits)?;
        Ok(result)
    }

    pub fn variable(value: ValueId, limits: &mut Limits) -> Result<Self, Unknown> {
        let mut result = Self::default();
        result.insert(vec![value], &Integer::from_u128(1), limits)?;
        Ok(result)
    }

    pub fn insert(
        &mut self,
        factors: Vec<ValueId>,
        coefficient: &Integer,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        limits.step(1 + factors.len() + self.terms.len().max(1).ilog2() as usize)?;
        if factors.len() > 3 {
            return Err(Unknown::DegreeLimit);
        }
        let coefficient = if let Some(old) = self.terms.get(&factors) {
            old.add(coefficient, &mut limits.arithmetic)?
        } else {
            limits.integer(coefficient)?
        };
        if coefficient.is_zero() {
            self.terms.remove(&factors);
            return Ok(());
        }
        if !self.terms.contains_key(&factors) && self.terms.len() >= limits.max_terms {
            return Err(Unknown::TermLimit);
        }
        self.terms.insert(factors, coefficient);
        Ok(())
    }

    pub fn add(&self, other: &Self, limits: &mut Limits) -> Result<Self, Unknown> {
        limits.step(1)?;
        let mut result = Self::default();
        for (factors, c) in self.terms.iter().chain(&other.terms) {
            result.insert(factors.clone(), c, limits)?;
        }
        Ok(result)
    }

    pub fn subtract(&self, other: &Self, limits: &mut Limits) -> Result<Self, Unknown> {
        limits.step(1)?;
        let mut result = Self::default();
        for (factors, c) in &self.terms {
            result.insert(factors.clone(), c, limits)?;
        }
        for (factors, c) in &other.terms {
            result.insert(factors.clone(), &c.negate(&mut limits.arithmetic)?, limits)?;
        }
        Ok(result)
    }

    pub fn multiply(&self, other: &Self, limits: &mut Limits) -> Result<Self, Unknown> {
        let mut result = Self::default();
        limits.step(1)?;
        for (a, x) in &self.terms {
            for (b, y) in &other.terms {
                limits.step(1)?;
                if a.len() + b.len() > 3 {
                    return Err(Unknown::DegreeLimit);
                }
                let mut factors = a.clone();
                factors.extend(b);
                factors.sort_unstable();
                let coefficient = x.multiply(y, &mut limits.arithmetic)?;
                result.insert(factors, &coefficient, limits)?;
            }
        }
        Ok(result)
    }

    pub fn copy(&self, limits: &mut Limits) -> Result<Self, Unknown> {
        self.add(&Self::default(), limits)
    }

    pub fn interval_enclosure(
        &self,
        facts: &Facts,
        limits: &mut Limits,
    ) -> Result<(Interval, BTreeMap<ValueId, Interval>), Unknown> {
        self.interval_enclosure_with_override(facts, None, limits)
    }

    /// Temporary interval-box restriction for discrete step proofs. This does
    /// not alter facts, versions, or propagate relational assumptions.
    fn interval_enclosure_with_override(
        &self,
        facts: &Facts,
        override_bound: Option<(ValueId, &Interval)>,
        limits: &mut Limits,
    ) -> Result<(Interval, BTreeMap<ValueId, Interval>), Unknown> {
        if !facts.reachable {
            return Err(Unknown::UnreachablePath);
        }
        let mut result = Interval::singleton(&Integer::zero(), limits)?;
        let mut premises = BTreeMap::new();
        for (factors, coefficient) in &self.terms {
            limits.step(1)?;
            let mut term = Interval::singleton(coefficient, limits)?;
            let mut index = 0;
            while index < factors.len() {
                let id = factors[index];
                let mut end = index + 1;
                while end < factors.len() && factors[end] == id {
                    end += 1;
                }
                let bound = facts.bound(id, limits)?;
                let bound = override_bound
                    .filter(|(selected, _)| *selected == id)
                    .map(|(_, replacement)| replacement)
                    .unwrap_or(bound);
                term = term.multiply(&bound.power(end - index, limits)?, limits)?;
                if !premises.contains_key(&id) {
                    premises.insert(id, bound.copy(limits)?);
                }
                index = end;
            }
            result = result.add(&term, limits)?;
        }
        Ok((result, premises))
    }

    pub fn prove_within(&self, target: &Interval, facts: &Facts, limits: &mut Limits) -> Outcome {
        let result = (|| {
            let (enclosure, premises) = self.enclosure_for_target(target, facts, limits)?;
            if !enclosure.within(target, limits)? {
                return Err(Unknown::InsufficientBounds);
            }
            Ok(RangeProof {
                lineage: Lineage(facts.versions.clone()),
                expression: self.copy(limits)?,
                premises,
                inequalities: facts.copy_relations(limits)?,
                enclosure,
                target: target.copy(limits)?,
            })
        })();
        match result {
            Ok(proof) => Outcome::Proven(proof),
            Err(reason) => Outcome::Unknown(reason),
        }
    }
}

impl fmt::Display for Polynomial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(polynomial (terms")?;
        for (factors, coefficient) in &self.terms {
            write!(f, " (term (coefficient {coefficient}) (factors")?;
            for factor in factors {
                write!(f, " {factor}")?;
            }
            f.write_str("))")?;
        }
        f.write_str("))")
    }
}

/// Assignment/invalidation advances a scalar place's version. This is not an
/// alias model: callers must invalidate every affected place for memory writes.
pub struct Facts {
    current: BTreeMap<PlaceId, (u64, Option<Interval>)>,
    inequalities: relations::RelationStore,
    conditions: Vec<(Condition, bool)>,
    // Forks share a clock: two assignments on different paths must never
    // accidentally denote the same value, even if their ranges are equal.
    versions: Arc<AtomicU64>,
    reachable: bool,
}

impl Default for Facts {
    fn default() -> Self {
        Self {
            current: BTreeMap::new(),
            inequalities: relations::RelationStore::default(),
            conditions: Vec::new(),
            versions: Arc::new(AtomicU64::new(0)),
            reachable: true,
        }
    }
}

impl Facts {
    /// Branches can share value identities only when they share this clock.
    pub fn shares_lineage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.versions, &other.versions)
    }

    pub fn current_entries(&self) -> &BTreeMap<PlaceId, (u64, Option<Interval>)> {
        &self.current
    }

    pub fn relation_rows(&self) -> impl Iterator<Item = &Polynomial> {
        (&self.inequalities).into_iter()
    }

    pub fn contains_relation(
        &self,
        row: &Polynomial,
        limits: &mut Limits,
    ) -> Result<bool, Unknown> {
        self.inequalities.contains(row, limits)
    }

    /// Record a relation already established by the caller's value transfer.
    /// This is an assumption operation, not a proof of the supplied row.
    pub fn assume_relation(&mut self, row: Polynomial, limits: &mut Limits) -> Result<(), Unknown> {
        self.inequalities.insert(row, limits)?;
        Ok(())
    }

    /// Apply an established boolean condition transactionally. Splitting honors
    /// short circuit paths; it never assumes both alternatives of an OR.
    pub fn assume(
        &mut self,
        condition: &Condition,
        truth: bool,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        // Literal truth adds no relation and cannot refine a value domain.
        // In particular wildcard patterns should not copy and re-solve every
        // existing polynomial each time they split their impossible failure.
        if let Condition::Bool(value) = condition {
            limits.step(1)?;
            self.reachable &= *value == truth;
            return Ok(());
        }
        if !self.reachable {
            return Ok(());
        }
        // Reasserting an entailed premise cannot refine this state. Keep its
        // logical form for later joins, but do not replay every saved choice.
        let known = self.known_condition(condition, limits)?;
        let known = match known {
            Some(value) => Some(value),
            None if matches!(condition, Condition::And(..) | Condition::Or(..)) => None,
            None => match condition.truth_by_enclosure(self, limits, 0) {
                Ok(value) => value,
                Err(Unknown::MissingBound(_)) => None,
                Err(error) => return Err(error),
            },
        };
        if let Some(value) = known {
            if value != truth {
                self.reachable = false;
            } else {
                self.remember_condition(condition, truth, limits)?;
            }
            return Ok(());
        }
        let had_retained = !self.conditions.is_empty();
        let mut next = self.fork(limits)?;
        next.assume_inner_mode(condition, truth, 0, limits, true)?;
        next.remember_condition(condition, truth, limits)?;
        if had_retained || next.conditions.len() > 1 {
            next.propagate_retained_conditions(limits)?;
        }
        *self = next;
        Ok(())
    }

    pub(crate) fn inequalities_mention(&self, value: ValueId) -> bool {
        if self.inequalities.is_empty() {
            return false;
        }
        (&self.inequalities)
            .into_iter()
            .any(|row| row.terms.keys().flatten().any(|id| *id == value))
    }

    fn inequalities_mention_place(&self, place: PlaceId) -> bool {
        if self.inequalities.is_empty() {
            return false;
        }
        (&self.inequalities)
            .into_iter()
            .any(|row| row.terms.keys().flatten().any(|id| id.place == place))
    }

    fn conditions_mention_place(
        &self,
        place: PlaceId,
        limits: &mut Limits,
    ) -> Result<bool, Unknown> {
        for (condition, _) in &self.conditions {
            if condition_mentions_place(condition, place, 0, limits)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn can_skip_independent_disjunction(
        &self,
        disjuncts: &[(&Condition, bool)],
        limits: &mut Limits,
    ) -> Result<bool, Unknown> {
        if disjuncts.len() < 2 {
            return Ok(false);
        }
        let mut counts: BTreeMap<PlaceId, usize> = BTreeMap::new();
        for (disjunct, _) in disjuncts {
            collect_condition_places(disjunct, 0, limits, &mut counts)?;
        }
        let mut first_place: Option<PlaceId> = None;
        for &(disjunct, enclosure_unknown_bounded) in disjuncts {
            if !enclosure_unknown_bounded {
                continue;
            }
            let mut inner = disjunct;
            while let Condition::Not(next) = inner {
                limits.step(1)?;
                inner = next;
            }
            let Condition::Compare { value, .. } = inner else {
                continue;
            };
            if self.value(value.place) != Some(*value) {
                continue;
            }
            if counts.get(&value.place).copied() != Some(1) {
                continue;
            }
            if self.inequalities_mention_place(value.place)
                || self.conditions_mention_place(value.place, limits)?
            {
                continue;
            }
            if let Some(prev) = first_place {
                if prev != value.place {
                    return Ok(true);
                }
            } else {
                first_place = Some(value.place);
            }
        }
        Ok(false)
    }

    fn disjunction_has_noop_branch(
        &self,
        disjuncts: &[(&Condition, bool)],
        truth: bool,
        limits: &mut Limits,
    ) -> Result<bool, Unknown> {
        if disjuncts.len() < 2 {
            return Ok(false);
        }
        for &(disjunct, _) in disjuncts {
            if self.known_condition(disjunct, limits)? == Some(!truth) {
                continue;
            }
            let mut eff_truth = truth;
            let mut inner = disjunct;
            while let Condition::Not(next) = inner {
                limits.step(1)?;
                eff_truth = !eff_truth;
                inner = next;
            }
            if self.known_condition(inner, limits)? == Some(!eff_truth) {
                continue;
            }
            match inner {
                Condition::PolynomialCompare {
                    left,
                    relation,
                    right,
                } => {
                    let effective = if eff_truth {
                        *relation
                    } else {
                        relation.negated()
                    };
                    if effective == Comparison::NotEqual
                        && self.relation_is_current(left, limits)?
                        && self.relation_is_current(right, limits)?
                    {
                        let difference = left.subtract(right, limits)?;
                        match difference.enclosure(self, limits) {
                            Ok((bound, _)) => {
                                if !(bound.lower.is_zero() && bound.upper.is_zero()) {
                                    return Ok(true);
                                }
                            }
                            Err(Unknown::MissingBound(_)) => {}
                            Err(error) => return Err(error),
                        }
                    }
                }
                Condition::Compare {
                    value,
                    relation,
                    constant,
                } => {
                    let effective = if eff_truth {
                        *relation
                    } else {
                        relation.negated()
                    };
                    if effective == Comparison::NotEqual {
                        match self.bound(*value, limits) {
                            Ok(bound) => {
                                if limits.compare(&bound.lower, constant)?.is_lt()
                                    && limits.compare(constant, &bound.upper)?.is_lt()
                                {
                                    return Ok(true);
                                }
                            }
                            Err(Unknown::MissingBound(_) | Unknown::StaleValue(_)) => {}
                            Err(error) => return Err(error),
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }

    fn assume_inner(
        &mut self,
        condition: &Condition,
        truth: bool,
        depth: usize,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        self.assume_inner_mode(condition, truth, depth, limits, false)
    }

    fn assume_inner_mode(
        &mut self,
        condition: &Condition,
        truth: bool,
        depth: usize,
        limits: &mut Limits,
        skip_known: bool,
    ) -> Result<(), Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        if !self.reachable {
            return Ok(());
        }
        // Conjunctive assumptions need no speculative proof of the entire
        // tree. Each atomic refinement propagates its new relations; process
        // the second operand in that refined state. The public entry owns the
        // transaction, so failure cannot publish a partial conjunction.
        if let Condition::And(a, b) | Condition::Or(a, b) = condition {
            if matches!(condition, Condition::And(..)) == truth {
                self.assume_inner_mode(a, truth, depth + 1, limits, false)?;
                return self.assume_inner_mode(b, truth, depth + 1, limits, false);
            }
        }
        if let Some(known) = if skip_known {
            None
        } else {
            self.known_condition(condition, limits)?
        } {
            if known != truth {
                self.reachable = false;
            }
            return Ok(());
        }
        // Interval truth can eliminate inactive disjuncts before allocating
        // their fork/join states. Disjunctions perform this in one linear pass
        // below so deep left-associated chains avoid quadratic re-traversal.
        if !matches!(condition, Condition::And(..) | Condition::Or(..)) {
            match condition.truth_by_enclosure(self, limits, depth) {
                Ok(Some(actual)) => {
                    self.reachable &= actual == truth;
                    return Ok(());
                }
                Ok(None) | Err(Unknown::MissingBound(_)) => {}
                Err(error) => return Err(error),
            }
        }
        let mut needs_propagation = true;
        match condition {
            Condition::Bool(value) => {
                if *value != truth {
                    self.reachable = false;
                }
                needs_propagation = false;
            }
            Condition::Compare {
                value,
                relation,
                constant,
            } => {
                self.assume_comparison(*value, *relation, constant, truth, limits)?;
                needs_propagation = self.inequalities_mention(*value);
            }
            Condition::PolynomialCompare {
                left,
                relation,
                right,
            } => {
                needs_propagation =
                    self.establish_relation(left, *relation, right, truth, limits)?;
            }
            Condition::Not(inner) => {
                self.assume_inner(inner, !truth, depth + 1, limits)?;
                needs_propagation = false;
            }
            Condition::And(..) | Condition::Or(..) => {
                let mut disjuncts = Vec::new();
                collect_disjuncts(condition, truth, depth, limits, &mut disjuncts)?;
                let mut surviving: Vec<(&Condition, bool)> = Vec::new();
                for disjunct in disjuncts {
                    match disjunct.truth_by_enclosure(self, limits, depth + 1) {
                        Ok(Some(actual)) if actual == truth => return Ok(()),
                        Ok(Some(_)) => {}
                        Ok(None) => surviving.push((disjunct, true)),
                        Err(Unknown::MissingBound(_)) => surviving.push((disjunct, false)),
                        Err(error) => return Err(error),
                    }
                }
                if surviving.is_empty() {
                    self.reachable = false;
                    return Ok(());
                }
                if surviving.len() == 1 {
                    return self.assume_inner(surviving[0].0, truth, depth + 1, limits);
                }
                if self.can_skip_independent_disjunction(&surviving, limits)?
                    || self.disjunction_has_noop_branch(&surviving, truth, limits)?
                {
                    needs_propagation = false;
                } else {
                    let mut remainder = self.fork(limits)?;
                    let mut joined: Option<Facts> = None;
                    let mut unchanged = false;
                    for (index, (disjunct, _)) in surviving.iter().enumerate() {
                        if !remainder.reachable {
                            break;
                        }
                        if index + 1 == surviving.len() {
                            remainder.assume_inner(disjunct, truth, depth + 1, limits)?;
                            if remainder.reachable {
                                joined = Some(match joined {
                                    Some(previous) => previous.join(&remainder, limits)?,
                                    None => remainder,
                                });
                            }
                            break;
                        }
                        let mut branch = remainder.fork(limits)?;
                        branch.assume_inner(disjunct, truth, depth + 1, limits)?;
                        if branch.reachable {
                            joined = Some(match joined {
                                Some(previous) => previous.join(&branch, limits)?,
                                None => branch,
                            });
                            if joined.as_ref().is_some_and(|state| {
                                state.current == self.current
                                    && state.inequalities == self.inequalities
                            }) {
                                unchanged = true;
                                break;
                            }
                        }
                        remainder.assume_inner(disjunct, !truth, depth + 1, limits)?;
                    }
                    if unchanged {
                        needs_propagation = false;
                    } else {
                        match joined {
                            Some(result) => {
                                if result.current == self.current
                                    && result.inequalities == self.inequalities
                                {
                                    needs_propagation = false;
                                }
                                *self = result;
                            }
                            None => self.reachable = false,
                        }
                    }
                }
            }
        }
        if needs_propagation {
            self.propagate_relations(limits)?;
        }
        Ok(())
    }

    pub fn fresh_version(&self) -> Result<u64, Unknown> {
        self.versions
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Unknown::VersionExhausted)
    }

    fn replace(&mut self, place: PlaceId, bound: Option<Interval>) -> Result<ValueId, Unknown> {
        if !self.reachable {
            return Err(Unknown::UnreachablePath);
        }
        let version = self.fresh_version()?;
        self.current.insert(place, (version, bound));
        Ok(ValueId { place, version })
    }

    pub fn assign(&mut self, place: PlaceId, bound: Interval) -> Result<ValueId, Unknown> {
        self.replace(place, Some(bound))
    }

    pub fn invalidate(&mut self, place: PlaceId) -> Result<ValueId, Unknown> {
        self.replace(place, None)
    }

    pub fn bound(&self, id: ValueId, limits: &mut Limits) -> Result<&Interval, Unknown> {
        limits.step(1 + self.current.len().max(1).ilog2() as usize)?;
        if !self.reachable {
            return Err(Unknown::UnreachablePath);
        }
        match self.current.get(&id.place) {
            Some((version, Some(bound))) if *version == id.version => Ok(bound),
            Some((version, _)) if *version != id.version => Err(Unknown::StaleValue(id)),
            _ => Err(Unknown::MissingBound(id)),
        }
    }

    pub fn retain_places(
        &mut self,
        places: &std::collections::BTreeSet<PlaceId>,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        limits.step(self.current.len())?;
        let selected = self
            .current
            .iter()
            .filter(|(place, _)| places.contains(place))
            .map(|(place, (version, _))| ValueId {
                place: *place,
                version: *version,
            })
            .collect();
        // A scoped binding may be the intermediate in a useful relation.
        // Project before removing it; only consequences about surviving values
        // remain, never the vanished binding's identity.
        let projected = if self.reachable {
            differences::project_guarantees(self, &selected, limits)?
        } else {
            Vec::new()
        };
        let mut rows = Vec::new();
        for row in self.copy_relations(limits)? {
            limits.step(row.terms.len())?;
            if row
                .terms
                .keys()
                .flatten()
                .all(|id| places.contains(&id.place))
            {
                rows.push(row);
            }
        }
        for row in projected {
            if rows.len() >= limits.max_terms {
                break;
            }
            if !relations::contains_relation(&rows, &row, limits)? {
                rows.push(row);
            }
        }
        self.current.retain(|place, _| places.contains(place));
        self.inequalities = relations::RelationStore::from_rows(rows, limits)?;
        Ok(())
    }

    pub fn is_reachable(&self) -> bool {
        self.reachable
    }

    pub fn value(&self, place: PlaceId) -> Option<ValueId> {
        self.current.get(&place).map(|(version, _)| ValueId {
            place,
            version: *version,
        })
    }

    /// Snapshot for a control-flow branch. Copying charges the shared budget;
    /// assignment identities remain unique across all forks of this analysis.
    pub fn fork(&self, limits: &mut Limits) -> Result<Self, Unknown> {
        limits.step(1)?;
        let mut current = BTreeMap::new();
        for (place, (version, bound)) in &self.current {
            limits.step(1)?;
            current.insert(
                *place,
                (
                    *version,
                    bound.as_ref().map(|b| b.copy(limits)).transpose()?,
                ),
            );
        }
        Ok(Self {
            current,
            versions: self.versions.clone(),
            reachable: self.reachable,
            inequalities: relations::RelationStore::from_rows(
                self.copy_relations(limits)?,
                limits,
            )?,
            conditions: self.copy_conditions(limits)?,
        })
    }

    /// Intersect a current value with facts established on this path. This
    /// does not execute a check or prove the supplied premise. Its caller must
    /// only use conditions known true (e.g. after a successful checked assumes).
    /// Contradiction marks the path unreachable; exhaustion leaves it unchanged.
    pub fn refine(
        &mut self,
        id: ValueId,
        lower: Option<&Integer>,
        upper: Option<&Integer>,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        let old = self.bound(id, limits)?;
        let lower = match lower {
            Some(x) if limits.compare(x, &old.lower)?.is_gt() => x,
            _ => &old.lower,
        };
        let upper = match upper {
            Some(x) if limits.compare(x, &old.upper)?.is_lt() => x,
            _ => &old.upper,
        };
        let empty = limits.compare(lower, upper)?.is_gt();
        let next = if empty {
            None
        } else {
            Some(Interval::new(
                limits.integer(lower)?,
                limits.integer(upper)?,
                limits,
            )?)
        };
        if empty {
            self.reachable = false;
        } else {
            self.current.insert(id.place, (id.version, next));
        }
        Ok(())
    }

    /// Record the selected truth branch of an integer comparison against a
    /// constant. Disequality only removes an endpoint; an interior hole cannot
    /// be represented by this convex domain and is conservatively retained.
    pub fn assume_comparison(
        &mut self,
        id: ValueId,
        relation: Comparison,
        constant: &Integer,
        truth: bool,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        let constant = limits.integer(constant)?;
        let relation = if truth { relation } else { relation.negated() };
        if self.value(id.place) != Some(id) {
            return Err(Unknown::StaleValue(id));
        }
        match self.bound(id, limits) {
            Err(Unknown::MissingBound(_)) => {
                // An unbounded mathematical integer still has a stable value
                // identity. Keep a half-line/excluded point without inventing
                // a finite machine bound.
                let condition = Condition::Compare {
                    value: id,
                    relation,
                    constant,
                };
                if self.known_condition(&condition, limits)? == Some(false) {
                    self.reachable = false;
                    return Ok(());
                }
                self.remember_condition(&condition, true, limits)?;
                return self.bound_from_retained_comparisons(id, limits);
            }
            Err(e) => return Err(e),
            Ok(_) => {}
        }
        match relation {
            Comparison::Equal => self.refine(id, Some(&constant), Some(&constant), limits),
            Comparison::LessEqual => self.refine(id, None, Some(&constant), limits),
            Comparison::GreaterEqual => self.refine(id, Some(&constant), None, limits),
            Comparison::Less => {
                let upper = constant.subtract(&Integer::from_u128(1), &mut limits.arithmetic)?;
                self.refine(id, None, Some(&upper), limits)
            }
            Comparison::Greater => {
                let lower = constant.add(&Integer::from_u128(1), &mut limits.arithmetic)?;
                self.refine(id, Some(&lower), None, limits)
            }
            Comparison::NotEqual => {
                let bound = self.bound(id, limits)?;
                if limits.compare(&bound.lower, &constant)?.is_eq() {
                    let lower = constant.add(&Integer::from_u128(1), &mut limits.arithmetic)?;
                    self.refine(id, Some(&lower), None, limits)
                } else if limits.compare(&bound.upper, &constant)?.is_eq() {
                    let upper =
                        constant.subtract(&Integer::from_u128(1), &mut limits.arithmetic)?;
                    self.refine(id, None, Some(&upper), limits)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Join alternatives using the convex hull of each surviving bound.
    /// Changed identities get a fresh merge value. Missing/unbounded values
    /// lose their bounds, and unreachable alternatives contribute no facts.
    pub fn join(&self, other: &Self, limits: &mut Limits) -> Result<Self, Unknown> {
        limits.step(1)?;
        if !Arc::ptr_eq(&self.versions, &other.versions) {
            return Err(Unknown::UnrelatedFacts);
        }
        if !self.reachable {
            return other.fork(limits);
        }
        if !other.reachable {
            return self.fork(limits);
        }
        let mut left_mapping = BTreeMap::new();
        let mut right_mapping = BTreeMap::new();
        for (place, (left, bound)) in &self.current {
            limits.step(1)?;
            if let (Some(_), Some((right, Some(_)))) = (bound, other.current.get(place)) {
                if left != right {
                    let merged = ValueId {
                        place: *place,
                        version: self.fresh_version()?,
                    };
                    left_mapping.insert(
                        ValueId {
                            place: *place,
                            version: *left,
                        },
                        merged,
                    );
                    right_mapping.insert(
                        ValueId {
                            place: *place,
                            version: *right,
                        },
                        merged,
                    );
                }
            }
        }
        if !left_mapping.is_empty() {
            let left = self.rename_values(&left_mapping, limits)?;
            let right = other.rename_values(&right_mapping, limits)?;
            // All bounded shared places now use the same fresh merge IDs, so
            // the recursive call cannot perform a second renaming round.
            return left.join(&right, limits);
        }
        let mut current = BTreeMap::new();
        for place in self.current.keys().chain(other.current.keys()) {
            limits.step(1)?;
            if current.contains_key(place) {
                continue;
            }
            let left = self.current.get(place);
            let right = other.current.get(place);
            let version = match (left, right) {
                (Some((a, _)), Some((b, _))) if a == b => *a,
                _ => self.fresh_version()?,
            };
            let bound = match (left, right) {
                (Some((_, Some(a))), Some((_, Some(b)))) => {
                    let lower = if limits.compare(&a.lower, &b.lower)?.is_lt() {
                        &a.lower
                    } else {
                        &b.lower
                    };
                    let upper = if limits.compare(&a.upper, &b.upper)?.is_gt() {
                        &a.upper
                    } else {
                        &b.upper
                    };
                    Some(Interval::new(
                        limits.integer(lower)?,
                        limits.integer(upper)?,
                        limits,
                    )?)
                }
                _ => None,
            };
            current.insert(*place, (version, bound));
        }
        let mut joined = Self {
            current,
            versions: self.versions.clone(),
            reachable: true,
            inequalities: relations::RelationStore::from_rows(
                self.common_relations(other, limits)?,
                limits,
            )?,
            conditions: self.common_conditions(other, limits)?,
        };
        differences::join_guarantees(self, other, &mut joined, limits)?;
        Ok(joined)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

/// Program condition over current scalar values; neither a type proposition
/// nor executable code. Its source check/control edge establishes its truth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    Bool(bool),
    Compare {
        value: ValueId,
        relation: Comparison,
        constant: Integer,
    },
    PolynomialCompare {
        left: Polynomial,
        relation: Comparison,
        right: Polynomial,
    },
    Not(Box<Condition>),
    And(Box<Condition>, Box<Condition>),
    Or(Box<Condition>, Box<Condition>),
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => write!(f, "(bool {value})"),
            Self::Compare {
                value,
                relation,
                constant,
            } => write!(
                f,
                "(compare (op {relation}) (value {value}) (constant {constant}))"
            ),
            Self::PolynomialCompare {
                left,
                relation,
                right,
            } => write!(
                f,
                "(polynomial-compare (op {relation}) (left {left}) (right {right}))"
            ),
            Self::Not(inner) => write!(f, "(not {inner})"),
            Self::And(a, b) => write!(f, "(and {a} {b})"),
            Self::Or(a, b) => write!(f, "(or {a} {b})"),
        }
    }
}

impl Comparison {
    pub fn negated(self) -> Self {
        match self {
            Self::Equal => Self::NotEqual,
            Self::NotEqual => Self::Equal,
            Self::Less => Self::GreaterEqual,
            Self::LessEqual => Self::Greater,
            Self::Greater => Self::LessEqual,
            Self::GreaterEqual => Self::Less,
        }
    }
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Equal => "eq",
            Self::NotEqual => "ne",
            Self::Less => "lt",
            Self::LessEqual => "le",
            Self::Greater => "gt",
            Self::GreaterEqual => "ge",
        })
    }
}

// Proofs are tied to one analysis and its forks. Equal local/version numbers
// in an unrelated analysis cannot authorize replay of an old proof.
#[derive(Clone, Debug)]
struct Lineage(Arc<AtomicU64>);

impl PartialEq for Lineage {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Lineage {}

impl fmt::Display for Facts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(program-facts (reachable {}) (values", self.reachable)?;
        for (place, (version, bound)) in &self.current {
            write!(
                f,
                " (fact {} ",
                ValueId {
                    place: *place,
                    version: *version
                }
            )?;
            match bound {
                Some(bound) => bound.fmt(f)?,
                None => f.write_str("(unknown-bound)")?,
            }
            f.write_str(")")?;
        }
        f.write_str(") (inequalities")?;
        for constraint in &self.inequalities {
            let current = constraint
                .terms
                .keys()
                .flatten()
                .all(|id| self.value(id.place) == Some(*id));
            write!(
                f,
                " ({} {constraint})",
                if current {
                    "less-equal-zero"
                } else {
                    "stale-less-equal-zero"
                }
            )?;
        }
        f.write_str(") (conditions")?;
        for (condition, truth) in &self.conditions {
            // Retained conditions can outlive a write to their inputs. Their
            // presence in storage is not evidence about the new versions.
            // Display never changes facts or turns a failed bounded inspection
            // into a claim that the condition is current (or definitely stale).
            let status = match condition.is_current(self, &mut Limits::default(), 0) {
                Ok(true) => "established",
                Ok(false) => "stale-condition",
                Err(_) => "unchecked-condition",
            };
            write!(f, " ({status} (truth {truth}) {condition})")?;
        }
        f.write_str("))")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeProof {
    lineage: Lineage,
    expression: Polynomial,
    premises: BTreeMap<ValueId, Interval>,
    inequalities: Vec<Polynomial>,
    enclosure: Interval,
    target: Interval,
}

impl RangeProof {
    pub fn enclosure(&self) -> &Interval {
        &self.enclosure
    }

    pub fn replay(&self, facts: &Facts, limits: &mut Limits) -> Result<(), Unknown> {
        limits.step(1)?;
        if !Arc::ptr_eq(&self.lineage.0, &facts.versions) {
            return Err(Unknown::UnrelatedFacts);
        }
        for constraint in &self.inequalities {
            if !facts.relation_is_current(constraint, limits)?
                || !facts.inequalities.contains(constraint, limits)?
            {
                return Err(Unknown::InsufficientBounds);
            }
        }
        for (id, bound) in &self.premises {
            let actual = facts.bound(*id, limits)?;
            if !limits.compare(&actual.lower, &bound.lower)?.is_eq()
                || !limits.compare(&actual.upper, &bound.upper)?.is_eq()
            {
                return Err(Unknown::StaleValue(*id));
            }
        }
        let (enclosure, _) = self
            .expression
            .enclosure_for_target(&self.target, facts, limits)?;
        if enclosure.within(&self.target, limits)? {
            Ok(())
        } else {
            Err(Unknown::InsufficientBounds)
        }
    }
}

impl fmt::Display for RangeProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(range-proof (expression {}) (premises", self.expression)?;
        for (id, bound) in &self.premises {
            write!(f, " (bound {id} {bound})")?;
        }
        f.write_str(") (inequalities")?;
        for constraint in &self.inequalities {
            write!(f, " (less-equal-zero {constraint})")?;
        }
        write!(
            f,
            ") (enclosure {}) (target {}))",
            self.enclosure, self.target
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Proven(RangeProof),
    Unknown(Unknown),
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Proven(proof) => write!(f, "(proven {proof})"),
            Self::Unknown(reason) => reason.fmt(f),
        }
    }
}

/// Infer a candidate constant endpoint difference from the side-effect-free
/// integer fragment. This is not a safety proof: actual evaluation, overflow,
/// versions and bounds must still be checked by program flow.
pub fn constant_integer_difference(
    start: &nir::program::Expr,
    end: &nir::program::Expr,
    limits: &mut Limits,
) -> Result<Option<Integer>, Unknown> {
    use nir::program::{Expr, ExprKind as K, IntegerOp, Literal};
    fn expression(
        expr: &Expr,
        limits: &mut Limits,
        depth: usize,
    ) -> Result<Option<Polynomial>, Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        let value = match &expr.kind {
            K::Literal(Literal::UnsignedInteger(n)) => {
                Polynomial::constant(Integer::from_u128(*n), limits)?
            }
            K::Literal(Literal::SignedInteger(n)) => {
                Polynomial::constant(Integer::from_i128(*n), limits)?
            }
            K::Literal(Literal::Integer(n)) => {
                let n = Integer::parse_literal(n, &mut limits.arithmetic)?;
                Polynomial::constant(n, limits)?
            }
            K::Local(id) => Polynomial::variable(
                ValueId {
                    place: PlaceId {
                        owner: u64::from(id.owner),
                        local: id.index,
                    },
                    version: 0,
                },
                limits,
            )?,
            K::IntegerNegate(value) => {
                let Some(value) = expression(value, limits, depth + 1)? else {
                    return Ok(None);
                };
                Polynomial::default().subtract(&value, limits)?
            }
            K::IntegerBinary { op, left, right } => {
                let Some(left) = expression(left, limits, depth + 1)? else {
                    return Ok(None);
                };
                let Some(right) = expression(right, limits, depth + 1)? else {
                    return Ok(None);
                };
                match op {
                    IntegerOp::Add => left.add(&right, limits)?,
                    IntegerOp::Subtract => left.subtract(&right, limits)?,
                    IntegerOp::Multiply => left.multiply(&right, limits)?,
                    _ => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }
    let Some(start) = expression(start, limits, 0)? else {
        return Ok(None);
    };
    let Some(end) = expression(end, limits, 0)? else {
        return Ok(None);
    };
    let difference = end.subtract(&start, limits)?;
    if difference.terms.keys().any(|factors| !factors.is_empty()) {
        return Ok(None);
    }
    difference
        .terms
        .get(&Vec::new())
        .map(|n| limits.integer(n))
        .transpose()
        .map(|n| Some(n.unwrap_or_else(Integer::zero)))
}

fn collect_disjuncts<'a>(
    condition: &'a Condition,
    truth: bool,
    depth: usize,
    limits: &mut Limits,
    out: &mut Vec<&'a Condition>,
) -> Result<(), Unknown> {
    limits.step(1)?;
    if depth >= 128 {
        return Err(Unknown::ConditionDepthLimit);
    }
    match condition {
        Condition::Or(a, b) if truth => {
            collect_disjuncts(a, truth, depth + 1, limits, out)?;
            collect_disjuncts(b, truth, depth + 1, limits, out)
        }
        Condition::And(a, b) if !truth => {
            collect_disjuncts(a, truth, depth + 1, limits, out)?;
            collect_disjuncts(b, truth, depth + 1, limits, out)
        }
        _ => {
            out.push(condition);
            Ok(())
        }
    }
}

fn polynomial_mentions_place(
    poly: &Polynomial,
    place: PlaceId,
    limits: &mut Limits,
) -> Result<bool, Unknown> {
    for factors in poly.terms.keys() {
        limits.step(1)?;
        if factors.iter().any(|id| id.place == place) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn condition_mentions_place(
    condition: &Condition,
    place: PlaceId,
    depth: usize,
    limits: &mut Limits,
) -> Result<bool, Unknown> {
    limits.step(1)?;
    if depth >= 128 {
        return Err(Unknown::ConditionDepthLimit);
    }
    match condition {
        Condition::Bool(_) => Ok(false),
        Condition::Compare { value, .. } => Ok(value.place == place),
        Condition::PolynomialCompare { left, right, .. } => {
            Ok(polynomial_mentions_place(left, place, limits)?
                || polynomial_mentions_place(right, place, limits)?)
        }
        Condition::Not(inner) => condition_mentions_place(inner, place, depth + 1, limits),
        Condition::And(a, b) | Condition::Or(a, b) => {
            Ok(condition_mentions_place(a, place, depth + 1, limits)?
                || condition_mentions_place(b, place, depth + 1, limits)?)
        }
    }
}

fn collect_condition_places(
    condition: &Condition,
    depth: usize,
    limits: &mut Limits,
    out: &mut BTreeMap<PlaceId, usize>,
) -> Result<(), Unknown> {
    limits.step(1)?;
    if depth >= 128 {
        return Err(Unknown::ConditionDepthLimit);
    }
    match condition {
        Condition::Bool(_) => Ok(()),
        Condition::Compare { value, .. } => {
            *out.entry(value.place).or_default() += 1;
            Ok(())
        }
        Condition::PolynomialCompare { left, right, .. } => {
            for poly in [left, right] {
                for factors in poly.terms.keys() {
                    limits.step(1)?;
                    for id in factors {
                        *out.entry(id.place).or_default() += 1;
                    }
                }
            }
            Ok(())
        }
        Condition::Not(inner) => collect_condition_places(inner, depth + 1, limits, out),
        Condition::And(a, b) | Condition::Or(a, b) => {
            collect_condition_places(a, depth + 1, limits, out)?;
            collect_condition_places(b, depth + 1, limits, out)
        }
    }
}

#[cfg(test)]
mod constant_assumption_tests {
    use super::*;

    #[test]
    fn constant_assumptions_preserve_facts_without_resolving_them() {
        let mut setup = Limits::default();
        let mut facts = Facts::default();
        for index in 0..32 {
            facts
                .assign(
                    PlaceId {
                        owner: 0,
                        local: index,
                    },
                    Interval::new(Integer::zero(), Integer::from_i128(1), &mut setup).unwrap(),
                )
                .unwrap();
        }
        let before = facts.to_string();
        let mut limits = Limits::default();
        limits.arithmetic.steps = 2;
        facts
            .assume(&Condition::Bool(true), true, &mut limits)
            .unwrap();
        facts
            .assume(&Condition::Bool(false), false, &mut limits)
            .unwrap();
        assert_eq!(facts.to_string(), before);
        assert_eq!(limits.arithmetic.steps, 0);
        assert!(
            facts
                .assume(&Condition::Bool(false), true, &mut limits)
                .is_err()
        );
        assert_eq!(facts.to_string(), before);
        limits.arithmetic.steps = 1;
        facts
            .assume(&Condition::Bool(false), true, &mut limits)
            .unwrap();
        assert!(!facts.is_reachable());
        limits.arithmetic.steps = 1;
        facts
            .assume(&Condition::Bool(true), true, &mut limits)
            .unwrap();
        assert!(!facts.is_reachable());
    }
}

#[cfg(test)]
mod disjunction_complexity_tests {
    use super::*;

    #[test]
    fn flattened_disjunction_avoids_exponential_splitting_and_preserves_short_circuit() {
        let mut setup = Limits::default();
        let mut facts = Facts::default();
        let start = facts
            .assign(
                PlaceId { owner: 1, local: 0 },
                Interval::machine(64, false, &mut setup).unwrap(),
            )
            .unwrap();
        let len = facts
            .assign(
                PlaceId { owner: 1, local: 1 },
                Interval::machine(64, false, &mut setup).unwrap(),
            )
            .unwrap();
        let b0 = facts
            .assign(
                PlaceId { owner: 1, local: 2 },
                Interval::machine(8, false, &mut setup).unwrap(),
            )
            .unwrap();
        let b1 = facts
            .assign(
                PlaceId { owner: 1, local: 3 },
                Interval::machine(8, false, &mut setup).unwrap(),
            )
            .unwrap();
        let b2 = facts
            .assign(
                PlaceId { owner: 1, local: 4 },
                Interval::machine(8, false, &mut setup).unwrap(),
            )
            .unwrap();
        let b3 = facts
            .assign(
                PlaceId { owner: 1, local: 5 },
                Interval::machine(8, false, &mut setup).unwrap(),
            )
            .unwrap();
        // Establish unrelated difference relations: start < len and len - start >= 4.
        let start_poly = Polynomial::variable(start, &mut setup).unwrap();
        let len_poly = Polynomial::variable(len, &mut setup).unwrap();
        facts
            .assume(
                &Condition::PolynomialCompare {
                    left: start_poly.clone(),
                    relation: Comparison::Less,
                    right: len_poly.clone(),
                },
                true,
                &mut setup,
            )
            .unwrap();
        let diff = len_poly.subtract(&start_poly, &mut setup).unwrap();
        facts
            .assume(
                &Condition::PolynomialCompare {
                    left: diff,
                    relation: Comparison::GreaterEqual,
                    right: Polynomial::constant(Integer::from_u128(4), &mut setup).unwrap(),
                },
                true,
                &mut setup,
            )
            .unwrap();
        assert!(facts.value(b0.place) == Some(b0));

        // 6-way left-associated disjunction: b1 < 128 || b1 > 191 || b2 < 128 || ...
        let mut disj = Condition::Or(
            Box::new(Condition::Compare {
                value: b1,
                relation: Comparison::Less,
                constant: Integer::from_u128(128),
            }),
            Box::new(Condition::Compare {
                value: b1,
                relation: Comparison::Greater,
                constant: Integer::from_u128(191),
            }),
        );
        for val in [b2, b3] {
            disj = Condition::Or(
                Box::new(disj),
                Box::new(Condition::Compare {
                    value: val,
                    relation: Comparison::Less,
                    constant: Integer::from_u128(128),
                }),
            );
            disj = Condition::Or(
                Box::new(disj),
                Box::new(Condition::Compare {
                    value: val,
                    relation: Comparison::Greater,
                    constant: Integer::from_u128(191),
                }),
            );
        }

        let mut yes = facts.fork(&mut setup).unwrap();
        let mut yes_limits = Limits::default();
        yes_limits.arithmetic.steps = 100_000;
        yes.assume(&disj, true, &mut yes_limits).unwrap();
        assert!(yes.is_reachable());

        let mut no = facts.fork(&mut setup).unwrap();
        let mut no_limits = Limits::default();
        no_limits.arithmetic.steps = 100_000;
        no.assume(&disj, false, &mut no_limits).unwrap();
        assert!(no.is_reachable());
        for val in [b1, b2, b3] {
            let bound = no.bound(val, &mut setup).unwrap();
            assert_eq!(bound.lower, Integer::from_u128(128));
            assert_eq!(bound.upper, Integer::from_u128(191));
        }

        // Verify left-to-right short-circuiting when a prefix disjunct is false.
        let mut sc = Facts::default();
        let x = sc
            .assign(
                PlaceId { owner: 1, local: 9 },
                Interval::new(Integer::from_u128(10), Integer::from_u128(255), &mut setup).unwrap(),
            )
            .unwrap();
        let cond = Condition::Or(
            Box::new(Condition::Compare {
                value: x,
                relation: Comparison::Less,
                constant: Integer::from_u128(10),
            }),
            Box::new(Condition::Compare {
                value: x,
                relation: Comparison::Greater,
                constant: Integer::from_u128(20),
            }),
        );
        sc.assume(&cond, true, &mut setup).unwrap();
        let bound = sc.bound(x, &mut setup).unwrap();
        assert_eq!(bound.lower, Integer::from_u128(21));
        assert_eq!(bound.upper, Integer::from_u128(255));
    }

    #[test]
    fn independent_disjuncts_shortcut_avoids_quadratic_joins_and_preserves_tightening_when_needed()
    {
        let mut setup = Limits::default();
        let mut facts = Facts::default();
        let mut vars = Vec::new();
        for idx in 0..20u32 {
            vars.push(
                facts
                    .assign(
                        PlaceId {
                            owner: 2,
                            local: idx,
                        },
                        Interval::machine(16, false, &mut setup).unwrap(),
                    )
                    .unwrap(),
            );
        }
        let mut conj = Condition::Compare {
            value: vars[0],
            relation: Comparison::Greater,
            constant: Integer::from_u128(10),
        };
        for &v in &vars[1..] {
            conj = Condition::And(
                Box::new(conj),
                Box::new(Condition::Compare {
                    value: v,
                    relation: Comparison::Greater,
                    constant: Integer::from_u128(10),
                }),
            );
        }
        // Negating a 20-way conjunction of independent bounded variables must finish in small budget
        // and still retain the disjunction for later unit propagation.
        let mut small = Limits::default();
        small.arithmetic.steps = 1_500;
        let mut branch = facts.fork(&mut setup).unwrap();
        branch.assume(&conj, false, &mut small).unwrap();
        assert!(branch.is_reachable());
        assert!(branch.conditions.len() == 1);

        // When only ONE disjunct is satisfiable (others are false on their intervals),
        // the shortcut must NOT apply and the surviving disjunct must still tighten.
        let impossible = Condition::Compare {
            value: vars[0],
            relation: Comparison::Greater,
            constant: Integer::from_u128(100_000),
        };
        let surviving = Condition::Compare {
            value: vars[1],
            relation: Comparison::Equal,
            constant: Integer::from_u128(42),
        };
        let mixed = Condition::Or(Box::new(impossible), Box::new(surviving));
        let mut single = facts.fork(&mut setup).unwrap();
        single.assume(&mixed, true, &mut setup).unwrap();
        let bound = single.bound(vars[1], &mut setup).unwrap();
        assert_eq!(bound.lower, Integer::from_u128(42));
        assert_eq!(bound.upper, Integer::from_u128(42));

        // When variables are constrained in `inequalities`, relational propagation must still run.
        let p0 = Polynomial::variable(vars[0], &mut setup).unwrap();
        let p1 = Polynomial::variable(vars[1], &mut setup).unwrap();
        let diff = p0.subtract(&p1, &mut setup).unwrap();
        let zero = Polynomial::constant(Integer::zero(), &mut setup).unwrap();
        let mut rel = facts.fork(&mut setup).unwrap();
        rel.assume(
            &Condition::PolynomialCompare {
                left: diff,
                relation: Comparison::Equal,
                right: zero,
            },
            true,
            &mut setup,
        )
        .unwrap();
        let rel_disj = Condition::Or(
            Box::new(Condition::Compare {
                value: vars[0],
                relation: Comparison::Less,
                constant: Integer::from_u128(5),
            }),
            Box::new(Condition::Compare {
                value: vars[1],
                relation: Comparison::Less,
                constant: Integer::from_u128(5),
            }),
        );
        rel.assume(&rel_disj, true, &mut setup).unwrap();
        assert_eq!(
            rel.bound(vars[0], &mut setup).unwrap().upper,
            Integer::from_u128(4)
        );
        assert_eq!(
            rel.bound(vars[1], &mut setup).unwrap().upper,
            Integer::from_u128(4)
        );
    }

    #[test]
    fn negated_relational_conjunction_skips_noop_join_within_small_budget() {
        let mut setup = Limits::default();
        let mut facts = Facts::default();
        let mut mk = |local| {
            facts
                .assign(
                    PlaceId { owner: 3, local },
                    Interval::machine(64, false, &mut setup).unwrap(),
                )
                .unwrap()
        };
        let a = mk(0);
        let b = mk(1);
        let c = mk(2);
        let d = mk(3);
        let s = mk(4);
        let n = mk(5);
        let p = |id, l: &mut Limits| Polynomial::variable(id, l).unwrap();
        let pa = p(a, &mut setup);
        let pb = p(b, &mut setup);
        let pc = p(c, &mut setup);
        let pd = p(d, &mut setup);
        let ps = p(s, &mut setup);
        let pn = p(n, &mut setup);
        let c_minus_a = pc.subtract(&pa, &mut setup).unwrap();
        let d_minus_a = pd.subtract(&pa, &mut setup).unwrap();

        let cmp = |left, relation, right| Condition::PolynomialCompare {
            left,
            relation,
            right,
        };
        let conj = Condition::And(
            Box::new(Condition::And(
                Box::new(Condition::And(
                    Box::new(Condition::And(
                        Box::new(cmp(pa.clone(), Comparison::GreaterEqual, pb)),
                        Box::new(cmp(pa.clone(), Comparison::LessEqual, pc)),
                    )),
                    Box::new(cmp(pa, Comparison::LessEqual, pd)),
                )),
                Box::new(cmp(c_minus_a, Comparison::Equal, ps.clone())),
            )),
            Box::new(cmp(pn.clone(), Comparison::LessEqual, d_minus_a)),
        );

        let mut small = Limits::default();
        small.arithmetic.steps = 50_000;
        let mut branch = facts.fork(&mut setup).unwrap();
        branch.assume(&conj, false, &mut small).unwrap();
        assert!(branch.is_reachable());

        let n_le_s = cmp(pn.clone(), Comparison::LessEqual, ps.clone());
        branch.assume(&n_le_s, true, &mut small).unwrap();
        assert!(branch.is_reachable());
        let diff = pn.subtract(&ps, &mut setup).unwrap();
        let (enclosure, _) = diff.enclosure(&branch, &mut setup).unwrap();
        assert!(enclosure.upper <= Integer::zero());
        // The negated conjunction must not leak an unnegated `s <= u64::MAX` / equality premise.
        assert_ne!(
            branch.known_condition(&conj, &mut setup).unwrap(),
            Some(true)
        );
    }

    #[test]
    fn refuted_not_equal_disjunct_still_refines_surviving_branch() {
        let mut setup = Limits::default();
        let mut facts = Facts::default();
        let x = facts
            .assign(
                PlaceId { owner: 4, local: 0 },
                Interval::machine(16, false, &mut setup).unwrap(),
            )
            .unwrap();
        let y = facts
            .assign(
                PlaceId { owner: 4, local: 1 },
                Interval::machine(16, false, &mut setup).unwrap(),
            )
            .unwrap();
        let z = facts
            .assign(
                PlaceId { owner: 4, local: 2 },
                Interval::machine(16, false, &mut setup).unwrap(),
            )
            .unwrap();
        let px = Polynomial::variable(x, &mut setup).unwrap();
        let py = Polynomial::variable(y, &mut setup).unwrap();
        facts
            .assume(
                &Condition::PolynomialCompare {
                    left: px.clone(),
                    relation: Comparison::LessEqual,
                    right: py.clone(),
                },
                true,
                &mut setup,
            )
            .unwrap();
        facts
            .assume(
                &Condition::PolynomialCompare {
                    left: py.clone(),
                    relation: Comparison::LessEqual,
                    right: px.clone(),
                },
                true,
                &mut setup,
            )
            .unwrap();
        let disj = Condition::Or(
            Box::new(Condition::PolynomialCompare {
                left: px,
                relation: Comparison::NotEqual,
                right: py,
            }),
            Box::new(Condition::Compare {
                value: z,
                relation: Comparison::LessEqual,
                constant: Integer::from_u128(5),
            }),
        );
        facts.assume(&disj, true, &mut setup).unwrap();
        assert_eq!(
            facts.bound(z, &mut setup).unwrap().upper,
            Integer::from_u128(5)
        );
    }
}

pub use differences::project_guarantees;
pub use relations::exact_equal_values;
