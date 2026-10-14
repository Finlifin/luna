//! Owned effect inputs and the suspended invocation's full-frame exit plan.
//! Direct handler arms have independent input and cleanup scopes. Scalar Copy
//! captures remain owned by the installer and cannot expose their addresses;
//! other lexical captures still require request-time ownership evidence.
use super::*;

impl<'a, F: FnMut(&Term) -> Result<Delivery, String>, G: FnMut(&Term) -> Result<Shape, String>>
    Analyzer<'a, F, G>
{
    pub(super) fn effect_escape_checkpoint(
        &mut self, flow: &Flow, source: SourceRange,
    ) -> Result<(), Error> {
        if !flow.live { return Ok(()); }
        let mut escaped = flow.clone();
        let locals = escaped.order.iter()
            .filter(|id| !self.persistent_roots.contains(id)).copied().collect();
        // This is a conditional full-frame unwind, not a normal function result
        // or a failed program predicate. In particular it must not participate
        // in joining the callable/tag evidence of successful closure returns.
        self.cleanup_exit(&mut escaped, locals, 0, 0, source, CleanupReason::EffectEscape)?;
        self.incomplete_exit(source, 0);
        self.report.effect_exits.push((source, escaped.state));
        self.effect_loop_checkpoints(flow, source)?;
        if self.effect_return_enabled && self.handler_result.is_none() {
            self.effect_return_checkpoint(flow, source, 1)?;
        }
        Ok(())
    }

    pub(super) fn effect_loop_checkpoints(&mut self, flow: &Flow, source: SourceRange) -> Result<(), Error> {
        self.fuel = self.fuel.checked_sub(self.effect_loop_targets.len())
            .ok_or_else(||error(source,"effect loop checkpoint budget exhausted"))?;
        for (target, is_break) in self.effect_loop_targets.clone() {
            if !self.loops.iter().any(|frame|frame.identity == target) {
                if self.handler_result.is_some() { continue; }
                return Err(error(source,"effect loop target is not active in its lexical installer"));
            }
            let mut edge = flow.clone();
            let first = self.report.cleanup_sites.len();
            self.loop_edge(target, is_break, &mut edge, source)?;
            for site in &mut self.report.cleanup_sites[first..] {
                site.reason = if is_break { CleanupReason::EffectBreak(target) } else { CleanupReason::EffectContinue(target) };
            }
        }
        Ok(())
    }

    pub(super) fn effect_return_checkpoint(
        &mut self, flow: &Flow, source: SourceRange, depth: usize,
    ) -> Result<(), Error> {
        if !flow.live { return Ok(()); }
        let mut returned = flow.clone();
        // Restore each handler result independently, then join candidates and
        // guarded aggregate leaves just as for ordinary function return paths.
        let mut first = true;
        returned.value_callable = None;
        returned.value_callables.clear();
        returned.value_variants.clear();
        self.fuel = self.fuel.checked_sub(self.effect_return_values.len())
            .ok_or_else(||error(source,"handler return checkpoint budget exhausted"))?;
        for value in &self.effect_return_values {
            let callable = value.callable.as_ref()
                .map(|evidence|restore_callable_evidence(evidence, &mut self.fuel, 0)).transpose()?;
            let mut fields = BTreeMap::new();
            for (path, evidence) in &value.fields {
                self.fuel = self.fuel.checked_sub(path.len().saturating_add(1))
                    .ok_or_else(||error(source,"handler return field budget exhausted"))?;
                fields.insert(path.clone(),restore_callable_evidence(evidence, &mut self.fuel, 0)?);
            }
            if first {
                returned.value_callable = callable;
                returned.value_callables = fields;
                returned.value_variants = copy_enum_domains(&value.variants, &mut self.fuel, source)?;
                first = false;
            } else {
                returned.value_callable = match (&returned.value_callable, callable) {
                    (Some(previous), Some(next)) => merge_callable_provenance(previous, &next, &mut self.fuel, source, 0)?,
                    _ => None,
                };
                returned.value_callables = join_guarded_callables(
                    &returned.value_callables, &fields, &returned.value_variants, &value.variants, &mut self.fuel, source)?;
                returned.value_variants = join_enum_domains(&returned.value_variants, &value.variants, &mut self.fuel, source)?;
            }
        }
        let callable = returned.value_callable.clone();
        let fields = returned.value_callables.clone();
        let variants = returned.value_variants.clone();
        self.check_function_result(&mut returned, BTreeMap::new(), source, depth + 1)?;
        returned.value_callable = callable;
        returned.value_callables = fields;
        returned.value_variants = variants;
        if returned.live { self.exit(&mut returned, source, CleanupReason::EffectReturn)?; }
        Ok(())
    }

    pub(super) fn effect_request_resources(
        &mut self, expr: &Expr, abi: nir::effects::Abi, signature: &Term,
        arguments: &[Expr], flow: &mut Flow, depth: usize,
    ) -> Result<(), Error> {
        if !matches!(abi, nir::effects::Abi::Direct | nir::effects::Abi::DirectPassing) {
            return Err(error(expr.source, "effect continuation resources require a capture protocol"));
        }
        let Term::Function { parameters, result } = signature else {
            return Err(error(expr.source, "effect request requires an operation signature"));
        };
        if arguments.len() != parameters.len() || expr.ty != **result
            || arguments.iter().zip(parameters).any(|(argument, parameter)| argument.ty != parameter.ty
                && argument.ty != Term::Primitive(nir::Primitive::Never)) {
            return Err(error(expr.source, "effect request input/result signature mismatch"));
        }
        if abi == nir::effects::Abi::DirectPassing && !arguments.is_empty() {
            return Err(error(expr.source, "direct-passing request cannot have runtime arguments"));
        }
        // Input preparation retains already evaluated inputs until the request
        // takes ownership. An exit in a later input still cleans earlier ones.
        self.pending_frame(expr.source, |this| {
            for (slot, argument) in arguments.iter().enumerate() {
                let origins = this.evaluate_value_origins(argument, flow, depth + 1)?;
                if !flow.live { return Ok(()); }
                if !origins.is_empty() || flow.memory_exposed {
                    return Err(error(expr.source, "effect request with pointer dependencies requires a memory contract"));
                }
                this.remember_pending_origins(argument, slot, flow, origins)?;
            }
            Ok(())
        })?;
        if !flow.live { return Ok(()); }
        if flow.memory_exposed {
            return Err(error(expr.source, "effect request with pointer dependencies requires a memory contract"));
        }
        flow.value_callable = None;
        flow.value_callables.clear();
        flow.value_variants.clear();
        if abi == nir::effects::Abi::Direct {
            // Input ownership has crossed to the handler. Only surviving
            // caller positions and enclosing incomplete constructions unwind.
            self.effect_escape_checkpoint(flow, expr.source)?;
            let mut rejected = flow.clone();
            self.exit(&mut rejected, expr.source, CleanupReason::PredicateFailure)?;
        }
        Ok(())
    }
}

