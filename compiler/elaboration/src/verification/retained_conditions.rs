//! Established Boolean facts retain disjunctions that interval joins forget.
//! Every referenced scalar identity must still denote its original value.
use super::*;

#[cfg(test)]
mod bounded_exclusion_tests {
    use super::*;

    #[test]
    fn rearranged_polynomial_predicates_keep_kind_versions_and_budget() {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let x = facts
            .assign(
                PlaceId { owner: 1, local: 0 },
                Interval::machine(8, true, &mut limits).unwrap(),
            )
            .unwrap();
        let r = facts
            .assign(
                PlaceId { owner: 1, local: 1 },
                Interval::machine(8, true, &mut limits).unwrap(),
            )
            .unwrap();
        let x_poly = Polynomial::variable(x, &mut limits).unwrap();
        let r_poly = Polynomial::variable(r, &mut limits).unwrap();
        let zero = Polynomial::constant(Integer::zero(), &mut limits).unwrap();
        let negative = zero.subtract(&x_poly, &mut limits).unwrap();
        let sum = r_poly.add(&x_poly, &mut limits).unwrap();
        for relation in [Comparison::Equal, Comparison::NotEqual] {
            let original = Condition::PolynomialCompare {
                left: r_poly.clone(),
                relation,
                right: negative.clone(),
            };
            let rearranged = Condition::PolynomialCompare {
                left: sum.clone(),
                relation,
                right: zero.clone(),
            };
            let reversed = Condition::PolynomialCompare {
                left: zero.clone(),
                relation,
                right: sum.clone(),
            };
            let other_kind = Condition::PolynomialCompare {
                left: sum.clone(),
                relation: relation.negated(),
                right: zero.clone(),
            };
            let wrong = Condition::PolynomialCompare {
                left: sum.clone(),
                relation,
                right: Polynomial::constant(Integer::from_i128(1), &mut limits).unwrap(),
            };
            assert!(original.equivalent(&rearranged, &mut limits, 0).unwrap());
            assert!(original.equivalent(&reversed, &mut limits, 0).unwrap());
            assert!(!original.equivalent(&other_kind, &mut limits, 0).unwrap());
            assert!(!original.equivalent(&wrong, &mut limits, 0).unwrap());
            let mut exhausted = Limits::default();
            exhausted.arithmetic.steps = 0;
            assert!(original.equivalent(&rearranged, &mut exhausted, 0).is_err());
            assert!(original.equivalent(&rearranged, &mut limits, 0).unwrap());
            let mut current = facts.fork(&mut limits).unwrap();
            // Bounded equalities live in inequality rows; retained conditions
            // deliberately store unresolved choices rather than duplicate them.
            let alternative = Condition::Compare {
                value: r,
                relation: Comparison::Equal,
                constant: Integer::from_i128(7),
            };
            let original = Condition::Or(Box::new(original), Box::new(alternative.clone()));
            let rearranged = Condition::Or(Box::new(rearranged), Box::new(alternative));
            current
                .remember_condition(&original, true, &mut limits)
                .unwrap();
            current
                .prove_condition(&rearranged, true, &mut limits)
                .unwrap();
            current.invalidate(x.place).unwrap();
            assert!(
                current
                    .prove_condition(&rearranged, true, &mut limits)
                    .is_err()
            );
        }
    }

    #[test]
    fn equal_copy_keeps_its_predicate_after_the_source_retires() {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let a = facts.invalidate(PlaceId { owner: 1, local: 0 }).unwrap();
        let b = facts.invalidate(PlaceId { owner: 1, local: 1 }).unwrap();
        let equality = Condition::PolynomialCompare {
            left: Polynomial::variable(a, &mut limits).unwrap(),
            relation: Comparison::Equal,
            right: Polynomial::variable(b, &mut limits).unwrap(),
        };
        facts.assume(&equality, true, &mut limits).unwrap();
        let predicate = |value| Condition::Compare {
            value,
            relation: Comparison::Greater,
            constant: Integer::from_u128(10),
        };
        facts.assume(&predicate(a), true, &mut limits).unwrap();
        facts.assume(&predicate(b), true, &mut limits).unwrap();
        facts.invalidate(a.place).unwrap();
        facts
            .prove_condition(&predicate(b), true, &mut limits)
            .unwrap();
    }

