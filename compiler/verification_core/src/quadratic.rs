//! Exact, bounded semidefinite certificates for program polynomials.
//! This is an arithmetic proof strategy, not a type-proposition solver.
use super::*;

impl Polynomial {
    pub(super) fn refine_semidefinite_enclosure(
        &self,
        enclosure: &mut Interval,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        self.refine_semidefinite_bounds(enclosure, &Integer::zero(), &Integer::zero(), limits)
    }

    pub(super) fn refine_semidefinite_target(
        &self,
        enclosure: &mut Interval,
        target: &Interval,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        // Plain interval arithmetic is already exact for these forms.
        if self.terms.len() < 2 || self.terms.keys().all(|factors| factors.len() < 2) {
            return Ok(());
        }
        self.refine_semidefinite_bounds(enclosure, &target.lower, &target.upper, limits)
    }

    fn refine_semidefinite_bounds(
        &self,
        enclosure: &mut Interval,
        lower: &Integer,
        upper: &Integer,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        let need_lower = limits.compare(&enclosure.lower, lower)?.is_lt();
        let need_upper = limits.compare(&enclosure.upper, upper)?.is_gt();
        if !need_lower && !need_upper {
            return Ok(());
        }
        let Some(mut matrix) = self.quadratic_matrix(limits)? else {
            return Ok(());
        };
        let constant = limits.integer(&matrix[0][0])?;
        if need_lower {
            let twice = lower.multiply(&Integer::from_u128(2), &mut limits.arithmetic)?;
            matrix[0][0] = constant.subtract(&twice, &mut limits.arithmetic)?;
            if semidefinite(&matrix, false, limits)? {
                enclosure.lower = limits.integer(lower)?;
            }
        }
        if need_upper {
            // Restore the original constant before forming p-U: the lower
            // candidate's shift must not leak into the independent upper proof.
            let twice = upper.multiply(&Integer::from_u128(2), &mut limits.arithmetic)?;
            matrix[0][0] = constant.subtract(&twice, &mut limits.arithmetic)?;
            if semidefinite(&matrix, true, limits)? {
                enclosure.upper = limits.integer(upper)?;
            }
        }
        Ok(())
    }
    fn quadratic_matrix(&self, limits: &mut Limits) -> Result<Option<Vec<Vec<Integer>>>, Unknown> {
        const MAX_COORDINATES: usize = 16;
        let mut coordinates = BTreeMap::new();
        // Coordinate zero is the homogeneous constant 1. Matrix entries
        // represent 2*p, so no rational coefficients are introduced.
        for factors in self.terms.keys() {
            limits.step(1 + factors.len())?;
            if factors.len() > 2 {
                return Ok(None);
            }
            for id in factors {
                let next = coordinates.len() + 1;
                if !coordinates.contains_key(id) {
                    if next >= MAX_COORDINATES {
                        return Ok(None);
                    }
                    coordinates.insert(*id, next);
                }
            }
        }
        let width = coordinates.len() + 1;
        limits.step(width * width)?;
        let mut matrix = vec![vec![Integer::zero(); width]; width];
        for (factors, coefficient) in &self.terms {
            limits.step(1)?;
            let (left, right) = match factors.as_slice() {
                [] => (0, 0),
                [id] => (0, coordinates[id]),
                [left, right] => (coordinates[left], coordinates[right]),
                _ => unreachable!(),
            };
            let value = if left == right {
                coefficient.multiply(&Integer::from_u128(2), &mut limits.arithmetic)?
            } else {
                limits.integer(coefficient)?
            };
            matrix[left][right] = limits.integer(&value)?;
            matrix[right][left] = value;
        }
        Ok(Some(matrix))
    }

