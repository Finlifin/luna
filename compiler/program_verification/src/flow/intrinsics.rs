//! Portable integer facts for target operations. This is program analysis,
//! not comptime invocation; floating target operations remain unknown.
use super::*;
use nir::intrinsics::Operation;

impl Walker<'_> {
    pub(super) fn intrinsic_value(
        &mut self,
        expr: &Expr,
        inputs: &[Expr],
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let checked = nir::intrinsics::checked(expr).map_err(|_| Reason::Unsupported)?;
        checked
            .validate_target_layout(self.target())
            .map_err(|_| Reason::InvalidSignature)?;
        let extremum = matches!(
            checked.operation(),
            Operation::IntegerMinimum | Operation::IntegerMaximum
        );
        let mut values = Vec::with_capacity(inputs.len());
        for input in inputs {
            let value = self.child(input, state, depth)?;
            // The next argument can overwrite this argument's source. Relations
            // must refer to the value already evaluated, not to that live place.
            values.push(
                if (extremum || checked.operation() == Operation::WrappingAbsoluteValue)
                    && state.facts.is_reachable()
                {
                    self.intrinsic_snapshot(value, &input.ty, state)?
                } else {
                    value
                },
            );
        }
        let mut result = self.unknown_value(&expr.ty)?;
        use Operation::*;
        if matches!(checked.operation(), BitAnd | BitOr | BitXor) {
            let Term::Primitive(primitive) = &expr.ty else {
                return Err(Reason::InvalidSignature);
            };
            let (signed, width) = self
                .integer_layout(*primitive)
                .ok_or(Reason::InvalidSignature)?;
            if let (Some(a), Some(b)) = (&values[0].range, &values[1].range) {
                let range = bitwise_range(a, b, checked.operation(), width, signed, self.limits)?;
                if self.limits.compare(range.lower(), range.upper())?.is_eq() {
                    result.scalar = Some(Scalar::Constant(self.limits.integer(range.lower())?));
                }
                result.range = Some(range);
            }
            return Ok(result);
        }
        if checked.operation() == WrappingAbsoluteValue {
            if let Some(input) = &values[0].range {
                let range = wrapping_absolute_range(
                    input,
                    checked
                        .width_for_target(self.target())
                        .ok_or(Reason::Unsupported)?,
                    self.limits,
                )?;
                if self.limits.compare(range.lower(), range.upper())?.is_eq() {
                    result.scalar = Some(Scalar::Constant(self.limits.integer(range.lower())?));
                }
                result.range = Some(range);
            }
            if state.facts.is_reachable() {
                result = self.intrinsic_snapshot(result, &expr.ty, state)?;
                if let (Some(output), Some(input)) = (
                    result.polynomial(self.limits)?,
                    values[0].polynomial(self.limits)?,
                ) {
                    let identity = Condition::PolynomialCompare {
                        left: output.copy(self.limits)?,
                        relation: C::Equal,
                        right: input.copy(self.limits)?,
                    };
                    let negated = Polynomial::constant(Integer::zero(), self.limits)?
                        .subtract(&input, self.limits)?;
                    let opposite = Condition::PolynomialCompare {
                        left: output,
                        relation: C::Equal,
                        right: negated.copy(self.limits)?,
                    };
                    let domain = domain_in(&expr.ty, self.target(), self.limits)?
                        .ok_or(Reason::InvalidSignature)?;
                    let condition = match values[0].range.as_ref() {
                        Some(range) if !range.lower().is_negative() => {
                            result.polynomial = Some(input);
                            identity
                        }
                        Some(range)
                            if range.upper().is_negative()
                                && self.limits.compare(range.lower(), domain.lower())?.is_gt() =>
                        {
                            result.polynomial = Some(negated);
                            opposite
                        }
                        // INT_MIN wraps to itself, so identity remains a valid
                        // alternative even for a negative input. Never export
                        // unconditional mathematical negation for the full domain.
                        _ => Condition::Or(Box::new(identity), Box::new(opposite)),
                    };
                    if matches!(condition, Condition::Or(..)) {
                        // Keep the checked choice without eagerly splitting
                        // and joining the state: such a join may replace the
                        // result identity whose evidence is being exported.
                        state
                            .facts
                            .remember_condition(&condition, true, self.limits)?;
                    } else {
                        state.facts.assume(&condition, true, self.limits)?;
                    }
                    result.evidence = None;
                    result.evidence = self.prepare_copy(&result, state)?;
                }
            }
            return Ok(result);
        }
        if matches!(
            checked.operation(),
            SaturatingAdd | SaturatingSubtract | IntegerMinimum | IntegerMaximum
        ) {
            let (Some(a), Some(b), Some(domain)) =
                (&values[0].range, &values[1].range, &result.range)
            else {
                return Ok(result);
            };
            // Inputs have already been evaluated in source order. Use their
            // value snapshots, never re-read an earlier argument's source place.
            let range = match checked.operation() {
                IntegerMinimum | IntegerMaximum => {
                    let maximum = checked.operation() == IntegerMaximum;
                    let choose = |a: &Integer, b: &Integer, limits: &mut Limits| {
                        let pick_a = if maximum {
                            !limits.compare(a, b)?.is_lt()
                        } else {
                            !limits.compare(a, b)?.is_gt()
                        };
                        limits.integer(if pick_a { a } else { b })
                    };
                    let lower = choose(a.lower(), b.lower(), self.limits)?;
                    let upper = choose(a.upper(), b.upper(), self.limits)?;
                    Interval::new(lower, upper, self.limits)?
                }
                _ => saturating_range(
                    a,
                    b,
                    domain,
                    checked.operation() == SaturatingSubtract,
                    self.limits,
                )?,
            };
            if self.limits.compare(range.lower(), range.upper())?.is_eq() {
                result.scalar = Some(Scalar::Constant(self.limits.integer(range.lower())?));
            }
            result.range = Some(range);
            if extremum && state.facts.is_reachable() {
                result = self.intrinsic_snapshot(result, &expr.ty, state)?;
                let mut alternatives = Vec::new();
                let mut tautology = false;
                for input in &values {
                    if let (Some(left), Some(right)) = (
                        result.polynomial(self.limits)?,
                        input.polynomial(self.limits)?,
                    ) {
                        tautology |= left == right;
                        alternatives.push(Condition::PolynomialCompare {
                            left: left.copy(self.limits)?,
                            relation: C::Equal,
                            right: right.copy(self.limits)?,
                        });
                        // These rows follow directly from the checked operation;
                        // its interval is already computed. Retain them without
                        // re-propagating the entire graph for each operand.
                        let row = if checked.operation() == IntegerMinimum {
                            left.subtract(&right, self.limits)?
                        } else {
                            right.subtract(&left, self.limits)?
                        };
                        copies::add_row(&mut state.facts, row, self.limits)?;
                    }
                }
                if alternatives.len() == 2 && !tautology {
                    let right = alternatives.pop().unwrap();
                    let left = alternatives.pop().unwrap();
                    // Integer min/max selects an input exactly. Preserve the
                    // choice without splitting the entire state eagerly.
                    state.facts.remember_condition(
                        &Condition::Or(Box::new(left), Box::new(right)),
                        true,
                        self.limits,
                    )?;
                }
                // Export the new relations with the returned value's evidence.
                result.evidence = None;
                result.evidence = self.prepare_copy(&result, state)?;
            }
            return Ok(result);
        }
        if !matches!(
            checked.operation(),
            BitReverse
                | ByteSwap
                | PopulationCount
                | LeadingZeros
                | TrailingZeros
                | FunnelShiftLeft
                | FunnelShiftRight
        ) {
            return Ok(result);
        }
        let width = checked
            .width_for_target(self.target())
            .ok_or(Reason::Unsupported)?;
        self.limits.step(values.len() + 1)?;
        let mut constants = Vec::new();
        for value in &values {
            let Some(range) = &value.range else {
                break;
            };
            if !self.limits.compare(range.lower(), range.upper())?.is_eq() {
                break;
            }
            constants.push(range.lower().to_u128().map_err(Unknown::from)?);
        }
        if constants.len() == values.len() {
            let a = constants[0];
            let mask = u128::MAX >> (128 - width);
            if constants.iter().any(|value| *value > mask) {
                return Err(Reason::InvalidSignature);
            }
            let exact = match checked.operation() {
                BitReverse => a.reverse_bits() >> (128 - width),
                ByteSwap => a.swap_bytes() >> (128 - width),
                PopulationCount => u128::from(a.count_ones()),
                LeadingZeros => u128::from(a.leading_zeros() - (128 - width)),
                TrailingZeros => u128::from(a.trailing_zeros().min(width)),
                FunnelShiftLeft | FunnelShiftRight => {
                    let b = constants[1];
                    let shift = (constants[2] % u128::from(width)) as u32;
                    if checked.operation() == FunnelShiftLeft {
                        if shift == 0 {
                            a
                        } else {
                            ((a << shift) | (b >> (width - shift))) & mask
                        }
                    } else if shift == 0 {
                        b
                    } else {
                        ((b >> shift) | (a << (width - shift))) & mask
                    }
                }
                _ => unreachable!(),
            };
            let exact = Integer::from_u128(exact);
            result.range = Some(Interval::singleton(&exact, self.limits)?);
            result.scalar = Some(Scalar::Constant(exact));
        } else if matches!(
            checked.operation(),
            PopulationCount | LeadingZeros | TrailingZeros
        ) {
            let nonzero = values[0]
                .range
                .as_ref()
                .map(|range| self.limits.compare(range.lower(), &Integer::zero()))
                .transpose()?
                .is_some_and(|order| order.is_gt());
            let (lower, upper) = match checked.operation() {
                PopulationCount => (u128::from(nonzero), u128::from(width)),
                _ => (0, u128::from(width - u32::from(nonzero))),
            };
            result.range = Some(Interval::new(
                Integer::from_u128(lower),
                Integer::from_u128(upper),
                self.limits,
            )?);
        }
        Ok(result)
    }
}