    #[test]
    fn retained_choice_eliminates_a_refuted_equal_input() {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let ids = (0..3)
            .map(|local| {
                facts
                    .assign(
                        PlaceId { owner: 1, local },
                        Interval::machine(8, true, &mut limits).unwrap(),
                    )
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let mut equal = |a, b| Condition::PolynomialCompare {
            left: Polynomial::variable(a, &mut limits).unwrap(),
            relation: Comparison::Equal,
            right: Polynomial::variable(b, &mut limits).unwrap(),
        };
        let first = equal(ids[0], ids[1]);
        let second = equal(ids[0], ids[2]);
        facts
            .assume(
                &Condition::Or(Box::new(first.clone()), Box::new(second.clone())),
                true,
                &mut limits,
            )
            .unwrap();
        facts.assume(&first, false, &mut limits).unwrap();
        facts.prove_condition(&second, true, &mut limits).unwrap();
    }

    #[test]
    fn retained_disjunction_uses_only_current_exact_aliases() {
        let mut limits = Limits::default();
        let mut facts = Facts::default();
        let mut ids = Vec::new();
        for local in 0..4 {
            ids.push(
                facts
                    .assign(
                        PlaceId { owner: 1, local },
                        Interval::machine(8, false, &mut limits).unwrap(),
                    )
                    .unwrap(),
            );
        }
        let choice = |a, b| {
            Condition::Or(
                Box::new(Condition::Compare {
                    value: a,
                    relation: Comparison::Equal,
                    constant: Integer::zero(),
                }),
                Box::new(Condition::Compare {
                    value: b,
                    relation: Comparison::Equal,
                    constant: Integer::zero(),
                }),
            )
        };
        let relation = |a, b, relation, limits: &mut Limits| Condition::PolynomialCompare {
            left: Polynomial::variable(a, limits).unwrap(),
            relation,
            right: Polynomial::variable(b, limits).unwrap(),
        };
        facts
            .assume(&choice(ids[0], ids[1]), true, &mut limits)
            .unwrap();
        let query = choice(ids[2], ids[3]);
        // Identical ranges do not establish identity.
        assert!(facts.prove_condition(&query, true, &mut limits).is_err());
        for (a, b) in [(ids[0], ids[2]), (ids[1], ids[3])] {
            let condition = relation(a, b, Comparison::LessEqual, &mut limits);
            facts.assume(&condition, true, &mut limits).unwrap();
        }
        assert!(facts.prove_condition(&query, true, &mut limits).is_err());
        for (a, b) in [(ids[0], ids[2]), (ids[1], ids[3])] {
            let condition = relation(a, b, Comparison::GreaterEqual, &mut limits);
            facts.assume(&condition, true, &mut limits).unwrap();
        }
        facts.prove_condition(&query, true, &mut limits).unwrap();
        facts
            .assign(
                ids[0].place,
                Interval::machine(8, false, &mut limits).unwrap(),
            )
            .unwrap();
        assert!(facts.prove_condition(&query, true, &mut limits).is_err());
    }

    #[test]
    fn bounded_holes_survive_copy_and_common_join_but_not_write_or_unproved_branch() {
        let mut limits = Limits::default();
        let mut initial = Facts::default();
        let place = PlaceId { owner: 1, local: 0 };
        let range = || {
            Interval::new(
                Integer::from_i128(-2),
                Integer::from_i128(2),
                &mut Limits::default(),
            )
            .unwrap()
        };
        let value = initial.assign(place, range()).unwrap();
        let excluded = Condition::Compare {
            value,
            relation: Comparison::NotEqual,
            constant: Integer::zero(),
        };
        let mut checked = initial.fork(&mut limits).unwrap();
        checked.assume(&excluded, true, &mut limits).unwrap();
        let proof = checked
            .prove_condition(&excluded, true, &mut limits)
            .unwrap();
        let copied = checked.fork(&mut limits).unwrap();
        proof.replay(&copied, &mut limits).unwrap();
        let common = checked.join(&copied, &mut limits).unwrap();
        common
            .prove_condition(&excluded, true, &mut limits)
            .unwrap();
        let optional = checked.join(&initial, &mut limits).unwrap();
        assert!(
            optional
                .prove_condition(&excluded, true, &mut limits)
                .is_err()
        );
        assert!(proof.replay(&optional, &mut limits).is_err());
        checked.assign(place, range()).unwrap();
        assert!(proof.replay(&checked, &mut limits).is_err());
        let current = Condition::Compare {
            value: checked.value(place).unwrap(),
            relation: Comparison::NotEqual,
            constant: Integer::zero(),
        };
        assert!(
            checked
                .prove_condition(&current, true, &mut limits)
                .is_err()
        );
    }

    #[test]
    fn unequal_bounded_booleans_refine_both_concrete_cases() {
        for bit in [0, 1] {
            let mut limits = Limits::default();
            let mut facts = Facts::default();
            let range = Interval::new(Integer::zero(), Integer::from_u128(1), &mut limits).unwrap();
            let left = facts
                .assign(PlaceId { owner: 1, local: 0 }, range.clone())
                .unwrap();
            let right = facts.assign(PlaceId { owner: 1, local: 1 }, range).unwrap();
            let equal = Condition::PolynomialCompare {
                left: Polynomial::variable(left, &mut limits).unwrap(),
                relation: Comparison::Equal,
                right: Polynomial::variable(right, &mut limits).unwrap(),
            };
            facts
                .assume(&Condition::Not(Box::new(equal.clone())), true, &mut limits)
                .unwrap();
            facts.prove_condition(&equal, false, &mut limits).unwrap();
            facts
                .assume(
                    &Condition::Compare {
                        value: left,
                        relation: Comparison::Equal,
                        constant: Integer::from_u128(bit),
                    },
                    true,
                    &mut limits,
                )
                .unwrap();
            let opposite = Condition::Compare {
                value: right,
                relation: Comparison::Equal,
                constant: Integer::from_u128(1 - bit),
            };
            facts.prove_condition(&opposite, true, &mut limits).unwrap();
        }
    }
}

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
                if (a == c && b == d) || (a == d && b == c) {
                    true
                } else {
                    // Mathematical predicates may move terms across equality:
                    // r == -x and r + x == 0 carry the same current evidence.
                    // Normalize only by bounded polynomial subtraction, not
                    // by dividing coefficients or altering comparison kinds.
                    let left = a.subtract(b, limits)?;
                    let right = c.subtract(d, limits)?;
                    left == right || left == d.subtract(c, limits)?
                }
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
    /// Revisit retained Boolean premises after new branch constraints arrive.
    /// Callers operate on private frames, so exhaustion cannot publish a
    /// partially refined program state.
    pub(super) fn propagate_retained_conditions(
        &mut self,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        if self.conditions.is_empty() || !self.is_reachable() {
            return Ok(());
        }
        let retained = self.copy_conditions(limits)?;
        // Bypass the "already known" shortcut while deriving new arithmetic
        // consequences. Every saved premise is current and already established.
        self.conditions.clear();
        // A disequality has no convex row representation. Keep those atomic
        // premises available while replaying choices, so a refuted disjunct
        // cannot become reachable merely because replay cleared its hole.
        for (premise, truth) in &retained {
            if let Condition::Compare { relation, .. }
            | Condition::PolynomialCompare { relation, .. } = premise
            {
                if (if *truth {
                    *relation
                } else {
                    relation.negated()
                }) == Comparison::NotEqual
                {
                    self.remember_condition(premise, *truth, limits)?;
                }
            }
        }
        // A bounded number of sweeps handles implication chains without
        // requiring nonlinear bounds to converge. Every derived fact remains
        // valid when this incomplete propagation stops before a fixed point.
        for _ in 0..retained.len().saturating_add(1) {
            let before = self.fork(limits)?;
            for (premise, truth) in &retained {
                self.assume_inner_mode(premise, *truth, 0, limits, true)?;
                if !self.is_reachable() {
                    break;
                }
            }
            if !self.is_reachable()
                || (self.current == before.current
                    && self.inequalities == before.inequalities
                    && self.conditions == before.conditions)
            {
                break;
            }
            limits.step(1)?;
        }
        for (premise, truth) in retained {
            self.remember_condition(&premise, truth, limits)?;
        }
        Ok(())
    }
    pub(super) fn known_condition(
        &self,
        condition: &Condition,
        limits: &mut Limits,
    ) -> Result<Option<bool>, Unknown> {
        // Validate the complete predicate before a Boolean shortcut can omit
        // a stale operand. Current interval evidence needs no alias search.
        if !condition.is_current(self, limits, 0)? {
            return Ok(None);
        }
        if let Some(truth) = condition.truth_by_interval(self, limits, 0)? {
            return Ok(Some(truth));
        }
        self.known_condition_mode(condition, limits, true)
    }
    fn known_condition_mode(
        &self,
        condition: &Condition,
        limits: &mut Limits,
        use_aliases: bool,
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
            if let Some(answer) =
                factors::nonzero_factor_entailment(&known, *truth, &query, limits)?
            {
                return Ok(Some(answer));
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
        // Equal value copies can give the same predicate different names.
        // Normalize only from exact, current equality evidence, never from
        // coincident intervals or a one-sided inequality.
        let compatible = |known: &Condition| match (known, condition) {
            (Condition::And(..), Condition::And(..))
            | (Condition::Or(..), Condition::Or(..))
            | (Condition::Not(..), Condition::Not(..)) => true,
            (
                Condition::Compare { relation: a, .. }
                | Condition::PolynomialCompare { relation: a, .. },
                Condition::Compare { relation: b, .. }
                | Condition::PolynomialCompare { relation: b, .. },
            ) => {
                matches!(b, Comparison::Equal | Comparison::NotEqual)
                    || matches!(
                        (a, b),
                        (
                            Comparison::Less | Comparison::Greater,
                            Comparison::Less | Comparison::Greater
                        ) | (
                            Comparison::LessEqual | Comparison::GreaterEqual,
                            Comparison::LessEqual | Comparison::GreaterEqual
                        )
                    )
            }
            _ => false,
        };
        if use_aliases && self.conditions.iter().any(|(known, _)| compatible(known)) {
            fn root(
                map: &BTreeMap<ValueId, ValueId>,
                mut id: ValueId,
                limits: &mut Limits,
            ) -> Result<ValueId, Unknown> {
                while let Some(parent) = map.get(&id) {
                    limits.step(1)?;
                    id = *parent;
                }
                Ok(id)
            }
            let mut aliases = BTreeMap::new();
            for (a, b) in relations::exact_equal_values(self, limits)? {
                let a = root(&aliases, a, limits)?;
                let b = root(&aliases, b, limits)?;
                if a != b {
                    aliases.insert(a.max(b), a.min(b));
                }
            }
            let mapping = aliases
                .keys()
                .map(|id| Ok((*id, root(&aliases, *id, limits)?)))
                .collect::<Result<BTreeMap<_, _>, Unknown>>()?;
            if !mapping.is_empty() {
                if let Some(query) = condition.renamed(&mapping, self, true, limits, 0)? {
                    for (known, truth) in &self.conditions {
                        if !compatible(known) {
                            continue;
                        }
                        if let Some(known) = known.renamed(&mapping, self, true, limits, 0)? {
                            if known.equivalent(&query, limits, 0)? {
                                return Ok(Some(*truth));
                            }
                            if let Some(answer) =
                                factors::nonzero_factor_entailment(&known, *truth, &query, limits)?
                            {
                                return Ok(Some(answer));
                            }
                        }
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
        self.remember_condition_at(condition, truth, limits, 0)
    }
    fn remember_condition_at(
        &mut self,
        condition: &Condition,
        truth: bool,
        limits: &mut Limits,
        depth: usize,
    ) -> Result<(), Unknown> {
        limits.step(1)?;
        if depth >= 128 {
            return Err(Unknown::ConditionDepthLimit);
        }
        if !self.reachable {
            return Ok(());
        }
        // Retain only the unresolved logical choice. A true conjunction (or
        // false disjunction) has already established both operands separately.
        match condition {
            Condition::Not(inner) => {
                return self.remember_condition_at(inner, !truth, limits, depth + 1);
            }
            Condition::And(a, b) if truth => {
                self.remember_condition_at(a, true, limits, depth + 1)?;
                return self.remember_condition_at(b, true, limits, depth + 1);
            }
            Condition::Or(a, b) if !truth => {
                self.remember_condition_at(a, false, limits, depth + 1)?;
                return self.remember_condition_at(b, false, limits, depth + 1);
            }
            _ => {}
        }
        // Finite intervals and polynomial inequalities can encode equality
        // and ordered comparisons, but not a hole in a bounded domain. Keep
        // disequality even for bounded values so joins and frame transfers do
        // not erase the checked choice (left < right or left > right).
        let disequality = match condition {
            Condition::Compare { relation, .. } | Condition::PolynomialCompare { relation, .. } => {
                (if truth { *relation } else { relation.negated() }) == super::Comparison::NotEqual
            }
            _ => false,
        };
        if let Condition::Compare { value, .. } = condition {
            match self.bound(*value, limits) {
                Ok(_) if !disequality => return Ok(()),
                Ok(_) => {}
                Err(Unknown::MissingBound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        if let Condition::PolynomialCompare { left, right, .. } = condition {
            match (
                left.interval_enclosure(self, limits),
                right.interval_enclosure(self, limits),
            ) {
                (Ok(_), Ok(_)) if !disequality => return Ok(()),
                (Ok(_), Ok(_)) => {}
                (Err(Unknown::MissingBound(_)), Ok(_) | Err(Unknown::MissingBound(_)))
                | (Ok(_), Err(Unknown::MissingBound(_))) => {}
                (Err(e), _) | (_, Err(e)) => return Err(e),
            }
        }
        if matches!(condition, Condition::Bool(_)) {
            return Ok(());
        }
        // Keep a copy's own predicate even if another equal value currently
        // proves it: that other identity may later leave scope or be written.
        if !self.reachable
            || self
                .known_condition_mode(condition, limits, false)?
                .is_some()
        {
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
