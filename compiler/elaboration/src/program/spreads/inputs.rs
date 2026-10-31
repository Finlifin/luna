//! Check routable inputs once and retain their NIR until source-order staging.
//! Packet width hints establish routing only; packet expressions are still
//! checked against the inferred context before any call is published.
use super::*;

pub(in crate::program) type PreparedInputs = BTreeMap<usize, (hir::HirId, n::Expr)>;

impl Checker<'_, '_> {
    fn spread_candidate(
        &self,
        scheme: &crate::calls::Scheme,
        declaration_inputs: &[DeclarationId],
        actual: &[Option<Term>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        structural: Option<&crate::calls::InputContext>,
        span: Span,
    ) -> Result<crate::calls::CallCandidate, Error> {
        let domains = declaration_inputs
            .iter()
            .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
            .collect::<Vec<_>>();
        let mut candidate = scheme
            .infer_declared_sparse_types_mode(
                declaration_inputs,
                &domains,
                actual,
                expected,
                partial,
                &self.context,
                crate::ptc::Budget::default(),
                self.active_effect_environment().is_some()
                    && !matches!(expected, Some(Term::Computation { .. })),
                &self.call_inference_domains(),
            )
            .map_err(|cause| call_inference_error(span, cause))?;
        if let Some(context) = structural {
            candidate
                .constrain_result_context(context, crate::ptc::Budget::default())
                .map_err(|cause| call_inference_error(span, cause))?;
        }
        candidate
            .refine_type_views_metered(
                |ty, budget| {
                    self.callable_input_view_metered(ty, budget, span)
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
        self.infer_packet_inputs(
            scheme,
            &[],
            width,
            args,
            None,
            Some(context),
            span,
            checked,
            packets,
        )
    }

    /// Alternate packet and peer evidence without discarding partially known
    /// callable/aggregate domains. Checked producers are retained for delivery;
    /// inference order never changes their runtime evaluation order.
    pub(in crate::program) fn infer_packet_inputs(
        &mut self,
        scheme: &crate::calls::Scheme,
        declaration_inputs: &[DeclarationId],
        width: usize,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        context: Option<&crate::calls::InputContext>,
        span: Span,
        checked: &mut PreparedInputs,
        packets: &mut BTreeMap<usize, n::Expr>,
    ) -> Result<crate::calls::CallCandidate, Error> {
        self.infer_routed_packet_inputs(
            scheme,
            declaration_inputs,
            vec![None; width],
            args,
            None,
            0,
            expected,
            None,
            context,
            &vec![false; width],
            span,
            checked,
            packets,
            false,
        )
    }

    /// Feed checked packet fields through declaration routing, including
    /// receivers and collected parameters. Only real producers enter NIR;
    /// type evidence remains separate from the retained operand cache.
    pub(in crate::program) fn infer_routed_packet_inputs(
        &mut self,
        scheme: &crate::calls::Scheme,
        declaration_inputs: &[DeclarationId],
        actual: Vec<Option<Term>>,
        args: &[Arg<'_>],
        declarations: Option<&[hir::common::FnParam<'_>]>,
        offset: usize,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        structural: Option<&crate::calls::InputContext>,
        stages: &[bool],
        span: Span,
        checked: &mut PreparedInputs,
        packets: &mut BTreeMap<usize, n::Expr>,
        defer_staged: bool,
    ) -> Result<crate::calls::CallCandidate, Error> {
        let mut widths = BTreeMap::new();
        let mut fields = BTreeMap::new();
        let mut fuel = 10_000usize;
        loop {
            let candidate = self.infer_spread_evidence(
                scheme,
                declaration_inputs,
                actual.clone(),
                args,
                declarations,
                offset,
                expected,
                partial,
                structural,
                Some(&widths),
                span,
                checked,
                defer_staged,
                &fields,
            )?;
            let types = (offset..actual.len())
                .map(|slot| {
                    candidate
                        .parameter_input_context(slot)
                        .map_err(|cause| call_inference_error(span, cause))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let positional =
                self.spread_parameter_input_context(types, declarations, stages, args, span)?;
            let layout = positional
                .as_ref()
                .map(|positional| {
                    self.spread_packet_layout_with_widths(args, positional, span, &widths)
                })
                .transpose()?;
            // A diverging packet has no payload type. Once live peers fix
            // the remaining layout, retain that destination width for routing
            // the other operands; never invent a payload type for Never.
            let mut settled = false;
            if let Some(layout) = &layout {
                for (index, packet) in packets.iter() {
                    if is_never(&packet.ty)
                        && !widths.contains_key(index)
                        && let Some(fields) = &layout[*index]
                    {
                        widths.insert(*index, fields.len());
                        settled = true;
                    }
                }
            }
            if settled {
                continue;
            }
            let mut progress = false;
            for (index, arg) in args.iter().enumerate() {
                let Arg::Expand(input) = arg else { continue };
                if matches!(input.kind, H::Tuple(_)) || packets.contains_key(&index) {
                    continue;
                }
                let context = layout.as_ref().and_then(|layout| layout[index].as_ref());
                // An opaque packet crossing a stage boundary must be handled
                // by the existing dependent/staged preparation machinery.
                if context.map_or(stages.iter().any(|stage| *stage), |fields| {
                    fields.iter().any(|(_, stage)| *stage)
                }) {
                    continue;
                }
                fuel = fuel
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "spread packet inference budget exhausted"))?;
                let packet_context = context.map_or(Input::Unknown, |fields| {
                    let fields = fields.iter().map(|(ty, _)| ty.clone()).collect();
                    match self.protocols.tuple {
                        Some(head) => Input::NormalTuple {
                            head,
                            fields,
                            list_domain: self.bound_tuple_list_domain(),
                        },
                        None => Input::Tuple(fields),
                    }
                });
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
                if !is_never(&value.ty) {
                    let Term::Tuple(payload) = self.structural_type(&value.ty, input.span)? else {
                        return Err(error(
                            input.span,
                            "type mismatch: positional spread requires a tuple value",
                        ));
                    };
                    if context.is_some_and(|fields| fields.len() != payload.len()) {
                        return Err(error(
                            input.span,
                            "argument count mismatch: spread packet width disagrees with payload",
                        ));
                    }
                    widths.insert(index, payload.len());
                    fields.insert(index, payload);
                }
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
        actual: Vec<Option<Term>>,
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
        self.infer_spread_evidence(
            scheme,
            &[],
            actual,
            args,
            declarations,
            offset,
            expected,
            partial,
            structural,
            settled_widths,
            span,
            checked,
            false,
            &BTreeMap::new(),
        )
    }

    fn infer_spread_evidence(
        &mut self,
        scheme: &crate::calls::Scheme,
        declaration_inputs: &[DeclarationId],
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
        defer_staged: bool,
        packet_fields: &BTreeMap<usize, Vec<Term>>,
    ) -> Result<crate::calls::CallCandidate, Error> {
        let mut evidence = BTreeMap::new();
        let mut flat = Vec::new();
        let mut post_positions = std::collections::BTreeSet::new();
        let mut fuel = 10_000usize;
        // A declaration slot is not necessarily one positional operand.
        // Collections, omitted/defaulted inputs and post-lambdas need the
        // declaration router; do not use telescope length as a packet width.
        let fixed_positions = declarations.is_none_or(|parameters| {
            parameters.iter().all(|parameter| {
                !parameter.ty.is_implicit()
                    && matches!(
                        parameter.ty.kind,
                        TyParamKind::Positional(_) | TyParamKind::PositionalDependencyCatched(..)
                    )
            })
        }) && !args
            .iter()
            .any(|arg| matches!(arg, Arg::Named(..) | Arg::PostLambda(_)));
        let layout = if structural.is_some() && fixed_positions {
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
                                scheme,
                                declaration_inputs,
                                &actual,
                                expected,
                                partial,
                                structural,
                                span,
                            );
                        };
                        fuel = fuel
                            .checked_sub(width)
                            .ok_or_else(|| error(span, "spread input routing budget exhausted"))?;
                        for field in 0..width {
                            if let Some(ty) = packet_fields
                                .get(&argument_index)
                                .and_then(|fields| fields.get(field))
                            {
                                evidence.insert(flat.len(), ty.clone());
                            }
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
        for route in &routes {
            if !route.is_collected() {
                if let Some(ty) = route.single_position().and_then(|position| {
                    checked
                        .get(&position)
                        .and_then(|(_, value)| arguments::inference_type(value))
                        .or_else(|| evidence.get(&position))
                }) {
                    actual[route.parameter + offset] = Some(ty.clone());
                }
            }
        }
        loop {
            let mut candidate = self.spread_candidate(
                scheme,
                declaration_inputs,
                &actual,
                expected,
                partial,
                structural,
                span,
            )?;
            // Retained collection elements provide partial tuple evidence.
            // Unknown packet fields stay candidate-local inference variables.
            for route in routes.iter().filter(|route| route.is_collected()) {
                let fields = route
                    .source_inputs()
                    .map(|(position, _)| {
                        checked
                            .get(&position)
                            .and_then(|(_, value)| arguments::inference_type(value))
                            .or_else(|| evidence.get(&position))
                            .map_or(crate::calls::InputContext::Unknown, |ty| {
                                crate::calls::InputContext::Known(ty.clone())
                            })
                    })
                    .collect();
                let context = match self.protocols.tuple {
                    Some(head) => crate::calls::InputContext::NormalTuple {
                        head,
                        fields,
                        list_domain: self.bound_tuple_list_domain(),
                    },
                    None => crate::calls::InputContext::Tuple(fields),
                };
                candidate
                    .constrain_parameter_context(
                        route.parameter + offset,
                        &context,
                        crate::ptc::Budget::default(),
                    )
                    .map_err(|e| call_inference_error(span, e))?;
            }
            // Collection evidence may settle an associated type's subject
            // after the initial candidate was formed. Normalize those views
            // before passing domains to callback and literal checking.
            if routes.iter().any(|route| route.is_collected()) {
                candidate
                    .refine_type_views_metered(
                        |ty, budget| {
                            self.callable_input_view_metered(ty, budget, span)
                                .map_err(|e| e.message)
                        },
                        crate::ptc::Budget::default(),
                    )
                    .map_err(|e| call_inference_error(span, e))?;
            }
            let mut progress = false;
            for route in &routes {
                if route.is_collected() {
                    let parameter = route.parameter + offset;
                    let staged = matches!(scheme.signature.callable_signature(), Term::Function { parameters, .. } if parameters[parameter].sort != Sort::Runtime);
                    if defer_staged && staged {
                        continue;
                    }
                    let context = candidate
                        .parameter_input_context(parameter)
                        .map_err(|e| call_inference_error(span, e))?;
                    let fields = match context {
                        crate::calls::InputContext::Known(ty) => {
                            route.require_collection_type(self, &ty, span)?;
                            let Term::Tuple(fields) = self.structural_type(&ty, span)? else {
                                unreachable!()
                            };
                            fields
                                .into_iter()
                                .map(crate::calls::InputContext::Known)
                                .collect::<Vec<_>>()
                        }
                        crate::calls::InputContext::Tuple(fields)
                        | crate::calls::InputContext::NormalTuple { fields, .. } => fields,
                        _ => continue,
                    };
                    if fields.len() != route.source_inputs().count() {
                        return Err(error(span, "variadic collection argument count mismatch"));
                    }
                    for ((position, input), field) in route.source_inputs().zip(&fields) {
                        if !flat[position].2 || checked.contains_key(&position) {
                            continue;
                        }
                        fuel = fuel.checked_sub(1).ok_or_else(|| {
                            error(span, "spread input inference budget exhausted")
                        })?;
                        let expected = match field {
                            crate::calls::InputContext::Known(ty) => Some(ty),
                            _ => None,
                        };
                        if let Some(value) = self.with_staged_input(staged, |this| {
                            this.argument_attempt_with_input(
                                input,
                                expected,
                                None,
                                false,
                                Some(field),
                            )
                        })? {
                            checked.insert(position, (input.hir_id, value));
                            progress = true;
                        }
                    }
                    continue;
                }
                let Some(position) = route.single_position() else {
                    unreachable!()
                };
                let (_, input, available) = &flat[position];
                if !available || checked.contains_key(&position) {
                    continue;
                }
                fuel = fuel
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "spread input inference budget exhausted"))?;
                let parameter = route.parameter + offset;
                if defer_staged
                    && matches!(scheme.signature.callable_signature(),
                    Term::Function { parameters, .. } if parameters[parameter].sort != Sort::Runtime)
                {
                    continue;
                }
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
                    if let Some(ty) = arguments::inference_type(&value) {
                        actual[parameter] = Some(ty.clone());
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
