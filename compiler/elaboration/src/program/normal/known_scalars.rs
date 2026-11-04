//! Value comparison deliberately differs from normal-form/term identity.
use super::observation::Failure;
use super::*;

pub(super) fn compare(
    left: &Term,
    right: &Term,
    op: n::IntegerOp,
    budget: &mut comptime_runtime::Budget,
) -> Result<bool, Failure> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let equality = op == n::IntegerOp::Equal;
    budget.consume(1)?;
    let ordering = match (left, right) {
        (Term::Bool(a), Term::Bool(b)) if equality => Some(a.cmp(b)),
        (Term::Char(a), Term::Char(b)) => Some(a.cmp(b)),
        (Term::Integer(a), Term::Integer(b)) => {
            budget.consume(
                a.bit_len()
                    .div_ceil(32)
                    .saturating_add(b.bit_len().div_ceil(32)),
            )?;
            Some(a.cmp(b))
        }
        (Term::String(a), Term::String(b)) | (Term::Symbol(a), Term::Symbol(b)) if equality => {
            budget.consume(a.len().min(b.len()).saturating_add(1))?;
            Some(a.cmp(b))
        }
        (Term::BitVec(a), Term::BitVec(b)) if equality => {
            budget.consume(a.words().len().saturating_add(b.words().len()))?;
            return Ok(a == b);
        }
        (Term::Float(a), Term::Float(b)) => Some(a.compare(b, budget)?),
        (Term::Float32(a), Term::Float32(b)) => f32::from_bits(*a).partial_cmp(&f32::from_bits(*b)),
        (Term::Float64(a), Term::Float64(b)) => f64::from_bits(*a).partial_cmp(&f64::from_bits(*b)),
        _ => return Err(Failure::Unavailable),
    };
    Ok(match op {
        n::IntegerOp::Equal => matches!(ordering, Some(Equal)),
        n::IntegerOp::Less => matches!(ordering, Some(Less)),
        n::IntegerOp::LessEqual => matches!(ordering, Some(Less | Equal)),
        n::IntegerOp::Greater => matches!(ordering, Some(Greater)),
        n::IntegerOp::GreaterEqual => matches!(ordering, Some(Greater | Equal)),
        _ => return Err(Failure::Unavailable),
    })
}

/// Read an admitted literal without synthesizing an expression or invoking code.
/// The enclosing expression/pattern owns its node charge; this charges payloads.
pub(in crate::program) fn literal(
    literal: &n::Literal,
    budget: &mut comptime_runtime::Budget,
) -> Result<Term, Failure> {
    use n::Literal as L;
    Ok(match literal {
        L::Bool(value) => Term::Bool(*value),
        L::Char(value) => Term::Char(*value),
        L::Float32(value) => Term::Float32(*value),
        L::Float64(value) => Term::Float64(*value),
        L::Float(value) => Term::Float(comptime_runtime::decimal::DecimalValue::parse_literal(
            value, budget,
        )?),
        L::BitVec { width, words } => Term::BitVec(comptime_runtime::bitvec::BitVec::from_words(
            *width, words, budget,
        )?),
        L::String(value) | L::Symbol(value) => {
            budget.consume(value.len())?;
            if matches!(literal, L::String(_)) {
                Term::String(value.as_str().into())
            } else {
                Term::Symbol(value.as_str().into())
            }
        }
        L::SignedInteger(value) => Term::Integer(comptime_runtime::Integer::from_i128(*value)),
        L::UnsignedInteger(value) => Term::Integer(comptime_runtime::Integer::from_u128(*value)),
        L::Integer(value) => {
            let (digits, negative) = value
                .strip_prefix('-')
                .map_or((value.as_str(), false), |digits| (digits, true));
            let value = comptime_runtime::Integer::parse_literal(digits, budget)?;
            Term::Integer(if negative {
                value.negate(budget)?
            } else {
                value
            })
        }
        // Pending literals require a receiving domain. Half-precision comparison
        // remains unsupported by this observer, as before this migration.
        L::PendingInteger { .. } | L::PendingFloat { .. } | L::Float16(_) | L::BFloat16(_) => {
            return Err(Failure::Unavailable);
        }
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
            assert_eq!(compare(&nan, &nan, op, &mut Default::default()), Ok(false));
        }
        assert_eq!(
            compare(
                &Term::Float64(0),
                &Term::Float64(1 << 63),
                n::IntegerOp::Equal,
                &mut Default::default()
            ),
            Ok(true)
        );
        assert_eq!(
            compare(
                &Term::Unit,
                &Term::Unit,
                n::IntegerOp::Equal,
                &mut Default::default()
            ),
            Err(Failure::Unavailable)
        );
        let mut exhausted = comptime_runtime::Budget {
            steps: 0,
            max_limbs: 4096,
        };
        assert_eq!(
            compare(&nan, &nan, n::IntegerOp::Equal, &mut exhausted),
            Err(Failure::Steps)
        );
        // Failure consumes no reusable verdict: a fresh budget can decide it.
        assert_eq!(
            compare(&nan, &nan, n::IntegerOp::Equal, &mut Default::default()),
            Ok(false)
        );
    }
    #[test]
    fn literal_limits_invalid_data_and_unresolved_domains_are_distinct() {
        let invalid = n::Literal::BitVec {
            width: 1,
            words: vec![2],
        };
        assert!(matches!(
            literal(&invalid, &mut Default::default()),
            Err(Failure::Invalid(_))
        ));
        let valid = n::Literal::BitVec {
            width: 65,
            words: vec![0, 0, 1],
        };
        let mut small = comptime_runtime::Budget {
            steps: 100,
            max_limbs: 1,
        };
        assert_eq!(literal(&valid, &mut small), Err(Failure::Size));
        assert!(small.steps > 0);
        assert!(matches!(
            literal(&valid, &mut Default::default()),
            Ok(Term::BitVec(_))
        ));
        let text = n::Literal::String("bounded".into());
        let mut short = comptime_runtime::Budget {
            steps: 1,
            ..Default::default()
        };
        assert_eq!(literal(&text, &mut short), Err(Failure::Steps));
        assert_eq!(
            literal(&text, &mut Default::default()),
            Ok(Term::String("bounded".into()))
        );
        assert_eq!(
            literal(
                &n::Literal::PendingInteger {
                    spelling: "1".into(),
                    negative: false
                },
                &mut Default::default()
            ),
            Err(Failure::Unavailable)
        );
        assert!(matches!(
            literal(&n::Literal::Integer("oops".into()), &mut Default::default()),
            Err(Failure::Invalid(_))
        ));
    }
}
