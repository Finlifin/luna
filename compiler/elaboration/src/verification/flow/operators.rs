//! Recursive scalar flow paths avoid retaining the general control-flow frame.
use super::*;

// Comparisons of already-bound scalar values are total observations. Their
// symbolic Boolean composition needs no fork/join; executable short-circuit
// evaluation is unchanged. Calls, loads and arithmetic deliberately do not fit.
fn total_boolean_observation(expr: &Expr, limits: &mut Limits) -> Result<bool, Unknown> {
    let leaf = |expr: &Expr| matches!(expr.kind, K::Local(_) | K::Literal(_));
    let mut pending = vec![expr];
    while let Some(expr) = pending.pop() {
        limits.step(1)?;
        if expr.ty != Term::Primitive(nir::Primitive::Bool) {
            return Ok(false);
        }
        match &expr.kind {
            K::Local(_) | K::Literal(_) => {}
            K::Not(value) => pending.push(value),
            K::And(a, b) | K::Or(a, b) => {
                pending.push(b);
                pending.push(a);
            }
            K::ScalarEqual(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b)
                if leaf(a) && leaf(b) => {}
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/*
 * Unlike total observations, these expressions may need path-local overflow
 * proofs. They still cannot write storage or change a binding. Analyze their
 * short-circuit paths normally, then retain the entry state instead of joining
 * two unchanged stores and reconstructing all their relational evidence.
 */
fn guarded_scalar_observation(expr: &Expr, limits: &mut Limits) -> Result<bool, Unknown> {
    let mut pending = vec![expr];
    while let Some(expr) = pending.pop() {
        limits.step(1)?;
        match &expr.kind {
            K::Local(_) | K::Literal(_) => {}
            K::Not(value) | K::IntegerNegate(value) => pending.push(value),
            K::And(a, b)
            | K::Or(a, b)
            | K::ScalarEqual(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b) => {
                pending.push(b);
                pending.push(a);
            }
            K::IntegerBinary { op, left, right }
                if op.is_comparison()
                    || matches!(
                        op,
                        IntegerOp::Add | IntegerOp::Subtract | IntegerOp::Multiply
                    ) =>
            {
                pending.push(right);
                pending.push(left);
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

impl Walker<'_> {
    // Only called after the complete tree passed total_boolean_observation.
    // Internal nodes construct evidence; the enclosing inner() evaluates it
    // once instead of proving every prefix before proving the whole tree again.
    fn boolean_observation_condition(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Option<Condition>, Reason> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        match &expr.kind {
            K::Not(value) => Ok(self
                .boolean_observation_condition(value, state, depth + 1)?
                .map(|condition| Condition::Not(Box::new(condition)))),
            K::And(a, b) | K::Or(a, b) => {
                let a = self.boolean_observation_condition(a, state, depth + 1)?;
                let b = self.boolean_observation_condition(b, state, depth + 1)?;
                Ok(a.zip(b).map(|(a, b)| {
                    if matches!(expr.kind, K::And(..)) {
                        Condition::And(Box::new(a), Box::new(b))
                    } else {
                        Condition::Or(Box::new(a), Box::new(b))
                    }
                }))
            }
            _ => Ok(self.child(expr, state, depth)?.condition),
        }
    }

    fn division_ranges(
        &mut self,
        dividend: &Interval,
        divisor: &Interval,
        excludes_zero: bool,
    ) -> Result<(Interval, Interval), Unknown> {
        if !excludes_zero
            || divisor.upper.is_negative()
            || (!divisor.lower.is_negative() && !divisor.lower.is_zero())
        {
            return dividend.div_rem_trunc(divisor, self.limits);
        }
        // A checked nonzero premise describes two intervals when the divisor
        // can have either sign. Enclose each component, then join the results;
        // do not silently assume a rounding policy or a sign for the source.
        let mut ranges = Vec::new();
        if divisor.lower.is_negative() {
            ranges.push(Interval::new(
                self.limits.integer(&divisor.lower)?,
                Integer::from_i128(-1),
                self.limits,
            )?);
        }
        if !divisor.upper.is_negative() && !divisor.upper.is_zero() {
            ranges.push(Interval::new(
                Integer::from_u128(1),
                self.limits.integer(&divisor.upper)?,
                self.limits,
            )?);
        }
        let mut result: Option<(Interval, Interval)> = None;
        for range in ranges {
            let (quotient, remainder) = dividend.div_rem_trunc(&range, self.limits)?;
            result = Some(match result {
                None => (quotient, remainder),
                Some((q, r)) => (
                    hull(&q, &quotient, self.limits)?,
                    hull(&r, &remainder, self.limits)?,
                ),
            });
        }
        result.ok_or(Unknown::InsufficientBounds)
    }
    #[inline(never)]
    pub(super) fn integer_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let mut result = Value::default();
        match &expr.kind {
            K::IntegerBinary { op, left, right } if op.is_comparison() => {
                let a = self.child(left, state, depth)?;
                let b = self.child(right, state, depth)?;
                let relation = match op {
                    IntegerOp::Equal => C::Equal,
                    IntegerOp::NotEqual => C::NotEqual,
                    IntegerOp::Less => C::Less,
                    IntegerOp::LessEqual => C::LessEqual,
                    IntegerOp::Greater => C::Greater,
                    IntegerOp::GreaterEqual => C::GreaterEqual,
                    _ => unreachable!(),
                };
                result.condition = compare_values(a, b, relation, self.limits)?;
            }
            K::IntegerBinary { op, left, right } => {
                let a = self.child(left, state, depth)?;
                let b = self.child(right, state, depth)?;
                let excludes_zero =
                    if matches!(op, IntegerOp::DivideTrunc | IntegerOp::RemainderTrunc)
                        && b.range.as_ref().is_some_and(|range| {
                            (range.lower.is_negative() || range.lower.is_zero())
                                && !range.upper.is_negative()
                        })
                    {
                        if let Some(divisor) = b.polynomial(self.limits)? {
                            let condition = Condition::PolynomialCompare {
                                left: divisor,
                                relation: C::NotEqual,
                                right: Polynomial::constant(Integer::zero(), self.limits)?,
                            };
                            state
                                .facts
                                .prove_condition(&condition, true, self.limits)
                                .and_then(|proof| proof.replay(&state.facts, self.limits))
                                .is_ok()
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                // A nonnegative dividend bounds both its unsigned quotient
                // and remainder. Keep only current-version evidence: evaluating
                // the divisor may have mutated the dividend's original place.
                let upper_bound =
                    if matches!(op, IntegerOp::DivideTrunc | IntegerOp::RemainderTrunc)
                        && a.range.as_ref().is_some_and(|r| !r.lower.is_negative())
                        && b.range
                            .as_ref()
                            .is_some_and(|r| !r.lower.is_negative() && !r.lower.is_zero())
                    {
                        match a.polynomial(self.limits)? {
                            Some(p) => match p.interval_enclosure(&state.facts, self.limits) {
                                Ok(_) => Some(p),
                                Err(Unknown::StaleValue(_) | Unknown::MissingBound(_)) => None,
                                Err(error) => return Err(error.into()),
                            },
                            None => None,
                        }
                    } else {
                        None
                    };
                let divisor_bound =
                    if matches!(op, IntegerOp::DivideTrunc | IntegerOp::RemainderTrunc)
                        && b.range
                            .as_ref()
                            .is_some_and(|r| !r.lower.is_negative() && !r.lower.is_zero())
                    {
                        match b.polynomial(self.limits)? {
                            Some(p) => match p.interval_enclosure(&state.facts, self.limits) {
                                Ok(_) => Some(p),
                                Err(Unknown::StaleValue(_) | Unknown::MissingBound(_)) => None,
                                Err(error) => return Err(error.into()),
                            },
                            None => None,
                        }
                    } else {
                        None
                    };
                result.polynomial = combine_polynomials(&a, &b, *op, self.limits)?;
                result.range = match (a.range, b.range) {
                    (Some(a), Some(b)) => Some(match op {
                        IntegerOp::Add => a.add(&b, self.limits)?,
                        IntegerOp::Subtract => Interval::new(
                            a.lower
                                .subtract(&b.upper, &mut self.limits.arithmetic)
                                .map_err(Unknown::from)?,
                            a.upper
                                .subtract(&b.lower, &mut self.limits.arithmetic)
                                .map_err(Unknown::from)?,
                            self.limits,
                        )?,
                        IntegerOp::Multiply => a.multiply(&b, self.limits)?,
                        IntegerOp::DivideTrunc | IntegerOp::RemainderTrunc => {
                            let (quotient, remainder) = self
                                .division_ranges(&a, &b, excludes_zero)
                                .map_err(|error| {
                                    if error == Unknown::InsufficientBounds {
                                        Reason::MissingDivisionProof
                                    } else {
                                        error.into()
                                    }
                                })?;
                            // The machine kernel requires both results to fit,
                            // including MIN % -1 even though its remainder is zero.
                            self.check_machine(expr, state, Some(&quotient))?;
                            if *op == IntegerOp::DivideTrunc {
                                quotient
                            } else {
                                remainder
                            }
                        }
                        _ => return Err(Reason::Unsupported),
                    }),
                    _ if matches!(op, IntegerOp::DivideTrunc | IntegerOp::RemainderTrunc) => {
                        return Err(Reason::MissingDivisionProof);
                    }
                    _ => None,
                };
                self.refine_polynomial(expr, &mut result, state)?;
                self.check_machine(expr, state, result.range.as_ref())?;
                if upper_bound.is_some() || divisor_bound.is_some() {
                    if let Some(root) = self.result_root(&expr.ty)? {
                        self.bind_result(state, root, &expr.ty, result)?;
                        result = self.local_value_at(state, root)?;
                        if *op == IntegerOp::DivideTrunc {
                            if let (Some(dividend), Some(divisor), Some(quotient)) = (
                                upper_bound.as_ref(),
                                divisor_bound.as_ref(),
                                result.polynomial(self.limits)?,
                            ) {
                                match quotient.multiply(divisor, self.limits) {
                                    Ok(product) => state.facts.assume(
                                        &Condition::PolynomialCompare {
                                            left: product,
                                            relation: C::LessEqual,
                                            right: dividend.copy(self.limits)?,
                                        },
                                        true,
                                        self.limits,
                                    )?,
                                    Err(Unknown::DegreeLimit | Unknown::TermLimit) => (),
                                    Err(error) => return Err(error.into()),
                                }
                            }
                        }
                        let remainder_bound = if *op == IntegerOp::RemainderTrunc {
                            divisor_bound
                        } else {
                            None
                        };
                        for (bound, relation) in
                            [(upper_bound, C::LessEqual), (remainder_bound, C::Less)]
                        {
                            if let (Some(upper), Some(lower)) =
                                (bound, result.polynomial(self.limits)?)
                            {
                                state.facts.assume(
                                    &Condition::PolynomialCompare {
                                        left: lower,
                                        relation,
                                        right: upper,
                                    },
                                    true,
                                    self.limits,
                                )?;
                            }
                        }
                    }
                }
                self.record(
                    expr,
                    state,
                    PointKind::Arithmetic(*op),
                    result.range.as_ref(),
                )?;
            }
            K::IntegerNegate(value) => {
                let value = self.child(value, state, depth)?;
                result.polynomial = value
                    .polynomial(self.limits)?
                    .map(|p| Polynomial::default().subtract(&p, self.limits))
                    .transpose()?;
                if let Some(bound) = value.range {
                    result.range = Some(Interval::new(
                        bound
                            .upper
                            .negate(&mut self.limits.arithmetic)
                            .map_err(Unknown::from)?,
                        bound
                            .lower
                            .negate(&mut self.limits.arithmetic)
                            .map_err(Unknown::from)?,
                        self.limits,
                    )?);
                }
                self.refine_polynomial(expr, &mut result, state)?;
                self.check_machine(expr, state, result.range.as_ref())?;
                self.record(expr, state, PointKind::Negate, result.range.as_ref())?;
            }
            _ => unreachable!("flow operator dispatcher selected an incompatible expression"),
        }
        Ok(result)
    }
    #[inline(never)]
    pub(super) fn logical_expression(
        &mut self,
        expr: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let mut result = Value::default();
        match &expr.kind {
            K::Not(value) => {
                let value = self.child(value, state, depth)?;
                if expr.ty == Term::Primitive(nir::Primitive::Bool) {
                    if let Some(range) = &value.range {
                        let one = Integer::from_u128(1);
                        result.range = Some(Interval::new(
                            one.subtract(&range.upper, &mut self.limits.arithmetic)
                                .map_err(Unknown::from)?,
                            one.subtract(&range.lower, &mut self.limits.arithmetic)
                                .map_err(Unknown::from)?,
                            self.limits,
                        )?);
                    }
                    if let Some(p) = value.polynomial(self.limits)? {
                        result.polynomial = Some(
                            Polynomial::constant(Integer::from_u128(1), self.limits)?
                                .subtract(&p, self.limits)?,
                        );
                    }
                }
                result.condition = value.condition.map(|q| Condition::Not(Box::new(q)));
            }
            K::TemplateEqual(a, b) => {
                self.child(a, state, depth)?;
                self.child(b, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::ScalarEqual(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b) => {
                let a = self.child(a, state, depth)?;
                let b = self.child(b, state, depth)?;
                let relation = match expr.kind {
                    K::ScalarEqual(..) => C::Equal,
                    K::ScalarLess(..) => C::Less,
                    K::ScalarLessEqual(..) => C::LessEqual,
                    K::ScalarGreater(..) => C::Greater,
                    _ => C::GreaterEqual,
                };
                result.condition = compare_values(a, b, relation, self.limits)?;
            }
            K::And(a, b) | K::Or(a, b)
                if matches!(expr.ty, Term::Primitive(nir::Primitive::BitVec(_))) =>
            {
                self.child(a, state, depth)?;
                self.child(b, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::And(a, b) | K::Or(a, b) => {
                if total_boolean_observation(expr, self.limits)? {
                    result = self.unknown_value(&expr.ty)?;
                    result.condition =
                        self.boolean_observation_condition(expr, state, depth.saturating_sub(1))?;
                    return Ok(result);
                }
                self.prime_fields(a, state, depth)?;
                self.prime_fields(b, state, depth)?;
                let a = self.child(a, state, depth)?.condition;
                let unchanged = if guarded_scalar_observation(expr, self.limits)? {
                    Some(state.fork(self.limits)?)
                } else {
                    None
                };
                let mut short = state.fork(self.limits)?;
                let continue_truth = matches!(expr.kind, K::And(..));
                if let Some(a) = &a {
                    short.facts.assume(a, !continue_truth, self.limits)?;
                    state.facts.assume(a, continue_truth, self.limits)?;
                }
                let continued = state.facts.is_reachable();
                let b = self.child(b, state, depth)?;
                let tail = if state.facts.is_reachable() {
                    Some(
                        b.range
                            .as_ref()
                            .map(|r| r.copy(self.limits))
                            .transpose()?
                            .unwrap_or(
                                domain_in(&expr.ty, self.target(), self.limits)?
                                    .ok_or(Reason::InvalidSignature)?,
                            ),
                    )
                } else {
                    None
                };
                let short_range = if short.facts.is_reachable() {
                    let bit = Integer::from_u128(u128::from(!continue_truth));
                    Some(Interval::new(bit.clone(), bit, self.limits)?)
                } else {
                    None
                };
                result.range = match (tail, short_range) {
                    (Some(a), Some(b)) => Some(hull(&a, &b, self.limits)?),
                    (a, b) => a.or(b),
                };
                let mut a = a;
                let mut b = b.condition;
                if let Some(unchanged) = unchanged {
                    *state = unchanged;
                } else {
                    state.join_aggregates(&short, self.limits)?;
                    state.join_paths_and_facts(&short, self.limits)?;
                }
                // A computed operand can have no surviving predicate, while
                // its sibling still describes current input observations.
                // Keep that sibling without pretending it equals the result:
                // model the unavailable operand by a fresh unconstrained Bool.
                // In particular, A and unknown implies A only on its true path;
                // A or unknown implies not A only on its false path.
                for condition in [&mut a, &mut b] {
                    if let Some(known) = condition {
                        if !known.is_current(&state.facts, self.limits, 0)? {
                            *condition = None;
                        }
                    }
                }
                if continued && state.facts.is_reachable() && (a.is_some() != b.is_some()) {
                    let root = self
                        .result_root(&expr.ty)?
                        .ok_or(Reason::InvalidSignature)?;
                    let range = domain_in(&expr.ty, self.target(), self.limits)?
                        .ok_or(Reason::InvalidSignature)?;
                    let value = state.facts.assign(root, range)?;
                    let unknown = Some(Condition::Compare {
                        value,
                        relation: C::NotEqual,
                        constant: Integer::from_u128(0),
                    });
                    if a.is_none() {
                        a = unknown;
                    } else {
                        b = unknown;
                    }
                }
                result.condition = match (a, b) {
                    (_, None) if !continued => Some(Condition::Bool(!continue_truth)),
                    (Some(Condition::Bool(a)), Some(b)) => Some(if a == continue_truth {
                        b
                    } else {
                        Condition::Bool(a)
                    }),
                    (Some(a), Some(Condition::Bool(b))) => Some(if b == continue_truth {
                        a
                    } else {
                        Condition::Bool(b)
                    }),
                    (Some(a), Some(b)) => Some(if continue_truth {
                        Condition::And(Box::new(a), Box::new(b))
                    } else {
                        Condition::Or(Box::new(a), Box::new(b))
                    }),
                    _ => None,
                };
                let current = match &result.condition {
                    Some(condition) => condition.is_current(&state.facts, self.limits, 0)?,
                    None => false,
                };
                if !current {
                    // A right-hand observation may exist only on the continued
                    // path. Its identity does not survive the join, but the
                    // already evaluated Boolean still has the joined range.
                    result.condition = None;
                    let root = self
                        .result_root(&expr.ty)?
                        .ok_or(Reason::InvalidSignature)?;
                    self.bind_result(state, root, &expr.ty, result)?;
                    result = self.local_value_at(state, root)?;
                }
            }
            _ => unreachable!("flow operator dispatcher selected an incompatible expression"),
        }
        Ok(result)
    }
}