impl Walker<'_> {
    fn intrinsic_snapshot(
        &mut self,
        value: Value,
        ty: &Term,
        state: &mut State,
    ) -> Result<Value, Reason> {
        match &value.scalar {
            Some(Scalar::Constant(_)) => return Ok(value),
            Some(Scalar::Local(id)) if state.facts.value(id.place) == Some(*id) => {
                let private_unchanged = u32::try_from(id.place.owner).ok().is_some_and(|owner| {
                    let local = LocalId {
                        owner,
                        index: id.place.local,
                    };
                    !self.addressed_locals.contains(&local) && !self.written_locals.contains(&local)
                });
                if state.frozen.contains_key(&id.place) || private_unchanged {
                    return Ok(value);
                }
            }
            _ => {}
        }
        let root = self.result_root(ty)?.ok_or(Reason::Unsupported)?;
        let copy = self.prepare_copy(&value, state)?;
        bind_value_place(state, root, ty, value, self.limits)?;
        if let Some(copy) = copy {
            let installed = self.install_copy_at(state, root, &[], copy)?;
            self.finish_copies(state, installed)?;
        }
        // This is an immutable value temporary, never addressable source storage.
        state.frozen.insert(root, ty.clone());
        self.local_value_at(state, root)
    }
}

/// Sound scalar bounds, not an encoding of bitwise relations as polynomials.
/// Operand intervals describe already evaluated values, not mutable places.
fn bitwise_range(
    a: &Interval,
    b: &Interval,
    operation: Operation,
    width: u32,
    signed: bool,
    limits: &mut Limits,
) -> Result<Interval, Unknown> {
    use Operation::*;
    limits.step(1)?;
    if !(1..=128).contains(&width) || !matches!(operation, BitAnd | BitOr | BitXor) {
        return Err(Unknown::InvalidInterval);
    }
    let domain = Interval::machine(width as u16, signed, limits)?;
    for input in [a, b] {
        if limits.compare(input.lower(), domain.lower())?.is_lt()
            || limits.compare(input.upper(), domain.upper())?.is_gt()
        {
            return Err(Unknown::InvalidInterval);
        }
    }
    let a_single = limits.compare(a.lower(), a.upper())?.is_eq();
    let b_single = limits.compare(b.lower(), b.upper())?.is_eq();
    let mask = u128::MAX >> (128 - width);
    if a_single && b_single {
        let bits = |value: &Integer| -> Result<u128, Unknown> {
            Ok(if signed {
                value.to_signed(width)? as u128 & mask
            } else {
                value.to_unsigned(width)?
            })
        };
        let (a, b) = (bits(a.lower())?, bits(b.lower())?);
        let result = match operation {
            BitAnd => a & b,
            BitOr => a | b,
            BitXor => a ^ b,
            _ => unreachable!(),
        };
        let result = if signed && result & (1u128 << (width - 1)) != 0 {
            Integer::from_i128((result | !mask) as i128)
        } else {
            Integer::from_u128(result)
        };
        return Interval::singleton(&result, limits);
    }
    let zero = Integer::zero();
    let ones = if signed {
        Integer::from_i128(-1)
    } else {
        Integer::from_u128(mask)
    };
    for (mask, singleton, other) in [(a, a_single, b), (b, b_single, a)] {
        if singleton {
            let identity = match operation {
                BitAnd => limits.compare(mask.lower(), &ones)?.is_eq(),
                BitOr | BitXor => limits.compare(mask.lower(), &zero)?.is_eq(),
                _ => unreachable!(),
            };
            if identity {
                return other.copy(limits);
            }
        }
    }
    match operation {
        BitAnd => {
            // A nonnegative operand clears the sign bit, even when the other
            // operand is signed and negative. Clearing bits cannot increase it.
            let mut upper = None;
            for input in [a, b] {
                if !input.lower().is_negative()
                    && upper.as_ref().map_or(Ok(true), |bound| {
                        limits
                            .compare(input.upper(), bound)
                            .map(|order| order.is_lt())
                    })?
                {
                    upper = Some(limits.integer(input.upper())?);
                }
            }
            if let Some(upper) = upper {
                return Interval::new(zero, upper, limits);
            }
        }
        BitOr => {
            // Unsigned OR is at least either operand. In signed arithmetic
            // this remains true for a negative operand whose sign bit stays set.
            let mut lower = None;
            for input in [a, b] {
                if (!signed || input.upper().is_negative())
                    && lower.as_ref().map_or(Ok(true), |bound| {
                        limits
                            .compare(input.lower(), bound)
                            .map(|order| order.is_gt())
                    })?
                {
                    lower = Some(limits.integer(input.lower())?);
                }
            }
            if let Some(lower) = lower {
                let upper = if signed {
                    ones
                } else {
                    limits.integer(domain.upper())?
                };
                return Interval::new(lower, upper, limits);
            }
        }
        BitXor => {}
        _ => unreachable!(),
    }
    Ok(domain)
}

