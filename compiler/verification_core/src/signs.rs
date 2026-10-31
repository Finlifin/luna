//! Finite sign reasoning for polynomials over current mathematical integers.
use super::*;

const NEGATIVE: u8 = 1;
const ZERO: u8 = 2;
const POSITIVE: u8 = 4;
const ALL: u8 = NEGATIVE | ZERO | POSITIVE;

fn accepted(relation: Comparison) -> u8 {
    match relation {
        Comparison::Equal => ZERO,
        Comparison::NotEqual => NEGATIVE | POSITIVE,
        Comparison::Less => NEGATIVE,
        Comparison::LessEqual => NEGATIVE | ZERO,
        Comparison::Greater => POSITIVE,
        Comparison::GreaterEqual => POSITIVE | ZERO,
    }
}

fn multiply(left: u8, right: u8) -> u8 {
    let mut result = 0;
    for (a, x) in [(NEGATIVE, -1), (ZERO, 0), (POSITIVE, 1)] {
        for (b, y) in [(NEGATIVE, -1), (ZERO, 0), (POSITIVE, 1)] {
            if left & a != 0 && right & b != 0 {
                result |= match x * y {
                    -1 => NEGATIVE,
                    0 => ZERO,
                    _ => POSITIVE,
                };
            }
        }
    }
    result
}

fn add(left: u8, right: u8) -> u8 {
    let mut result = 0;
    for a in [NEGATIVE, ZERO, POSITIVE] {
        for b in [NEGATIVE, ZERO, POSITIVE] {
            if left & a != 0 && right & b != 0 {
                result |= if a == ZERO {
                    b
                } else if b == ZERO || a == b {
                    a
                } else {
                    ALL
                };
            }
        }
    }
    result
}

fn variable_sign(value: ValueId, facts: &Facts, limits: &mut Limits) -> Result<u8, Unknown> {
    limits.step(1)?;
    let mut signs = ALL;
    match facts.bound(value, limits) {
        Ok(bound) => {
            if !bound.lower().is_negative() {
                signs &= !NEGATIVE;
            }
            if bound.upper().is_negative() || bound.upper().is_zero() {
                signs &= !POSITIVE;
            }
            if bound.upper().is_negative()
                || (!bound.lower().is_negative() && !bound.lower().is_zero())
            {
                signs &= !ZERO;
            }
        }
        Err(Unknown::MissingBound(_)) => {}
        Err(failure) => return Err(failure),
    }
    for relation in [Comparison::Less, Comparison::Equal, Comparison::Greater] {
        let query = Condition::Compare {
            value,
            relation,
            constant: Integer::zero(),
        };
        if let Some(truth) = facts.known_condition(&query, limits)? {
            signs &= if truth {
                accepted(relation)
            } else {
                ALL & !accepted(relation)
            };
        }
    }
    Ok(signs)
}

impl Polynomial {
    pub(super) fn sign_comparison(
        &self,
        facts: &Facts,
        relation: Comparison,
        limits: &mut Limits,
    ) -> Result<Option<bool>, Unknown> {
        limits.step(1)?;
        let mut signs = ZERO;
        let mut variables = BTreeMap::new();
        for (factors, coefficient) in &self.terms {
            limits.step(1)?;
            let mut term = if coefficient.is_zero() {
                ZERO
            } else if coefficient.is_negative() {
                NEGATIVE
            } else {
                POSITIVE
            };
            let mut offset = 0;
            while offset < factors.len() {
                let value = factors[offset];
                let count = factors[offset..]
                    .iter()
                    .take_while(|&&next| next == value)
                    .count();
                limits.step(count)?;
                let mut factor = match variables.get(&value) {
                    Some(sign) => *sign,
                    None => {
                        let sign = variable_sign(value, facts, limits)?;
                        variables.insert(value, sign);
                        sign
                    }
                };
                // Repeated occurrences are the same value, not independent
                // sign choices. An even power is positive or zero.
                if count % 2 == 0 {
                    factor = (factor & ZERO)
                        | if factor & (NEGATIVE | POSITIVE) != 0 {
                            POSITIVE
                        } else {
                            0
                        };
                }
                term = multiply(term, factor);
                offset += count;
            }
            if term == 0 {
                return Ok(None);
            }
            signs = add(signs, term);
            // Once all signs are possible, adding further independently
            // approximated terms cannot narrow this domain. Other solvers
            // may still recover coefficient or cross-term correlations.
            if signs == ALL {
                return Ok(None);
            }
        }
        if signs == 0 {
            return Ok(None);
        }
        let accepted = accepted(relation);
        Ok(if signs & !accepted == 0 {
            Some(true)
        } else if signs & accepted == 0 {
            Some(false)
        } else {
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_sign_tables_match_concrete_sums_and_products() {
        let sign = |value: i32| {
            if value < 0 {
                NEGATIVE
            } else if value == 0 {
                ZERO
            } else {
                POSITIVE
            }
        };
        for left in 1..=ALL {
            for right in 1..=ALL {
                let mut sum = 0;
                let mut product = 0;
                for x in -3..=3 {
                    for y in -3..=3 {
                        if left & sign(x) != 0 && right & sign(y) != 0 {
                            sum |= sign(x + y);
                            product |= sign(x * y);
                        }
                    }
                }
                assert_eq!(add(left, right), sum);
                assert_eq!(multiply(left, right), product);
            }
        }
    }
}
