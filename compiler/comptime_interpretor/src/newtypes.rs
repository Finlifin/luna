//! Newtype conversions transfer a whole payload, never expose a record field.
use super::*;
impl Interpreter<'_> {
    #[inline(never)]
    pub(super) fn eval_newtype_convert(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::NewtypeConvert(operand) = &expr.kind else {
            unreachable!()
        };
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let plan = if let Some(graph) = self.graph {
            // IIR descriptors are already instantiated, so their nominal keys
            // must select the concrete representation rather than reapplying
            // declaration arguments to an empty telescope.
            graph.newtype_conversion(&operand.ty, &expr.ty, &mut fuel)
        } else {
            nir::newtypes::conversion_from(
                self.newtypes.values().copied(),
                &operand.ty,
                &expr.ty,
                &[],
                &mut fuel,
            )
        };
        self.budget.steps -= (before - fuel) as u64;
        let plan = plan
            .map_err(|e| {
                self.failure(
                    match e {
                        nir::newtypes::ConversionError::Budget => ErrorKind::StepsExhausted,
                        _ => ErrorKind::Type,
                    },
                    Some(expr.source),
                )
            })?
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
        let value = value!(self.eval(operand, environment));
        if !self.has_type(&value, &operand.ty) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let value = match plan.direction() {
            nir::newtypes::Direction::Wrap => Value::Newtype {
                ty: expr.ty.clone(),
                value: Box::new(value),
            },
            nir::newtypes::Direction::Unwrap => {
                let Value::Newtype { ty, value } = value else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                if ty != operand.ty {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                *value
            }
        };
        if !self.has_type(&value, &expr.ty) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        Ok(Flow::Value(value))
    }
}
