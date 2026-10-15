//! Bounded mathematical integer reasoning about versioned program values.
//! Independent of PTC; this module grants neither memory access nor ownership.
pub mod contracts;
pub mod copy;
mod differences;
mod division;
pub mod drop;
pub mod effect_usage;
pub mod flow;
pub mod memory;
pub mod moves;
mod observation_region;
pub mod optimization;
pub mod ownership;
mod pattern_facts;
mod predicates;
mod rejection;
mod relations;
mod retained_conditions;
pub use predicates::PredicateProof;

use comptime_runtime::{Budget, Error as ArithmeticError, Integer};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlaceId {
    pub owner: u64,
    pub local: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
    fn step(&mut self, n: usize) -> Result<(), Unknown> {
        self.arithmetic.consume(n).map_err(Into::into)
    }
    fn integer(&mut self, x: &Integer) -> Result<Integer, Unknown> {
        let limbs = x.bit_len().div_ceil(32);
        if limbs > self.arithmetic.max_limbs {
            return Err(ArithmeticError::SizeLimit.into());
        }
        self.step(limbs.saturating_add(1))?;
        Ok(x.clone())
    }
    fn compare(&mut self, a: &Integer, b: &Integer) -> Result<std::cmp::Ordering, Unknown> {
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

    pub fn lower(&self) -> &Integer {
        &self.lower
    }
    pub fn upper(&self) -> &Integer {
        &self.upper
    }
    fn copy(&self, limits: &mut Limits) -> Result<Self, Unknown> {
        Ok(Self {
            lower: limits.integer(&self.lower)?,
            upper: limits.integer(&self.upper)?,
        })
    }
    fn singleton(value: &Integer, limits: &mut Limits) -> Result<Self, Unknown> {
        Ok(Self {
            lower: limits.integer(value)?,
            upper: limits.integer(value)?,
        })
    }
    fn add(&self, other: &Self, limits: &mut Limits) -> Result<Self, Unknown> {
        Ok(Self {
            lower: self.lower.add(&other.lower, &mut limits.arithmetic)?,
            upper: self.upper.add(&other.upper, &mut limits.arithmetic)?,
        })
    }
    fn multiply(&self, other: &Self, limits: &mut Limits) -> Result<Self, Unknown> {
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

    fn within(&self, target: &Self, limits: &mut Limits) -> Result<bool, Unknown> {
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
    fn insert(
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
    fn copy(&self, limits: &mut Limits) -> Result<Self, Unknown> {
        self.add(&Self::default(), limits)
    }
    fn interval_enclosure(
        &self,
        facts: &Facts,
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
            let (enclosure, premises) = self.enclosure(facts, limits)?;
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
    inequalities: Vec<Polynomial>,
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
            inequalities: Vec::new(),
            conditions: Vec::new(),
            versions: Arc::new(AtomicU64::new(0)),
            reachable: true,
        }
    }
}
impl Facts {
    /// Apply an established boolean condition transactionally. Splitting honors
    /// short circuit paths; it never assumes both alternatives of an OR.
    pub fn assume(
        &mut self,
        condition: &Condition,
        truth: bool,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        let mut next = self.fork(limits)?;
        next.assume_inner(condition, truth, 0, limits)?;
        next.remember_condition(condition, truth, limits)?;
        *self = next;
        Ok(())
    }

    fn assume_inner(
        &mut self,
        condition: &Condition,
        truth: bool,
        depth: usize,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        if !self.reachable {
            return Ok(());
        }
        if let Some(known) = self.known_condition(condition, limits)? {
            if known != truth {
                self.reachable = false;
            }
            return Ok(());
        }
        match condition {
            Condition::Bool(value) => {
                if *value != truth {
                    self.reachable = false;
                }
            }
            Condition::Compare {
                value,
                relation,
                constant,
            } => self.assume_comparison(*value, *relation, constant, truth, limits)?,
            Condition::PolynomialCompare {
                left,
                relation,
                right,
            } => self.establish_relation(left, *relation, right, truth, limits)?,
            Condition::Not(inner) => self.assume_inner(inner, !truth, depth + 1, limits)?,
            Condition::And(a, b) | Condition::Or(a, b) => {
                let sequential = matches!(condition, Condition::And(..)) == truth;
                if sequential {
                    self.assume_inner(a, truth, depth + 1, limits)?;
                    self.assume_inner(b, truth, depth + 1, limits)?;
                } else {
                    let mut short_circuit = self.fork(limits)?;
                    short_circuit.assume_inner(a, truth, depth + 1, limits)?;
                    self.assume_inner(a, !truth, depth + 1, limits)?;
                    self.assume_inner(b, truth, depth + 1, limits)?;
                    *self = self.join(&short_circuit, limits)?;
                }
            }
        }
        self.propagate_relations(limits)
    }

    fn fresh_version(&self) -> Result<u64, Unknown> {
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

    pub(super) fn retain_places(
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
        self.inequalities = rows;
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
            inequalities: self.copy_relations(limits)?,
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
            inequalities: self.common_relations(other, limits)?,
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
            write!(f, " (established (truth {truth}) {condition})")?;
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
                || !relations::contains_relation(&facts.inequalities, constraint, limits)?
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
        let (enclosure, _) = self.expression.enclosure(facts, limits)?;
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
