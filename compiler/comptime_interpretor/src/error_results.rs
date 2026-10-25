//! Error results are tagged data; construction never captures a continuation.
use super::*;

impl Interpreter<'_> {
    pub(super) fn eval_error_convert(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            this.eval_error_convert_payload(expr, environment)
        })
    }

    fn apply_error_transport(
        &mut self,
        plan: &nir::error_results::RepresentationTransport,
        value: Value,
        source: SourceRange,
        depth: usize,
    ) -> Result<Value, Error> {
        use nir::error_results::RepresentationTransport as Transport;
        self.tick(Some(source))?;
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        match (plan, value) {
            (Transport::Identity, value) => Ok(value),
            (
                Transport::Error {
                    source: expected,
                    target,
                    tags,
                },
                Value::ErrorResult { ty, error, value },
            ) => {
                if &ty != expected {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let tag = error.map_or(0, |index| index + 1);
                let tag = *tags
                    .get(tag)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                Ok(Value::ErrorResult {
                    ty: target.clone(),
                    error: tag.checked_sub(1),
                    value,
                })
            }
            (Transport::Tuple(plans), Value::Tuple(values)) if plans.len() == values.len() => {
                let mut result = Vec::new();
                for (plan, value) in plans.iter().zip(values) {
                    result.push(self.apply_error_transport(plan, value, source, depth + 1)?);
                }
                Ok(Value::Tuple(result))
            }
            (Transport::Record(plans), Value::AnonymousRecord(values))
                if plans.len() == values.len() =>
            {
                let mut result = Vec::new();
                for ((expected, plan), (name, value)) in plans.iter().zip(values) {
                    if expected != &name {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    }
                    result.push((
                        name,
                        self.apply_error_transport(plan, value, source, depth + 1)?,
                    ));
                }
                Ok(Value::AnonymousRecord(result))
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }

    fn eval_error_convert_payload(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        use nir::error_results::Conversion;
        let K::ErrorConvert {
            conversion,
            value: operand,
        } = &expr.kind
        else {
            unreachable!()
        };
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let plan = nir::error_results::validate_conversion_with_tuple(
            *conversion,
            &operand.ty,
            &expr.ty,
            &mut fuel,
            &mut |left, right| left == right,
            &|head| {
                self.normal_forms
                    .get(&head)
                    .is_some_and(|m| m.role == nir::normal_forms::Role::Tuple)
            },
        );
        self.budget.steps -= (before - fuel) as u64;
        plan.map_err(|e| {
            self.failure(
                if e == nir::error_results::Error::Budget {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                Some(expr.source),
            )
        })?;
        self.admit_type(&expr.ty, 0)?;
        let payload = value!(self.eval(operand, environment));
        if !self.has_type(&payload, &operand.ty) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.retain_prepared(0, operand.source, &payload)?;
        if *conversion == Conversion::Product {
            let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let before = fuel;
            let plan = nir::error_results::transport_plan(
                &operand.ty,
                &expr.ty,
                true,
                &mut fuel,
                &mut |a, b| a == b,
                &|head| {
                    self.normal_forms
                        .get(&head)
                        .is_some_and(|m| m.role == nir::normal_forms::Role::Tuple)
                },
            );
            self.budget.steps -= (before - fuel) as u64;
            let plan = plan.map_err(|e| {
                self.failure(
                    if e == nir::error_results::Error::Budget {
                        ErrorKind::StepsExhausted
                    } else {
                        ErrorKind::Type
                    },
                    Some(expr.source),
                )
            })?;
            let result = self.apply_error_transport(&plan, payload, expr.source, 0)?;
            if !self.has_type(&result, &expr.ty) {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            }
            return Ok(Flow::Value(result));
        }
        let Term::ErrorResult { errors, .. } = &expr.ty else {
            unreachable!()
        };
        let (error, value) = match conversion {
            Conversion::Product => unreachable!(),
            Conversion::Success => (None, Box::new(payload)),
            Conversion::Failure => (
                Some(
                    errors
                        .iter()
                        .position(|label| label == &operand.ty)
                        .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?,
                ),
                Box::new(payload),
            ),
            Conversion::Embed => {
                let Value::ErrorResult { ty, error, value } = payload else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let Term::ErrorResult { errors: source, .. } = ty else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let error = match error {
                    None => None,
                    Some(index) => {
                        let label = source
                            .get(index)
                            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                        Some(
                            errors
                                .iter()
                                .position(|target| target == label)
                                .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?,
                        )
                    }
                };
                (error, value)
            }
        };
        let result = Value::ErrorResult {
            ty: expr.ty.clone(),
            error,
            value,
        };
        if !self.has_type(&result, &expr.ty) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        Ok(Flow::Value(result))
    }
}
