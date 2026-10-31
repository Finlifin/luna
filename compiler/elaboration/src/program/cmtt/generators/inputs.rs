//! Shared source-ordered input preparation for contextual code and data calls.
use super::*;

impl Checker<'_, '_> {
    #[inline(never)]
    pub(in crate::program::cmtt) fn prepare_contextual_inputs<
        O: nir::code_interfaces::SignatureOutput + Clone,
    >(
        &mut self,
        owner: OwnerId,
        template: Generator<O>,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected_type: Option<Term>,
        output_shape: impl Fn(&O) -> Term,
        result_context: Option<&crate::calls::InputContext>,
        packets: &mut spreads::SpreadPlan<'_>,
        prepared_types: Option<GeneratorTypePlan>,
    ) -> Result<(Generator<O>, Vec<Option<Argument>>), Error> {
        let explicit = match callee.kind {
            H::NFApplication(_, types) => Some(types),
            _ => None,
        };
        let (ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
            &self.package.item(owner).unwrap().kind
        else {
            unreachable!()
        };
        let (mut routes, _) = route_arguments(
            arguments,
            Some(signature.params),
            template.inputs.len(),
            site.span,
        )?;
        for route in &mut routes {
            if matches!(
                signature.params[route.parameter].ty.kind,
                TyParamKind::Varadic(..)
            ) {
                route.expand_literal_packet(&mut self.required_steps)?;
            }
        }
        let (mut parameters, mut type_plan, pending, deferred_clauses) =
            if let Some(plan) = prepared_types {
                plan
            } else if let Some(explicit) = explicit {
                (
                    vec![],
                    self.type_clause_arguments(owner, explicit, None, site.span)?,
                    vec![],
                    vec![],
                )
            } else {
                self.generator_clause_plan(owner, site.span)?
            };
        // Ordinary Type values have their own inference slots, but are never
        // inferred as supplied values. Their evaluated identities constrain
        // these slots later, at their original source positions.
        parameters.extend(
            template
                .input_binders
                .values()
                .map(|binder| binder.declaration),
        );
        let planned = self.instantiate_code_generator(&template, &type_plan, site.span)?;
        // Previously completed argument interfaces are safe inference evidence:
        // reading their types neither runs a generator nor reads runtime storage.
        let mut evidence = vec![None; planned.inputs.len()];
        for route in &routes {
            if route.is_collected() {
                continue;
            }
            let expression = route.expression(site);
            let id = match expression.kind {
                H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => Some(local(id)),
                _ => None,
            };
            if let Some(id) = id {
                let terms: Vec<&Term> = match &planned.inputs[route.parameter].1 {
                    Input::Code(_) => self
                        .code_values
                        .get(&id)
                        .map(|code| {
                            std::iter::once(&code.result)
                                .chain(code.context.values())
                                .collect()
                        })
                        .unwrap_or_default(),
                    Input::Pattern(_) => self
                        .pattern_codes
                        .get(&id)
                        .map(|code| {
                            std::iter::once(&code.subject)
                                .chain(code.outputs.values().map(|(_, ty)| ty))
                                .collect()
                        })
                        .unwrap_or_default(),
                    Input::Data(_) => self
                        .locals
                        .get(&id)
                        .or_else(|| packets.field_type(id))
                        .into_iter()
                        .collect(),
                };
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let checked = terms
                    .iter()
                    .try_for_each(|ty| nir::type_computations::charge_term(ty, &mut fuel));
                self.required_steps = fuel as u64;
                checked.map_err(|e| error(site.span, e.to_string()))?;
                if !terms.is_empty() {
                    evidence[route.parameter] = Some(match &planned.inputs[route.parameter].1 {
                        Input::Code(_) | Input::Pattern(_) => {
                            Term::Tuple(terms.into_iter().cloned().collect())
                        }
                        Input::Data(_) => terms[0].clone(),
                    });
                }
            }
        }
        // A producer's established result is evidence even when its value is
        // not available yet. Inspecting the signature must not evaluate any
        // argument, default, contract, or producer body out of source order.
        for route in &routes {
            if route.is_collected()
                || evidence[route.parameter].is_some()
                || !matches!(planned.inputs[route.parameter].1, Input::Data(_))
            {
                continue;
            }
            let expression = route.expression(site);
            evidence[route.parameter] = self.generator_expression_type_hint(&expression, 0)?;
        }
        let inference_signature = Term::Function {
            parameters: planned
                .inputs
                .iter()
                .map(|(_, input)| Parameter {
                    sort: Sort::Runtime,
                    ty: match input {
                        Input::Data(ty) => ty.clone(),
                        Input::Pattern(interface) => Term::Tuple(
                            std::iter::once(interface.subject.clone())
                                .chain(interface.outputs.values().cloned())
                                .collect(),
                        ),
                        Input::Code(interface) => Term::Tuple(
                            std::iter::once(interface.result.clone())
                                .chain(interface.context.values().cloned())
                                .collect(),
                        ),
                    },
                })
                .collect(),
            result: Box::new(output_shape(&planned.output)),
        };
        let scheme = crate::calls::Scheme {
            parameter_sorts: self
                .input_binders(&parameters)
                .iter()
                .map(|p| p.sort)
                .collect(),
            type_parameters: parameters.len(),
            signature: inference_signature
                .abstract_parameters(&self.input_binders(&parameters))
                .map_err(|e| error(site.span, e.to_string()))?,
        };
        let mut partial = vec![crate::calls::InputContext::Unknown; planned.inputs.len()];
        for route in &routes {
            let Input::Data(ty) = &planned.inputs[route.parameter].1 else {
                continue;
            };
            if !parameters.iter().any(|id| ty.mentions_rigid(*id)) {
                continue;
            }
            let expression = route.expression(site);
            if matches!(expression.kind, H::Tuple(_) | H::CollectedProperties(_)) {
                partial[route.parameter] = self.generator_partial_input_hint(
                    &expression,
                    Some(ty),
                    &parameters,
                    packets,
                    0,
                )?;
            }
        }
        let mut completed = clauses::CompletedDefaults::default();
        let mut input_contexts = vec![crate::calls::InputContext::Unknown; planned.inputs.len()];
        let mut hinted = if parameters.is_empty() {
            None
        } else {
            let candidate = self.generator_call_candidate_with_result_context(
                owner,
                &parameters,
                &mut type_plan,
                &pending,
                &deferred_clauses,
                &mut completed,
                &scheme,
                &evidence,
                &partial,
                expected_type.as_ref(),
                result_context,
                site.span,
            )?;
            input_contexts = (0..planned.inputs.len())
                .map(|index| {
                    candidate
                        .parameter_input_context(index)
                        .map_err(|e| call_inference_error(site.span, e))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Some(self.generator_inference_view(
                &template,
                &parameters,
                &type_plan,
                &candidate,
                site.span,
            )?)
        };
        // Intrinsically typed literals and complete collections constrain a data slot before
        // an earlier tactic callback is checked. This probe never invokes a
        // producer, and numeric defaults remain disabled. Established target
        // contexts take precedence over unconstrained literal types.
        if !parameters.is_empty() {
            let mut learned = false;
            for route in &routes {
                self.required_steps = self
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(site.span, "generator input evidence budget exhausted"))?;
                if evidence[route.parameter].is_some() {
                    continue;
                }
                let Input::Data(Term::Rigid {
                    declaration,
                    sort: Sort::Type,
                }) = &hinted.as_ref().unwrap_or(&planned).inputs[route.parameter].1
                else {
                    continue;
                };
                if !parameters.contains(declaration) {
                    continue;
                }
                let expression = route.expression(site);
                let hint = if route.is_collected() {
                    let mut fields = Vec::new();
                    let mut complete = true;
                    for (_, input) in route.source_inputs() {
                        let packet_type = match input.kind {
                            H::ResolvedLocal(_, id) => packets.field_type(local(id)),
                            _ => None,
                        };
                        let ty = if let Some(ty) = packet_type {
                            Some(self.copy_alias_type(ty, input.span)?)
                        } else {
                            self.generator_intrinsic_input_hint(input, 0)?
                        };
                        let Some(ty) = ty else {
                            complete = false;
                            break;
                        };
                        fields.push(ty);
                    }
                    if complete {
                        Some(self.tuple_value_type(fields, site.span)?)
                    } else {
                        None
                    }
                } else {
                    self.generator_intrinsic_input_hint(&expression, 0)?
                };
                if let Some(ty) = hint {
                    evidence[route.parameter] = Some(ty);
                    learned = true;
                }
            }
            if learned {
                let candidate = self.generator_call_candidate_with_result_context(
                    owner,
                    &parameters,
                    &mut type_plan,
                    &pending,
                    &deferred_clauses,
                    &mut completed,
                    &scheme,
                    &evidence,
                    &partial,
                    expected_type.as_ref(),
                    result_context,
                    site.span,
                )?;
                input_contexts = (0..planned.inputs.len())
                    .map(|index| {
                        candidate
                            .parameter_input_context(index)
                            .map_err(|e| call_inference_error(site.span, e))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                hinted = Some(self.generator_inference_view(
                    &template,
                    &parameters,
                    &type_plan,
                    &candidate,
                    site.span,
                )?);
            }
        }
        let mut values: Vec<Option<Argument>> = std::iter::repeat_with(|| None)
            .take(template.inputs.len())
            .collect();
        let mut actual = vec![None; template.inputs.len()];
        let mut events = Vec::new();
        let mut collected: Vec<Vec<(Argument, Term)>> = std::iter::repeat_with(Vec::new)
            .take(routes.len())
            .collect();
        let mut collection_types = vec![None; routes.len()];
        let properties = routes
            .iter()
            .map(|route| {
                let expression = route.expression(site);
                if let H::CollectedProperties(fields) = expression.kind {
                    Some(fields)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for (route_index, route) in routes.iter().enumerate() {
            if let Some(fields) = properties[route_index] {
                for (element, field) in fields.iter().enumerate() {
                    events.push((field.expr.span.lo().0, route_index, element, field.expr));
                }
                continue;
            }
            if route.is_collected() {
                let Input::Data(ty) =
                    &hinted.as_ref().unwrap_or(&planned).inputs[route.parameter].1
                else {
                    return Err(error(
                        site.span,
                        "code collection requires code-container evidence",
                    ));
                };
                if !parameters.iter().any(|id| ty.mentions_rigid(*id)) {
                    route.require_collection_type(self, ty, site.span)?;
                    let Term::Tuple(fields) = self.structural_type(ty, site.span)? else {
                        unreachable!()
                    };
                    collection_types[route_index] = Some(fields);
                }
            }
            for (element, (_, argument)) in route.source_inputs().enumerate() {
                events.push((argument.span.lo().0, route_index, element, argument));
            }
        }
        for (_, packet) in &packets.empty {
            events.push((packets.source_start(*packet), usize::MAX, *packet, site));
        }
        events.sort_by_key(|(position, _, _, _)| *position);
        for (_, route_index, element, argument) in events {
            if route_index == usize::MAX {
                self.prepare_generator_packet(packets, element, None)?;
                continue;
            }
            let route = &routes[route_index];
            let adapted = signature.params[route.parameter]
                .ty
                .is_lambda()
                .then(|| route.expression(site));
            let argument = adapted.as_ref().unwrap_or(argument);
            let (_, input) = &hinted.as_ref().unwrap_or(&planned).inputs[route.parameter];
            if route.is_collected() && collection_types[route_index].is_none() {
                if let Input::Data(ty) = input {
                    if !parameters.iter().any(|id| ty.mentions_rigid(*id)) {
                        route.require_collection_type(self, ty, site.span)?;
                        let Term::Tuple(fields) = self.structural_type(ty, site.span)? else {
                            unreachable!()
                        };
                        collection_types[route_index] = Some(fields);
                    }
                }
            }
            let (value, ty) = match input {
                Input::Pattern(interface) => {
                    if packets.fields.contains_key(&local(argument.hir_id)) {
                        return Err(error(
                            argument.span,
                            "data packet cannot supply pattern code evidence",
                        ));
                    }
                    let subject = (!parameters
                        .iter()
                        .any(|id| interface.subject.mentions_rigid(*id)))
                    .then_some(&interface.subject);
                    let partial = match &input_contexts[route.parameter] {
                        context @ (crate::calls::InputContext::Tuple(_)
                        | crate::calls::InputContext::Known(_)) => Some(context),
                        _ => None,
                    };
                    let code = self.pattern_input_value(argument, interface, subject, partial)?;
                    let ty = Term::Tuple(
                        std::iter::once(code.subject.clone())
                            .chain(code.outputs.values().map(|(_, ty)| ty.clone()))
                            .collect(),
                    );
                    (Argument::Code(CodeValue::Pattern(code)), ty)
                }
                Input::Code(interface) => {
                    if packets.fields.contains_key(&local(argument.hir_id)) {
                        return Err(error(
                            argument.span,
                            "data packet cannot supply code evidence",
                        ));
                    }
                    let known = !parameters
                        .iter()
                        .any(|id| interface.result.mentions_rigid(*id));
                    let mut expected = interface.clone();
                    expected.infer_context = true;
                    let partial_result = match &input_contexts[route.parameter] {
                        crate::calls::InputContext::Tuple(fields) => fields.first(),
                        _ => None,
                    };
                    let code = self.code_value_in_context(
                        argument,
                        known.then_some(&expected),
                        partial_result,
                        0,
                    )?;
                    let ty = Term::Tuple(
                        std::iter::once(code.result.clone())
                            .chain(code.context.values().cloned())
                            .collect(),
                    );
                    (Argument::Code(CodeValue::Expression(code)), ty)
                }
                Input::Data(ty) => {
                    let property_context = if let Some(fields) = properties[route_index] {
                        match self.structural_type(ty, site.span)? {
                            Term::Record(types) => types
                                .get(element)
                                .filter(|(name, _)| name == fields[element].ident.name.as_str())
                                .map(|(_, ty)| ty.clone()),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let expected = if properties[route_index].is_some() {
                        property_context.as_ref()
                    } else if route.is_collected() {
                        collection_types[route_index]
                            .as_ref()
                            .and_then(|fields| fields.get(element))
                    } else {
                        (!parameters.iter().any(|id| ty.mentions_rigid(*id))).then_some(ty)
                    };
                    let context = if expected.is_some() {
                        None
                    } else {
                        use crate::calls::InputContext as C;
                        match (
                            &input_contexts[route.parameter],
                            route.is_collected(),
                            properties[route_index],
                        ) {
                            (C::Tuple(fields) | C::NormalTuple { fields, .. }, true, _) => {
                                fields.get(element)
                            }
                            (C::Record(fields), _, Some(properties)) => fields
                                .get(element)
                                .filter(|(name, _)| name == properties[element].ident.name.as_str())
                                .map(|(_, context)| context),
                            (context, false, None) => Some(context),
                            _ => None,
                        }
                    };
                    if let Some((packet, element)) =
                        packets.fields.get(&local(argument.hir_id)).copied()
                    {
                        let context = self.generator_packet_context(
                            packets,
                            packet,
                            &routes,
                            hinted.as_ref().unwrap_or(&planned),
                            &parameters,
                            site.span,
                        )?;
                        self.prepare_generator_packet(packets, packet, context.as_ref())?;
                        self.generator_packet_field(
                            packets,
                            packet,
                            element,
                            expected,
                            argument.span,
                        )?
                    } else if template.input_binders.contains_key(&route.parameter) {
                        let value = self.generator_required_type(argument)?;
                        (
                            Argument::Data(Some(n::Expr {
                                source: source(argument.span),
                                ty: p(P::Type),
                                kind: K::TypeValue(value),
                            })),
                            p(P::Type),
                        )
                    } else if self.checking_code_generator {
                        let value = self.check_generator_data_input(argument, expected, context)?;
                        (Argument::Data(None), value.ty)
                    } else {
                        let (value, ty) = if matches!(
                            signature.params[route.parameter].ty.kind,
                            TyParamKind::Itself { is_ref: false }
                        ) {
                            self.generator_receiver_argument(argument, expected, context)?
                        } else {
                            self.generator_data_expression_in_context(argument, expected, context)?
                        };
                        (
                            if self.generator_value_needs_live_storage(&value, argument.span)? {
                                Argument::LiveData(value)
                            } else {
                                Argument::Data(Some(self.materialize_generator_data(
                                    value,
                                    ty.clone(),
                                    argument.span,
                                )?))
                            },
                            ty,
                        )
                    }
                }
            };
            if route.is_collected() || properties[route_index].is_some() {
                if !parameters.is_empty() {
                    use crate::calls::InputContext as C;
                    // Publish only this completed field's type. Later fields keep
                    // their provisional contexts and their original source positions.
                    if !matches!(
                        partial[route.parameter],
                        C::Tuple(_) | C::NormalTuple { .. } | C::Record(_)
                    ) {
                        partial[route.parameter] = if let Some(fields) = properties[route_index] {
                            C::Record(
                                fields
                                    .iter()
                                    .map(|field| (field.ident.name.to_string(), C::Unknown))
                                    .collect(),
                            )
                        } else {
                            let fields = vec![C::Unknown; route.source_inputs().count()];
                            match self.protocols.tuple {
                                Some(head) => C::NormalTuple {
                                    head,
                                    fields,
                                    list_domain: self.bound_tuple_list_domain(),
                                },
                                None => C::Tuple(fields),
                            }
                        };
                    }
                    let field = match &mut partial[route.parameter] {
                        C::Tuple(fields) | C::NormalTuple { fields, .. } => &mut fields[element],
                        C::Record(fields) => &mut fields[element].1,
                        _ => unreachable!(),
                    };
                    *field = C::Known(ty.clone());
                }
                collected[route_index].push((value, ty));
            } else {
                if let Some(binder) = template.input_binders.get(&route.parameter) {
                    let Argument::Data(Some(n::Expr {
                        kind: K::TypeValue(value),
                        ..
                    })) = &value
                    else {
                        return Err(error(
                            argument.span,
                            "dependent contextual input requires a completed Type value",
                        ));
                    };
                    completed.inputs.insert(
                        binder.declaration,
                        self.copy_alias_type(value, argument.span)?,
                    );
                }
                values[route.parameter] = Some(value);
                actual[route.parameter] = Some(ty.clone());
                evidence[route.parameter] = Some(ty);
            }
            if !parameters.is_empty() {
                let candidate = self.generator_call_candidate_with_result_context(
                    owner,
                    &parameters,
                    &mut type_plan,
                    &pending,
                    &deferred_clauses,
                    &mut completed,
                    &scheme,
                    &evidence,
                    &partial,
                    expected_type.as_ref(),
                    result_context,
                    site.span,
                )?;
                input_contexts = (0..planned.inputs.len())
                    .map(|index| {
                        candidate
                            .parameter_input_context(index)
                            .map_err(|e| call_inference_error(site.span, e))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                hinted = Some(self.generator_inference_view(
                    &template,
                    &parameters,
                    &type_plan,
                    &candidate,
                    site.span,
                )?);
            }
        }
        for (route_index, route) in routes.iter().enumerate() {
            if !route.is_collected() && properties[route_index].is_none() {
                continue;
            }
            let elements = std::mem::take(&mut collected[route_index]);
            let ty = if let Some(fields) = properties[route_index] {
                Term::Record(
                    fields
                        .iter()
                        .zip(&elements)
                        .map(|(field, (_, ty))| (field.ident.name.to_string(), ty.clone()))
                        .collect(),
                )
            } else {
                self.tuple_value_type(
                    elements.iter().map(|(_, ty)| ty.clone()).collect(),
                    site.span,
                )?
            };
            let live = elements
                .iter()
                .any(|(value, _)| matches!(value, Argument::LiveData(_)));
            let value = if self.checking_code_generator {
                Argument::Data(None)
            } else if live {
                // Rehydrate only already-frozen siblings. Source expressions have
                // been evaluated once, in source order, above.
                let mut fields = Vec::with_capacity(elements.len());
                for (value, _) in elements {
                    fields.push(match value {
                        Argument::LiveData(value) => value,
                        Argument::Data(Some(value)) => {
                            self.required_normal_input(site, value, vec![])?.0
                        }
                        _ => {
                            return Err(error(
                                site.span,
                                "generator collection requires completed data",
                            ));
                        }
                    });
                }
                let value = if let Some(properties) = properties[route_index] {
                    comptime_interpretor::Value::AnonymousRecord(
                        properties
                            .iter()
                            .zip(fields)
                            .map(|(field, value)| (field.ident.name.to_string(), value))
                            .collect(),
                    )
                } else {
                    comptime_interpretor::Value::Tuple(fields)
                };
                self.generator_value_needs_live_storage(&value, site.span)?;
                Argument::LiveData(value)
            } else {
                let fields = elements
                    .into_iter()
                    .map(|(value, _)| match value {
                        Argument::Data(Some(value)) => Ok(value),
                        _ => Err(error(
                            site.span,
                            "generator collection requires completed data",
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Argument::Data(Some(n::Expr {
                    ty: ty.clone(),
                    source: source(site.span),
                    kind: if properties[route_index].is_some() {
                        K::AnonymousRecord { fields }
                    } else {
                        K::Tuple(fields)
                    },
                }))
            };
            values[route.parameter] = Some(value);
            actual[route.parameter] = Some(ty);
        }
        let mut prepared_defaults = BTreeSet::new();
        if let Some(last) = template
            .input_binders
            .keys()
            .copied()
            .filter(|slot| values[*slot].is_none())
            .max()
        {
            let candidate = self.generator_call_candidate_with_result_context(
                owner,
                &parameters,
                &mut type_plan,
                &pending,
                &deferred_clauses,
                &mut completed,
                &scheme,
                &actual,
                &partial,
                expected_type.as_ref(),
                result_context,
                site.span,
            )?;
            let view = self.generator_inference_view(
                &template,
                &parameters,
                &type_plan,
                &candidate,
                site.span,
            )?;
            // Clause hints may type-check prefix data, but ordinary Type
            // inputs must still come from their supplied or default values.
            let types = view
                .types
                .iter()
                .map(|(_, ty)| ty.clone())
                .collect::<Vec<_>>();
            let definition = self.instantiate_code_generator(&template, &types, site.span)?;
            prepared_defaults = self.complete_generator_type_defaults(
                owner,
                definition,
                last,
                &mut values,
                &mut actual,
                &mut completed.inputs,
                site.span,
            )?;
        }
        let types = if parameters.is_empty() {
            type_plan
        } else {
            // This tuple is an internal inference product, never a Flurry Tuple
            // constructor or an erased code value. Context evidence stays in values.
            let mut candidate = self.generator_call_candidate_with_result_context(
                owner,
                &parameters,
                &mut type_plan,
                &pending,
                &deferred_clauses,
                &mut completed,
                &scheme,
                &actual,
                &partial,
                expected_type.as_ref(),
                result_context,
                site.span,
            )?;
            if !self.settle_function_defaults(
                owner,
                &parameters,
                &type_plan,
                &pending,
                &mut completed.types,
                &mut candidate,
                site.span,
            )? {
                return Err(error(
                    site.span,
                    "code generator type defaults remain unresolved",
                ));
            }
            let all = self.type_parameters[&owner].clone();
            for &slot in &deferred_clauses {
                if self.checking_code_generator {
                    // Definition checking retains abstract clause inputs. A
                    // real invocation validates defaults before emitting code.
                    let index = parameters.iter().position(|id| *id == all[slot]).unwrap();
                    if candidate
                        .type_argument(index)
                        .map_err(|e| call_inference_error(site.span, e))?
                        .is_none()
                    {
                        candidate
                            .constrain_type_argument(
                                index,
                                &Term::Rigid {
                                    declaration: all[slot],
                                    sort: candidate
                                        .type_argument_sort(index)
                                        .map_err(|e| call_inference_error(site.span, e))?,
                                },
                                crate::ptc::Budget::default(),
                            )
                            .map_err(|e| call_inference_error(site.span, e))?;
                    }
                    continue;
                }
                if !completed.clauses.contains_key(&slot) {
                    return Err(error(
                        site.span,
                        "code generator clause defaults remain unresolved",
                    ));
                }
            }
            let instance = candidate
                .finish()
                .map_err(|e| call_inference_error(site.span, e))?;
            let mut types = type_plan
                .iter()
                .map(|ty| {
                    let binders = self.input_binders(&parameters);
                    self.instantiate_generator_type(
                        ty,
                        &binders,
                        &instance.type_arguments,
                        site.span,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            for (slot, value) in completed.clauses {
                types[slot] = value;
            }
            types
        };
        // Only the callee's contract sees its captured environment. Caller
        // argument evaluation above must keep the caller's own instantiation.
        let saved_captures = template
            .captures
            .iter()
            .map(|(id, value)| {
                (
                    *id,
                    self.alias_substitution.insert(*id, Arc::new(value.clone())),
                )
            })
            .collect::<Vec<_>>();
        let formation = self.check_type_application(owner, &types, site.span);
        for (id, saved) in saved_captures {
            match saved {
                Some(saved) => {
                    self.alias_substitution.insert(id, saved);
                }
                None => {
                    self.alias_substitution.remove(&id);
                }
            }
        }
        formation?;
        let mut definition = self.instantiate_code_generator(&template, &types, site.span)?;
        definition.prepared_defaults = prepared_defaults;
        for (&slot, binder) in &template.input_binders {
            let value = completed
                .inputs
                .get(&binder.declaration)
                .ok_or_else(|| error(site.span, "contextual Type input was not completed"))?;
            self.specialize_generator_input(&mut definition, slot, value, site.span)?;
        }
        for ((value, actual), (_, input)) in values.iter_mut().zip(&actual).zip(&definition.inputs)
        {
            match (value, input) {
                (Some(Argument::Code(CodeValue::Pattern(code))), Input::Pattern(interface)) => {
                    self.ascribe_pattern_input(code, interface, site.span)?;
                }
                (Some(Argument::Code(CodeValue::Expression(code))), Input::Code(interface)) => {
                    *code = self.ascribe_code(code.clone(), Some(interface), site.span)?
                }
                (Some(Argument::Data(_) | Argument::LiveData(_)), Input::Data(ty)) => {
                    self.require(actual.as_ref().unwrap(), ty, site.span)?
                }
                (None, _) => {}
                _ => unreachable!("generator input category changed during type instantiation"),
            }
        }
        Ok((definition, values))
    }
}
