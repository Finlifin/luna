//! Extended application shares ordinary positional routing, while colon
//! properties remain a distinct input protocol from named whole-value supply.
use super::*;

pub(super) mod layout;

impl Checker<'_, '_> {
    pub(super) fn extended_call(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<Option<n::Expr>, Error> {
        let head = if let H::NFApplication(head, _) = callee.kind {
            head
        } else {
            callee
        };
        if let H::ResolvedDef { def_id, .. } = head.kind {
            if self
                .package
                .definition_owner(def_id)
                .is_some_and(|owner| self.is_code_generator(owner))
            {
                return Err(error(
                    expr.span,
                    "code generator must be invoked in a comptime code position",
                ));
            }
        }
        let type_field = matches!(callee.kind, H::Projection(..))
            && self.local_field_domain(callee)? == Some(p(P::Type));
        let callable = match &head.kind {
            H::ResolvedDef { .. } if self.associated_reference_owner(head).is_some() => {
                let owner = self.associated_reference_owner(head).unwrap();
                let value = self.lexical_associated_value(owner, callee.span)?;
                let ty = self.structural_type(&value.ty, callee.span)?;
                matches!(
                    ty,
                    Term::Function { .. }
                        | Term::Closure { .. }
                        | Term::ComptimeFunction(_)
                        | Term::PureFunction(_)
                )
            }
            H::ResolvedDef { def_id, .. } => self.package.definition_owner(*def_id)
                .and_then(|owner| self.package.item(owner))
                .is_some_and(|item| matches!(&item.kind, ItemKind::Fn(sig, _) if !sig.modifiers.is_normal_form_head)),
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                if let Some(ty) = self.locals.get(&local(*id)) {
                    let ty = self.structural_type(ty, callee.span)?;
                    matches!(
                    ty,
                    Term::Function { .. }
                        | Term::Closure { .. }
                        | Term::ComptimeFunction(_)
                        | Term::PureFunction(_)
                )
                } else { false }
            }
            H::Closure(..) => true,
            H::Projection(..) if type_field => false,
            H::Projection(base, name) => {
                !self.is_type_head(base)
                    || self.static_method(base, name.name.as_str(), callee.span, callee.hir_id)?.is_some()
            },
            _ => false,
        };
        if !callable {
            if self.is_type_head(callee) {
                return Ok(None);
            }
            // A factory, conditional or constant can supply a callable just as
            // a local can. Retain the checked callee through routing: probing
            // its type must not elaborate or execute its producer twice.
            // Type-valued constants are consumed as types, without a runtime
            // Splice conversion. Function-valued constants still follow the
            // ordinary invocation phase rules below.
            if let H::ResolvedDef { def_id, .. } = callee.kind {
                if let Some(owner) = self.package.definition_owner(def_id) {
                    if matches!(
                        self.package.item(owner).map(|item| &item.kind),
                        Some(ItemKind::Const(..))
                    ) {
                        let value = if self.is_build_context(owner) {
                            self.build_context_value(callee, owner)?
                        } else {
                            self.constant_value(owner, callee.span)?
                        };
                        if let K::TypeValue(ty) = value.kind {
                            return self.construct_typed(expr, ty, args, expected).map(Some);
                        }
                    }
                }
            }
            // Definition checking determines which source operands form Types.
            // Acquire them in generator storage before reading any mutable
            // operand. The layout itself still belongs to this invocation.
            // A Type field supplies the constructor at comptime. Looking up
            // its record as runtime data here would require a spurious Splice.
            let constructed = if type_field
                && self.checking_code_generator
                && self.in_code_body
                && self.runtime_value_context()
            {
                let ty = self.generator_type_data(callee)?.identity(callee.span)?;
                self.record_generator_constructor_operand(callee);
                Some(ty)
            } else {
                self.generator_constructed_type(callee)?
            };
            if let Some(ty) = constructed {
                return self
                    .with_constructor_inputs(callee, ty, expr.span, |this, ty| {
                        this.construct_typed(expr, ty, args, expected)
                    })
                    .map(Some);
            }
            let value = self.invocation_callee(callee)?;
            let ty = self.structural_type(&value.ty, callee.span)?;
            if ty == p(P::Type) {
                if self.checking_code_generator && self.in_code_body && self.runtime_value_context()
                {
                    self.record_generator_constructor_operand(callee);
                    let ty = self
                        .generator_type_data_with(callee, |_| Ok(value))?
                        .identity(callee.span)?;
                    return self
                        .with_constructor_inputs(callee, ty, expr.span, |this, ty| {
                            this.construct_typed(expr, ty, args, expected)
                        })
                        .map(Some);
                }
                let prepared = self.prepare_required_expression(
                    callee,
                    Some(&p(P::Type)),
                    Some(value),
                    Vec::new(),
                    false,
                )?;
                let (comptime_interpretor::Value::Type(ty), _) = self.invoke_required_publishing(
                    prepared,
                    Vec::new(),
                    callee.hir_id.owner,
                    callee.span,
                )?
                else {
                    return Err(error(callee.span, "construction requires a Type value"));
                };
                return self.construct_typed(expr, ty, args, expected).map(Some);
            }
            if args.iter().any(|arg| matches!(arg, Arg::Named(..))) {
                return Err(error(
                    expr.span,
                    "attribute collection requires declaration-side protocol metadata",
                ));
            }
            // A value never recovers names, defaults or collection protocols
            // merely because constant folding exposed a named function.
            let value = n::Expr {
                source: value.source,
                ty: value.ty.clone(),
                kind: K::Block {
                    statements: vec![],
                    result: Some(Box::new(value)),
                },
            };
            if let Some(flat) = arguments::literal_spread_arguments(args) {
                return self
                    .finish_call(expr, value, &flat, Vec::new(), expected, partial)
                    .map(Some);
            }
            if value.ty.is_comptime_callable()
                && self.protocols.splice.is_some()
                && self.runtime_value_context()
            {
                let result = self.with_staged_input(true, |this| {
                    this.spread_call_resolved(
                        expr,
                        callee,
                        args,
                        None,
                        None,
                        Some((value, Vec::new())),
                        None,
                    )
                })?;
                return self
                    .deliver_comptime_result(expr, result, expected)
                    .map(Some);
            }
            return self
                .spread_call_resolved(
                    expr,
                    callee,
                    args,
                    expected,
                    partial,
                    Some((value, Vec::new())),
                    None,
                )
                .map(Some);
        }
        if args.iter().any(|arg| matches!(arg, Arg::Named(..))) {
            return self
                .collect_properties(expr, callee, head, args, expected, partial)
                .map(Some);
        }
        self.call(expr, callee, args, expected, partial).map(Some)
    }

