//! Error-result tags and payload facts belong to program flow, not PTC.
use super::*;
use nir::error_results::{Conversion, Error as ConversionError};

fn conversion_failure(error: ConversionError) -> Reason {
    match error {
        ConversionError::Budget => {
            Reason::Unknown(Unknown::from(comptime_runtime::Error::BudgetExhausted))
        }
        _ => Reason::InvalidSignature,
    }
}

impl Walker<'_> {
    pub(super) fn error_payload_type<'t>(
        &mut self,
        ty: &'t Term,
        tag: usize,
    ) -> Result<&'t Term, Reason> {
        self.limits.step(1)?;
        let Term::ErrorResult { errors, result } = ty else {
            return Err(Reason::InvalidSignature);
        };
        if tag == 0 {
            Ok(result)
        } else {
            errors.get(tag - 1).ok_or(Reason::InvalidSignature)
        }
    }

    pub(super) fn error_pattern_tag(
        &mut self,
        ty: &Term,
        label: Option<&Term>,
    ) -> Result<usize, Reason> {
        let Term::ErrorResult { errors, .. } = ty else {
            return Err(Reason::InvalidSignature);
        };
        let Some(label) = label else {
            return Ok(0);
        };
        for (index, candidate) in errors.iter().enumerate() {
            nir::type_computations::charge_term(candidate, &mut self.limits.arithmetic.steps)
                .map_err(|_| conversion_failure(ConversionError::Budget))?;
            if candidate == label {
                return Ok(index + 1);
            }
        }
        Err(Reason::InvalidSignature)
    }

    pub(super) fn error_condition(
        &mut self,
        input: &Value,
        ty: &Term,
        selected: usize,
        state: &mut State,
    ) -> Result<Option<Condition>, Reason> {
        self.error_payload_type(ty, selected)?;
        match input.aggregate_kind {
            Some(AggregateKind::ErrorResult(actual)) => {
                Ok(Some(Condition::Bool(actual == selected)))
            }
            None => {
                let Term::ErrorResult { errors, .. } = ty else {
                    unreachable!()
                };
                let mut tag = Value {
                    range: Some(Interval::new(
                        Integer::from_u128(0),
                        Integer::from_u128(errors.len() as u128),
                        self.limits,
                    )?),
                    ..Value::default()
                };
                self.project_component(
                    state,
                    &mut tag,
                    input.projection.clone(),
                    (AggregateKind::ErrorTag, 0),
                )?;
                Ok(match tag.scalar {
                    Some(Scalar::Local(value)) => Some(Condition::Compare {
                        value,
                        relation: C::Equal,
                        constant: Integer::from_u128(selected as u128),
                    }),
                    _ => None,
                })
            }
            _ => Err(Reason::InvalidSignature),
        }
    }

    pub(super) fn error_conversion(
        &mut self,
        expr: &Expr,
        conversion: Conversion,
        operand: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        let package = self.package;
        nir::error_results::validate_conversion_with_layouts(
            conversion,
            &operand.ty,
            &expr.ty,
            &mut self.limits.arithmetic.steps,
            &mut |a, b| a == b,
            &|ty| {
                package?
                    .normal_forms
                    .iter()
                    .find_map(|head| head.tuple_elements(ty))
            },
        )
        .map_err(conversion_failure)?;
        let mut value = self.child(operand, state, depth)?;
        if !state.facts.is_reachable() {
            return Ok(Value::default());
        }
        match conversion {
            Conversion::Success | Conversion::Failure => {
                let tag = self.error_pattern_tag(
                    &expr.ty,
                    (conversion == Conversion::Failure).then_some(&operand.ty),
                )?;
                value.evidence = self.prepare_copy(&value, state)?;
                let result = Value {
                    fields: Some(vec![value.snapshot(self.limits, 0)?]),
                    nominal_type: Some(std::sync::Arc::new(expr.ty.clone())),
                    aggregate_kind: Some(AggregateKind::ErrorResult(tag)),
                    ..Value::default()
                };
                self.materialize_aggregate(result, &expr.ty, state)
            }
            Conversion::Embed | Conversion::Product => {
                if operand.ty == expr.ty {
                    return Ok(value);
                }
                // Keep known payload snapshots when widening a concrete branch.
                // Unknown tags and product transports lose relational precision;
                // never reuse source tag/path identities under another layout.
                if conversion == Conversion::Embed {
                    if let Some(AggregateKind::ErrorResult(source_tag)) = value.aggregate_kind {
                        let label = if source_tag == 0 {
                            None
                        } else {
                            Some(self.error_payload_type(&operand.ty, source_tag)?.clone())
                        };
                        let target_tag = self.error_pattern_tag(&expr.ty, label.as_ref())?;
                        value.clear_evidence(self.limits)?;
                        value.projection = None;
                        value.nominal_type = Some(std::sync::Arc::new(expr.ty.clone()));
                        value.aggregate_kind = Some(AggregateKind::ErrorResult(target_tag));
                        return self.materialize_aggregate(value, &expr.ty, state);
                    }
                }
                self.unknown_value(&expr.ty)
            }
        }
    }
}