// Exact interval hull of signed wrapping absolute value. INT_MIN is a
// discontinuity: a range containing both INT_MIN and its successor produces
// both INT_MIN and INT_MAX, so claiming nonnegative output would be unsound.
fn wrapping_absolute_range(
    input: &Interval,
    width: u32,
    limits: &mut Limits,
) -> Result<Interval, Unknown> {
    limits.step(1)?;
    let width = u16::try_from(width).map_err(|_| Unknown::InvalidInterval)?;
    let domain = Interval::machine(width, true, limits)?;
    if limits.compare(input.lower(), domain.lower())?.is_lt()
        || limits.compare(input.upper(), domain.upper())?.is_gt()
    {
        return Err(Unknown::InvalidInterval);
    }
    if limits.compare(input.lower(), domain.lower())?.is_eq() {
        return if limits.compare(input.upper(), domain.lower())?.is_eq() {
            input.copy(limits)
        } else {
            Ok(domain)
        };
    }
    if !input.lower().is_negative() {
        return input.copy(limits);
    }
    let magnitude = Integer::zero().subtract(input.lower(), &mut limits.arithmetic)?;
    if input.upper().is_negative() {
        let lower = Integer::zero().subtract(input.upper(), &mut limits.arithmetic)?;
        return Interval::new(lower, magnitude, limits);
    }
    let upper = if limits.compare(&magnitude, input.upper())?.is_gt() {
        magnitude
    } else {
        limits.integer(input.upper())?
    };
    Interval::new(Integer::zero(), upper, limits)
}

