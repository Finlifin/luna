//! Optional construction preserves the whole input, including inner absence.
use super::*;

impl Interpreter<'_> {
    pub(super) fn eval_optional(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let Term::Optional(payload_ty) = &expr.ty else {
                return Err(this.failure(ErrorKind::Type, Some(expr.source)));
            };
            this.admit_type(&expr.ty, 0)?;
            let value = match &expr.kind {
                K::OptionalAbsent => None,
                K::OptionalPresent(operand) => {
                    if operand.ty != **payload_ty {
                        return Err(this.failure(ErrorKind::Type, Some(operand.source)));
                    }
                    let value = value!(this.eval(operand, environment));
                    if !this.has_type(&value, payload_ty) {
                        return Err(this.failure(ErrorKind::Type, Some(operand.source)));
                    }
                    this.retain_prepared(0, operand.source, &value)?;
                    Some(Box::new(value))
                }
                _ => unreachable!(),
            };
            Ok(Flow::Value(Value::Optional {
                ty: expr.ty.clone(),
                value,
            }))
        })
    }
}
