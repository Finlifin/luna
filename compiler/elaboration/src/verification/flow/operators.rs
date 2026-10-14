//! Recursive scalar flow paths avoid retaining the general control-flow frame.
use super::*;

impl Walker<'_> {
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
                // A nonnegative dividend bounds both its unsigned quotient
                // and remainder. Keep only current-version evidence: evaluating
                // the divisor may have mutated the dividend's original place.
                let upper_bound = if matches!(op, IntegerOp::DivideTrunc | IntegerOp::RemainderTrunc)
                    && a.range.as_ref().is_some_and(|r| !r.lower.is_negative())
                    && b.range.as_ref().is_some_and(|r| !r.lower.is_negative() && !r.lower.is_zero()) {
                    match a.polynomial(self.limits)? {
                        Some(p) => match p.interval_enclosure(&state.facts, self.limits) {
                            Ok(_) => Some(p),
                            Err(Unknown::StaleValue(_) | Unknown::MissingBound(_)) => None,
                            Err(error) => return Err(error.into()),
                        },
                        None => None,
                    }
                } else { None };
                let divisor_bound = if matches!(op, IntegerOp::DivideTrunc | IntegerOp::RemainderTrunc)
                    && b.range.as_ref().is_some_and(|r| !r.lower.is_negative() && !r.lower.is_zero()) {
                    match b.polynomial(self.limits)? {
                        Some(p) => match p.interval_enclosure(&state.facts, self.limits) {
                            Ok(_) => Some(p),
                            Err(Unknown::StaleValue(_) | Unknown::MissingBound(_)) => None,
                            Err(error) => return Err(error.into()),
                        },
                        None => None,
                    }
                } else { None };
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
                            let (quotient, remainder) = a.div_rem_trunc(&b, self.limits).map_err(|error| {
                                if error == Unknown::InsufficientBounds { Reason::MissingDivisionProof } else { error.into() }
                            })?;
                            // The machine kernel requires both results to fit,
                            // including MIN % -1 even though its remainder is zero.
                            self.check_machine(expr, state, Some(&quotient))?;
                            if *op == IntegerOp::DivideTrunc { quotient } else { remainder }
                        },
                        _ => return Err(Reason::Unsupported),
                    }),
                    _ if matches!(op, IntegerOp::DivideTrunc | IntegerOp::RemainderTrunc) => return Err(Reason::MissingDivisionProof),
                    _ => None,
                };
                self.refine_polynomial(&mut result, state)?;
                self.check_machine(expr, state, result.range.as_ref())?;
                if upper_bound.is_some() || divisor_bound.is_some() {
                    if let Some(root) = self.result_root(&expr.ty)? {
                        self.bind_result(state, root, &expr.ty, result)?;
                        result = self.local_value_at(state, root)?;
                        if *op == IntegerOp::DivideTrunc {
                            if let (Some(dividend), Some(divisor), Some(quotient)) =
                                (upper_bound.as_ref(), divisor_bound.as_ref(), result.polynomial(self.limits)?) {
                                match quotient.multiply(divisor, self.limits) {
                                    Ok(product) => state.facts.assume(&Condition::PolynomialCompare {
                                        left: product, relation: C::LessEqual, right: dividend.copy(self.limits)?,
                                    }, true, self.limits)?,
                                    Err(Unknown::DegreeLimit | Unknown::TermLimit) => (),
                                    Err(error) => return Err(error.into()),
                                }
                            }
                        }
                        let remainder_bound = if *op == IntegerOp::RemainderTrunc { divisor_bound } else { None };
                        for (bound, relation) in [(upper_bound, C::LessEqual), (remainder_bound, C::Less)] {
                            if let (Some(upper), Some(lower)) = (bound, result.polynomial(self.limits)?) {
                                state.facts.assume(&Condition::PolynomialCompare {
                                    left: lower, relation, right: upper,
                                }, true, self.limits)?;
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
                self.refine_polynomial(&mut result, state)?;
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
            K::And(a, b) | K::Or(a, b) if matches!(expr.ty, Term::Primitive(nir::Primitive::BitVec(_))) => {
                self.child(a, state, depth)?;
                self.child(b, state, depth)?;
                result = self.unknown_value(&expr.ty)?;
            }
            K::And(a, b) | K::Or(a, b) => {
                self.prime_fields(a, state, depth)?;
                self.prime_fields(b, state, depth)?;
                let a = self.child(a, state, depth)?.condition;
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
                                domain(&expr.ty, self.limits)?.ok_or(Reason::InvalidSignature)?,
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
                let b = b.condition;
                state.join_aggregates(&short, self.limits)?;
                state.join_paths_and_facts(&short, self.limits)?;
                result.condition = match (a, b) {
                    (_, None) if !continued => Some(Condition::Bool(!continue_truth)),
                    (Some(Condition::Bool(a)), Some(b)) => Some(if a == continue_truth {b} else {Condition::Bool(a)}),
                    (Some(a), Some(Condition::Bool(b))) => Some(if b == continue_truth {a} else {Condition::Bool(b)}),
                    (Some(a), Some(b)) => Some(if continue_truth {
                        Condition::And(Box::new(a), Box::new(b))
                    } else {
                        Condition::Or(Box::new(a), Box::new(b))
                    }),
                    _ => None,
                };
            }
            _ => unreachable!("flow operator dispatcher selected an incompatible expression"),
        }
        Ok(result)
    }
}