/// Saturation is monotone in the mathematical result, but not a polynomial
/// equality to the inputs. Retain only the resulting bounds (or a singleton).
fn saturating_range(
    a: &Interval,
    b: &Interval,
    domain: &Interval,
    subtract: bool,
    limits: &mut Limits,
) -> Result<Interval, Unknown> {
    limits.step(1)?;
    for input in [a, b] {
        if limits.compare(input.lower(), domain.lower())?.is_lt()
            || limits.compare(input.upper(), domain.upper())?.is_gt()
        {
            return Err(Unknown::InvalidInterval);
        }
    }
    let (lower, upper) = if subtract {
        (
            a.lower().subtract(b.upper(), &mut limits.arithmetic)?,
            a.upper().subtract(b.lower(), &mut limits.arithmetic)?,
        )
    } else {
        (
            a.lower().add(b.lower(), &mut limits.arithmetic)?,
            a.upper().add(b.upper(), &mut limits.arithmetic)?,
        )
    };
    let clamp = |value: Integer, limits: &mut Limits| -> Result<Integer, Unknown> {
        if limits.compare(&value, domain.lower())?.is_lt() {
            limits.integer(domain.lower())
        } else if limits.compare(&value, domain.upper())?.is_gt() {
            limits.integer(domain.upper())
        } else {
            Ok(value)
        }
    };
    let lower = clamp(lower, limits)?;
    let upper = clamp(upper, limits)?;
    Interval::new(lower, upper, limits)
}

