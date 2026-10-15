//! Check routable inputs once and retain their NIR until source-order staging.
//! Packet width hints establish routing only; packet expressions are still
//! checked against the inferred context before any call is published.
use super::*;

pub(in crate::program) type PreparedInputs = BTreeMap<usize, (hir::HirId, n::Expr)>;

impl Checker<'_, '_> {
    fn spread_candidate(
        &self,
        scheme: &crate::calls::Scheme,
        actual: &[Option<Term>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        structural: Option<&crate::calls::InputContext>,
        span: Span,
    ) -> Result<crate::calls::CallCandidate, Error> {
        let mut candidate = scheme
            .infer_scoped_types_mode(
                actual,
                expected,
                partial,
                &self.context,
                crate::ptc::Budget::default(),
                false,
                &self.call_inference_scope(),
            )
            .map_err(|cause| call_inference_error(span, cause))?;
        if let Some(context) = structural {
            candidate
                .constrain_result_context(context, crate::ptc::Budget::default())
                .map_err(|cause| call_inference_error(span, cause))?;
        }
        candidate
            .refine_type_views(
                |ty, budget| {
                    self.callable_input_view_budgeted(ty, budget, span)
                        .map_err(|e| e.message)
                },
                crate::ptc::Budget::default(),
            )
            .map_err(|cause| call_inference_error(span, cause))?;
        Ok(candidate)
    }

    /// Settle packet evidence before defaulting dependent peer expressions.
    pub(in crate::program) fn infer_enum_spread_inputs(
        &mut self,
        scheme: &crate::calls::Scheme,
        width: usize,
        args: &[Arg<'_>],
        context: &crate::calls::InputContext,
        span: Span,
        checked: &mut PreparedInputs,
        packets: &mut BTreeMap<usize, n::Expr>,
    ) -> Result<crate::calls::CallCandidate, Error> {
        let slots = (0..width).collect::<Vec<_>>();
        let mut widths = BTreeMap::new();
        let mut packet_fields = BTreeMap::<usize, Vec<Term>>::new();
        let mut actual = vec![None; width];
        let mut fuel = 10_000usize;
        loop {
            let layout = self.spread_packet_layout_with_widths(args, &slots, span, &widths)?;
            for (index, fields) in &packet_fields {
                if let Some(slots) = &layout[*index] {
                    if slots.len() != fields.len() {
                        return Err(error(span, "spread packet width disagrees with payload"));
                    }
                    for (slot, ty) in slots.iter().zip(fields) {
                        actual[*slot] = Some(ty.clone());
                    }
                }
            }
            for (slot, (_, value)) in checked.iter() {
                if !is_never(&value.ty) {
                    actual[*slot] = Some(value.ty.clone());
                }
            }
            let candidate = self.infer_spread_inputs(
                scheme,
                actual.clone(),
                args,
                None,
                0,
                None,
                None,
                Some(context),
                Some(&widths),
                span,
                checked,
            )?;
            let mut progress = false;
            for (index, arg) in args.iter().enumerate() {
                let Arg::Expand(input) = arg else {
                    continue;
                };
                if matches!(input.kind, H::Tuple(_)) || packets.contains_key(&index) {
                    continue;
                }
                let slots = &layout[index];
                fuel = fuel
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "spread packet inference budget exhausted"))?;
                let packet_context = if let Some(slots) = slots {
                    let fields = slots
                        .iter()
                        .map(|slot| {
                            candidate
                                .parameter_input_context(*slot)
                                .map_err(|e| call_inference_error(span, e))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    match self.protocols.tuple {
                        Some(head) => crate::calls::InputContext::NormalTuple { head, fields },
                        None => crate::calls::InputContext::Tuple(fields),
                    }
                } else {
                    crate::calls::InputContext::Unknown
                };
                let Some(value) = self.argument_attempt_with_input(
                    input,
                    None,
                    None,
                    false,
                    Some(&packet_context),
                )?
                else {
                    continue;
                };
                if is_never(&value.ty) {
                    packets.insert(index, value);
                    progress = true;
                    break;
                }
                let Term::Tuple(fields) = self.structural_type(&value.ty, input.span)? else {
                    return Err(error(
                        input.span,
                        "positional spread requires a tuple value",
                    ));
                };
                if slots
                    .as_ref()
                    .is_some_and(|slots| fields.len() != slots.len())
                {
                    return Err(error(
                        input.span,
                        "argument count mismatch: spread packet width disagrees with payload",
                    ));
                }
                widths.insert(index, fields.len());
                packet_fields.insert(index, fields);
                packets.insert(index, value);
                progress = true;
                break;
            }
            if !progress {
                return Ok(candidate);
            }
        }
    }

    pub(in crate::program) fn infer_spread_inputs(
        &mut self,
        scheme: &crate::calls::Scheme,
        mut actual: Vec<Option<Term>>,
        args: &[Arg<'_>],
        declarations: Option<&[hir::common::FnParam<'_>]>,
        offset: usize,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        structural: Option<&crate::calls::InputContext>,
        settled_widths: Option<&BTreeMap<usize, usize>>,
        span: Span,
        checked: &mut PreparedInputs,
    ) -> Result<crate::calls::CallCandidate, Error> {
        let mut flat = Vec::new();
        let mut post_positions = std::collections::BTreeSet::new();
        let mut fuel = 10_000usize;
        let layout = if structural.is_some() {
            Some(self.spread_packet_layout_with_widths(
                args,
                &actual[offset..],
                span,
                settled_widths.unwrap_or(&BTreeMap::new()),
            )?)
        } else {
            None
        };
        for (argument_index, arg) in args.iter().enumerate() {
            match arg {
                Arg::Positional(value) => flat.push((None, (*value).clone(), true)),
                Arg::PostLambda(value) => {
                    post_positions.insert(flat.len());
                    flat.push((None, (*value).clone(), true));
                }
                Arg::Named(name, value) => flat.push((Some(name.clone()), (*value).clone(), true)),
                Arg::Expand(value) => {
                    if let H::Tuple(items) = value.kind {
                        flat.extend(items.iter().cloned().map(|item| (None, item, true)));
                    } else {
                        let known_width = match settled_widths
                            .and_then(|widths| widths.get(&argument_index).copied())
                        {
                            Some(width) => Some(width),
                            None => self.spread_width(value, &mut fuel, 0)?,
                        };
                        let Some(width) = known_width.or_else(|| {
                            layout
                                .as_ref()
                                .and_then(|layout| layout[argument_index].as_ref().map(Vec::len))
                        }) else {
                            // Keep the established result-context path when a
                            // partition is not yet known; never invent a width.
                            return self.spread_candidate(
                                scheme, &actual, expected, partial, structural, span,
                            );
                        };
                        fuel = fuel
                            .checked_sub(width)
                            .ok_or_else(|| error(span, "spread input routing budget exhausted"))?;
                        for _ in 0..width {
                            flat.push((
                                None,
                                Expr {
                                    hir_id: value.hir_id,
                                    span: value.span,
                                    kind: H::TyVoid,
                                },
                                false,
                            ));
                        }
                    }
                }
                Arg::Implicit(_) => {
                    return Err(error(span, "implicit argument routing is not implemented"));
                }
            }
        }
        let routed: Vec<_> = flat
            .iter()
            .enumerate()
            .map(|(index, (name, value, _))| match name {
                _ if post_positions.contains(&index) => Arg::PostLambda(value),
                Some(name) => Arg::Named(name.clone(), value),
                None => Arg::Positional(value),
            })
            .collect();
        let (routes, _) = route_arguments(&routed, declarations, actual.len() - offset, span)?;
        loop {
            let candidate =
                self.spread_candidate(scheme, &actual, expected, partial, structural, span)?;
            let mut progress = false;
            for route in &routes {
                let Some(position) = route.single_position() else {
                    continue;
                };
                let (_, input, available) = &flat[position];
                if !available || checked.contains_key(&position) {
                    continue;
                }
                fuel = fuel
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "spread input inference budget exhausted"))?;
                let parameter = route.parameter + offset;
                let context = candidate
                    .parameter_type(parameter)
                    .map_err(|e| call_inference_error(span, e))?;
                let partial = if context.is_none() {
                    candidate
                        .parameter_callable_context(parameter)
                        .map_err(|e| call_inference_error(span, e))?
                } else {
                    None
                };
                let input_context = candidate
                    .parameter_input_context(parameter)
                    .map_err(|e| call_inference_error(span, e))?;
                if let Some(value) =
                    self.with_staged_input(
                        matches!(scheme.signature.callable_signature(),Term::Function {parameters,..} if parameters[parameter].sort!=Sort::Runtime),
                        |this|this.argument_attempt_with_input(&route.expression(input), context.as_ref(), partial.as_ref(), false, Some(&input_context)),
                    )?
                {
                    if !is_never(&value.ty) {
                        actual[parameter] = Some(value.ty.clone());
                    }
                    checked.insert(position, (input.hir_id, value));
                    progress = true;
                }
            }
            if progress {
                continue;
            }
            // Unchecked packets may still constrain numeric literals. Leave
            // defaulting to complete call inference after packet preparation.
            return Ok(candidate);
        }
    }
}
