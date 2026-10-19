//! Value comparison deliberately differs from normal-form/term identity.
use super::*;

pub(super) fn compare(
    left: &Term,
    right: &Term,
    op: n::IntegerOp,
    budget: &mut comptime_runtime::Budget,
) -> Option<bool> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let equality = op == n::IntegerOp::Equal;
    budget.consume(1).ok()?;
    let ordering = match (left, right) {
        (Term::Bool(a), Term::Bool(b)) if equality => Some(a.cmp(b)),
        (Term::Char(a), Term::Char(b)) => Some(a.cmp(b)),
        (Term::Integer(a), Term::Integer(b)) => {
            budget
                .consume(
                    a.bit_len()
                        .div_ceil(32)
                        .saturating_add(b.bit_len().div_ceil(32)),
                )
                .ok()?;
            Some(a.cmp(b))
        }
        (Term::String(a), Term::String(b)) | (Term::Symbol(a), Term::Symbol(b)) if equality => {
            budget
                .consume(a.len().min(b.len()).saturating_add(1))
                .ok()?;
            Some(a.cmp(b))
        }
        (Term::BitVec(a), Term::BitVec(b)) if equality => {
            budget
                .consume(a.words().len().saturating_add(b.words().len()))
                .ok()?;
            return Some(a == b);
        }
        (Term::Float(a), Term::Float(b)) => Some(a.compare(b, budget).ok()?),
        (Term::Float32(a), Term::Float32(b)) => f32::from_bits(*a).partial_cmp(&f32::from_bits(*b)),
        (Term::Float64(a), Term::Float64(b)) => f64::from_bits(*a).partial_cmp(&f64::from_bits(*b)),
        _ => return None,
    };
    Some(match op {
        n::IntegerOp::Equal => matches!(ordering, Some(Equal)),
        n::IntegerOp::Less => matches!(ordering, Some(Less)),
        n::IntegerOp::LessEqual => matches!(ordering, Some(Less | Equal)),
        n::IntegerOp::Greater => matches!(ordering, Some(Greater)),
        n::IntegerOp::GreaterEqual => matches!(ordering, Some(Greater | Equal)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unordered_is_false_but_unknown_and_exhausted_are_not_false() {
        let nan = Term::Float64(0x7ff8000000000001);
        for op in [
            n::IntegerOp::Equal,
            n::IntegerOp::Less,
            n::IntegerOp::LessEqual,
            n::IntegerOp::Greater,
            n::IntegerOp::GreaterEqual,
        ] {
            assert_eq!(
                compare(&nan, &nan, op, &mut Default::default()),
                Some(false)
            );
        }
        assert_eq!(
            compare(
                &Term::Float64(0),
                &Term::Float64(1 << 63),
                n::IntegerOp::Equal,
                &mut Default::default()
            ),
            Some(true)
        );
        assert_eq!(
            compare(
                &Term::Unit,
                &Term::Unit,
                n::IntegerOp::Equal,
                &mut Default::default()
            ),
            None
        );
        let mut exhausted = comptime_runtime::Budget {
            steps: 0,
            max_limbs: 4096,
        };
        assert_eq!(
            compare(&nan, &nan, n::IntegerOp::Equal, &mut exhausted),
            None
        );
        // Failure consumes no reusable verdict: a fresh budget can decide it.
        assert_eq!(
            compare(&nan, &nan, n::IntegerOp::Equal, &mut Default::default()),
            Some(false)
        );
    }
}