#[cfg(test)]
mod saturation_tests {
    use super::*;

    #[test]
    fn wrapping_absolute_interval_matches_every_i8_interval() {
        for lower in -128i16..=127 {
            for upper in lower..=127 {
                let mut limits = Limits::default();
                let input = Interval::new(
                    Integer::from_i128(lower.into()),
                    Integer::from_i128(upper.into()),
                    &mut limits,
                )
                .unwrap();
                let actual = wrapping_absolute_range(&input, 8, &mut limits).unwrap();
                let results = (lower..=upper)
                    .map(|value| (value as i8).wrapping_abs() as i16)
                    .collect::<Vec<_>>();
                assert_eq!(
                    *actual.lower(),
                    Integer::from_i128((*results.iter().min().unwrap()).into()),
                    "{lower}..{upper}"
                );
                assert_eq!(
                    *actual.upper(),
                    Integer::from_i128((*results.iter().max().unwrap()).into()),
                    "{lower}..{upper}"
                );
            }
        }
    }

    #[test]
    fn wrapping_absolute_interval_bounds_budget_and_recovery() {
        let mut limits = Limits::default();
        let domain = Interval::machine(128, true, &mut limits).unwrap();
        let minimum = Interval::singleton(domain.lower(), &mut limits).unwrap();
        assert_eq!(
            wrapping_absolute_range(&minimum, 128, &mut limits).unwrap(),
            minimum
        );
        assert_eq!(
            wrapping_absolute_range(&domain, 128, &mut limits).unwrap(),
            domain
        );
        let successor = domain
            .lower()
            .add(&Integer::from_i128(1), &mut limits.arithmetic)
            .unwrap();
        let next = Interval::singleton(&successor, &mut limits).unwrap();
        assert_eq!(
            wrapping_absolute_range(&next, 128, &mut limits).unwrap(),
            Interval::singleton(domain.upper(), &mut limits).unwrap()
        );
        let mut exhausted = Limits::default();
        exhausted.arithmetic.steps = 0;
        assert!(wrapping_absolute_range(&next, 128, &mut exhausted).is_err());
        let mut too_small = Limits::default();
        too_small.arithmetic.max_limbs = 3;
        assert!(wrapping_absolute_range(&next, 128, &mut too_small).is_err());
        for width in [0, 129, 65536] {
            assert!(wrapping_absolute_range(&next, width, &mut limits).is_err());
        }
        assert_eq!(
            wrapping_absolute_range(&next, 128, &mut limits).unwrap(),
            Interval::singleton(domain.upper(), &mut limits).unwrap()
        );
        let outside = Interval::singleton(&Integer::from_i128(128), &mut limits).unwrap();
        assert_eq!(
            wrapping_absolute_range(&outside, 8, &mut limits),
            Err(Unknown::InvalidInterval)
        );
        let outside = Interval::singleton(&Integer::from_i128(-129), &mut limits).unwrap();
        assert_eq!(
            wrapping_absolute_range(&outside, 8, &mut limits),
            Err(Unknown::InvalidInterval)
        );
    }

