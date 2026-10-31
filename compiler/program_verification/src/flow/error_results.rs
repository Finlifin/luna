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
                let plan = nir::error_results::transport_plan_with_layouts(
                    &operand.ty,
                    &expr.ty,
                    conversion == Conversion::Product,
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
                let evidence = self
                    .prepare_copy(&value, state)?
                    .map(|copy| copy.transport(&plan, self.limits, 0))
                    .transpose()?;
                let mut snapshot = value.snapshot(self.limits, 0)?;
                snapshot.clear_evidence(self.limits)?;
                let mut mapped = self.transport_snapshot(snapshot, &expr.ty, &plan, 0)?;
                mapped.evidence = evidence;
                self.materialize_aggregate(mapped, &expr.ty, state)
            }
        }
    }
}

// A tag observation stores a number, whereas a payload path stores identity.
// Reordering a row can rename payload paths directly, but cannot reuse the
// old numeric tag as if it were already the destination tag.
pub(super) fn transport_component<'p>(
    plan: &'p nir::error_results::RepresentationTransport,
    component: Component,
    limits: &mut Limits,
) -> Result<Option<(Component, &'p nir::error_results::RepresentationTransport)>, Reason> {
    use nir::error_results::RepresentationTransport as Plan;
    const IDENTITY: Plan = Plan::Identity;
    limits.step(1)?;
    let (kind, slot) = component;
    Ok(Some(match plan {
        Plan::Identity => (component, &IDENTITY),
        Plan::Tuple(fields) if kind == AggregateKind::Tuple => {
            (component, fields.get(slot).ok_or(Reason::InvalidSignature)?)
        }
        Plan::Record(fields) if kind == AggregateKind::AnonymousRecord => (
            component,
            &fields.get(slot).ok_or(Reason::InvalidSignature)?.1,
        ),
        Plan::Error { tags, .. } => match kind {
            AggregateKind::ErrorResult(tag) if slot == 0 => {
                let mapped = *tags.get(tag).ok_or(Reason::InvalidSignature)?;
                ((AggregateKind::ErrorResult(mapped), slot), &IDENTITY)
            }
            AggregateKind::ErrorTag if slot == 0 => {
                limits.step(tags.len())?;
                if tags.iter().enumerate().any(|(a, b)| a != *b) {
                    return Ok(None);
                }
                (component, &IDENTITY)
            }
            _ => return Err(Reason::InvalidSignature),
        },
        _ => return Err(Reason::InvalidSignature),
    }))
}

impl Walker<'_> {
    fn transport_snapshot(
        &mut self,
        mut value: Value,
        target: &Term,
        plan: &nir::error_results::RepresentationTransport,
        depth: usize,
    ) -> Result<Value, Reason> {
        use nir::error_results::RepresentationTransport as Plan;
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        match plan {
            Plan::Identity => return Ok(value),
            Plan::Error { tags, .. } => {
                value.aggregate_kind = match value.aggregate_kind {
                    Some(AggregateKind::ErrorResult(tag)) => Some(AggregateKind::ErrorResult(
                        *tags.get(tag).ok_or(Reason::InvalidSignature)?,
                    )),
                    None => None,
                    _ => return Err(Reason::InvalidSignature),
                };
            }
            Plan::Tuple(plans) => {
                let types = self
                    .tuple_field_types(target)?
                    .ok_or(Reason::InvalidSignature)?;
                if types.len() != plans.len() {
                    return Err(Reason::InvalidSignature);
                }
                if let Some(fields) = value.fields.take() {
                    if fields.len() != plans.len() {
                        return Err(Reason::InvalidSignature);
                    }
                    value.fields = Some(
                        fields
                            .into_iter()
                            .zip(plans)
                            .zip(types)
                            .map(|((field, plan), ty)| {
                                self.transport_snapshot(field, &ty, plan, depth + 1)
                            })
                            .collect::<Result<_, _>>()?,
                    );
                }
                value.aggregate_kind = Some(AggregateKind::Tuple);
            }
            Plan::Record(plans) => {
                let Term::Record(types) = target else {
                    return Err(Reason::InvalidSignature);
                };
                if types.len() != plans.len() {
                    return Err(Reason::InvalidSignature);
                }
                if let Some(fields) = value.fields.take() {
                    if fields.len() != plans.len() {
                        return Err(Reason::InvalidSignature);
                    }
                    let mut mapped = Vec::with_capacity(fields.len());
                    for ((field, (name, plan)), (expected, ty)) in
                        fields.into_iter().zip(plans).zip(types)
                    {
                        self.limits.step(name.len())?;
                        if name != expected {
                            return Err(Reason::InvalidSignature);
                        }
                        mapped.push(self.transport_snapshot(field, ty, plan, depth + 1)?);
                    }
                    value.fields = Some(mapped);
                }
                value.aggregate_kind = Some(AggregateKind::AnonymousRecord);
            }
        }
        value.nominal_type = Some(std::sync::Arc::new(target.clone()));
        Ok(value)
    }
}