impl<'a, F: FnMut(&Term) -> Result<Delivery, String>, G: FnMut(&Term) -> Result<Shape, String>>
    Analyzer<'a, F, G>
{
    pub(super) fn handler_resources(
        &mut self, expression: &Expr, handlers: &[nir::program::EffectHandler], outer: &Flow, depth: usize,
    ) -> Result<(bool, Vec<std::sync::Arc<HandlerReturnValue>>, BTreeSet<(LocalId, bool)>), Error> {
        let mut installation_may_reject = false;
        let mut installation_returns = Vec::new();
        let mut installation_loop_targets = BTreeSet::new();
        let installation = expression.source;
        let mut addressed_roots = None;
        for (handler_index, handler) in handlers.iter().enumerate() {
            if handler.abi != nir::effects::Abi::Direct
                && !(handler.abi == nir::effects::Abi::DirectPassing
                    && nir::effects::scalar_passing_signature(&handler.signature)) {
                return Err(error(installation,"handler slot ownership requires an installation resource protocol"));
            }
            let Term::Function { parameters, result } = &handler.signature else {
                return Err(error(installation,"handler lacks an operation signature"));
            };
            // Void carries no owned slot payload; all represented scalar
            // results need an actual Copy proof before repeated delivery.
            if handler.abi == nir::effects::Abi::DirectPassing
                && **result != Term::Primitive(nir::Primitive::Void)
                && (self.delivery)(result).map_err(|reason|error(installation,reason))? != Delivery::Copy {
                return Err(error(installation,"scalar effect slot requires checked Copy permission"));
            }
            // This tuple is the internal operation argument vector, not a
            // replacement for the language's normal-form Tuple constructor.
            for parameter in parameters {
                crate::associated::charge_projection_copy(&parameter.ty, &mut self.fuel)
                    .map_err(|reason|error(installation,reason))?;
            }
            crate::associated::charge_projection_copy(result, &mut self.fuel)
                .map_err(|reason|error(installation,reason))?;
            let input = Term::Tuple(parameters.iter().map(|p| p.ty.clone()).collect());
            for (arm_index, arm) in handler.arms.iter().enumerate() {
                if self.fuel == 0 || depth > 128 {
                    return Err(error(installation,"handler resource budget exhausted"));
                }
                self.fuel -= 1;
                // Traverse the complete arm before retaining a cloned report,
                // including unreachable branches and nested closure bodies.
                let free = nir::free_bindings::collect_arm(arm, &mut self.fuel)
                    .map_err(|reason|error(installation,reason))?;
                let mut captures = Vec::new();
                for binding in &free {
                    let place = Place {root:root(*binding),fields:vec![]};
                    outer.state.require_present(&place)
                        .map_err(|_|error(installation,"handler lexical capture is unavailable at installation"))?;
                    let ty = outer.types.get(&place.root)
                        .ok_or_else(||error(installation,"handler lexical capture has no checked storage type"))?;
                    if !matches!(ty, Term::Primitive(_)) || !nir::program::intrinsic_pointer_value(ty)
                        || (self.delivery)(ty).map_err(|reason|error(installation,reason))? != Delivery::Copy {
                        return Err(error(installation,"handler lexical capture requires scalar Copy evidence; affine/aggregate captures need request-time ownership"));
                    }
                    if outer.memory_exposed {
                        return Err(error(installation,"handler lexical captures with exposed memory require a memory contract"));
                    }
                    captures.push((*binding,ty.clone()));
                }
                if !free.is_empty() {
                    // Copy scalar reads/writes preserve initialization. Taking
                    // an address could invalidate that invariant between two
                    // requests, including through subject arguments/closures.
                    if addressed_roots.is_none() {
                        let mut addressed = BTreeSet::new();
                        // LocalIds identify lexical bindings, not runtime storage:
                        // closure bodies use private capture cells, while capture
                        // initializers still evaluate in the installing scope.
                        let mut pending = vec![expression];
                        while let Some(scope) = pending.pop() {
                            nir::visit::expression_pruned(scope, &mut self.fuel, |node, fuel| {
                                match &node.kind {
                                    K::AddressOf(value) => {
                                        if let Some(place) = expression_place(value) {
                                            addressed.insert(place.root);
                                        }
                                    }
                                    K::Closure { captures, .. } => {
                                        *fuel = fuel.checked_sub(captures.len())
                                            .ok_or("handler capture traversal budget exhausted")?;
                                        pending.extend(captures.iter().filter_map(|capture| capture.value.as_deref()));
                                        return Ok(false);
                                    }
                                    K::Call { callee, arguments, .. } => {
                                        *fuel = fuel.checked_sub(arguments.len() + 1)
                                            .ok_or("handler call traversal budget exhausted")?;
                                        pending.push(callee);
                                        // Defaults observe the callee parameter environment.
                                        pending.extend(arguments.iter().filter(|arg| !arg.is_default).map(|arg| &arg.value));
                                        return Ok(false);
                                    }
                                    _ => {}
                                }
                                Ok(true)
                            }).map_err(|reason|error(installation,reason))?;
                        }
                        addressed_roots = Some(addressed);
                    }
                    if free.iter().any(|binding|addressed_roots.as_ref().unwrap().contains(&root(*binding))) {
                        return Err(error(installation,"handler lexical capture address exposure requires a memory contract"));
                    }
                }
                let outer_report = std::mem::take(&mut self.report);
                let outer_pending = std::mem::take(&mut self.pending);
                let outer_loops = std::mem::take(&mut self.loops);
                let outer_persistent = std::mem::take(&mut self.persistent_roots);
                let outer_captured = std::mem::take(&mut self.captured_roots);
                let outer_contract = self.exit_contract.take();
                let outer_handler = self.handler_result.replace((**result).clone());
                let outer_effect_return = std::mem::replace(&mut self.effect_return_enabled, false);
                let outer_return_values = std::mem::take(&mut self.effect_return_values);
                let outer_effect_loops = std::mem::take(&mut self.effect_loop_targets);
                let mut frame = Flow { live:true, ..Default::default() };
                // Shared captures are observed at request time, not copied at
                // installation. They remain owned by the installing scope.
                let checked = self.in_temporary_scope(arm.body.source, TemporaryBoundary::Match, &mut frame, |this, frame| {
                    for (binding, ty) in &captures {
                        this.declare(frame, *binding, ty, installation)?;
                        this.persistent_roots.insert(root(*binding));
                    }
                    let (pattern, mut guards) = outer_guards(&arm.pattern);
                    guards.extend(arm.guard.as_ref());
                    this.bind_pattern(pattern, &input, &BTreeMap::new(), frame,
                        arm.body.source, &mut vec![], depth + 1)?;
                    for guard in guards {
                        this.observe_guard(guard, frame, depth + 1)?;
                        if constant_bool(guard,0) == Some(false) { frame.live=false; return Ok(()); }
                    }
                    let origins = this.evaluate_value_origins(&arm.body, frame, depth + 1)?;
                    if frame.live {
                        this.check_return_origins(frame, &origins, arm.body.source)?;
                        this.exit(frame, arm.body.source, CleanupReason::HandlerEnd)?;
                    }
                    Ok(())
                });
                let invocation = std::mem::replace(&mut self.report, outer_report);
                self.pending = outer_pending;
                self.loops = outer_loops;
                self.persistent_roots = outer_persistent;
                self.captured_roots = outer_captured;
                self.exit_contract = outer_contract;
                self.handler_result = outer_handler;
                self.effect_return_enabled = outer_effect_return;
                self.effect_return_values = outer_return_values;
                self.effect_loop_targets = outer_effect_loops;
                checked?;
                self.fuel = self.fuel.checked_sub(invocation.lexical_returns.len())
                    .ok_or_else(||error(installation,"handler return summary budget exhausted"))?;
                installation_returns.extend(invocation.lexical_returns.iter().cloned());
                self.fuel = self.fuel.checked_sub(invocation.lexical_loop_exits.len())
                    .ok_or_else(||error(installation,"handler loop summary budget exhausted"))?;
                installation_loop_targets.extend(invocation.lexical_loop_exits.iter().copied());
                installation_may_reject |= handler.abi == nir::effects::Abi::DirectPassing
                    && !invocation.failed_exits.is_empty();
                self.report.handlers.push(HandlerReport {
                    installation, handler:handler_index, arm:arm_index,
                    definition:arm.clone(), result:(**result).clone(), invocation,
                });
            }
        }
        Ok((installation_may_reject, installation_returns, installation_loop_targets))
    }
}
