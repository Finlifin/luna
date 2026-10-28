//! Integer extrema of a univariate cubic via its quadratic finite difference.
use super::*;

impl Polynomial {
    pub(super) fn refine_univariate_cubic(
        &self,
        enclosure: &mut Interval,
        facts: &Facts,
        limits: &mut Limits,
    ) -> Result<bool, Unknown> {
        let mut variable = None;
        let mut coefficients = [
            Integer::zero(),
            Integer::zero(),
            Integer::zero(),
            Integer::zero(),
        ];
        for (factors, coefficient) in &self.terms {
            limits.step(1 + factors.len())?;
            for id in factors {
                if variable.is_some_and(|previous| previous != *id) {
                    return Ok(false);
                }
                variable = Some(*id);
            }
            coefficients[factors.len()] = limits.integer(coefficient)?;
        }
        if coefficients[3].is_zero() {
            return Ok(false);
        }
        let bound = facts.bound(variable.unwrap(), limits)?.copy(limits)?;
        let one = Integer::from_u128(1);
        let two = Integer::from_u128(2);
        let three_a = coefficients[3].multiply(&Integer::from_u128(3), &mut limits.arithmetic)?;
        let difference = [
            coefficients[3]
                .add(&coefficients[2], &mut limits.arithmetic)?
                .add(&coefficients[1], &mut limits.arithmetic)?,
            three_a.add(
                &coefficients[2].multiply(&two, &mut limits.arithmetic)?,
                &mut limits.arithmetic,
            )?,
            limits.integer(&three_a)?,
        ];
        let mut candidates = vec![limits.integer(&bound.lower)?, limits.integer(&bound.upper)?];
        if limits.compare(&bound.lower, &bound.upper)?.is_lt() {
            let last_step = bound.upper.subtract(&one, &mut limits.arithmetic)?;
            let mut cuts = vec![limits.integer(&bound.lower)?, limits.integer(&last_step)?];
            let mut numerator = difference[1].negate(&mut limits.arithmetic)?;
            let mut denominator = difference[2].multiply(&two, &mut limits.arithmetic)?;
            if denominator.is_negative() {
                numerator = numerator.negate(&mut limits.arithmetic)?;
                denominator = denominator.negate(&mut limits.arithmetic)?;
            }
            let floor = super::relations::divide_floor(&numerator, &denominator, limits)?;
            let next = floor.add(&one, &mut limits.arithmetic)?;
            for value in [floor, next] {
                if !limits.compare(&value, &bound.lower)?.is_lt()
                    && !limits.compare(&value, &last_step)?.is_gt()
                {
                    cuts.push(value);
                }
            }
            // At most four cuts. The interval crossing the real vertex has
            // at most one integer step; every other segment is monotone.
            for index in 1..cuts.len() {
                for current in (1..=index).rev() {
                    if limits.compare(&cuts[current - 1], &cuts[current])?.is_gt() {
                        cuts.swap(current - 1, current);
                    }
                }
            }
            for cut in &cuts {
                candidates.push(limits.integer(cut)?);
                candidates.push(cut.add(&one, &mut limits.arithmetic)?);
            }
            for segment in cuts.windows(2) {
                let left = evaluate(&difference, &segment[0], limits)?;
                let right = evaluate(&difference, &segment[1], limits)?;
                if left.is_zero() || (left.is_negative() == right.is_negative() && !right.is_zero())
                {
                    continue;
                }
                let rising = left.is_negative();
                let mut low = limits.integer(&segment[0])?;
                let mut high = limits.integer(&segment[1])?;
                while limits.compare(&low, &high)?.is_lt() {
                    limits.step(1)?;
                    let width = high.subtract(&low, &mut limits.arithmetic)?;
                    let half = super::relations::divide_floor(&width, &two, limits)?;
                    let middle = low.add(&half, &mut limits.arithmetic)?;
                    let value = evaluate(&difference, &middle, limits)?;
                    let crossed = if rising {
                        !value.is_negative()
                    } else {
                        value.is_negative() || value.is_zero()
                    };
                    if crossed {
                        high = middle;
                    } else {
                        low = middle.add(&one, &mut limits.arithmetic)?;
                    }
                }
                candidates.push(limits.integer(&low)?);
                candidates.push(low.add(&one, &mut limits.arithmetic)?);
            }
        }
        let mut exact: Option<Interval> = None;
        for point in candidates {
            let value = evaluate(&coefficients, &point, limits)?;
            match &mut exact {
                None => exact = Some(Interval::singleton(&value, limits)?),
                Some(range) => {
                    if limits.compare(&value, &range.lower)?.is_lt() {
                        range.lower = limits.integer(&value)?;
                    }
                    if limits.compare(&value, &range.upper)?.is_gt() {
                        range.upper = value;
                    }
                }
            }
        }
        let exact = exact.unwrap();
        if limits.compare(&exact.lower, &enclosure.lower)?.is_gt() {
            enclosure.lower = exact.lower;
        }
        if limits.compare(&exact.upper, &enclosure.upper)?.is_lt() {
            enclosure.upper = exact.upper;
        }
        Ok(true)
    }
}

fn evaluate(
    coefficients: &[Integer],
    value: &Integer,
    limits: &mut Limits,
) -> Result<Integer, Unknown> {
    let mut result = Integer::zero();
    for coefficient in coefficients.iter().rev() {
        limits.step(1)?;
        result = result
            .multiply(value, &mut limits.arithmetic)?
            .add(coefficient, &mut limits.arithmetic)?;
    }
    Ok(result)
}
