//! Extended application shares ordinary positional routing, while colon
//! properties remain a distinct input protocol from named whole-value supply.
use super::*;

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
        if let H::ResolvedDef {def_id,..}=head.kind {
            if self.package.definition_owner(def_id).is_some_and(|owner|self.is_code_generator(owner)) {
                return Err(error(expr.span,"code generator must be invoked in a comptime code position"));
            }
        }
        let callable = match &head.kind {
            H::ResolvedDef { .. } if self.associated_reference_owner(head).is_some() => {
                let owner = self.associated_reference_owner(head).unwrap();
                let value = self.lexical_associated_value(owner, callee.span)?;
                let ty = self.structural_type(&value.ty, callee.span)?;
                matches!(ty, Term::Function { .. } | Term::Closure { .. } | Term::ComptimeFunction(_))
            }
            H::ResolvedDef { def_id, .. } => self.package.definition_owner(*def_id)
                .and_then(|owner| self.package.item(owner))
                .is_some_and(|item| matches!(&item.kind, ItemKind::Fn(sig, _) if !sig.modifiers.is_normal_form_head)),
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                if let Some(ty) = self.locals.get(&local(*id)) {
                    let ty = self.structural_type(ty, callee.span)?;
                    matches!(ty, Term::Function { .. } | Term::Closure { .. } | Term::ComptimeFunction(_))
                } else { false }
            }
            H::Closure(..) => true,
            H::Projection(base, name) => {
                !self.is_type_head(base)
                    || self.static_method(base, name.name.as_str(), callee.span)?.is_some()
            },
            _ => false,
        };
        if !callable {
            return Ok(None);
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
        let resolved = if let Some(method) = self.resolve_receiver(expr, callee)? {
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
                self.package.owners().find_map(|(owner, _)| {
                    (declaration(self.package, &self.package_identities, owner) == id)
                        .then_some(owner)
                })
            })
        }
        .ok_or_else(|| {
            error(
                expr.span,
                "attribute collection requires declaration-side protocol metadata",
            )
        })?;
        let comptime = self.package.item(owner).is_some_and(|item|
            matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) if sig.modifiers.is_comptime));
        if comptime && self.protocols.splice.is_some() && self.runtime_value_context() {
            let value = self.with_staged_input(true, |this|
                this.collect_properties_resolved(expr, callee, args, None, None, resolved, owner))?;
            return self.deliver_comptime_result(expr, value, expected);
        }
        self.collect_properties_resolved(expr, callee, args, expected, partial, resolved, owner)
    }

    fn collect_properties_resolved(
        &mut self, expr: &Expr<'_>, callee: &Expr<'_>, args: &[Arg<'_>],
        expected: Option<&Term>, partial: Option<&crate::calls::CallableContext>,
        resolved: Option<(n::Expr, Vec<n::Expr>)>, owner: OwnerId,
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
                expr, callee, args, &inputs, expected, partial, resolved.as_ref(),
                |this, flat| this.collect_properties_resolved(
                    expr, callee, flat, expected, partial, resolved.clone(), owner,
                ),
            );
        }
        let receiver_count = resolved.as_ref().map_or(0, |(_, prefix)| prefix.len());
        let mut call = if let Some((method, prefix)) = resolved {
            self.finish_call(expr, method, &inputs, prefix, expected, partial)?
        } else {
            self.call(expr, callee, &inputs, expected, partial)?
        };
        if matches!(call.kind, K::CallPreparation { .. }) {
            return self.merge_property_preparation(
                expr,
                args,
                call,
                parameter_index,
                receiver_count,
            );
        }
        let anchor = n::SourceRange {
            start: expr.span.lo().0,
            end: expr.span.lo().0,
        };
        let K::Call { arguments, .. } = &mut call.kind else {
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
        let K::AnonymousRecord { fields } = &mut packet.kind else {
            return Err(error(
                expr.span,
                "property collection did not produce a record packet",
            ));
        };
        let mut statements = Vec::new();
        for receiver in arguments.iter_mut().take(receiver_count) {
            self.stage_property_input(&mut receiver.value, &mut statements, expr.span)?;
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
        let mut fields = fields.iter_mut();
        // Argument inference may check these in a different order. Runtime
        // evaluation always uses the original producer order, exactly once.
        for input in args {
            let value = match input {
                Arg::Named(..) => fields.next().expect("one field per property"),
                Arg::Positional(_) | Arg::PostLambda(_) => {
                    &mut arguments[positional_slots.next().expect("one anchored input")].value
                }
                _ => unreachable!(),
            };
            self.stage_property_input(value, &mut statements, expr.span)?;
        }
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

    fn property_collector(
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
        let signature = resolved
            .map(|value| value.ty.clone())
            .unwrap_or_else(|| self.signatures[&owner].clone());
        let Term::Function { parameters, .. } = signature.callable_signature() else {
            unreachable!()
        };
        let fixed = match resolved.map(|value| &value.kind) {
            Some(K::FunctionInstance { arguments, .. }) => arguments.len(),
            Some(K::TraitMethod { interface_arguments, method_arguments, .. }) => 1 + interface_arguments.len() + method_arguments.len(),
            Some(_) => self.type_parameters[&owner].len(),
            None => 0,
        };
        let remaining = &self.type_parameters[&owner][fixed..];
        let inferred = if !remaining.is_empty() {
            let scheme = crate::calls::Scheme {
                type_parameters: remaining.len(),
                signature: signature
                    .abstract_types(remaining)
                    .map_err(|cause| error(span, cause.to_string()))?,
            };
            Some(
                scheme
                    .infer_contextual_types(
                        &vec![None; parameters.len()],
                        expected,
                        partial,
                        &self.context,
                        crate::ptc::Budget::default(),
                    )
                    .map_err(|cause| call_inference_error(span, cause))?,
            )
        } else {
            None
        };
        let mut eligible = Vec::new();
        for &(name, index) in collectors {
            let ty = if let Some(inferred) = &inferred {
                inferred
                    .parameter_type(index)
                    .map_err(|cause| call_inference_error(span, cause))?
            } else {
                Some(parameters[index].ty.clone())
            };
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
        receiver_count: usize,
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
        let slot = statements
            .iter()
            .position(
                |statement| matches!(&statement.kind, K::Let { binding: id, .. } if *id == binding),
            )
            .ok_or_else(|| error(expr.span, "property packet binding is missing"))?;
        let mut packet = statements.remove(slot);
        let K::Let { value, .. } = &mut packet.kind else {
            unreachable!()
        };
        let K::AnonymousRecord { fields } = &mut value.kind else {
            return Err(error(
                expr.span,
                "property collection did not produce a record packet",
            ));
        };
        let mut remaining = statements.into_iter();
        let mut prepared: Vec<_> = remaining.by_ref().take(receiver_count).collect();
        let mut fields = fields.iter_mut();
        for argument in args {
            match argument {
                Arg::Named(..) => self.stage_property_input(
                    fields.next().expect("one field per property"),
                    &mut prepared,
                    expr.span,
                )?,
                Arg::Positional(_) | Arg::PostLambda(_) => prepared.push(remaining.next().ok_or_else(|| {
                    error(expr.span, "positional collection preparation is missing")
                })?),
                _ => unreachable!(),
            }
        }
        if remaining.next().is_some() {
            return Err(error(
                expr.span,
                "unexpected remaining collection preparation",
            ));
        }
        prepared.push(packet);
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

    fn stage_property_input(
        &mut self,
        value: &mut n::Expr,
        statements: &mut Vec<n::Expr>,
        span: Span,
    ) -> Result<(), Error> {
        let binding = self.spread_binding(span)?;
        let anchor = n::SourceRange {
            start: span.lo().0,
            end: span.lo().0,
        };
        let local = n::Expr {
            ty: value.ty.clone(),
            source: anchor,
            kind: K::Local(binding),
        };
        let value = std::mem::replace(value, local);
        statements.push(n::Expr {
            ty: if is_never(&value.ty) {
                p(P::Never)
            } else {
                p(P::Void)
            },
            source: anchor,
            kind: K::Let {
                binding,
                value: Box::new(value),
            },
        });
        Ok(())
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
        if let Some(fields) = partial {
            if fields.len() != properties.len()
                || fields
                    .iter()
                    .zip(properties)
                    .any(|((name, _), property)| name != property.ident.name.as_str())
            {
                return Err(error(
                    expr.span,
                    "property argument fields do not match the expected record shape",
                ));
            }
        }
        let context = expected
            .map(|ty| self.structural_type(ty, expr.span))
            .transpose()?;
        let mut fields = Vec::new();
        let mut types = Vec::new();
        for property in properties {
            let partial_field = partial
                .and_then(|fields| fields.get(types.len()))
                .map(|(_, context)| context);
            let field_context = match &context {
                Some(Term::Record(types)) => types
                    .get(fields.len())
                    .filter(|(name, _)| name == property.ident.name.as_str())
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
        let ty = Term::Record(types);
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind: K::AnonymousRecord { fields },
        })
    }


}
