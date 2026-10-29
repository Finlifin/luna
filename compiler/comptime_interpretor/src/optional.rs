//! Optional construction preserves the whole input, including inner absence.
use super::*;

impl Interpreter<'_> {
    pub(super) fn prepare_optional(
        &mut self,
        ty: &Term,
        input: Option<(&Term, SourceRange)>,
        source: SourceRange,
    ) -> Result<(), Error> {
        let Term::Optional(payload_ty) = ty else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        self.admit_type(ty, 0)?;
        if let Some((input, at)) = input {
            if input != payload_ty.as_ref() {
                return Err(self.failure(ErrorKind::Type, Some(at)));
            }
        }
        Ok(())
    }

    pub(super) fn complete_optional(
        &mut self,
        ty: &Term,
        value: Option<Value>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let Term::Optional(payload_ty) = ty else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        if value
            .as_ref()
            .is_some_and(|value| !self.has_type(value, payload_ty))
        {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(Value::Optional {
            ty: ty.clone(),
            value: value.map(Box::new),
        })
    }

    pub(super) fn eval_optional(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let operand = match &expr.kind {
                K::OptionalAbsent => None,
                K::OptionalPresent(operand) => Some(operand.as_ref()),
                _ => unreachable!(),
            };
            this.prepare_optional(&expr.ty, operand.map(|v| (&v.ty, v.source)), expr.source)?;
            let value = match operand {
                Some(operand) => Some(value!(this.eval(operand, environment))),
                None => None,
            };
            let at = operand.map_or(expr.source, |v| v.source);
            let result = this.complete_optional(&expr.ty, value, at)?;
            if let Value::Optional {
                value: Some(value), ..
            } = &result
            {
                this.retain_prepared(0, at, value)?;
            }
            Ok(Flow::Value(result))
        })
    }
}