    /// The integer extremum of an unbounded univariate quadratic needs no
    /// input interval: evaluate the two integers around -b/(2*a). Do not use
    /// this integer-only certificate for a continuous derivative.
    fn global_integer_quadratic_extremum(
        &self,
        limits: &mut Limits,
    ) -> Result<Option<(Integer, bool)>, Unknown> {
        let mut variable = None;
        let mut coefficients = [Integer::zero(), Integer::zero(), Integer::zero()];
        for (factors, coefficient) in &self.terms {
            limits.step(1 + factors.len())?;
            if factors.len() > 2 {
                return Ok(None);
            }
            for id in factors {
                if variable.is_some_and(|previous| previous != *id) {
                    return Ok(None);
                }
                variable = Some(*id);
            }
            coefficients[factors.len()] = limits.integer(coefficient)?;
        }
        let [c, b, a] = coefficients;
        if a.is_zero() {
            return Ok(None);
        }
        let minimum = !a.is_negative();
        let mut numerator = b.negate(&mut limits.arithmetic)?;
        let mut denominator = a.multiply(&Integer::from_u128(2), &mut limits.arithmetic)?;
        if denominator.is_negative() {
            numerator = numerator.negate(&mut limits.arithmetic)?;
            denominator = denominator.negate(&mut limits.arithmetic)?;
        }
        let floor = super::relations::divide_floor(&numerator, &denominator, limits)?;
        let next = floor.add(&Integer::from_u128(1), &mut limits.arithmetic)?;
        let mut evaluate = |x: &Integer| -> Result<Integer, Unknown> {
            Ok(a.multiply(x, &mut limits.arithmetic)?
                .add(&b, &mut limits.arithmetic)?
                .multiply(x, &mut limits.arithmetic)?
                .add(&c, &mut limits.arithmetic)?)
        };
        let left = evaluate(&floor)?;
        let right = evaluate(&next)?;
        let order = limits.compare(&left, &right)?;
        Ok(Some((
            if (minimum && order.is_gt()) || (!minimum && order.is_lt()) {
                right
            } else {
                left
            },
            minimum,
        )))
    }

    /// Global certificates require no interval premises. The caller must
    /// validate every original input version before cancellation/normalization.
    pub(super) fn global_quadratic_comparison(
        &self,
        relation: Comparison,
        limits: &mut Limits,
    ) -> Result<Option<bool>, Unknown> {
        limits.step(self.terms.len())?;
        if self.terms.keys().all(|factors| factors.len() < 2) {
            return Ok(None);
        }
        if let Some((bound, minimum)) = self.global_integer_quadratic_extremum(limits)? {
            let sign = limits.compare(&bound, &Integer::zero())?;
            use Comparison::*;
            return Ok(match (minimum, relation) {
                (true, GreaterEqual) if !sign.is_lt() => Some(true),
                (true, Less) if !sign.is_lt() => Some(false),
                (true, Greater | NotEqual) if sign.is_gt() => Some(true),
                (true, LessEqual | Equal) if sign.is_gt() => Some(false),
                (false, LessEqual) if !sign.is_gt() => Some(true),
                (false, Greater) if !sign.is_gt() => Some(false),
                (false, Less | NotEqual) if sign.is_lt() => Some(true),
                (false, GreaterEqual | Equal) if sign.is_lt() => Some(false),
                _ => None,
            });
        }
        let Some(mut matrix) = self.quadratic_matrix(limits)? else {
            return Ok(None);
        };
        let constant = limits.integer(&matrix[0][0])?;
        let mut certify = |bound: i128, upper: bool| -> Result<bool, Unknown> {
            let twice = Integer::from_i128(bound)
                .multiply(&Integer::from_u128(2), &mut limits.arithmetic)?;
            matrix[0][0] = constant.subtract(&twice, &mut limits.arithmetic)?;
            semidefinite(&matrix, upper, limits)
        };
        use Comparison::*;
        Ok(match relation {
            GreaterEqual => {
                if certify(0, false)? {
                    Some(true)
                } else if certify(-1, true)? {
                    Some(false)
                } else {
                    None
                }
            }
            Greater => {
                if certify(1, false)? {
                    Some(true)
                } else if certify(0, true)? {
                    Some(false)
                } else {
                    None
                }
            }
            LessEqual => {
                if certify(0, true)? {
                    Some(true)
                } else if certify(1, false)? {
                    Some(false)
                } else {
                    None
                }
            }
            Less => {
                if certify(-1, true)? {
                    Some(true)
                } else if certify(0, false)? {
                    Some(false)
                } else {
                    None
                }
            }
            Equal | NotEqual => {
                if certify(1, false)? || certify(-1, true)? {
                    Some(relation == NotEqual)
                } else if certify(0, false)? && certify(0, true)? {
                    Some(relation == Equal)
                } else {
                    None
                }
            }
        })
    }
}

