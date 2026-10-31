//! Shared exact and machine floating-point operations, independent of carrier.
use super::*;
use comptime_compiler::scalar::Scalar;
use nir::program::FloatOp;

impl Interpreter<'_> {
    pub(super) fn prepare_float(
        &self,
        scalar: Scalar<'_>,
        op: Option<FloatOp>,
        source: SourceRange,
    ) -> Result<(), Error> {
        let never = Term::Primitive(Primitive::Never);
        let supported = |ty: &Term| {
            matches!(
                ty,
                Term::Primitive(
                    Primitive::F16
                        | Primitive::Bf16
                        | Primitive::F32
                        | Primitive::F64
                        | Primitive::Never
                )
            ) || (op != Some(FloatOp::Divide) && *ty == Term::Primitive(Primitive::Float))
        };
        let expected = if *scalar.left == never || scalar.right.is_some_and(|ty| *ty == never) {
            &never
        } else {
            scalar.left
        };
        if scalar.right.is_some() != op.is_some()
            || scalar.result != expected
            || !supported(scalar.left)
            || scalar.right.is_some_and(|ty| {
                !supported(ty) || (*scalar.left != never && *ty != never && ty != scalar.left)
            })
        {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(())
    }

    pub(super) fn float_values(
        &mut self,
        scalar: Scalar<'_>,
        op: Option<FloatOp>,
        a: Value,
        b: Option<Value>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if !self.has_type(&a, scalar.left)
            || b.as_ref()
                .zip(scalar.right)
                .is_some_and(|(value, ty)| !self.has_type(value, ty))
        {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        if let Some(op) = op {
            let b = b.ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
            Ok(match (a, b) {
                (Value::Float(a), Value::Float(b)) => {
                    Value::Float(self.integer_operation(Some(source), |budget| match op {
                        nir::program::FloatOp::Add => a.add(&b, budget),
                        nir::program::FloatOp::Subtract => a.subtract(&b, budget),
                        nir::program::FloatOp::Multiply => a.multiply(&b, budget),
                        nir::program::FloatOp::Divide => {
                            unreachable!("unsupported exact division")
                        }
                    })?)
                }
                (Value::Float16(a), Value::Float16(b)) => {
                    Value::Float16(self.integer_operation(Some(source), |budget| {
                        op.evaluate_half(
                            comptime_runtime::float16::Float16Format::Binary16,
                            a,
                            b,
                            budget,
                        )
                    })?)
                }
                (Value::BFloat16(a), Value::BFloat16(b)) => {
                    Value::BFloat16(self.integer_operation(Some(source), |budget| {
                        op.evaluate_half(
                            comptime_runtime::float16::Float16Format::BFloat16,
                            a,
                            b,
                            budget,
                        )
                    })?)
                }
                (Value::Float32(a), Value::Float32(b)) => Value::Float32(op.evaluate_f32(a, b)),
                (Value::Float64(a), Value::Float64(b)) => Value::Float64(op.evaluate_f64(a, b)),
                _ => return Err(self.failure(ErrorKind::Type, Some(source))),
            })
        } else {
            Ok(match a {
                // Negation flips the sign even for zero and NaN, without
                // arithmetic rounding or payload canonicalization.
                Value::Float(value) => Value::Float(
                    self.integer_operation(Some(source), |budget| value.negate(budget))?,
                ),
                Value::Float16(bits) => Value::Float16(bits ^ 0x8000),
                Value::BFloat16(bits) => Value::BFloat16(bits ^ 0x8000),
                Value::Float32(bits) => Value::Float32(bits ^ (1 << 31)),
                Value::Float64(bits) => Value::Float64(bits ^ (1 << 63)),
                _ => return Err(self.failure(ErrorKind::Type, Some(source))),
            })
        }
    }
}