    #[test]
    fn saturation_interval_matches_enumerated_small_domains() {
        for (signed, segments) in [
            (true, vec![(-128i64, -120), (-10, 10), (100, 127)]),
            (false, vec![(0i64, 5), (10, 20), (250, 255)]),
        ] {
            let mut limits = Limits::default();
            let domain = Interval::machine(8, signed, &mut limits).unwrap();
            let (min, max) = if signed { (-128, 127) } else { (0, 255) };
            for &(a0, a1) in &segments {
                for &(b0, b1) in &segments {
                    let a = Interval::new(
                        Integer::from_i128(a0.into()),
                        Integer::from_i128(a1.into()),
                        &mut limits,
                    )
                    .unwrap();
                    let b = Interval::new(
                        Integer::from_i128(b0.into()),
                        Integer::from_i128(b1.into()),
                        &mut limits,
                    )
                    .unwrap();
                    for subtract in [false, true] {
                        let actual =
                            saturating_range(&a, &b, &domain, subtract, &mut limits).unwrap();
                        let mut observed = Vec::new();
                        for x in a0..=a1 {
                            for y in b0..=b1 {
                                observed
                                    .push((if subtract { x - y } else { x + y }).clamp(min, max));
                            }
                        }
                        assert_eq!(
                            *actual.lower(),
                            Integer::from_i128((*observed.iter().min().unwrap()).into())
                        );
                        assert_eq!(
                            *actual.upper(),
                            Integer::from_i128((*observed.iter().max().unwrap()).into())
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn saturation_interval_rejects_invalid_bounds_and_exhaustion_then_recovers() {
        let mut limits = Limits::default();
        let domain = Interval::machine(128, false, &mut limits).unwrap();
        let maximum = Interval::singleton(domain.upper(), &mut limits).unwrap();
        let mut exhausted = Limits::default();
        exhausted.arithmetic.steps = 0;
        assert!(saturating_range(&maximum, &maximum, &domain, false, &mut exhausted).is_err());
        let mut too_small = Limits::default();
        too_small.arithmetic.max_limbs = 4;
        assert!(saturating_range(&maximum, &maximum, &domain, false, &mut too_small).is_err());
        assert_eq!(
            saturating_range(&maximum, &maximum, &domain, false, &mut limits).unwrap(),
            maximum
        );
        let negative = Interval::singleton(&Integer::from_i128(-1), &mut limits).unwrap();
        assert_eq!(
            saturating_range(&negative, &maximum, &domain, false, &mut limits),
            Err(Unknown::InvalidInterval)
        );
        assert_eq!(
            saturating_range(&maximum, &maximum, &domain, true, &mut limits).unwrap(),
            Interval::singleton(&Integer::zero(), &mut limits).unwrap()
        );
    }
}

#[cfg(test)]
mod bitwise_tests {
    use super::*;

    #[test]
    fn bitwise_interval_bounds_enclose_every_eight_bit_result() {
        for signed in [false, true] {
            let ranges: &[(i128, i128)] = if signed {
                &[
                    (-128, 127),
                    (-128, -1),
                    (-128, -128),
                    (-20, -3),
                    (-5, 5),
                    (-1, -1),
                    (0, 0),
                    (0, 127),
                    (3, 15),
                ]
            } else {
                &[
                    (0, 255),
                    (0, 0),
                    (255, 255),
                    (1, 254),
                    (3, 15),
                    (128, 255),
                    (85, 170),
                ]
            };
            for &(al, au) in ranges {
                for &(bl, bu) in ranges {
                    for operation in [Operation::BitAnd, Operation::BitOr, Operation::BitXor] {
                        let mut limits = Limits::default();
                        let a = Interval::new(
                            Integer::from_i128(al),
                            Integer::from_i128(au),
                            &mut limits,
                        )
                        .unwrap();
                        let b = Interval::new(
                            Integer::from_i128(bl),
                            Integer::from_i128(bu),
                            &mut limits,
                        )
                        .unwrap();
                        let range =
                            bitwise_range(&a, &b, operation, 8, signed, &mut limits).unwrap();
                        let lower = range.lower().to_i128().unwrap();
                        let upper = range.upper().to_i128().unwrap();
                        for a in al..=au {
                            for b in bl..=bu {
                                let bits = match operation {
                                    Operation::BitAnd => (a as u8) & (b as u8),
                                    Operation::BitOr => (a as u8) | (b as u8),
                                    Operation::BitXor => (a as u8) ^ (b as u8),
                                    _ => unreachable!(),
                                };
                                let result = if signed {
                                    i128::from(bits as i8)
                                } else {
                                    i128::from(bits)
                                };
                                assert!(
                                    (lower..=upper).contains(&result),
                                    "{operation:?}: {a}, {b} -> {result} outside {lower}..{upper}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bitwise_constants_validate_domains_and_recover_after_budget_failure() {
        let singleton = |value| {
            Interval::singleton(&Integer::from_i128(value), &mut Limits::default()).unwrap()
        };
        let a = singleton(i128::MIN);
        let b = singleton(-1);
        let exact =
            bitwise_range(&a, &b, Operation::BitXor, 128, true, &mut Limits::default()).unwrap();
        assert_eq!(exact.lower().to_i128().unwrap(), i128::MAX);
        assert_eq!(exact.lower(), exact.upper());
        for width in [0, 129] {
            assert!(
                bitwise_range(
                    &a,
                    &b,
                    Operation::BitAnd,
                    width,
                    true,
                    &mut Limits::default()
                )
                .is_err()
            );
        }
        assert!(
            bitwise_range(
                &singleton(128),
                &singleton(0),
                Operation::BitAnd,
                8,
                true,
                &mut Limits::default()
            )
            .is_err()
        );
        assert!(
            bitwise_range(&b, &b, Operation::BitAnd, 8, false, &mut Limits::default()).is_err()
        );
        let mut limits = Limits::default();
        limits.arithmetic.steps = 0;
        assert!(bitwise_range(&a, &b, Operation::BitXor, 128, true, &mut limits).is_err());
        assert!(
            bitwise_range(&a, &b, Operation::BitXor, 128, true, &mut Limits::default()).is_ok()
        );
    }
}
