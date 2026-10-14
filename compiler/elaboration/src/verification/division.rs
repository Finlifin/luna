//! Bounded interval enclosure for the explicitly rounded IR operations.
//! This is program arithmetic, never a PTC type proposition.
use super::{Integer, Interval, Limits, Unknown};

impl Interval {
    pub(super) fn div_rem_trunc(
        &self,
        divisor: &Self,
        limits: &mut Limits,
    ) -> Result<(Self, Self), Unknown> {
        limits.step(1)?;
        let zero = Integer::zero();
        if !limits.compare(&divisor.lower, &zero)?.is_gt()
            && !limits.compare(&divisor.upper, &zero)?.is_lt()
        {
            return Err(Unknown::InsufficientBounds);
        }
        // On a divisor interval of one sign, extrema of truncating quotient
        // occur at endpoint pairs, including when the dividend spans zero.
        let mut quotients = Vec::with_capacity(4);
        for a in [&self.lower, &self.upper] {
            for b in [&divisor.lower, &divisor.upper] {
                limits.step(1)?;
                quotients.push(a.div_rem_trunc(b, &mut limits.arithmetic)?.0);
            }
        }
        let (mut low, mut high) = (0, 0);
        for i in 1..4 {
            if limits.compare(&quotients[i], &quotients[low])?.is_lt() {
                low = i;
            }
            if limits.compare(&quotients[i], &quotients[high])?.is_gt() {
                high = i;
            }
        }
        let quotient = Self::new(
            limits.integer(&quotients[low])?,
            limits.integer(&quotients[high])?,
            limits,
        )?;
        if self.lower == self.upper && divisor.lower == divisor.upper {
            let remainder = self
                .lower
                .div_rem_trunc(&divisor.lower, &mut limits.arithmetic)?
                .1;
            return Ok((quotient, Self::singleton(&remainder, limits)?));
        }
        // |r| < |b|, |r| <= |a|, with the dividend's sign. Do not
        // infer an exact remainder from endpoint remainders: it is periodic.
        let max_divisor = if limits.compare(&divisor.lower, &zero)?.is_gt() {
            limits.integer(&divisor.upper)?
        } else {
            divisor.lower.negate(&mut limits.arithmetic)?
        };
        let cap = max_divisor.subtract(&Integer::from_u128(1), &mut limits.arithmetic)?;
        let negative_cap = cap.negate(&mut limits.arithmetic)?;
        let lower = if !limits.compare(&self.lower, &zero)?.is_lt() {
            zero.clone()
        } else if limits.compare(&self.lower, &negative_cap)?.is_gt() {
            limits.integer(&self.lower)?
        } else {
            negative_cap
        };
        let upper = if !limits.compare(&self.upper, &zero)?.is_gt() {
            zero
        } else if limits.compare(&self.upper, &cap)?.is_lt() {
            limits.integer(&self.upper)?
        } else {
            cap
        };
        Ok((quotient, Self::new(lower, upper, limits)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn interval(a: i128, b: i128) -> Interval {
        Interval::new(
            Integer::from_i128(a),
            Integer::from_i128(b),
            &mut Limits::default(),
        )
        .unwrap()
    }
    #[test]
    fn interval_division_encloses_every_small_signed_value() {
        for lo in -5..=5 {
            for hi in lo..=5 {
                for dl in -5..=5 {
                    for dh in dl..=5 {
                        let result = interval(lo, hi)
                            .div_rem_trunc(&interval(dl, dh), &mut Limits::default());
                        if dl <= 0 && dh >= 0 {
                            assert_eq!(result, Err(Unknown::InsufficientBounds));
                            continue;
                        }
                        let (q, r) = result.unwrap();
                        let (ql, qh) = (q.lower.to_i128().unwrap(), q.upper.to_i128().unwrap());
                        let (rl, rh) = (r.lower.to_i128().unwrap(), r.upper.to_i128().unwrap());
                        for a in lo..=hi {
                            for b in dl..=dh {
                                assert!(ql <= a / b && a / b <= qh, "{lo}..{hi} / {dl}..{dh}");
                                assert!(rl <= a % b && a % b <= rh, "{lo}..{hi} % {dl}..{dh}");
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn division_budget_failure_does_not_change_later_results() {
        let (a, b) = (interval(-7, 7), interval(2, 3));
        let mut limits = Limits::default();
        limits.arithmetic.steps = 0;
        assert!(a.div_rem_trunc(&b, &mut limits).is_err());
        let (q, r) = a.div_rem_trunc(&b, &mut Limits::default()).unwrap();
        assert_eq!(q, interval(-3, 3));
        assert_eq!(r, interval(-2, 2));
    }
}
