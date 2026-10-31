//! Shared scalar serialization for constant and staged expression materializers.
use super::Value;
use comptime_runtime::{Budget, Error};
use nir::program::Literal;

impl Value {
    /// Serialize a scalar representation without admitting its declared type or
    /// executing code. The caller retains phase, domain and formation checks.
    /// Non-scalar values return None; all inspection/copy work consumes fuel.
    pub fn freeze_literal(&self, budget: &mut Budget) -> Result<Option<Literal>, Error> {
        budget.consume(1)?;
        Ok(Some(match self {
            Self::Bool(value) => Literal::Bool(*value),
            Self::Char(value) => Literal::Char(*value),
            Self::SignedInteger(value) => Literal::SignedInteger(*value),
            Self::UnsignedInteger(value) => Literal::UnsignedInteger(*value),
            Self::Float16(value) => Literal::Float16(*value),
            Self::BFloat16(value) => Literal::BFloat16(*value),
            Self::Float32(value) => Literal::Float32(*value),
            Self::Float64(value) => Literal::Float64(*value),
            Self::Symbol(value) => {
                budget.consume(value.len())?;
                Literal::Symbol(value.to_string())
            }
            Self::String(value) => {
                budget.consume(value.len())?;
                Literal::String(value.to_string())
            }
            Self::Integer(value) => Literal::Integer(value.to_hex_literal(budget)?),
            Self::Float(value) => Literal::Float(value.to_literal(budget)?),
            Self::BitVec(value) => {
                if value.words().len() > budget.max_limbs {
                    return Err(Error::SizeLimit);
                }
                budget.consume(value.words().len())?;
                Literal::BitVec {
                    width: value.width(),
                    words: value.words().to_vec(),
                }
            }
            _ => return Ok(None),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Flow, Interpreter};
    use nir::program::{Expr, ExprKind, Package, SourceRange};
    use nir::{Primitive as P, Term};

    #[test]
    fn scalar_literals_round_trip_exact_payloads_without_domain_erasure() {
        let mut construction = Budget::default();
        let values = [
            (Value::Bool(true), P::Bool),
            (Value::Char('雪'), P::Char),
            (Value::String("line\n\0雪".into()), P::Str),
            (Value::Symbol("name\"\\雪".into()), P::Symbol),
            (Value::SignedInteger(i128::MIN), P::I128),
            (Value::UnsignedInteger(u128::MAX), P::U128),
            (Value::Float16(0x7e11), P::F16),
            (Value::BFloat16(0xffc1), P::Bf16),
            (Value::Float32(0x80000000), P::F32),
            (Value::Float64(0x7ff8000000000001), P::F64),
            (
                Value::Integer(comptime_runtime::Integer::from_i128(i128::MIN)),
                P::Integer,
            ),
            (
                Value::Float(
                    comptime_runtime::decimal::DecimalValue::parse_literal(
                        "1.0000000000000000000000000000001",
                        &mut construction,
                    )
                    .unwrap(),
                ),
                P::Float,
            ),
            (
                Value::BitVec(
                    comptime_runtime::bitvec::BitVec::from_words(
                        65,
                        &[0x12345678, 0xfedcba98, 1],
                        &mut construction,
                    )
                    .unwrap(),
                ),
                P::BitVec(65),
            ),
        ];
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
        for (value, domain) in values {
            let literal = value
                .freeze_literal(&mut Budget::default())
                .unwrap()
                .unwrap();
            let expression = Expr {
                source: SourceRange { start: 0, end: 0 },
                ty: Term::Primitive(domain),
                kind: ExprKind::Literal(literal),
            };
            let Flow::Value(restored) = vm.eval(&expression, &mut Default::default()).unwrap()
            else {
                panic!("literal control transfer")
            };
            assert_eq!(restored, value);
        }
    }

    #[test]
    fn variable_payload_copy_obeys_current_budget_and_can_retry() {
        let bits = Value::BitVec(
            comptime_runtime::bitvec::BitVec::from_words(65, &[1, 2, 1], &mut Budget::default())
                .unwrap(),
        );
        for value in [Value::String("a long payload".into()), bits.clone()] {
            let mut fuel = Budget {
                steps: 1,
                ..Default::default()
            };
            assert_eq!(value.freeze_literal(&mut fuel), Err(Error::BudgetExhausted));
            assert_eq!(fuel.steps, 0);
            assert!(
                value
                    .freeze_literal(&mut Budget::default())
                    .unwrap()
                    .is_some()
            );
        }
        let mut small = Budget {
            max_limbs: 2,
            ..Default::default()
        };
        assert_eq!(bits.freeze_literal(&mut small), Err(Error::SizeLimit));
        assert!(
            bits.freeze_literal(&mut Budget::default())
                .unwrap()
                .is_some()
        );
        assert_eq!(
            Value::Unit.freeze_literal(&mut Budget::default()).unwrap(),
            None
        );
    }
}
