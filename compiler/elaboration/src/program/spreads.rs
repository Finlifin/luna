//! Tuple spreads are evaluated once, then unpacked in the enclosing call's
//! preparation scope. Packet storage is not a new full-expression boundary.
use super::*;
mod inputs;
mod layout;

pub(super) struct PendingPacket {
    origin: (hir::HirId, Span),
    bindings: Vec<n::LocalId>,
    value: n::Expr,
    staged: bool,
    tuple: bool,
    frozen: Option<(n::Expr, comptime_interpretor::Value)>,
}

impl Checker<'_, '_> {
    /// Infer against the one collected record entrance, then retain each field
    /// as its own source producer while expanding the positional packets.
    /// The normal collection checker builds the final call from that flat list.
    pub(super) fn prepare_property_spreads(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        collected: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        resolved: Option<&(n::Expr, Vec<n::Expr>)>,
        build: impl FnOnce(&mut Self, &[Arg<'_>], bool) -> Result<n::Expr, Error>,
    ) -> Result<n::Expr, Error> {
        // Literal tuples must reach destination checking before numeric
        // defaulting or staged-input interpretation.
        if args
            .iter()
            .all(|arg| !matches!(arg, Arg::Expand(value) if !matches!(value.kind, H::Tuple(_))))
        {
            let mut flat = Vec::new();
            for arg in args {
                match arg {
                    Arg::Expand(value) => {
                        let H::Tuple(fields) = value.kind else {
                            unreachable!()
                        };
                        flat.extend(fields.iter().map(Arg::Positional));
                    }
                    other => flat.push(other.clone()),
                }
            }
            return build(self, &flat, false);
        }
        let mut checked = inputs::PreparedInputs::new();
        let context = self.spread_context(
            callee,
            collected,
            expected,
            partial,
            resolved,
            expr.span,
            &mut checked,
        )?;
        let (contexts, partial_contexts, stages) = if let Some(context) = context {
            let types = context.iter().map(|(ty, _)| ty.clone()).collect::<Vec<_>>();
            let stages = context.iter().map(|(_, stage)| *stage).collect::<Vec<_>>();
            let (contexts, partial_contexts) =
                self.spread_partial_packet_contexts(collected, &types, expr.span)?;
            (
                contexts,
                partial_contexts,
                self.spread_packet_layout(collected, &stages, expr.span)?
                    .into_iter()
                    .map(|fields| {
                        fields.is_some_and(|fields| fields.into_iter().any(|stage| stage))
                    })
                    .collect::<Vec<_>>(),
            )
        } else {
            (
                vec![None; collected.len()],
                vec![None; collected.len()],
                vec![false; collected.len()],
            )
        };
        let mut source_contexts = Vec::new();
        let mut source_partial_contexts = Vec::new();
        let mut source_stages = Vec::new();
        let mut collected_index = 0;
        let mut record_seen = false;
        for arg in args {
            if matches!(arg, Arg::Named(..)) {
                source_contexts.push(None);
                source_partial_contexts.push(None);
                source_stages.push(false);
                if !record_seen {
                    collected_index += 1;
                    record_seen = true;
                }
            } else {
                source_contexts.push(contexts[collected_index].clone());
                source_partial_contexts.push(partial_contexts[collected_index].clone());
                source_stages.push(stages[collected_index]);
                collected_index += 1;
            }
        }
        // Inference may already have checked peer fields or positional inputs.
        // Remap those owned expressions, never re-elaborate their producers.
        let mut source_checked = inputs::PreparedInputs::new();
        if !checked.is_empty() {
            let mut source_slot = 0;
            let mut collected_slot = 0;
            let mut record_slot = None;
            let mut fields = Vec::new();
            let mut fuel = 10_000;
            for arg in args {
                match arg {
                    Arg::Named(_, value) => {
                        if record_slot.is_none() {
                            record_slot = Some(collected_slot);
                            collected_slot += 1;
                        }
                        fields.push((source_slot, value.hir_id));
                        source_slot += 1;
                    }
                    _ => {
                        let width = match arg {
                            Arg::Expand(value) => {
                                self.spread_width(value, &mut fuel, 0)?.ok_or_else(|| {
                                    error(
                                        value.span,
                                        "checked spread inputs have no stable packet layout",
                                    )
                                })?
                            }
                            _ => 1,
                        };
                        for _ in 0..width {
                            if let Some(value) = checked.remove(&collected_slot) {
                                source_checked.insert(source_slot, value);
                            }
                            source_slot += 1;
                            collected_slot += 1;
                        }
                    }
                }
            }
            if let Some(record) = record_slot.and_then(|slot| checked.remove(&slot)) {
                let K::AnonymousRecord { fields: values } = record.1.kind else {
                    return Err(error(
                        expr.span,
                        "checked properties lost their record construction",
                    ));
                };
                if fields.len() != values.len() {
                    return Err(error(expr.span, "checked property input layout changed"));
                }
                for ((slot, identity), value) in fields.into_iter().zip(values) {
                    source_checked.insert(slot, (identity, value));
                }
            }
            if !checked.is_empty() {
                return Err(error(
                    expr.span,
                    "checked property input mapping is inconsistent",
                ));
            }
        }
        self.prepare_spreads_checked(
            expr,
            args,
            &source_contexts,
            &source_stages,
            Some(&source_partial_contexts),
            source_checked,
            BTreeMap::new(),
            1,
            build,
        )
    }

    pub(super) fn prepare_spread_input(
        &mut self,
        binding: n::LocalId,
        source: &Expr<'_>,
    ) -> Result<Option<(n::LocalId, usize)>, Error> {
        let Some((key, index)) = self.pending_spread_inputs.iter().find_map(|(key, packet)| {
            packet
                .bindings
                .iter()
                .position(|id| *id == binding)
                .map(|index| (*key, index))
        }) else {
            return Ok(None);
        };
        let packet = &self.pending_spread_inputs[&key];
        if !packet.staged {
            return Err(error(
                source.span,
                "dependent Type input requires a comptime packet",
            ));
        }
        if packet.frozen.is_none() {
            let value = packet.value.clone();
            // The synthetic field uses the enclosing call's identity. Required
            // evaluation must track the packet producer instead, or evaluating
            // a constant call would falsely appear to recurse into itself.
            let mut origin = source.clone();
            origin.hir_id = packet.origin.0;
            origin.span = packet.origin.1;
            let (completed, ty) = self.required_normal_input(&origin, value, vec![])?;
            let frozen = self.materialize_local_constant(completed.clone(), ty, origin.span)?;
            self.pending_spread_inputs.get_mut(&key).unwrap().frozen = Some((frozen, completed));
        }
        Ok(Some((key, index)))
    }

    pub(super) fn completed_packet_key(&self, binding: n::LocalId) -> Option<n::LocalId> {
        self.pending_spread_inputs.iter().find_map(|(key, packet)| {
            (packet.frozen.is_some() && packet.bindings.contains(&binding)).then_some(*key)
        })
    }

    pub(super) fn completed_packet_graph(
        &self,
        key: n::LocalId,
    ) -> (comptime_interpretor::Value, Term) {
        let (expression, value) = self.pending_spread_inputs[&key]
            .frozen
            .as_ref()
            .expect("completed packet");
        (value.clone(), expression.ty.clone())
    }

    pub(super) fn set_completed_packet_expression(&mut self, key: n::LocalId, expression: n::Expr) {
        self.pending_spread_inputs
            .get_mut(&key)
            .unwrap()
            .frozen
            .as_mut()
            .expect("completed packet")
            .0 = expression;
    }

    pub(super) fn spread_input_value(
        &mut self,
        binding: n::LocalId,
        source: &Expr<'_>,
    ) -> Result<Option<comptime_interpretor::Value>, Error> {
        let Some((key, index)) = self.prepare_spread_input(binding, source)? else {
            return Ok(None);
        };
        let packet = &self.pending_spread_inputs[&key];
        let (_, completed) = packet.frozen.as_ref().unwrap();
        let selected = if packet.tuple {
            let comptime_interpretor::Value::Tuple(fields) = completed else {
                return Err(error(
                    source.span,
                    "dependent spread did not produce a tuple",
                ));
            };
            fields
                .get(index)
                .cloned()
                .ok_or_else(|| error(source.span, "dependent spread field is missing"))?
        } else {
            completed.clone()
        };
        Ok(Some(selected))
    }
    pub(super) fn spread_input_symbol(
        &mut self,
        binding: n::LocalId,
        ty: &Term,
        source: &Expr<'_>,
    ) -> Result<Option<Term>, Error> {
        let Some(selected) = self.spread_input_value(binding, source)? else {
            return Ok(None);
        };
        // Delivery remains attached to the original packet. Only the selected
        // dependency obtains a symbol; a closure's checked snapshot factory is
        // used by annotation computations, not to split the delivery graph.
        if matches!(&selected, comptime_interpretor::Value::Closure(closure)
            if closure.declaration_instance().is_none())
        {
            return self
                .freeze_completed_dependency(selected, ty.clone(), source.span)
                .map(|(_, symbol)| Some(symbol));
        }
        self.required_normal_term(selected, source.span).map(Some)
    }
    pub(super) fn spread_binding(&mut self, span: Span) -> Result<n::LocalId, Error> {
        let index = self.next_temporary_binding;
        self.next_temporary_binding = index
            .checked_add(1)
            .ok_or_else(|| error(span, "spread temporary binding space exhausted"))?;
        Ok(n::LocalId {
            owner: u32::MAX,
            index,
        })
    }

    pub(super) fn spread_call(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        // Literal packets already expose their element expressions. Route them
        // directly so each destination supplies both type and stage context;
        // a speculative runtime temporary would erase that distinction.
        if let Some(flat) = arguments::literal_spread_arguments(args) {
            return self
                .call(expr, callee, &flat, expected, partial)
                .map_err(|mut failure| {
                    if failure.source == source(expr.span)
                        && failure.message.ends_with("too many positional inputs")
                    {
                        failure.message =
                            "argument count mismatch: spread tuple widths exceed positional slots"
                                .into();
                    }
                    failure
                });
        }
        if let H::NFApplication(head, clauses) = callee.kind {
            if let H::ResolvedDef { def_id, .. } = head.kind {
                if let Some(owner) = self.package.definition_owner(def_id) {
                    self.ensure_signature(owner, expr.span)?;
                    if self.open_functions.contains_key(&owner) {
                        self.require_non_trait_body_reference(owner, callee.span)?;
                        return self.with_explicit_open_clauses(
                            owner,
                            clauses,
                            expr.span,
                            |this| {
                                this.spread_call_resolved(
                                    expr,
                                    callee,
                                    args,
                                    expected,
                                    partial,
                                    None,
                                    Some(owner),
                                )
                            },
                        );
                    }
                }
            }
        }
        let resolved = if let Some(method) = self.resolve_receiver(expr, callee)? {
            Some(method)
        } else if let H::NFApplication(head, types) = callee.kind {
            Some((
                self.explicit_function_instance(callee, head, types, None, true)?,
                Vec::new(),
            ))
        } else if matches!(callee.kind, H::ResolvedDef { .. })
            && self.associated_reference_owner(callee).is_none()
        {
            None
        } else {
            Some((self.invocation_callee(callee)?, Vec::new()))
        };
        if resolved
            .as_ref()
            .is_some_and(|(callee, _)| callee.ty.is_comptime_callable())
            && self.protocols.splice.is_some()
            && self.runtime_value_context()
        {
            let value = self.with_staged_input(true, |this| {
                this.spread_call_resolved(expr, callee, args, None, None, resolved, None)
            })?;
            return self.deliver_comptime_result(expr, value, expected);
        }
        self.spread_call_resolved(expr, callee, args, expected, partial, resolved, None)
    }

    pub(super) fn spread_call_resolved(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        resolved: Option<(n::Expr, Vec<n::Expr>)>,
        open_instance: Option<OwnerId>,
    ) -> Result<n::Expr, Error> {
        let mut checked = inputs::PreparedInputs::new();
        let context = self.spread_context(
            callee,
            args,
            expected,
            partial,
            resolved.as_ref(),
            expr.span,
            &mut checked,
        )?;
        let (packet_contexts, partial_contexts) = if let Some(types) = &context {
            self.spread_partial_packet_contexts(
                args,
                &types.iter().map(|(ty, _)| ty.clone()).collect::<Vec<_>>(),
                expr.span,
            )?
        } else {
            (vec![None; args.len()], vec![None; args.len()])
        };
        let packet_stages = if let Some(types) = &context {
            let markers = types.iter().map(|(_, staged)| *staged).collect::<Vec<_>>();
            self.spread_packet_layout(args, &markers, expr.span)?
                .into_iter()
                .map(|fields| fields.is_some_and(|fields| fields.into_iter().any(|staged| staged)))
                .collect::<Vec<_>>()
        } else {
            vec![false; args.len()]
        };
        self.prepare_spreads_checked(
            expr,
            args,
            &packet_contexts,
            &packet_stages,
            Some(&partial_contexts),
            checked,
            BTreeMap::new(),
            0,
            |this, inputs, diverges| {
                let expected = expected.filter(|_| !diverges);
                let partial = partial.filter(|_| !diverges);
                if let Some((callee, prefix)) = resolved {
                    this.finish_call(expr, callee, inputs, prefix, expected, partial)
                } else if let Some(owner) = open_instance {
                    this.instantiated_open_call(expr, owner, inputs, expected)
                } else {
                    this.call(expr, callee, inputs, expected, partial)
                }
            },
        )
    }

    pub(super) fn prepare_spreads(
        &mut self,
        expr: &Expr<'_>,
        args: &[Arg<'_>],
        packet_contexts: &[Option<Term>],
        build: impl FnOnce(&mut Self, &[Arg<'_>], bool) -> Result<n::Expr, Error>,
    ) -> Result<n::Expr, Error> {
        self.prepare_spreads_checked(
            expr,
            args,
            packet_contexts,
            &vec![false; args.len()],
            None,
            inputs::PreparedInputs::new(),
            BTreeMap::new(),
            0,
            build,
        )
    }

    pub(super) fn prepare_spreads_with_input(
        &mut self,
        expr: &Expr<'_>,
        args: &[Arg<'_>],
        contexts: &[Option<crate::calls::InputContext>],
        checked: inputs::PreparedInputs,
        packets: BTreeMap<usize, n::Expr>,
        build: impl FnOnce(&mut Self, &[Arg<'_>]) -> Result<n::Expr, Error>,
    ) -> Result<n::Expr, Error> {
        self.prepare_spreads_checked(
            expr,
            args,
            &vec![None; args.len()],
            &vec![false; args.len()],
            Some(contexts),
            checked,
            packets,
            0,
            |this, inputs, _| build(this, inputs),
        )
    }

    fn prepare_spreads_checked(
        &mut self,
        expr: &Expr<'_>,
        args: &[Arg<'_>],
        packet_contexts: &[Option<Term>],
        packet_stages: &[bool],
        input_contexts: Option<&[Option<crate::calls::InputContext>]>,
        checked: inputs::PreparedInputs,
        packets: BTreeMap<usize, n::Expr>,
        trailing: usize,
        build: impl FnOnce(&mut Self, &[Arg<'_>], bool) -> Result<n::Expr, Error>,
    ) -> Result<n::Expr, Error> {
        let anchor = n::SourceRange {
            start: expr.span.lo().0,
            end: expr.span.lo().0,
        };
        self.with_prepared_spreads(
            expr,
            args,
            packet_contexts,
            packet_stages,
            input_contexts,
            checked,
            packets,
            |this, inputs, mut spreads, flat_count| {
                let saved = this.preparing_spread.replace(source(expr.span));
                // The enclosing expression cannot produce the callee's result
                // when any packet producer exits. Keep checking every input,
                // but let the caller discard only its result expectation.
                let diverges = spreads.iter().any(|(_, statement)| is_never(&statement.ty));
                let call = build(this, inputs, diverges);
                this.preparing_spread = saved;
                let call = call?;
                for (_, statement) in &mut spreads {
                    let (key, value) = match &mut statement.kind {
                        K::Let { binding, value } => (*binding, value),
                        K::LetPattern {
                            pattern: n::Pattern::Tuple(fields),
                            value,
                            ..
                        } => {
                            let Some(n::Pattern::Binding { binding, .. }) = fields.first() else {
                                continue;
                            };
                            (*binding, value)
                        }
                        _ => continue,
                    };
                    if let Some((frozen, _)) = this
                        .pending_spread_inputs
                        .get(&key)
                        .and_then(|packet| packet.frozen.as_ref())
                    {
                        **value = frozen.clone();
                    }
                }
                if spreads.is_empty() {
                    return Ok(call);
                }
                let prepared = this.insert_spread_preparation(
                    call, spreads, flat_count, trailing, anchor, expr.span,
                )?;
                this.freeze_staged_preparation(prepared, expr)
            },
        )
    }

    /// Share checked packet layout and temporary bindings with a scoped
    /// consumer. Executable callers publish a preparation plan; definition
    /// description can consume the same typed inputs without instantiating a
    /// callee. Temporary locals and pending packets are restored on all exits.
    pub(super) fn with_prepared_spreads<R>(
        &mut self,
        expr: &Expr<'_>,
        args: &[Arg<'_>],
        packet_contexts: &[Option<Term>],
        packet_stages: &[bool],
        input_contexts: Option<&[Option<crate::calls::InputContext>]>,
        mut checked: inputs::PreparedInputs,
        mut packets: BTreeMap<usize, n::Expr>,
        build: impl FnOnce(&mut Self, &[Arg<'_>], Vec<(usize, n::Expr)>, usize) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let anchor = n::SourceRange {
            start: expr.span.lo().0,
            end: expr.span.lo().0,
        };
        let mut introduced = Vec::new();
        let outcome = (|| {
            let mut flat = Vec::new();
            let mut post_positions = std::collections::BTreeSet::new();
            let mut spreads = Vec::new();
            for (argument_index, arg) in args.iter().enumerate() {
                match arg {
                    Arg::Positional(value) => flat.push((None, (*value).clone())),
                    Arg::PostLambda(value) => {
                        post_positions.insert(flat.len());
                        flat.push((None, (*value).clone()));
                    }
                    Arg::Named(name, value) => flat.push((Some(name.clone()), (*value).clone())),
                    Arg::Expand(value) => {
                        if let H::Tuple(items) = value.kind {
                            // Literal elements can receive contexts from their
                            // destination slots; no opaque packet needs probing.
                            flat.extend(items.iter().cloned().map(|item| (None, item)));
                            continue;
                        }
                        // The callee's dependent telescope is not a tuple
                        // binder. Infer this packet's own type, then check its
                        // fields after the Type inputs have been obtained.
                        let origin = (value.hir_id, value.span);
                        let packet_context = match &packet_contexts[argument_index] {
                            Some(ty) if ty.check_scope(&[]).is_err() => None,
                            context => context.clone(),
                        };
                        let value = if let Some(value) = packets.remove(&argument_index) {
                            value
                        } else {
                            self.with_staged_input(packet_stages[argument_index], |this| {
                                match (
                                    packet_context.as_ref(),
                                    input_contexts
                                        .and_then(|contexts| contexts[argument_index].as_ref()),
                                ) {
                                    (None, Some(context)) => {
                                        this.input_context_expression(value, context)
                                    }
                                    (expected, _) => this.expr(value, expected),
                                }
                            })?
                        };
                        let shape = if is_never(&value.ty) {
                            match packet_context.as_ref() {
                                Some(context) => self.structural_type(context, expr.span)?,
                                None => value.ty.clone(),
                            }
                        } else {
                            self.structural_type(&value.ty, expr.span)?
                        };
                        let Term::Tuple(fields) = shape else {
                            return Err(error(
                                expr.span,
                                "positional spread requires a tuple value",
                            ));
                        };
                        let at = flat.len();
                        let mut pattern = Vec::new();
                        let mut packet_bindings = Vec::new();
                        for ty in fields {
                            let binding = self.spread_binding(expr.span)?;
                            self.locals.insert(binding, ty);
                            introduced.push(binding);
                            packet_bindings.push(binding);
                            let id = hir::HirId::new(
                                hir::OwnerId::INVALID,
                                hir::ItemLocalId::new(binding.index),
                            );
                            flat.push((
                                None,
                                Expr {
                                    hir_id: expr.hir_id,
                                    span: expr.span.shrink_to_lo(),
                                    kind: H::ResolvedLocal(
                                        hir::common::Ident::new("$spread", expr.span).name,
                                        id,
                                    ),
                                },
                            ));
                            pattern.push(n::Pattern::Binding {
                                binding,
                                inner: Box::new(n::Pattern::Wild),
                            });
                        }
                        if let Some(key) = packet_bindings.first().copied() {
                            self.pending_spread_inputs.insert(
                                key,
                                PendingPacket {
                                    origin,
                                    bindings: packet_bindings,
                                    value: value.clone(),
                                    staged: packet_stages[argument_index],
                                    tuple: true,
                                    frozen: None,
                                },
                            );
                        }
                        // Pattern residual storage is keyed by its own source
                        // identity. Do not share the collection's construction
                        // anchor, which retires completed aggregate temporaries.
                        let unpack_anchor = n::SourceRange {
                            start: value.source.end,
                            end: value.source.end,
                        };
                        spreads.push((
                            at,
                            n::Expr {
                                ty: if is_never(&value.ty) {
                                    p(P::Never)
                                } else {
                                    p(P::Void)
                                },
                                source: unpack_anchor,
                                kind: K::LetPattern {
                                    pattern: n::Pattern::Tuple(pattern),
                                    value: Box::new(value),
                                    guard: None,
                                    otherwise: None,
                                },
                            },
                        ));
                    }
                    Arg::Implicit(_) => {
                        return Err(error(
                            expr.span,
                            "implicit argument routing is not implemented",
                        ));
                    }
                }
            }
            for (at, (_, input)) in flat.iter_mut().enumerate() {
                if let Some((identity, value)) = checked.remove(&at) {
                    if identity != input.hir_id {
                        return Err(error(
                            expr.span,
                            "spread input layout changed during preparation",
                        ));
                    }
                    let binding = self.spread_binding(expr.span)?;
                    self.locals.insert(binding, value.ty.clone());
                    introduced.push(binding);
                    self.pending_spread_inputs.insert(
                        binding,
                        PendingPacket {
                            origin: (input.hir_id, input.span),
                            bindings: vec![binding],
                            value: value.clone(),
                            staged: true,
                            tuple: false,
                            frozen: None,
                        },
                    );
                    input.kind = H::ResolvedLocal(
                        hir::common::Ident::new("$spread-input", input.span).name,
                        hir::HirId::new(
                            hir::OwnerId::INVALID,
                            hir::ItemLocalId::new(binding.index),
                        ),
                    );
                    spreads.push((
                        at,
                        n::Expr {
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
                        },
                    ));
                }
            }
            if !checked.is_empty() || !packets.is_empty() {
                return Err(error(
                    expr.span,
                    "spread input layout changed during preparation",
                ));
            }
            spreads.sort_by_key(|(at, _)| *at);
            let inputs: Vec<_> = flat
                .iter()
                .enumerate()
                .map(|(index, (name, value))| {
                    if post_positions.contains(&index) {
                        Arg::PostLambda(value)
                    } else if let Some(name) = name {
                        Arg::Named(name.clone(), value)
                    } else {
                        Arg::Positional(value)
                    }
                })
                .collect();
            build(self, &inputs, spreads, flat.len())
        })();
        for binding in introduced {
            self.locals.remove(&binding);
            self.pending_spread_inputs.remove(&binding);
        }
        outcome
    }

    /// Invocation-dependent slots are filled after routing, not by the clause
    /// inference frame. Retain only independent hints during packet discovery.
    fn dependent_spread_hints(
        signature: &Term,
        generics: &[DeclarationId],
        span: Span,
    ) -> Result<Option<Vec<Option<Term>>>, Error> {
        let Term::Function { parameters, result } = signature.callable_signature() else {
            return Ok(None);
        };
        let dependent = parameters
            .iter()
            .map(|p| &p.ty)
            .chain(std::iter::once(result.as_ref()))
            .map(Term::substitution_parameters)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|cause| error(span, cause.to_string()))?
            .iter()
            .any(|slots| !slots.is_empty());
        if !dependent {
            return Ok(None);
        }
        parameters
            .iter()
            .map(|parameter| {
                let ty = &parameter.ty;
                let independent = ty
                    .substitution_parameters()
                    .map_err(|cause| error(span, cause.to_string()))?
                    .is_empty()
                    && !generics.iter().any(|id| ty.mentions_rigid(*id));
                Ok(independent.then(|| ty.clone()))
            })
            .collect::<Result<Vec<_>, Error>>()
            .map(Some)
    }

    /// Obtain positional slot contexts independently of packet partitioning.
    /// Packet lengths are resolved separately, without executing their code.
    fn spread_context(
        &mut self,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        resolved: Option<&(n::Expr, Vec<n::Expr>)>,
        span: Span,
        checked: &mut inputs::PreparedInputs,
    ) -> Result<Option<Vec<(Option<Term>, bool)>>, Error> {
        if let Some((callee, prefix)) = resolved {
            let shape = self.structural_type(&callee.ty, span)?;
            let shape = if self.incomplete_method_instance(callee) {
                // Method-local binders must enter inference before projections
                // depending on them can be selected.
                shape
            } else {
                self.callable_input_view(&shape, span)?
            };
            let signature = if let Term::Closure { signature, .. } = &shape {
                signature.as_ref()
            } else {
                &shape
            };
            let Term::Function { parameters, .. } = signature.callable_signature() else {
                return Ok(None);
            };
            let offset = prefix.len();
            if offset > parameters.len() {
                return Err(error(span, "method receiver count mismatch"));
            }
            let direct = match callee.kind {
                K::Function(id)
                | K::FunctionInstance {
                    declaration: id, ..
                } => Some(id),
                K::TraitMethod { member, .. } => Some(member),
                _ => None,
            };
            let declarations = direct.and_then(|id| {
                self.package.owners().find_map(|(owner, _)| {
                    if declaration(self.package, &self.package_identities, owner) != id {
                        return None;
                    }
                    match &self.package.item(owner)?.kind {
                        ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature) => {
                            Some(&signature.params[offset..])
                        }
                        _ => None,
                    }
                })
            });
            let generics = self
                .method_instantiation_inputs(callee)
                .map(|(owner, arguments)| self.type_parameters[&owner][arguments.len()..].to_vec())
                .unwrap_or_default();
            let types =
                if let Some(hints) = Self::dependent_spread_hints(signature, &generics, span)? {
                    hints[offset..].to_vec()
                } else if self.incomplete_method_instance(callee) {
                    let (owner, arguments) = self.method_instantiation_inputs(callee).unwrap();
                    let remaining = &self.type_parameters[&owner][arguments.len()..];
                    let scheme = crate::calls::Scheme {
                        parameter_sorts: self
                            .input_binders(remaining)
                            .iter()
                            .map(|input| input.sort)
                            .collect(),
                        type_parameters: remaining.len(),
                        signature: signature
                            .abstract_parameters(&self.input_binders(remaining))
                            .map_err(|cause| error(span, cause.to_string()))?,
                    };
                    let mut actual = vec![None; parameters.len()];
                    for (index, value) in prefix.iter().enumerate() {
                        if !is_never(&value.ty) {
                            actual[index] = Some(value.ty.clone());
                        }
                    }
                    let candidate = self.infer_spread_inputs(
                        &scheme,
                        actual,
                        args,
                        declarations,
                        offset,
                        expected,
                        partial,
                        None,
                        None,
                        span,
                        checked,
                    )?;
                    (offset..parameters.len())
                        .map(|index| {
                            candidate
                                .parameter_type(index)
                                .map_err(|cause| call_inference_error(span, cause))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    parameters[offset..]
                        .iter()
                        .map(|parameter| Some(parameter.ty.clone()))
                        .collect()
                };
            return self.spread_parameter_context(
                types,
                declarations,
                &parameters[offset..]
                    .iter()
                    .map(|p| callee.ty.is_comptime_callable() || p.sort != Sort::Runtime)
                    .collect::<Vec<_>>(),
                args,
                span,
            );
        }
        let (head, explicit) = match &callee.kind {
            H::NFApplication(head, types) => (*head, Some(*types)),
            _ => (callee, None),
        };
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return Ok(None);
        };
        let Some(owner) = self.package.definition_owner(def_id) else {
            return Ok(None);
        };
        let Some(ItemKind::Fn(declaration, _)) = self.package.item(owner).map(|item| &item.kind)
        else {
            return Ok(None);
        };
        if declaration.modifiers.is_normal_form_head {
            return Ok(None);
        }
        let declarations = declaration.params;
        self.ensure_signature(owner, span)?;
        // An open result does not erase known input domains. Preserve those
        // contexts for packet literals, but do not execute deferred annotations
        // here: they may need inputs that have not yet been prepared.
        if let Some(definition) = self.open_functions.get(&owner) {
            let domains = definition.input_domains();
            let mut types = domains
                .iter()
                .map(|(_, ty)| {
                    let Some(ty) = ty else { return Ok(None) };
                    let ty = self.specialize_open_clause_type(owner, ty, span)?;
                    let free = ty
                        .rigid_parameters()
                        .map_err(|cause| error(span, cause.to_string()))?;
                    // Unsolved clause parameters are inference inputs, not an
                    // expected type to impose on numeric packet expressions.
                    if self.type_parameters[&owner]
                        .iter()
                        .any(|id| free.contains_key(id))
                        || !ty
                            .substitution_parameters()
                            .map_err(|cause| error(span, cause.to_string()))?
                            .is_empty()
                    {
                        return Ok(None);
                    }
                    Ok(Some(ty))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if !self.type_parameters[&owner].is_empty()
                && types.iter().any(Option::is_none)
                && let Some(candidate) =
                    self.open_spread_candidate(owner, expected, args, checked, span)?
            {
                for (slot, ty) in types.iter_mut().enumerate() {
                    if ty.is_none() {
                        *ty = candidate
                            .parameter_type(slot)
                            .map_err(|cause| call_inference_error(span, cause))?;
                    }
                }
            }
            return self.spread_parameter_context(
                types,
                Some(declarations),
                &domains
                    .iter()
                    .map(|(sort, _)| declaration.modifiers.is_comptime || *sort != Sort::Runtime)
                    .collect::<Vec<_>>(),
                args,
                span,
            );
        }
        let signature = self.signatures[&owner].clone();
        let type_parameters = self.type_parameters[&owner].clone();
        if let Some(types) = Self::dependent_spread_hints(&signature, &type_parameters, span)? {
            let Term::Function { parameters, .. } = signature.callable_signature() else {
                unreachable!()
            };
            return self.spread_parameter_context(
                types,
                Some(declarations),
                &parameters
                    .iter()
                    .map(|p| declaration.modifiers.is_comptime || p.sort != Sort::Runtime)
                    .collect::<Vec<_>>(),
                args,
                span,
            );
        }
        let types = if let Some(explicit) = explicit {
            let value = self.explicit_function_instance(callee, head, explicit, None, true)?;
            let signature = self.callable_input_view(&value.ty, span)?;
            let Term::Function { parameters, .. } = signature.callable_signature() else {
                return Ok(None);
            };
            parameters
                .iter()
                .map(|p| Some(p.ty.clone()))
                .collect::<Vec<_>>()
        } else if type_parameters.is_empty() {
            let signature = self.callable_input_view(&signature, span)?;
            let Term::Function { parameters, .. } = signature.callable_signature() else {
                return Ok(None);
            };
            parameters.iter().map(|p| Some(p.ty.clone())).collect()
        } else {
            let scheme = crate::calls::Scheme {
                parameter_sorts: self
                    .input_binders(&type_parameters)
                    .iter()
                    .map(|input| input.sort)
                    .collect(),
                type_parameters: type_parameters.len(),
                signature: signature
                    .abstract_parameters(&self.input_binders(&type_parameters))
                    .map_err(|cause| error(span, cause.to_string()))?,
            };
            let candidate = self.infer_spread_inputs(
                &scheme,
                vec![None; declarations.len()],
                args,
                Some(declarations),
                0,
                expected,
                partial,
                None,
                None,
                span,
                checked,
            )?;
            (0..declarations.len())
                .map(|index| {
                    candidate
                        .parameter_type(index)
                        .map_err(|cause| call_inference_error(span, cause))
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        self.spread_parameter_context(
            types,
            Some(declarations),
            &declarations
                .iter()
                .map(|p| declaration.modifiers.is_comptime || p.ty.is_comptime())
                .collect::<Vec<_>>(),
            args,
            span,
        )
    }

    pub(super) fn spread_parameter_context(
        &mut self,
        types: Vec<Option<Term>>,
        declarations: Option<&[hir::common::FnParam<'_>]>,
        stages: &[bool],
        args: &[Arg<'_>],
        span: Span,
    ) -> Result<Option<Vec<(Option<Term>, bool)>>, Error> {
        let post_target = arguments::post_lambda_target(args, declarations, types.len(), span)?;
        let mut inputs = Vec::new();
        let mut unresolved = None;
        for (index, ty) in types.into_iter().enumerate() {
            let declaration = declarations.map(|parameters| &parameters[index]);
            if post_target == Some(index) {
                continue;
            }
            if declaration.is_some_and(|parameter| {
                parameter.ty.is_implicit() || matches!(parameter.ty.kind, TyParamKind::Optional(..))
            }) {
                continue;
            }
            // Reserve complete named packages before asking for their type:
            // they contribute no positional slots, including when generic.
            if declaration.is_some_and(|parameter|
                matches!(parameter.ty.kind, TyParamKind::Varadic(..)) && args.iter()
                    .any(|arg| matches!(arg, Arg::Named(name, _) if name.name == parameter.name.name))) {
                continue;
            }
            if declaration
                .is_some_and(|parameter| matches!(parameter.ty.kind, TyParamKind::Varadic(..)))
            {
                let structure = ty
                    .as_ref()
                    .map(|ty| self.structural_type(ty, span))
                    .transpose()?;
                if let Some(Term::Tuple(fields)) = structure {
                    inputs.extend(fields.into_iter().map(|ty| (Some(ty), stages[index])));
                } else {
                    // One unresolved collection has a unique remainder after
                    // all anchors. Width evidence routes operands only; the
                    // final call still has to infer and check its tuple type.
                    if unresolved.replace((inputs.len(), stages[index])).is_some() {
                        return Ok(None);
                    }
                }
            } else {
                inputs.push((ty, stages[index]));
            }
        }
        if let Some((position, stage)) = unresolved {
            let mut fuel = 10_000usize;
            let mut width = 0usize;
            for argument in args {
                let count = match argument {
                    Arg::Positional(_) => 1,
                    Arg::Expand(value) => {
                        let Some(width) = self.spread_width(value, &mut fuel, 0)? else {
                            return Ok(None);
                        };
                        width
                    }
                    _ => 0,
                };
                fuel = fuel
                    .checked_sub(count)
                    .ok_or_else(|| error(span, "spread input routing budget exhausted"))?;
                width += count;
            }
            let Some(width) = width.checked_sub(inputs.len()) else {
                return Ok(None);
            };
            inputs.splice(
                position..position,
                std::iter::repeat_n((None, stage), width),
            );
        }
        Ok(Some(inputs))
    }

    fn insert_spread_preparation(
        &mut self,
        call: n::Expr,
        spreads: Vec<(usize, n::Expr)>,
        flat_count: usize,
        trailing: usize,
        anchor: n::SourceRange,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let source = call.source;
        let ty = call.ty.clone();
        // Completed live arguments may share a materialization scope around
        // the call. Insert packet producers into the enclosed call plan.
        if let K::Block {
            statements,
            result: Some(result),
        } = call.kind
        {
            let result = self
                .insert_spread_preparation(*result, spreads, flat_count, trailing, anchor, span)?;
            return Ok(n::Expr {
                source,
                ty: result.ty.clone(),
                kind: K::Block {
                    statements,
                    result: Some(Box::new(result)),
                },
            });
        }
        let (mut call, mut statements, prepared) = match call.kind {
            K::CallPreparation {
                statements,
                result: Some(call),
            } => (*call, statements, true),
            K::Call { .. } | K::Variant { .. } => (call, Vec::new(), false),
            _ => return Err(error(span, "spread application requires an ordinary call")),
        };
        let mut leading = Vec::new();
        let stage = |this: &mut Self, value: &mut n::Expr| -> Result<n::Expr, Error> {
            let binding = this.spread_binding(span)?;
            let local = n::Expr {
                ty: value.ty.clone(),
                source: anchor,
                kind: K::Local(binding),
            };
            let value = std::mem::replace(value, local);
            Ok(n::Expr {
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
            })
        };
        if let K::Variant { fields, .. } = &mut call.kind {
            for field in fields {
                statements.push(stage(self, field)?);
            }
        } else if !prepared {
            let K::Call {
                callee, arguments, ..
            } = &mut call.kind
            else {
                unreachable!()
            };
            // An arbitrary callee is evaluated before any explicit input. Keep
            // direct declarations intact so their protocol metadata survives.
            if !matches!(
                callee.kind,
                K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. }
            ) {
                leading.push(stage(self, callee)?);
            } else {
                callee.source = anchor;
            }
            for argument in arguments.iter_mut().filter(|arg| !arg.is_default) {
                statements.push(stage(self, &mut argument.value)?);
            }
        }
        // Collection construction follows every source producer. It is not a
        // receiver prefix and must not shift the spread insertion positions.
        let tail_start = statements
            .len()
            .checked_sub(trailing)
            .ok_or_else(|| error(span, "spread preparation tail is inconsistent"))?;
        let tail = statements.split_off(tail_start);
        let prefix = statements
            .len()
            .checked_sub(flat_count)
            .ok_or_else(|| error(span, "spread preparation input mapping is inconsistent"))?;
        let mut spreads = spreads.into_iter().peekable();
        for (index, statement) in statements.into_iter().enumerate() {
            if index >= prefix {
                while spreads.peek().is_some_and(|(at, _)| *at == index - prefix) {
                    leading.push(spreads.next().unwrap().1);
                }
            }
            leading.push(statement);
        }
        // Empty packets still execute, including a packet after the last input.
        for (at, spread) in spreads {
            if at != flat_count {
                return Err(error(span, "spread preparation position is inconsistent"));
            }
            leading.push(spread);
        }
        leading.extend(tail);
        let ty = if leading.iter().any(|statement| is_never(&statement.ty)) {
            p(P::Never)
        } else {
            ty
        };
        Ok(n::Expr {
            source,
            ty,
            kind: K::CallPreparation {
                statements: leading,
                result: Some(Box::new(call)),
            },
        })
    }
}
