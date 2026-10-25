//! Integer residues certify nonzero polynomials, optionally using interval facts.
use super::*;

impl Polynomial {
    pub(super) fn excludes_integer_zero(
        &self,
        facts: &Facts,
        limits: &mut Limits,
    ) -> Result<bool, Unknown> {
        if self.excludes_zero_by_common_factor(facts, limits)?
            || self.excludes_zero_by_coefficient_gcd(limits)?
        {
            return Ok(true);
        }
        // A root over the integers must also be a root modulo every modulus.
        // Exhaust a small residue domain; a partial search is never evidence.
        if !self.terms.keys().any(|factors| factors.len() >= 2) {
            return Ok(false);
        }
        let mut variables = BTreeMap::new();
        for factors in self.terms.keys() {
            limits.step(factors.len().saturating_add(1))?;
            for value in factors {
                let index = variables.len();
                variables.entry(*value).or_insert(index);
                if variables.len() > 8 {
                    return Ok(false);
                }
            }
        }
        // Prime powers detect obstructions invisible over their residue
        // fields: x²-6 has roots modulo 2, 3 and 5, but none modulo 4.
        for modulus in [2_usize, 3, 4, 5, 8, 9] {
            let mut residues = vec![Vec::new(); variables.len()];
            let mut volume = 1;
            for (value, index) in &variables {
                limits.step(1)?;
                residues[*index] = match facts.bound(*value, limits) {
                    Ok(bound) => interval_residues(&bound, modulus, limits)?,
                    Err(Unknown::MissingBound(_)) => (0..modulus).collect(),
                    Err(failure) => return Err(failure),
                };
                volume *= residues[*index].len();
            }
            if volume > 256 {
                continue;
            }
            let divisor = Integer::from_u128(modulus as u128);
            let mut terms = Vec::with_capacity(self.terms.len());
            for (factors, coefficient) in &self.terms {
                limits.step(factors.len().saturating_add(1))?;
                let remainder = coefficient
                    .div_rem_trunc(&divisor, &mut limits.arithmetic)?
                    .1;
                // Truncated negative remainders must be normalized to the
                // same residue representatives as nonnegative assignments.
                let coefficient = remainder.to_i128()?.rem_euclid(modulus as i128) as usize;
                terms.push((
                    factors.iter().map(|v| variables[v]).collect::<Vec<_>>(),
                    coefficient,
                ));
            }
            let mut root = false;
            for assignment in 0..volume {
                limits.step(1)?;
                let mut digits = [0; 8];
                let mut remaining = assignment;
                for (digit, domain) in digits.iter_mut().zip(&residues) {
                    *digit = domain[remaining % domain.len()];
                    remaining /= domain.len();
                }
                let mut sum = 0;
                for (factors, coefficient) in &terms {
                    limits.step(factors.len().saturating_add(1))?;
                    let mut term = *coefficient;
                    for index in factors {
                        term = term * digits[*index] % modulus;
                    }
                    sum = (sum + term) % modulus;
                }
                if sum == 0 {
                    root = true;
                    break;
                }
            }
            if !root {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn excludes_zero_by_coefficient_gcd(&self, limits: &mut Limits) -> Result<bool, Unknown> {
        let Some(constant) = self.terms.get(&Vec::new()) else {
            return Ok(false);
        };
        if constant.is_zero() {
            return Ok(false);
        }
        let mut gcd = Integer::zero();
        for (factors, coefficient) in &self.terms {
            limits.step(1)?;
            if factors.is_empty() {
                continue;
            }
            let mut next = if coefficient.is_negative() {
                coefficient.negate(&mut limits.arithmetic)?
            } else {
                limits.integer(coefficient)?
            };
            while !next.is_zero() {
                limits.step(1)?;
                let remainder = gcd.div_rem_trunc(&next, &mut limits.arithmetic)?.1;
                gcd = next;
                next = remainder;
            }
            if limits.compare(&gcd, &Integer::from_u128(1))?.is_eq() {
                return Ok(false);
            }
        }
        if gcd.is_zero() {
            return Ok(false);
        }
        Ok(!constant
            .div_rem_trunc(&gcd, &mut limits.arithmetic)?
            .1
            .is_zero())
    }
}

fn interval_residues(
    bound: &Interval,
    modulus: usize,
    limits: &mut Limits,
) -> Result<Vec<usize>, Unknown> {
    let width = bound.upper.subtract(&bound.lower, &mut limits.arithmetic)?;
    if !limits
        .compare(&width, &Integer::from_u128((modulus - 1) as u128))?
        .is_lt()
    {
        return Ok((0..modulus).collect());
    }
    let count = width.to_i128()? as usize + 1;
    let divisor = Integer::from_u128(modulus as u128);
    let start = bound
        .lower
        .div_rem_trunc(&divisor, &mut limits.arithmetic)?
        .1
        .to_i128()?
        .rem_euclid(modulus as i128) as usize;
    limits.step(count)?;
    Ok((0..count)
        .map(|offset| (start + offset) % modulus)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_residue_sets_match_concrete_signed_enumeration() {
        for lower in -12_i128..=12 {
            for upper in lower..=12 {
                for modulus in [2_usize, 3, 4, 5, 8, 9] {
                    let mut limits = Limits::default();
                    let bound = Interval::new(
                        Integer::from_i128(lower),
                        Integer::from_i128(upper),
                        &mut limits,
                    )
                    .unwrap();
                    let actual = interval_residues(&bound, modulus, &mut limits).unwrap();
                    let expected = (lower..=upper)
                        .map(|v| v.rem_euclid(modulus as i128) as usize)
                        .collect::<std::collections::BTreeSet<_>>();
                    assert_eq!(actual.len(), expected.len());
                    assert_eq!(
                        actual
                            .into_iter()
                            .collect::<std::collections::BTreeSet<_>>(),
                        expected
                    );
                }
            }
        }
    }
}