    fn collect_properties(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        head: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        // An open declaration is instantiated by the invocation path, after
        // routing its runtime inputs. It cannot first become an ordinary
        // function value just because the call uses explicit type clauses.
        let direct_open = if let H::ResolvedDef { def_id, .. } = head.kind {
            if let Some(owner) = self.package.definition_owner(def_id) {
                self.ensure_signature(owner, callee.span)?;
                self.open_functions.contains_key(&owner)
            } else {
                false
            }
        } else {
            false
        };
        let resolved = if direct_open {
            None
        } else if let Some(method) = self.resolve_receiver(expr, callee)? {
            Some(method)
        } else if let H::NFApplication(base, types) = callee.kind {
            // Retain the completed instance for both context propagation and
            // final invocation. Type-producing arguments must not be replayed.
            Some((
                self.explicit_function_instance(callee, base, types, None, true)?,
                Vec::new(),
            ))
        } else if matches!(head.kind, H::ResolvedDef { .. }) {
            None
        } else {
            Some((self.invocation_callee(callee)?, Vec::new()))
        };
        let owner = if let H::ResolvedDef { def_id, .. } = head.kind {
            self.package.definition_owner(def_id)
        } else {
            resolved.as_ref().and_then(|(method, _)| {
                let id = match method.kind {
                    K::Function(id)
                    | K::FunctionInstance {
                        declaration: id, ..
                    } => id,
                    K::TraitMethod { member, .. } => member,
                    _ => return None,
                };
                self.source_declarations.owner(id)
            })
        }
        .ok_or_else(|| {
            error(
                expr.span,
                "attribute collection requires declaration-side protocol metadata",
            )
        })?;
        if direct_open {
            if let H::NFApplication(_, clauses) = callee.kind {
                return self.with_explicit_open_clauses(owner, clauses, expr, |this| {
                    this.collect_properties_resolved(
                        expr, callee, args, expected, partial, None, owner, true,
                    )
                });
            }
        }
        let comptime = self.package.item(owner).is_some_and(|item|
            matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) if sig.modifiers.is_comptime));
        if comptime && self.protocols.splice.is_some() && self.runtime_value_context() {
            let value = self.with_staged_input(true, |this| {
                this.collect_properties_resolved(
                    expr, callee, args, None, None, resolved, owner, false,
                )
            })?;
            return self.deliver_comptime_result(expr, value, expected);
        }
        self.collect_properties_resolved(
            expr, callee, args, expected, partial, resolved, owner, false,
        )
    }

    fn collect_properties_resolved(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        resolved: Option<(n::Expr, Vec<n::Expr>)>,
        owner: OwnerId,
        open_instantiated: bool,
    ) -> Result<n::Expr, Error> {
        self.prepare_call_inputs(expr, |this| {
            this.collect_properties_in_preparation(
                expr,
                callee,
                args,
                expected,
                partial,
                resolved,
                owner,
                open_instantiated,
            )
        })
    }

    fn collect_properties_in_preparation(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        resolved: Option<(n::Expr, Vec<n::Expr>)>,
        owner: OwnerId,
        open_instantiated: bool,
    ) -> Result<n::Expr, Error> {
        let Some(ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            return Err(error(
                expr.span,
                "attribute collection requires a function declaration",
            ));
        };
        let collectors: Vec<_> = signature
            .params
            .iter()
            .filter(|parameter| matches!(parameter.ty.kind, TyParamKind::Varadic(..)))
            .map(|parameter| (parameter.name, parameter.index as usize))
            .collect();
        let (name, parameter_index) = self.property_collector(
            owner,
            &collectors,
            resolved.as_ref().map(|(callee, _)| callee),
            expected,
            partial,
            expr.span,
        )?;
        let mut properties = Vec::new();
        let mut names = BTreeSet::new();
        for argument in args {
            match argument {
                Arg::Named(name, value) => {
                    if !names.insert(name.name.to_string()) {
                        return Err(error(name.span, "property supplied more than once"));
                    }
                    properties.push(hir::expr::FieldExpr {
                        ident: *name,
                        expr: value,
                        span: value.span,
                    });
                }
                Arg::Positional(_) | Arg::PostLambda(_) | Arg::Expand(_) => (),
                _ => {
                    return Err(error(
                        expr.span,
                        "implicit property inputs require an implicit argument protocol",
                    ));
                }
            }
        }
        let packet = Expr {
            hir_id: expr.hir_id,
            span: expr.span.shrink_to_lo(),
            kind: H::CollectedProperties(&properties),
        };
        let mut inputs = Vec::new();
        let mut supplied_properties = false;
        for input in args {
            match input {
                Arg::Positional(value) => inputs.push(Arg::Positional(*value)),
                Arg::PostLambda(value) => inputs.push(Arg::PostLambda(*value)),
                Arg::Expand(value) => inputs.push(Arg::Expand(*value)),
                Arg::Named(..) if !supplied_properties => {
                    inputs.push(Arg::Named(name, &packet));
                    supplied_properties = true;
                }
                Arg::Named(..) => (),
                _ => unreachable!(),
            }
        }
        if args.iter().any(|arg| matches!(arg, Arg::Expand(_))) {
            return self.prepare_property_spreads(
                expr,
                callee,
                args,
                &inputs,
                expected,
                partial,
                resolved.as_ref(),
                |this, flat, diverges| {
                    this.collect_properties_resolved(
                        expr,
                        callee,
                        flat,
                        expected.filter(|_| !diverges),
                        partial.filter(|_| !diverges),
                        resolved.clone(),
                        owner,
                        open_instantiated,
                    )
                },
            );
        }
        let receiver_count = resolved.as_ref().map_or(0, |(_, prefix)| prefix.len());
        let mut call = if let Some((method, prefix)) = resolved {
            self.finish_call(expr, method, &inputs, prefix, expected, partial)?
        } else if open_instantiated {
            self.instantiated_open_call(expr, owner, &inputs, expected)?
        } else {
            self.call(expr, callee, &inputs, expected, partial)?
        };
        if matches!(call.kind, K::CallPreparation { .. }) {
            return self.merge_property_preparation(expr, args, call, parameter_index);
        }
        let anchor = arguments::preparation_anchor(expr.span);
        let K::Call {
            callee, arguments, ..
        } = &mut call.kind
        else {
            return Err(error(
                expr.span,
                "property preparation requires a direct normalized call",
            ));
        };
        let slot = arguments
            .iter()
            .position(|argument| argument.parameter == parameter_index)
            .ok_or_else(|| error(expr.span, "property packet was not routed to its parameter"))?;
        let binding = self.spread_binding(expr.span)?;
        let local = n::Expr {
            ty: arguments[slot].value.ty.clone(),
            source: anchor,
            kind: K::Local(binding),
        };
        let mut packet = std::mem::replace(&mut arguments[slot].value, local);
        layout::unprepare_packet(&mut packet, expr.span)?;
        let field_positions = layout::field_positions(&packet.ty, expr.span)?;
        let K::AnonymousRecord { fields } = &mut packet.kind else {
            return Err(error(
                expr.span,
                "property collection did not produce a record packet",
            ));
        };
        let mut statements = Vec::new();
        statements.extend(self.prepare_call_callee(callee, expr.span)?);
        for receiver in arguments.iter_mut().take(receiver_count) {
            if let Some(operand) =
                arguments::operand_to_prepare(&mut receiver.value, receiver.passing)
            {
                statements.push(self.prepare_call_operand(operand, expr.span)?);
            }
        }
        let positional_slots: Vec<_> = arguments
            .iter()
            .enumerate()
            .filter(|(_, argument)| {
                !argument.is_default
                    && argument.parameter >= receiver_count
                    && argument.parameter != parameter_index
            })
            .map(|(slot, _)| slot)
            .collect();
        let mut positional_slots = positional_slots.into_iter();
        // Argument inference may check these in a different order. Runtime
        // evaluation always uses the original producer order, exactly once.
        let mut offsets = Vec::with_capacity(args.len() + 1);
        for input in args {
            offsets.push(statements.len());
            let value = match input {
                Arg::Named(name, _) => Some(&mut fields[field_positions[name.name.as_str()]]),
                Arg::Positional(_) | Arg::PostLambda(_) => {
                    let arg = &mut arguments[positional_slots.next().expect("one anchored input")];
                    arguments::operand_to_prepare(&mut arg.value, arg.passing)
                }
                _ => unreachable!(),
            };
            if let Some(value) = value {
                statements.push(self.prepare_call_operand(value, expr.span)?);
            }
        }
        offsets.push(statements.len());
        self.preparing_inputs.as_mut().unwrap().offsets = offsets;
        statements.push(n::Expr {
            ty: p(P::Void),
            source: anchor,
            kind: K::Let {
                binding,
                value: Box::new(packet),
            },
        });
        let diverges = statements.iter().any(|statement| is_never(&statement.ty));
        Ok(n::Expr {
            ty: if diverges {
                p(P::Never)
            } else {
                call.ty.clone()
            },
            source: source(expr.span),
            kind: K::CallPreparation {
                statements,
                result: Some(Box::new(call)),
            },
        })
    }

    pub(super) fn property_collector(
        &mut self,
        owner: OwnerId,
        collectors: &[(hir::common::Ident, usize)],
        resolved: Option<&n::Expr>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        span: Span,
    ) -> Result<(hir::common::Ident, usize), Error> {
        if let [collector] = collectors {
            return Ok(*collector);
        }
        self.ensure_signature(owner, span)?;
        if let Some(definition) = self.open_functions.get(&owner) {
            // Open signatures need not have an entry in the ordinary signature
            // table. Classify only known domains; deferred domains remain
            // candidates rather than being evaluated ahead of their inputs.
            let domains = definition.input_domains();
            let candidates = collectors
                .iter()
                .map(|&(name, index)| {
                    let ty = domains[index]
                        .1
                        .as_ref()
                        .map(|ty| self.specialize_open_clause_type(owner, ty, span))
                        .transpose()?;
                    Ok((name, index, ty))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            return self.select_property_collector(candidates, span);
        }
        let signature = resolved
            .map(|value| value.ty.clone())
            .unwrap_or_else(|| self.signatures[&owner].clone());
        let Term::Function { parameters, .. } = signature.callable_signature() else {
            unreachable!()
        };
        let fixed = match resolved.map(|value| &value.kind) {
            Some(K::FunctionInstance { arguments, .. }) => arguments.len(),
            Some(K::TraitMethod {
                interface_arguments,
                method_arguments,
                ..
            }) => 1 + interface_arguments.len() + method_arguments.len(),
            Some(_) => self.type_parameters[&owner].len(),
            None => 0,
        };
        let remaining = &self.type_parameters[&owner][fixed..];
        let inferred = if !remaining.is_empty() {
            let scheme = crate::calls::Scheme {
                parameter_sorts: self
                    .input_binders(remaining)
                    .iter()
                    .map(|input| input.sort)
                    .collect(),
                type_parameters: remaining.len(),
                signature: signature
                    .callable_signature()
                    .abstract_parameters(&self.input_binders(remaining))
                    .map_err(|cause| error(span, cause.to_string()))?,
            };
            let input_domains = remaining
                .iter()
                .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
                .collect::<Vec<_>>();
            Some(
                scheme
                    .infer_declared_sparse_types_mode(
                        remaining,
                        &input_domains,
                        &vec![None; parameters.len()],
                        expected,
                        partial,
                        &self.context,
                        crate::ptc::Budget::default(),
                        false,
                        &[],
                    )
                    .map_err(|cause| call_inference_error(span, cause))?,
            )
        } else {
            None
        };
        let mut candidates = Vec::new();
        for &(name, index) in collectors {
            let inferred_type = inferred
                .as_ref()
                .map(|candidate| candidate.parameter_type(index))
                .transpose()
                .map_err(|cause| call_inference_error(span, cause))?
                .flatten();
            // The element types may still be open while the declared
            // collection already supplies an unambiguous outer shape.
            let ty = inferred_type.unwrap_or_else(|| parameters[index].ty.clone());
            candidates.push((name, index, Some(ty)));
        }
        self.select_property_collector(candidates, span)
    }

    fn select_property_collector(
        &mut self,
        candidates: Vec<(hir::common::Ident, usize, Option<Term>)>,
        span: Span,
    ) -> Result<(hir::common::Ident, usize), Error> {
        let mut eligible = Vec::new();
        for (name, index, ty) in candidates {
            let shape = ty
                .as_ref()
                .map(|ty| self.structural_type(ty, span))
                .transpose()?;
            // Default property collection builds a record. Unknown shapes
            // remain candidates; a known tuple is a distinct input protocol.
            if matches!(
                shape,
                None | Some(
                    Term::Record(_)
                        | Term::Rigid { .. }
                        | Term::NormalForm { .. }
                        | Term::AssociatedType { .. }
                        | Term::Bound { .. }
                        | Term::Hole { .. }
                )
            ) {
                eligible.push((name, index));
            }
        }
        let [selected] = eligible.as_slice() else {
            return Err(error(
                span,
                "attribute collection requires one variadic entrance eligible for the default record protocol; properties are not named parameter supply",
            ));
        };
        Ok(*selected)
    }

    /// Tuple collection has already staged the receiver and each explicit
    /// input. Replace its single property-packet step with interleaved field
    /// producers while preserving every other producer and final binding.
    fn merge_property_preparation(
        &mut self,
        expr: &Expr<'_>,
        args: &[Arg<'_>],
        call: n::Expr,
        parameter: usize,
    ) -> Result<n::Expr, Error> {
        let K::CallPreparation {
            mut statements,
            result: Some(result),
        } = call.kind
        else {
            return Err(error(
                expr.span,
                "property collection requires a completed preparation plan",
            ));
        };
        let K::Call { arguments, .. } = &result.kind else {
            return Err(error(
                expr.span,
                "property collection requires a normalized invocation",
            ));
        };
        let binding = arguments
            .iter()
            .find(|argument| argument.parameter == parameter)
            .and_then(|argument| {
                if let K::Local(binding) = argument.value.kind {
                    Some(binding)
                } else {
                    None
                }
            })
            .ok_or_else(|| error(expr.span, "property packet preparation is missing"))?;
        let offsets = std::mem::take(&mut self.preparing_inputs.as_mut().unwrap().offsets);
        // The collapsed call has one property input at the first written
        // property's position. Its final step binds the constructed record.
        let first = args
            .iter()
            .position(|arg| matches!(arg, Arg::Named(..)))
            .ok_or_else(|| error(expr.span, "missing property input"))?;
        let end = offsets
            .get(first + 1)
            .copied()
            .filter(|end| *end > offsets[first] && *end <= statements.len())
            .ok_or_else(|| error(expr.span, "property packet preparation is missing"))?;
        let mut packet = statements.remove(end - 1);
        let K::Let { binding: id, value } = &mut packet.kind else {
            return Err(error(expr.span, "property packet binding is missing"));
        };
        if *id != binding {
            return Err(error(expr.span, "property packet binding mismatch"));
        }
        layout::unprepare_packet(value, expr.span)?;
        let field_positions = layout::field_positions(&value.ty, expr.span)?;
        let K::AnonymousRecord { fields } = &mut value.kind else {
            return Err(error(
                expr.span,
                "property collection requires a completed record",
            ));
        };
        let mut remaining = statements.into_iter();
        let mut prepared: Vec<_> = remaining.by_ref().take(offsets[0]).collect();
        let mut lengths = offsets.windows(2).map(|w| w[1] - w[0]);
        let mut new_offsets = Vec::with_capacity(args.len() + 1);
        for (index, argument) in args.iter().enumerate() {
            new_offsets.push(prepared.len());
            if index == first || !matches!(argument, Arg::Named(..)) {
                let len = lengths
                    .next()
                    .ok_or_else(|| error(expr.span, "missing input boundary"))?;
                prepared.extend(remaining.by_ref().take(len - usize::from(index == first)));
            }
            if let Arg::Named(name, _) = argument {
                prepared.push(self.prepare_call_operand(
                    &mut fields[field_positions[name.name.as_str()]],
                    expr.span,
                )?);
            }
        }
        if lengths.next().is_some() {
            return Err(error(expr.span, "unexpected remaining property inputs"));
        }
        new_offsets.push(prepared.len());
        self.preparing_inputs.as_mut().unwrap().offsets = new_offsets;
        prepared.push(packet);
        prepared.extend(remaining);
        let diverges = prepared.iter().any(|statement| is_never(&statement.ty));
        Ok(n::Expr {
            ty: if diverges {
                p(P::Never)
            } else {
                result.ty.clone()
            },
            source: source(expr.span),
            kind: K::CallPreparation {
                statements: prepared,
                result: Some(result),
            },
        })
    }

    pub(super) fn property_packet(
        &mut self,
        expr: &Expr<'_>,
        properties: &[hir::expr::FieldExpr<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        self.property_packet_with_context(expr, properties, expected, None)
    }

    pub(super) fn property_packet_with_context(
        &mut self,
        expr: &Expr<'_>,
        properties: &[hir::expr::FieldExpr<'_>],
        expected: Option<&Term>,
        partial: Option<&[(String, crate::calls::InputContext)]>,
    ) -> Result<n::Expr, Error> {
        let context = expected
            .map(|ty| self.structural_type(ty, expr.span))
            .transpose()?;
        let names: Option<Vec<_>> = match &context {
            Some(Term::Record(fields)) => {
                Some(fields.iter().map(|(name, _)| name.clone()).collect())
            }
            _ => partial.map(|fields| fields.iter().map(|(name, _)| name.clone()).collect()),
        };
        let order = layout::property_order(properties, names.as_deref(), expr.span)?;
        let mut fields = Vec::new();
        let mut types = Vec::new();
        for property in properties {
            let partial_field = partial
                .and_then(|fields| {
                    fields
                        .iter()
                        .find(|(name, _)| name == property.ident.name.as_str())
                })
                .map(|(_, context)| context);
            let field_context = match &context {
                Some(Term::Record(types)) => types
                    .iter()
                    .find(|(name, _)| name == property.ident.name.as_str())
                    .map(|(_, ty)| ty),
                _ => match partial_field {
                    Some(crate::calls::InputContext::Known(ty)) => Some(ty),
                    _ => None,
                },
            };
            let field = match partial_field {
                Some(context) if field_context.is_none() => {
                    self.input_context_expression(property.expr, context)?
                }
                _ => self.expr(property.expr, field_context)?,
            };
            let field_type = if is_never(&field.ty) {
                field_context.cloned().unwrap_or_else(|| field.ty.clone())
            } else {
                field.ty.clone()
            };
            types.push((property.ident.name.to_string(), field_type));
            fields.push(field);
        }
        // This internal packet retains its hypothetical completed shape for
        // argument inference, even when a producer exits. collect_properties
        // stages every producer and propagates Never to the whole call before
        // publishing NIR; no packet is constructed along that exit path.
        let packet = self.finish_property_packet(expr.span, types, fields, &order)?;
        if let Some(expected) = expected {
            self.require(&packet.ty, expected, expr.span)?;
        }
        Ok(packet)
    }
}