/// With a positive pivot a, PSD is equivalent to PSD of a*B-v*v^T.
/// Divide each Schur update by the previous positive pivot (initially 1).
/// This fraction-free normalization controls coefficient growth; exact
/// division is checked, and positive scaling preserves the PSD equivalence.
/// A zero diagonal in a PSD matrix requires its complete row to be zero.
/// Negative pivots or nonzero zero-pivot rows fail the certificate, not the
/// original proposition. All coefficient growth consumes the shared budget.
fn semidefinite(
    input: &[Vec<Integer>],
    negate: bool,
    limits: &mut Limits,
) -> Result<bool, Unknown> {
    let width = input.len();
    limits.step(width * width)?;
    let mut matrix = input
        .iter()
        .map(|row| {
            row.iter()
                .map(|value| {
                    if negate {
                        value.negate(&mut limits.arithmetic).map_err(Unknown::from)
                    } else {
                        limits.integer(value)
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut previous = Integer::from_u128(1);
    for pivot in 0..width {
        limits.step(1)?;
        let diagonal = limits.integer(&matrix[pivot][pivot])?;
        if diagonal.is_negative() {
            return Ok(false);
        }
        if diagonal.is_zero() {
            for value in &matrix[pivot][pivot + 1..] {
                limits.step(1)?;
                if !value.is_zero() {
                    return Ok(false);
                }
            }
            continue;
        }
        for row in pivot + 1..width {
            for column in row..width {
                limits.step(1)?;
                let scaled = diagonal.multiply(&matrix[row][column], &mut limits.arithmetic)?;
                let product =
                    matrix[row][pivot].multiply(&matrix[pivot][column], &mut limits.arithmetic)?;
                let numerator = scaled.subtract(&product, &mut limits.arithmetic)?;
                let (next, remainder) =
                    numerator.div_rem_trunc(&previous, &mut limits.arithmetic)?;
                if !remainder.is_zero() {
                    // Never truncate a certificate coefficient. An unexpected
                    // non-integral update leaves this proof path unknown.
                    return Err(Unknown::InsufficientBounds);
                }
                matrix[row][column] = limits.integer(&next)?;
                matrix[column][row] = next;
            }
        }
        previous = diagonal;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix(rows: &[Vec<i128>]) -> Vec<Vec<Integer>> {
        rows.iter()
            .map(|row| row.iter().map(|value| Integer::from_i128(*value)).collect())
            .collect()
    }

    #[test]
    fn dense_gram_certificate_stays_within_the_default_budget() {
        // I + u*u^T is positive definite for any u. Its entries and minors
        // are small, even though unnormalized Schur steps grow exponentially.
        let rows = (0..16)
            .map(|i| {
                (0..16)
                    .map(|j| ((i + 1) * (j + 1) + usize::from(i == j)) as i128)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            semidefinite(&matrix(&rows), false, &mut Limits::default()),
            Ok(true)
        );
    }

    #[test]
    fn symmetric_three_by_three_certificates_match_all_principal_minors() {
        for a in -2i128..=2 {
            for b in -2i128..=2 {
                for c in -2i128..=2 {
                    for d in -2i128..=2 {
                        for e in -2i128..=2 {
                            for f in -2i128..=2 {
                                let rows = vec![vec![a, b, c], vec![b, d, e], vec![c, e, f]];
                                let determinant =
                                    a * d * f + 2 * b * c * e - a * e * e - d * c * c - f * b * b;
                                let expected = a >= 0
                                    && d >= 0
                                    && f >= 0
                                    && a * d >= b * b
                                    && a * f >= c * c
                                    && d * f >= e * e
                                    && determinant >= 0;
                                assert_eq!(
                                    semidefinite(&matrix(&rows), false, &mut Limits::default()),
                                    Ok(expected),
                                    "{rows:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod growth_tests {
    use super::*;

    #[test]
    fn gram_matrices_keep_singular_and_zero_rows_through_exact_normalization() {
        for width in [2, 4, 8, 16] {
            for rank in [0, 1, 3, width] {
                let vectors = (0..width)
                    .map(|row| {
                        (0..rank)
                            .map(|column| {
                                if row % 4 == 0 {
                                    0
                                } else {
                                    ((row * 7 + column * 3 + row * column) % 7) as i128 - 3
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                let matrix = (0..width)
                    .map(|row| {
                        (0..width)
                            .map(|column| {
                                Integer::from_i128(
                                    vectors[row]
                                        .iter()
                                        .zip(&vectors[column])
                                        .map(|(a, b)| a * b)
                                        .sum(),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                let original = matrix.clone();
                assert_eq!(
                    semidefinite(&matrix, false, &mut Limits::default()),
                    Ok(true),
                    "width={width}, rank={rank}"
                );
                let negative = matrix
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|value| value.negate(&mut Budget::default()).unwrap())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    semidefinite(&negative, true, &mut Limits::default()),
                    Ok(true)
                );
                let mut tiny = Limits::default();
                tiny.arithmetic.steps = 1;
                assert!(semidefinite(&matrix, false, &mut tiny).is_err());
                assert_eq!(matrix, original);
                assert_eq!(
                    semidefinite(&matrix, false, &mut Limits::default()),
                    Ok(true)
                );
                let mut indefinite = matrix.clone();
                indefinite[width - 1][width - 1] = Integer::from_i128(-1);
                assert_eq!(
                    semidefinite(&indefinite, false, &mut Limits::default()),
                    Ok(false)
                );
            }
        }
    }
}
