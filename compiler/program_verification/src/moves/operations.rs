//! Expression operations keep large call, closure, and write state off the
//! recursive dispatcher stack. Each helper owns one operation's evaluation;
//! the enclosing expr wrapper still transports result evidence.
use super::*;

impl<F: FnMut(&Term) -> Result<Delivery, String>, G: FnMut(&Term) -> Result<Shape, String>>
    Analyzer<'_, F, G>
{
    #[inline(never)]
    pub(super) fn expr_operation(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<(), Error> {
        let source = expr.source;
        match &expr.kind {
            K::Intrinsic { inputs, .. } => {
                nir::intrinsics::checked(expr).map_err(|_| {
                    error(
                        source,
                        "intrinsic ownership behavior requires a valid target protocol",
                    )
                })?;
                for input in inputs {
                    self.expr(input, flow, depth + 1)?;
                }
            }
            K::AddressOf(place) => {
                flow.memory_exposed = true;
                let target = expression_place(place)
                    .ok_or_else(|| error(source, "address requires a local place"))?;
                flow.state
                    .require_place(&target)
                    .map_err(|e| error(source, format!("address target unavailable: {e:?}")))?;
            }
            K::PointerAnonymousField { .. } | K::PointerField { .. } => {
                self.evaluate_pointer_value(expr, flow, depth + 1)?;
            }
            K::ReadPointer(_) => self.expr_read_pointer(expr, flow, depth)?,
            K::BitWritePointer { .. } => self.expr_bit_write_pointer(expr, flow, depth)?,
            K::WritePointer { pointer, value } => {
                self.write_pointer(pointer, value, flow, source, depth)?;
            }
            K::Local(_) | K::AnonymousField { .. } | K::Field { .. } => {
                let Some(p) = expression_place(expr) else {
                    return self.project_temporary(expr, flow, depth + 1).map(|_| ());
                };
                flow.state.require_present(&p).map_err(|reason| {
                    error(
                        source,
                        if reason == crate::ownership::Error::MissingPlace {
                            format!("missing place or field shape semantics for {p}")
                        } else {
                            format!("use of unavailable value {p}")
                        },
                    )
                })?;
                if (self.delivery)(&expr.ty).map_err(|e| error(source, e))? == Delivery::Move {
                    flow.state
                        .take(&p)
                        .map_err(|e| error(source, format!("move: {e:?}")))?;
                    self.report.transfers.push((source, p.clone()));
                    self.forget_callable_fields(flow, &p, source)?;
                    if p.fields.is_empty() {
                        flow.callables.remove(&p.root);
                    }
                } else {
                    self.report.copies.push((source, p));
                }
            }
            K::AssociatedValue { .. } => self.observe_associated_value(&expr.ty, source)?,
            K::ComptimeValue(_)
            | K::Literal(_)
            | K::TypeValue(_)
            | K::Unit
            | K::Function(_)
            | K::FunctionInstance { .. }
            | K::TraitMethod { .. } => (),
            K::Closure { .. } => self.expr_closure(expr, flow, depth)?,
            K::Let { .. } => {
                self.in_temporary_scope(
                    source,
                    TemporaryBoundary::FullExpression,
                    flow,
                    |this, flow| this.prepared_binding(expr, flow, depth),
                )?;
            }
            K::Match { value, arms } => {
                self.in_temporary_scope(source, TemporaryBoundary::Match, flow, |this, flow| {
                    this.match_arms(value, arms, flow, source, depth + 1)
                        .map(|_| ())
                })?
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => self.in_temporary_scope(
                source,
                TemporaryBoundary::FullExpression,
                flow,
                |this, flow| {
                    this.let_pattern(
                        pattern,
                        value,
                        guard.as_deref(),
                        otherwise.as_deref(),
                        flow,
                        source,
                        depth + 1,
                    )
                },
            )?,
            K::BitAssign { .. } | K::BitUpdate { .. } => self.expr_bit_assign(expr, flow, depth)?,
            K::Assign { .. } => self.expr_assign(expr, flow, depth)?,
            K::Loop { identity, body } => {
                self.loop_body(*identity, body, flow, source, depth + 1)?
            }
            K::Break(identity) => self.loop_edge(*identity, true, flow, source)?,
            K::Continue(identity) => self.loop_edge(*identity, false, flow, source)?,
            K::Return(_) => self.expr_return(expr, flow, depth)?,
            K::Call { .. } => self.expr_call(expr, flow, depth)?,
            K::Tuple(_)
            | K::AnonymousRecord { .. }
            | K::Record { .. }
            | K::OptionalPresent(_)
            | K::OptionalAbsent
            | K::Variant { .. }
            | K::ErrorConvert { .. } => {
                self.evaluate_value_origins(expr, flow, depth + 1)?;
            }
            K::EffectHandle { .. } => {
                self.evaluate_value_origins(expr, flow, depth + 1)?;
            }
            K::EffectResume(value) => {
                let result = self
                    .handler_result
                    .as_ref()
                    .ok_or_else(|| error(source, "resume requires a handler resource context"))?;
                if value.ty != *result && value.ty != Term::Primitive(nir::Primitive::Never) {
                    return Err(error(source, "handler resume result type mismatch"));
                }
                let origins = self.in_temporary_scope(
                    source,
                    TemporaryBoundary::ReturnValue,
                    flow,
                    |this, flow| this.evaluate_value_origins(value, flow, depth + 1),
                )?;
                if flow.live {
                    self.check_return_origins(flow, &origins, source)?;
                    self.capture_handler_completion(flow, &origins, source)?;
                    self.exit(flow, source, CleanupReason::EffectResume)?;
                }
            }
            K::EffectRequest {
                abi,
                signature,
                arguments,
                ..
            } => {
                self.effect_request_resources(expr, *abi, signature, arguments, flow, depth + 1)?;
            }
            K::EffectComputation { .. }
            | K::EffectRun { .. }
            | K::Snapshot(_)
            | K::NewtypeConvert(_) => {
                self.evaluate_value_origins(expr, flow, depth + 1)?;
            }
            K::RequirePredicate(condition)
            | K::EnsurePredicate(condition)
            | K::CheckPredicate(condition) => {
                if expr.ty != Term::Primitive(nir::Primitive::Void)
                    || !nir::program::checkable_predicate(condition, &mut self.fuel)
                        .map_err(|reason| error(condition.source, reason))?
                {
                    return Err(error(source, "invalid runtime predicate check"));
                }
                self.check_contract_predicate(condition, flow, depth + 1)?;
            }
            K::UnsafeScope(value) => self.expr(value, flow, depth + 1)?,
            K::Discard(value) => {
                self.in_temporary_scope(
                    source,
                    TemporaryBoundary::FullExpression,
                    flow,
                    |this, flow| {
                        let origins = this.evaluate_value_origins(value, flow, depth + 1)?;
                        if flow.live {
                            this.temporary(value, origins, flow)?;
                        }
                        Ok(())
                    },
                )?;
            }
            K::EnumName(value) => {
                if let Some(place) = expression_place(value) {
                    flow.state.require_present(&place).map_err(|reason| {
                        error(
                            source,
                            format!("enum name requires an available value: {reason:?}"),
                        )
                    })?;
                } else {
                    let origins = self.evaluate_value_origins(value, flow, depth + 1)?;
                    if flow.live {
                        self.temporary(value, origins, flow)?;
                    }
                }
            }
            K::Not(value)
            | K::IntegerNegate(value)
            | K::FloatNegate(value)
            | K::BitConvert(value)
            | K::TakeBits(value)
            | K::TextQuery { subject: value, .. }
            | K::IntegerRepresentation(value)
            | K::TryIntegerConvert(value)
            | K::ComptimeIntegerConvert(value)
            | K::SymbolName(value)
            | K::TypeKind { operand: value, .. }
            | K::TypeLayout { operand: value, .. }
            | K::BitSlice { value, .. } => self.expr(value, flow, depth + 1)?,
            K::BitWindow { value, start, end } => {
                self.expr(value, flow, depth + 1)?;
                self.expr(start, flow, depth + 1)?;
                self.expr(end, flow, depth + 1)?;
            }
            K::TemplateEqual(left, right) => {
                self.observe_template_identity(left, flow)?;
                self.observe_template_identity(right, flow)?;
            }
            K::IntegerBinary { left, right, .. }
            | K::FloatBinary { left, right, .. }
            | K::ScalarEqual(left, right)
            | K::ScalarLess(left, right)
            | K::ScalarLessEqual(left, right)
            | K::ScalarGreater(left, right)
            | K::ScalarGreaterEqual(left, right)
            | K::BitEqual(left, right)
            | K::BitConcat(left, right)
            | K::BitIndex(left, right) => {
                self.expr(left, flow, depth + 1)?;
                self.expr(right, flow, depth + 1)?;
            }
            K::And(left, right) | K::Or(left, right) => {
                self.expr(left, flow, depth + 1)?;
                if flow.live && expr.ty != Term::Primitive(nir::Primitive::Bool) {
                    // BitVec and/or always evaluate both operands. In particular,
                    // RHS ownership transfers must not become conditional.
                    self.scope(right, flow, depth + 1)?;
                } else if flow.live {
                    let skip = constant_bool(left, 0).map(|value| {
                        if matches!(expr.kind, K::And(..)) {
                            !value
                        } else {
                            value
                        }
                    });
                    match skip {
                        Some(true) => (),
                        Some(false) => self.scope(right, flow, depth + 1)?,
                        None => self.fork(right, None, flow, depth)?,
                    }
                }
            }
            _ => {
                return Err(error(
                    source,
                    "operation requires additional move/observation semantics",
                ));
            }
        }
        Ok(())
    }
    #[inline(never)]
    fn expr_closure(&mut self, expr: &Expr, flow: &mut Flow, depth: usize) -> Result<(), Error> {
        let source = expr.source;
        match &expr.kind {
            K::Closure {
                parameters,
                captures,
                body,
            } => {
                let Term::Function {
                    parameters: parameter_types,
                    ..
                } = expr.ty.callable_signature()
                else {
                    return Err(error(source, "closure lacks a callable signature"));
                };
                if parameters.len() != parameter_types.len() {
                    return Err(error(source, "closure parameter count mismatch"));
                }
                let expected_captures = match &expr.ty {
                    Term::Closure { environment, .. } => environment.as_slice(),
                    Term::Function { .. } | Term::UnsafeFunction(_) => &[],
                    _ => return Err(error(source, "unsupported closure phase")),
                };
                if expected_captures.len() != captures.len() {
                    return Err(error(source, "closure capture count mismatch"));
                }
                let mut capture_origins = BTreeMap::new();
                let mut borrowed_roots = BTreeMap::new();
                let mut capture_guards = ValuePointerGuards::new();
                let mut construction = Callable {
                    choices: vec![],
                    enum_captures: Default::default(),
                    definition: expr.clone(),
                    alternatives: vec![],
                    generation: 0,
                    captures: BTreeMap::new(),
                };
                let mut inner = Flow {
                    pointer_slot_exposures: Some(self.pointer_slot_exposures(expr)?),
                    borrowed_scope: self.fresh_borrowed_scope(source)?,
                    live: true,
                    ..Default::default()
                };
                self.pending_frame(source, |this| {
                    for (slot, (capture, expected)) in
                        captures.iter().zip(expected_captures).enumerate()
                    {
                        if &capture.ty != expected {
                            return Err(error(source, "closure capture type mismatch"));
                        }
                        let lexical;
                        let value = if let Some(value) = &capture.value {
                            value.as_ref()
                        } else {
                            lexical = Expr {
                                source,
                                ty: capture.ty.clone(),
                                kind: K::Local(capture.binding),
                            };
                            &lexical
                        };
                        let callable = callable_source(value, flow, &mut this.fuel)?;
                        // Sample each capture at its own evaluation point. Earlier
                        // initializers may legitimately replace a later source.
                        let origins = this.evaluate_value_origins(value, flow, depth + 1)?;
                        if !flow.live {
                            return Ok(());
                        }
                        for (mut path, mut entries) in
                            std::mem::take(&mut flow.value_pointer_guards)
                        {
                            this.fuel = this
                                .fuel
                                .checked_sub(path.len().saturating_add(1))
                                .ok_or_else(|| {
                                    error(source, "capture pointer guard budget exhausted")
                                })?;
                            path.insert(0, slot);
                            for conditions in entries.values_mut() {
                                this.charge_pointer_conditions(conditions, source)?;
                                for (path, _) in conditions {
                                    path.insert(0, slot);
                                }
                            }
                            capture_guards.insert(path, entries);
                        }
                        let callable = flow.value_callable.clone().or(callable);
                        let fields = flow.value_callables.clone();
                        let domains =
                            copy_enum_domains(&flow.value_variants, &mut this.fuel, source)?;
                        if !domains.is_empty() {
                            construction
                                .enum_captures
                                .insert(root(capture.binding), domains.clone());
                        }

                        construction
                            .captures
                            .retain(|place, _| place.root != root(capture.binding));
                        if let Some(value) = &callable {
                            construction.captures.insert(
                                Place {
                                    root: root(capture.binding),
                                    fields: vec![],
                                },
                                value.clone(),
                            );
                        }
                        for (path, value) in fields {
                            if !path.is_empty() {
                                this.fuel = this
                                    .fuel
                                    .checked_sub(path.len().saturating_add(1))
                                    .ok_or_else(|| {
                                        error(source, "capture field budget exhausted")
                                    })?;
                                construction.captures.insert(
                                    Place {
                                        root: root(capture.binding),
                                        fields: path,
                                    },
                                    value,
                                );
                            }
                        }
                        for (path, targets) in &origins {
                            this.fuel = this
                                .fuel
                                .checked_sub(path.len().saturating_add(1))
                                .ok_or_else(|| {
                                    error(source, "closure capture origin budget exhausted")
                                })?;
                            this.pointer_work(targets, source)?;
                            let mut captured = vec![slot];
                            captured.extend(path);
                            capture_origins.insert(captured, targets.clone());
                        }
                        let input_origins = this.memory_input_targets(
                            &capture.ty,
                            &origins,
                            &flow.value_variants,
                            source,
                        )?;
                        this.remember_pending_origins(value, slot, flow, origins)?;
                        this.declare(&mut inner, capture.binding, &capture.ty, source)?;
                        let fields = this.declare_input_field_borrows(
                            &mut inner,
                            capture.binding,
                            &capture.ty,
                            &input_origins,
                            source,
                        )?;
                        for (path, targets) in fields {
                            for target in targets {
                                let pointee = inner.types.get(&target.root).ok_or_else(|| {
                                    error(source, "closure captured field type missing")
                                })?;
                                crate::memory::bound_type(pointee, &mut this.fuel)
                                    .map_err(|reason| error(source, reason))?;
                                borrowed_roots.insert(
                                    target.root,
                                    BorrowedRoot::Parameter {
                                        parameter: parameters.len() + slot,
                                        input_fields: path.clone(),
                                        pointee: pointee.clone(),
                                    },
                                );
                            }
                        }
                        if let Some((_, subject)) = crate::memory::pointer_representation(
                            this.package,
                            &capture.ty,
                            &mut this.fuel,
                        )
                        .map_err(|reason| error(source, reason))?
                        {
                            this.declare_closure_borrow(
                                &mut inner,
                                parameters.len() + slot,
                                capture.binding,
                                &subject,
                                source,
                            )?;
                            borrowed_roots.insert(
                                PlaceId {
                                    owner: u64::MAX - 1,
                                    local: u32::try_from(parameters.len() + slot).map_err(
                                        |_| error(source, "too many closure pointer parameters"),
                                    )?,
                                },
                                BorrowedRoot::Parameter {
                                    parameter: parameters.len() + slot,
                                    input_fields: vec![],
                                    pointee: subject,
                                },
                            );
                        }
                        this.install_enum_domains(
                            &mut inner,
                            &Place {
                                root: root(capture.binding),
                                fields: vec![],
                            },
                            domains,
                            source,
                        )?;
                        if let Some(callable) = callable {
                            this.store_callable(
                                &mut inner,
                                root(capture.binding),
                                callable,
                                source,
                            )?;
                        }
                        for (place, evidence) in &construction.captures {
                            if place.root == root(capture.binding) && !place.fields.is_empty() {
                                inner
                                    .callable_fields
                                    .insert(place.clone(), evidence.clone());
                            }
                        }
                    }
                    Ok(())
                })?;
                if !flow.live {
                    return Ok(());
                }
                let capture_roots = inner.roots.clone();
                for (slot, (binding, parameter)) in
                    parameters.iter().zip(parameter_types).enumerate()
                {
                    self.declare(&mut inner, *binding, &parameter.ty, source)?;
                    if let Some((_, subject)) = crate::memory::pointer_representation(
                        self.package,
                        &parameter.ty,
                        &mut self.fuel,
                    )
                    .map_err(|reason| error(source, reason))?
                    {
                        self.declare_closure_borrow(&mut inner, slot, *binding, &subject, source)?;
                        borrowed_roots.insert(
                            PlaceId {
                                owner: u64::MAX - 1,
                                local: u32::try_from(slot).map_err(|_| {
                                    error(source, "too many closure pointer parameters")
                                })?,
                            },
                            BorrowedRoot::Parameter {
                                parameter: slot,
                                input_fields: vec![],
                                pointee: subject,
                            },
                        );
                    }
                    if self.package.is_some()
                        && self.storage_inputs_are_closed(&parameter.ty, source)?
                    {
                        // Formation has no concrete arguments. Assume each input
                        // field conditionally; invocation still establishes the
                        // actual targets and checks the full memory footprint.
                        let layout = crate::memory::pointer_input_layout(
                            self.package,
                            &parameter.ty,
                            &mut self.fuel,
                        )
                        .map_err(|reason| error(source, reason))?;
                        let premises = layout
                            .into_iter()
                            .filter(|leaf| !leaf.input_fields.is_empty())
                            .map(|leaf| (leaf.input_fields, BTreeSet::new()))
                            .collect();
                        let fields = self.declare_input_field_borrows(
                            &mut inner,
                            *binding,
                            &parameter.ty,
                            &premises,
                            source,
                        )?;
                        for (path, targets) in fields {
                            for target in targets {
                                let pointee = inner.types.get(&target.root).ok_or_else(|| {
                                    error(source, "closure parameter field type missing")
                                })?;
                                crate::memory::bound_type(pointee, &mut self.fuel)
                                    .map_err(|reason| error(source, reason))?;
                                borrowed_roots.insert(
                                    target.root,
                                    BorrowedRoot::Parameter {
                                        parameter: slot,
                                        input_fields: path.clone(),
                                        pointee: pointee.clone(),
                                    },
                                );
                            }
                        }
                    }
                }
                // Analyze a possible invocation independently from construction.
                // Its surviving environment remains stored; return only cleans
                // invocation-local responsibilities, never retained captures.
                self.conditional_handler_borrows(&mut inner, &borrowed_roots, source)?;
                let outer_report = std::mem::take(&mut self.report);
                // Formation uses the same conditional provenance as invocation.
                // It is evidence for observations, never ownership of pointees.
                self.report.borrowed_roots = borrowed_roots;
                let outer_captured_callables = std::mem::replace(
                    &mut self.captured_roots,
                    captures.iter().map(|c| root(c.binding)).collect(),
                );
                let outer_persistent =
                    std::mem::replace(&mut self.persistent_roots, capture_roots.clone());
                let outer_pending = std::mem::take(&mut self.pending);
                let outer_loops = std::mem::take(&mut self.loops);
                let outer_contract = self.exit_contract.take();
                let outer_handler = self.handler_result.take();
                let outer_effect_return = std::mem::replace(&mut self.effect_return_enabled, false);
                let outer_return_values = std::mem::take(&mut self.effect_return_values);
                let outer_effect_loops = std::mem::take(&mut self.effect_loop_targets);
                let outer_effect_memory = std::mem::take(&mut self.effect_memory_inputs);
                let outer_effect_results = std::mem::take(&mut self.effect_result_inputs);
                let outer_completions = std::mem::take(&mut self.handler_completions);
                let outer_defer = std::mem::replace(&mut self.defer_unknown_calls, true);
                let result = self.in_temporary_scope(
                    body.source,
                    TemporaryBoundary::FunctionResult,
                    &mut inner,
                    |this, inner| this.expr(body, inner, depth + 1),
                );
                self.defer_unknown_calls = outer_defer;
                self.loops = outer_loops;
                self.exit_contract = outer_contract;
                self.handler_result = outer_handler;
                self.effect_return_enabled = outer_effect_return;
                self.effect_return_values = outer_return_values;
                self.effect_loop_targets = outer_effect_loops;
                self.effect_memory_inputs = outer_effect_memory;
                self.effect_result_inputs = outer_effect_results;
                self.handler_completions = outer_completions;
                let result = result.and_then(|()| {
                    if inner.live {
                        self.exit(&mut inner, source, CleanupReason::FunctionEnd)
                    } else {
                        Ok(())
                    }
                });
                self.pending = outer_pending;
                let invocation = std::mem::replace(&mut self.report, outer_report);
                self.persistent_roots = outer_persistent;
                self.captured_roots = outer_captured_callables;
                result?;
                self.report.closures.push(ClosureReport {
                    invoked: false,
                    definition: expr.clone(),
                    source,
                    captures: captures
                        .iter()
                        .map(|capture| root(capture.binding))
                        .collect(),
                    invocation,
                });
                // Capture initializer code leaves are not fields of the newly
                // constructed callable; its environment has separate evidence.
                flow.value_callables.clear();
                flow.value_variants =
                    self.capture_storage_domains(captures, &construction.enum_captures, source)?;
                flow.value_callable = Some(construction);
                flow.constructed_capture_origins = capture_origins;
                flow.value_pointer_guards = capture_guards;
            }
            _ => unreachable!("expr_closure dispatcher"),
        }
        Ok(())
    }
    #[inline(never)]
    fn expr_call(&mut self, expr: &Expr, flow: &mut Flow, depth: usize) -> Result<(), Error> {
        let source = expr.source;
        match &expr.kind {
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } => {
                if self.defer_unknown_calls
                    && !matches!(callee.kind, K::Local(_))
                    && expression_place(callee).is_some()
                    && callable_source(callee, flow, &mut self.fuel)?.is_none()
                {
                    self.expr(callee, flow, depth + 1)?;
                    self.prepare_values(
                        source,
                        arguments.iter().map(|a| (a.parameter, &a.value)),
                        flow,
                        depth + 1,
                    )?;
                    self.report.deferred_calls.push(source);
                    flow.value_callable = None;
                    flow.value_callables.clear();
                    flow.value_variants.clear();
                    return Ok(());
                }
                if !matches!(
                    callee.kind,
                    K::Local(_)
                        | K::Function(_)
                        | K::FunctionInstance { .. }
                        | K::TraitMethod { .. }
                ) {
                    return self
                        .call_temporary(
                            expr,
                            callee,
                            arguments,
                            parameter_bindings,
                            flow,
                            depth + 1,
                        )
                        .map(|_| ());
                }
                if let K::Local(binding) = callee.kind
                    && let Some(callable) = flow
                        .callables
                        .get(&root(binding))
                        .filter(|c| !c.alternatives.is_empty())
                        .cloned()
                {
                    self.observe_named_callee(callee, flow)?;
                    if !flow.live {
                        return Ok(());
                    }
                    return self
                        .call_named_candidates(
                            expr,
                            &callable,
                            arguments,
                            parameter_bindings,
                            flow,
                            depth + 1,
                        )
                        .map(|_| ());
                }
                if let Some(resolved) = self.indirect_named_call(expr, flow) {
                    self.observe_named_callee(callee, flow)?;
                    if !flow.live {
                        return Ok(());
                    }
                    return self.expr(&resolved, flow, depth + 1);
                }
                if let K::Local(binding) = callee.kind
                    && let Some(callable) = flow
                        .callables
                        .get(&root(binding))
                        .filter(|c| matches!(c.definition.kind, K::Closure { .. }))
                        .cloned()
                {
                    return self
                        .call_closure(root(binding), &callable, arguments, flow, source, depth + 1)
                        .map(|_| ());
                }
                let mut pointer_arguments = false;
                for argument in arguments {
                    // An opaque aggregate local may already carry pointers even
                    // when evaluating it contains no address-taking expression.
                    if self.package.is_some()
                        && !matches!(
                            argument.value.ty,
                            Term::Function { .. }
                                | Term::UnsafeFunction(_)
                                | Term::Closure { .. }
                                | Term::Rigid { .. }
                                | Term::Bound { .. }
                                | Term::Hole { .. }
                                | Term::Runtime(_)
                                | Term::AssociatedType { .. }
                        )
                        && self.storage_inputs_are_closed(&argument.value.ty, source)?
                    {
                        pointer_arguments |= !crate::memory::pointer_input_layout(
                            self.package,
                            &argument.value.ty,
                            &mut self.fuel,
                        )
                        .map_err(|reason| error(source, reason))?
                        .is_empty();
                    }
                    // A pointer constructed inside a record/block is just as
                    // observable as a pointer-valued root argument. Inspect
                    // evaluation structure without evaluating it twice or
                    // treating deferred closure bodies as argument effects.
                    nir::visit::expression_in_scope(
                        &argument.value,
                        &mut self.fuel,
                        |node, fuel| {
                            if !pointer_arguments {
                                pointer_arguments = crate::memory::pointer_representation(
                                    self.package,
                                    &node.ty,
                                    fuel,
                                )?
                                .is_some();
                            }
                            Ok(())
                        },
                    )
                    .map_err(|reason| error(source, reason))?;
                }
                if matches!(callee.kind, K::TraitMethod { .. })
                    && (flow.memory_exposed || pointer_arguments)
                {
                    let package = self.package.ok_or_else(|| {
                        error(source, "method memory analysis requires a package")
                    })?;
                    let selected = self
                        .method_callees
                        .select(package, callee, &mut self.fuel)
                        .map_err(|reason| error(source, reason))?;
                    let resolved = Expr {
                        source,
                        ty: expr.ty.clone(),
                        kind: K::Call {
                            callee: Box::new(selected),
                            arguments: arguments.clone(),
                            parameter_bindings: parameter_bindings.clone(),
                        },
                    };
                    return self.expr(&resolved, flow, depth + 1);
                }
                if flow.memory_exposed || pointer_arguments {
                    if matches!(callee.kind, K::Function(_) | K::FunctionInstance { .. }) {
                        let mut prepared_callables = vec![];
                        let mut prepared_enums = vec![];
                        let mut prepared_borrowed = vec![];
                        let result = self
                            .memory_call(
                                callee,
                                arguments,
                                parameter_bindings,
                                &[],
                                &mut prepared_callables,
                                &mut prepared_enums,
                                &mut prepared_borrowed,
                                flow,
                                source,
                                depth + 1,
                            )
                            .map(|_| ());
                        result?;
                        flow.value_callable = if flow.live {
                            self.record_named_call(
                                callee,
                                prepared_callables,
                                prepared_enums,
                                &prepared_borrowed,
                                source,
                                flow,
                            )?
                        } else {
                            None
                        };
                        return Ok(());
                    }
                }
                if self.defer_unknown_calls
                    && matches!(callee.kind, K::Local(_))
                    && matches!(
                        callee.ty.callable_signature(),
                        Term::Function { .. } | Term::UnsafeFunction(_)
                    )
                {
                    // Construction does not execute this body. Defer the unknown
                    // callback's state transform; actual calls are reanalyzed
                    // with argument provenance and cannot use this evidence.
                    self.prepare_values(
                        source,
                        arguments.iter().map(|a| (a.parameter, &a.value)),
                        flow,
                        depth + 1,
                    )?;
                    self.report.deferred_calls.push(source);
                    flow.value_callable = None;
                    flow.value_callables.clear();
                    flow.value_variants.clear();
                    return Ok(());
                }
                if flow.memory_exposed || pointer_arguments {
                    return Err(error(
                        source,
                        "call with pointer dependencies requires a memory contract",
                    ));
                }
                if let K::Local(binding) = &callee.kind {
                    if let Some(definition) = flow.callables.get(&root(*binding)).cloned() {
                        return self
                            .call_closure(
                                root(*binding),
                                &definition,
                                arguments,
                                flow,
                                source,
                                depth + 1,
                            )
                            .map(|_| ());
                    }
                    // Moving a closure removes its callable evidence. Report
                    // the unavailable owner before diagnosing unknown code;
                    // known, partially consumed closures retain their own
                    // per-capture invocation checks above.
                    let place = Place {
                        root: root(*binding),
                        fields: vec![],
                    };
                    flow.state.require_present(&place).map_err(|reason| {
                        error(
                            source,
                            format!("call of unavailable value {place}: {reason:?}"),
                        )
                    })?;
                }
                if matches!(callee.ty, Term::Closure { .. })
                    || matches!(callee.kind, K::Closure { .. })
                {
                    return Err(error(
                        source,
                        "closure environment state transfer requires a known live local closure",
                    ));
                }
                self.expr(callee, flow, depth + 1)?;
                if !flow.live {
                    return Ok(());
                }
                if arguments.iter().any(|a| a.is_default) {
                    self.prepare_defaults(
                        callee,
                        arguments,
                        parameter_bindings,
                        flow,
                        source,
                        depth + 1,
                    )?;
                } else {
                    let mut callables = vec![];
                    let mut enums = vec![];
                    self.pending_frame(source, |this| {
                        for argument in arguments {
                            let callable = callable_source(&argument.value, flow, &mut this.fuel)?;
                            let origins =
                                this.evaluate_value_origins(&argument.value, flow, depth + 1)?;
                            let callable = flow.value_callable.clone().or(callable);
                            if !flow.live {
                                return Ok(());
                            }
                            record_enum_parameter(
                                &mut enums,
                                argument.parameter,
                                &flow.value_variants,
                                &mut this.fuel,
                                source,
                            )?;
                            this.remember_pending_origins(
                                &argument.value,
                                argument.parameter,
                                flow,
                                origins,
                            )?;
                            if let Some(callable) = callable {
                                callables.push(export_callable_parameter(
                                    argument.parameter,
                                    &callable,
                                    &mut this.fuel,
                                )?);
                            }
                            for (path, value) in &flow.value_callables {
                                if !path.is_empty() {
                                    let mut input = export_callable_parameter(
                                        argument.parameter,
                                        value,
                                        &mut this.fuel,
                                    )?;
                                    input.fields = path.clone();
                                    callables.push(input);
                                }
                            }
                        }
                        this.check_interface_preparation(callee, flow, source)
                    })?;
                    if flow.live {
                        flow.value_callable =
                            self.record_named_call(callee, callables, enums, &[], source, flow)?;
                    }
                }
                if flow.live
                    && self
                        .call_may_reject(callee)
                        .map_err(|reason| error(source, reason))?
                {
                    // Complete arguments already belong to the callee. Only
                    // caller-owned positions and enclosing preparation survive.
                    let mut failed = flow.clone();
                    self.exit(&mut failed, source, CleanupReason::PredicateFailure)?;
                }
                if flow.live && flow.memory_exposed {
                    return Err(error(
                        source,
                        "call with pointer dependencies requires a memory contract",
                    ));
                }
                self.check_interface_result(callee, expr, flow)?;
            }
            _ => unreachable!("expr_call dispatcher"),
        }
        Ok(())
    }
    #[inline(never)]
    fn expr_read_pointer(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<(), Error> {
        let source = expr.source;
        match &expr.kind {
            K::ReadPointer(pointer) => {
                if !crate::memory::pointer_input_layout(self.package, &expr.ty, &mut self.fuel)
                    .map_err(|reason| error(source, reason))?
                    .is_empty()
                    && crate::memory::pointer_representation(self.package, &expr.ty, &mut self.fuel)
                        .map_err(|reason| error(source, reason))?
                        .is_none()
                {
                    self.read_aggregate_origins(expr, pointer, flow, depth + 1)?;
                    return Ok(());
                }
                let target = self.evaluate_pointer_value(pointer, flow, depth + 1)?;
                if !flow.live {
                    return Ok(());
                }
                let target = target
                    .ok_or_else(|| error(source, "pointer target provenance is not established"))?;
                if !crate::memory::readable_representation(self.package, &expr.ty, &mut self.fuel)
                    .map_err(|reason| error(source, reason))?
                {
                    return Err(error(
                        source,
                        "owned pointer read requires a memory transfer contract",
                    ));
                }
                if (self.delivery)(&expr.ty).map_err(|e| error(source, e))? != Delivery::Copy {
                    return Err(error(
                        source,
                        "pointer read requires checked Copy permission",
                    ));
                }
                let mut domains = None;
                let mut code = None;
                for target in &target {
                    flow.state.require_present(target).map_err(|e| {
                        error(source, format!("pointer read target unavailable: {e:?}"))
                    })?;
                    let next = self.stored_enum_domains(flow, target, source)?;
                    let next_code = self.stored_code(flow, target, source)?;
                    code = Some(if let Some(previous) = code {
                        join_guarded_callables(
                            &previous,
                            &next_code,
                            domains.as_ref().unwrap(),
                            &next,
                            &mut self.fuel,
                            source,
                        )?
                    } else {
                        next_code
                    });
                    domains = Some(if let Some(previous) = domains {
                        join_enum_domains(&previous, &next, &mut self.fuel, source)?
                    } else {
                        next
                    });
                }
                flow.value_variants = domains.unwrap_or_default();
                flow.value_callables = code.unwrap_or_default();
                flow.value_callable = flow.value_callables.get(&vec![]).cloned();
            }
            _ => unreachable!("expr_read_pointer dispatcher"),
        }
        Ok(())
    }
    #[inline(never)]
    fn expr_bit_write_pointer(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<(), Error> {
        let source = expr.source;
        match &expr.kind {
            K::BitWritePointer {
                pointer,
                selections,
                value,
            } => {
                let Term::Pointer(storage) = &pointer.ty else {
                    return Err(error(source, "bit writeback requires a pointer"));
                };
                self.fuel = self
                    .fuel
                    .checked_sub(selections.len())
                    .ok_or_else(|| error(source, "bit selection ownership budget exhausted"))?;
                if let Some(reason) = nir::program::bit_update_error_in(
                    storage,
                    selections,
                    &value.ty,
                    &expr.ty,
                    self.package.and_then(|package| package.target),
                ) {
                    return Err(error(source, reason));
                }
                self.in_temporary_scope(
                    source,
                    TemporaryBoundary::FullExpression,
                    flow,
                    |this, flow| {
                        let target = this.evaluate_pointer_value(pointer, flow, depth + 1)?;
                        if !flow.live {
                            return Ok(());
                        }
                        let targets = target.ok_or_else(|| {
                            error(source, "pointer target provenance is not established")
                        })?;
                        this.check_observation_write_targets(&targets, source)?;
                        for input in selections
                            .iter()
                            .flat_map(nir::program::BitSelection::expressions)
                        {
                            this.expr(input, flow, depth + 1)?;
                        }
                        this.expr(value, flow, depth + 1)?;
                        if flow.live {
                            for target in targets {
                                flow.state.require_present(&target).map_err(|reason| {
                                    error(
                                        source,
                                        format!("bit writeback target unavailable: {reason:?}"),
                                    )
                                })?;
                            }
                        }
                        Ok(())
                    },
                )?;
            }
            _ => unreachable!("expr_bit_write_pointer dispatcher"),
        }
        Ok(())
    }
    #[inline(never)]
    fn expr_bit_assign(&mut self, expr: &Expr, flow: &mut Flow, depth: usize) -> Result<(), Error> {
        let source = expr.source;
        match &expr.kind {
            K::BitAssign {
                place: target,
                storage_type,
                value,
                ..
            }
            | K::BitUpdate {
                place: target,
                storage_type,
                value,
                ..
            } => {
                let reason = match &expr.kind {
                    K::BitAssign { start, end, .. } => nir::program::bit_writeback_error_in(
                        storage_type,
                        *start,
                        *end,
                        &value.ty,
                        &expr.ty,
                        self.package.and_then(|package| package.target),
                    ),
                    K::BitUpdate { selections, .. } => {
                        self.fuel = self.fuel.checked_sub(selections.len()).ok_or_else(|| {
                            error(source, "bit selection ownership budget exhausted")
                        })?;
                        nir::program::bit_update_error_in(
                            storage_type,
                            selections,
                            &value.ty,
                            &expr.ty,
                            self.package.and_then(|package| package.target),
                        )
                    }
                    _ => unreachable!(),
                };
                if let Some(reason) = reason {
                    return Err(error(source, reason));
                }
                let mut actual = flow
                    .types
                    .get(&root(target.root))
                    .cloned()
                    .ok_or_else(|| error(source, "bit writeback root is not in scope"))?;
                for (declaration, index) in &target.fields {
                    if !match (&actual, declaration) {
                        (
                            Term::Nominal {
                                declaration: owner, ..
                            },
                            nir::program::FieldOwner::Nominal(id),
                        ) => owner == id,
                        (Term::Record(_), nir::program::FieldOwner::Anonymous) => true,
                        _ => false,
                    } {
                        return Err(error(source, "bit writeback field declaration mismatch"));
                    }
                    actual = self.field_type(&actual, &[*index], source)?;
                }
                if actual != *storage_type {
                    return Err(error(source, "bit writeback storage type mismatch"));
                }
                let destination = Place {
                    root: root(target.root),
                    fields: target.fields.iter().map(|(_, index)| *index).collect(),
                };
                self.in_temporary_scope(
                    source,
                    TemporaryBoundary::FullExpression,
                    flow,
                    |this, flow| {
                        if let K::BitUpdate { selections, .. } = &expr.kind {
                            for input in selections
                                .iter()
                                .flat_map(nir::program::BitSelection::expressions)
                            {
                                this.expr(input, flow, depth + 1)?;
                            }
                        }
                        // Preserved bits are read after the RHS finishes. A RHS may
                        // replace the destination, move its owner, or exit entirely.
                        this.expr(value, flow, depth + 1)?;
                        if flow.live {
                            flow.state.require_present(&destination).map_err(|reason| {
                                error(
                                    source,
                                    format!("bit writeback target unavailable: {reason:?}"),
                                )
                            })?;
                        }
                        // This updates scalar contents only. It neither initializes
                        // an absent owner nor replaces ownership/alias identities.
                        Ok(())
                    },
                )?;
            }
            _ => unreachable!("expr_bit_assign dispatcher"),
        }
        Ok(())
    }
    #[inline(never)]
    fn expr_assign(&mut self, expr: &Expr, flow: &mut Flow, depth: usize) -> Result<(), Error> {
        let source = expr.source;
        match &expr.kind {
            K::Assign {
                place: target,
                value,
            } => {
                if self.captured_roots.contains(&root(target.root))
                    && !matches!(
                        value.ty,
                        Term::Function { .. } | Term::UnsafeFunction(_) | Term::Closure { .. }
                    )
                    && !crate::memory::capture_pointer_replacement_layout(
                        self.package,
                        &value.ty,
                        &mut self.fuel,
                    )
                    .map_err(|reason| {
                        error(
                            source,
                            format!(
                                "captured write requires an established memory contract: {reason}"
                            ),
                        )
                    })?
                {
                    crate::memory::require_pointer_free_owned(
                        self.package,
                        &value.ty,
                        &mut self.fuel,
                        0,
                    )
                    .map_err(|reason| {
                        error(
                            source,
                            format!(
                                "captured write requires an established memory contract: {reason}"
                            ),
                        )
                    })?;
                }

                let callable = callable_source(value, flow, &mut self.fuel)?;
                // RHS completes before retiring the previous destination. A
                // self-move therefore does not schedule destruction of the RHS.
                self.in_temporary_scope(
                    source,
                    TemporaryBoundary::FullExpression,
                    flow,
                    |this, flow| {
                        let origins = this.evaluate_value_origins(value, flow, depth + 1)?;
                        let pointer_guards =
                            this.copy_value_pointer_guards(&flow.value_pointer_guards, source)?;
                        let callable = flow.value_callable.clone().or(callable);
                        if flow.live {
                            if target.fields.is_empty() {
                                flow.callables.remove(&root(target.root));
                                if let Some(callable) = callable {
                                    this.store_callable(flow, root(target.root), callable, source)?;
                                }
                            }
                            let destination = Place {
                                root: root(target.root),
                                fields: target.fields.iter().map(|(_, index)| *index).collect(),
                            };
                            let (variants, domains) =
                                this.cleanup_variants(flow, &destination, source)?;
                            if let Some(action) = this.owned_action(
                                &flow.state,
                                destination.clone(),
                                &value.ty,
                                crate::drop::PointerMemory {
                                    types: &flow.types,
                                    slots: &flow.pointers,
                                    enum_domains: &flow.enum_domains,
                                    pointer_conditions: &flow.pointer_guards,
                                    retired: &[],
                                },
                                &variants,
                                &domains,
                                source,
                            )? {
                                this.report.cleanup_sites.push(CleanupSite {
                                    default_origin: None,
                                    result_origin: None,
                                    interface_precondition: false,
                                    interface_postcondition: false,
                                    source,
                                    reason: CleanupReason::Replacement,
                                    actions: vec![action],
                                });
                            }
                            let leaves = flow.value_callables.clone();
                            this.install_callable_fields(flow, &destination, leaves, source)?;
                            this.install_enum_domains(
                                flow,
                                &destination,
                                flow.value_variants.clone(),
                                source,
                            )?;
                            this.install_value_origins(flow, &destination, origins, source)?;
                            this.install_pointer_guards(
                                flow,
                                &destination,
                                pointer_guards,
                                source,
                            )?;
                            for (place, availability) in flow
                                .state
                                .replace_with_budget(&destination, &mut this.fuel)
                                .map_err(|e| error(source, format!("assignment: {e:?}")))?
                                .into_iter()
                                .rev()
                            {
                                this.report.cleanups.push(Cleanup {
                                    source,
                                    place,
                                    availability,
                                });
                            }
                        }
                        Ok(())
                    },
                )?;
            }
            _ => unreachable!("expr_assign dispatcher"),
        }
        Ok(())
    }
    #[inline(never)]
    fn expr_return(&mut self, expr: &Expr, flow: &mut Flow, depth: usize) -> Result<(), Error> {
        let source = expr.source;
        match &expr.kind {
            K::Return(value) => {
                let origins = if let Some(value) = value {
                    self.in_temporary_scope(
                        source,
                        TemporaryBoundary::ReturnValue,
                        flow,
                        |this, flow| this.evaluate_value_origins(value, flow, depth + 1),
                    )?
                } else {
                    BTreeMap::new()
                };
                if flow.live {
                    let result_callable = flow.value_callable.clone();
                    let result_fields = flow.value_callables.clone();
                    let result_variants = flow.value_variants.clone();
                    if self.handler_result.is_some() {
                        if let Some(value) = value {
                            self.check_handler_return_layout(&value.ty, flow, &origins, source)?;
                        }
                        self.check_return_origins(flow, &origins, source)?;
                        self.check_handler_return_origins(flow, &origins, source)?;
                        let callable = flow
                            .value_callable
                            .as_ref()
                            .map(|value| export_callable_evidence(value, &mut self.fuel, 0))
                            .transpose()?;
                        let mut fields = BTreeMap::new();
                        for (path, value) in &flow.value_callables {
                            if path.is_empty() {
                                continue;
                            }
                            self.fuel = self
                                .fuel
                                .checked_sub(path.len().saturating_add(1))
                                .ok_or_else(|| {
                                    error(source, "handler return field budget exhausted")
                                })?;
                            fields.insert(
                                path.clone(),
                                export_callable_evidence(value, &mut self.fuel, 0)?,
                            );
                        }
                        let variants =
                            copy_enum_domains(&flow.value_variants, &mut self.fuel, source)?;
                        let pointer_guards =
                            self.copy_value_pointer_guards(&flow.value_pointer_guards, source)?;
                        self.report
                            .lexical_returns
                            .push(std::sync::Arc::new(HandlerReturnValue {
                                source,
                                origins,
                                pointer_guards,
                                callable,
                                fields,
                                variants,
                            }));
                    } else {
                        self.check_function_result(flow, origins, source, depth + 1)?;
                    }
                    if flow.live {
                        flow.value_callable = result_callable;
                        flow.value_callables = result_fields;
                        flow.value_variants = result_variants;
                    }
                }
                if flow.live {
                    self.exit(flow, source, CleanupReason::Return)?;
                }
            }
            _ => unreachable!("expr_return dispatcher"),
        }
        Ok(())
    }
}
