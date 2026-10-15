//! Owned effect inputs and the suspended invocation's full-frame exit plan.
//! Direct handler arms have independent input and cleanup scopes. Copy scalar
//! products and scalar-payload enums remain owned by the installer. Stable pointers to scalar leaves of
//! pointer-free Copy storage use borrowed frames and conditional memory premises;
//! other lexical captures still require request-time ownership/provenance evidence.
use super::*;

fn mutated_pointer_slot(node: &Expr) -> Option<PlaceId> {
    match &node.kind {
        K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } => {
            Some(root(place.root))
        }
        K::AddressOf(value) => expression_place(value).map(|place| place.root),
        _ => None,
    }
}

impl<'a, F: FnMut(&Term) -> Result<Delivery, String>, G: FnMut(&Term) -> Result<Shape, String>>
    Analyzer<'a, F, G>
{
    fn require_capture_value_layout(
        &mut self,
        ty: &Term,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        if !super::super::memory::effect_data_layout(self.package, ty, &mut self.fuel, depth)
            .map_err(|reason| error(source, reason))?
        {
            return Err(error(
                source,
                "handler aggregate capture requires scalar data without pointer or callable fields",
            ));
        }
        Ok(())
    }

    pub(super) fn pointer_slot_mutations(
        &mut self,
        scope: &Expr,
    ) -> Result<std::sync::Arc<BTreeSet<PlaceId>>, Error> {
        let mut changed = BTreeSet::new();
        nir::visit::expression(scope, &mut self.fuel, |node, _| {
            if let Some(slot) = mutated_pointer_slot(node) {
                changed.insert(slot);
            }
            Ok(())
        })
        .map_err(|reason| error(scope.source, reason))?;
        Ok(std::sync::Arc::new(changed))
    }
    pub(super) fn effect_escape_checkpoint(
        &mut self,
        flow: &Flow,
        source: SourceRange,
    ) -> Result<(), Error> {
        if !flow.live {
            return Ok(());
        }
        let mut escaped = flow.clone();
        let locals = escaped
            .order
            .iter()
            .filter(|id| !self.persistent_roots.contains(id))
            .copied()
            .collect();
        // This is a conditional full-frame unwind, not a normal function result
        // or a failed program predicate. In particular it must not participate
        // in joining the callable/tag evidence of successful closure returns.
        self.cleanup_exit(
            &mut escaped,
            locals,
            0,
            0,
            source,
            CleanupReason::EffectEscape,
        )?;
        self.incomplete_exit(source, 0);
        self.report.effect_exits.push((source, escaped.state));
        self.effect_loop_checkpoints(flow, source)?;
        if self.effect_return_enabled && self.handler_result.is_none() {
            self.effect_return_checkpoint(flow, source, 1)?;
        }
        Ok(())
    }

    pub(super) fn effect_loop_checkpoints(
        &mut self,
        flow: &Flow,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.fuel = self
            .fuel
            .checked_sub(self.effect_loop_targets.len())
            .ok_or_else(|| error(source, "effect loop checkpoint budget exhausted"))?;
        for (target, is_break) in self.effect_loop_targets.clone() {
            if !self.loops.iter().any(|frame| frame.identity == target) {
                if self.handler_result.is_some() {
                    continue;
                }
                return Err(error(
                    source,
                    "effect loop target is not active in its lexical installer",
                ));
            }
            let mut edge = flow.clone();
            let first = self.report.cleanup_sites.len();
            self.loop_edge(target, is_break, &mut edge, source)?;
            for site in &mut self.report.cleanup_sites[first..] {
                site.reason = if is_break {
                    CleanupReason::EffectBreak(target)
                } else {
                    CleanupReason::EffectContinue(target)
                };
            }
        }
        Ok(())
    }

    pub(super) fn effect_return_checkpoint(
        &mut self,
        flow: &Flow,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        if !flow.live {
            return Ok(());
        }
        let mut returned = flow.clone();
        // Restore each handler result independently, then join candidates and
        // guarded aggregate leaves just as for ordinary function return paths.
        let mut first = true;
        returned.value_callable = None;
        returned.value_callables.clear();
        returned.value_variants.clear();
        self.fuel = self
            .fuel
            .checked_sub(self.effect_return_values.len())
            .ok_or_else(|| error(source, "handler return checkpoint budget exhausted"))?;
        for value in &self.effect_return_values {
            let callable = value
                .callable
                .as_ref()
                .map(|evidence| restore_callable_evidence(evidence, &mut self.fuel, 0))
                .transpose()?;
            let mut fields = BTreeMap::new();
            for (path, evidence) in &value.fields {
                self.fuel = self
                    .fuel
                    .checked_sub(path.len().saturating_add(1))
                    .ok_or_else(|| error(source, "handler return field budget exhausted"))?;
                fields.insert(
                    path.clone(),
                    restore_callable_evidence(evidence, &mut self.fuel, 0)?,
                );
            }
            if first {
                returned.value_callable = callable;
                returned.value_callables = fields;
                returned.value_variants =
                    copy_enum_domains(&value.variants, &mut self.fuel, source)?;
                first = false;
            } else {
                returned.value_callable = match (&returned.value_callable, callable) {
                    (Some(previous), Some(next)) => {
                        merge_callable_provenance(previous, &next, &mut self.fuel, source, 0)?
                    }
                    _ => None,
                };
                returned.value_callables = join_guarded_callables(
                    &returned.value_callables,
                    &fields,
                    &returned.value_variants,
                    &value.variants,
                    &mut self.fuel,
                    source,
                )?;
                returned.value_variants = join_enum_domains(
                    &returned.value_variants,
                    &value.variants,
                    &mut self.fuel,
                    source,
                )?;
            }
        }
        let callable = returned.value_callable.clone();
        let fields = returned.value_callables.clone();
        let variants = returned.value_variants.clone();
        self.check_function_result(&mut returned, BTreeMap::new(), source, depth + 1)?;
        returned.value_callable = callable;
        returned.value_callables = fields;
        returned.value_variants = variants;
        if returned.live {
            self.exit(&mut returned, source, CleanupReason::EffectReturn)?;
        }
        Ok(())
    }

    pub(super) fn effect_request_resources(
        &mut self,
        expr: &Expr,
        abi: nir::effects::Abi,
        signature: &Term,
        arguments: &[Expr],
        flow: &mut Flow,
        depth: usize,
    ) -> Result<(), Error> {
        if !matches!(
            abi,
            nir::effects::Abi::Direct | nir::effects::Abi::DirectPassing
        ) {
            return Err(error(
                expr.source,
                "effect continuation resources require a capture protocol",
            ));
        }
        let Term::Function { parameters, result } = signature else {
            return Err(error(
                expr.source,
                "effect request requires an operation signature",
            ));
        };
        if arguments.len() != parameters.len()
            || expr.ty != **result
            || arguments
                .iter()
                .zip(parameters)
                .any(|(argument, parameter)| {
                    argument.ty != parameter.ty
                        && argument.ty != Term::Primitive(nir::Primitive::Never)
                })
        {
            return Err(error(
                expr.source,
                "effect request input/result signature mismatch",
            ));
        }
        if abi == nir::effects::Abi::DirectPassing && !arguments.is_empty() {
            return Err(error(
                expr.source,
                "direct-passing request cannot have runtime arguments",
            ));
        }
        // Input preparation retains already evaluated inputs until the request
        // takes ownership. An exit in a later input still cleans earlier ones.
        self.pending_frame(expr.source, |this| {
            for (slot, argument) in arguments.iter().enumerate() {
                let origins = this.evaluate_value_origins(argument, flow, depth + 1)?;
                if !flow.live {
                    return Ok(());
                }
                if !origins.is_empty() {
                    return Err(error(
                        expr.source,
                        "effect request with pointer dependencies requires a memory contract",
                    ));
                }
                this.check_effect_memory_context(expr, flow)?;
                this.remember_pending_origins(argument, slot, flow, origins)?;
            }
            Ok(())
        })?;
        if !flow.live {
            return Ok(());
        }
        self.check_effect_memory_context(expr, flow)?;
        flow.value_callable = None;
        flow.value_callables.clear();
        flow.value_variants.clear();
        let may_resume = self.restore_effect_result(expr, flow)?;
        if abi == nir::effects::Abi::Direct {
            // Input ownership has crossed to the handler. Only surviving
            // caller positions and enclosing incomplete constructions unwind.
            self.effect_escape_checkpoint(flow, expr.source)?;
            let mut rejected = flow.clone();
            self.exit(&mut rejected, expr.source, CleanupReason::PredicateFailure)?;
        }
        if !may_resume {
            flow.live = false;
        }
        Ok(())
    }
}

impl<'a, F: FnMut(&Term) -> Result<Delivery, String>, G: FnMut(&Term) -> Result<Shape, String>>
    Analyzer<'a, F, G>
{
    fn stable_handler_pointees(
        &mut self,
        outer: &Flow,
        targets: &BTreeSet<Place>,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.pointer_work(targets, source)?;
        if targets.is_empty() {
            return Err(error(source, "handler pointer has no established target"));
        }
        for target in targets {
            let ty = outer
                .types
                .get(&target.root)
                .ok_or_else(|| error(source, "handler pointer target is not a known local"))?;
            if (!outer.roots.contains(&target.root) && !outer.effect_borrows.contains(&target.root))
                || target.fields.contains(&VARIANT_PATH)
                || (self.delivery)(ty).map_err(|reason| error(source, reason))? != Delivery::Copy
            {
                return Err(error(
                    source,
                    "handler pointer requires a live local Copy scalar target",
                ));
            }
            // Borrowed frames import the entire root layout. Establish it fully,
            // and exclude embedded aliases whose provenance could be changed by
            // replacing another field while the handler is suspended.
            super::super::memory::require_pointer_free_owned(self.package, ty, &mut self.fuel, 0)
                .map_err(|reason| {
                error(
                    source,
                    format!("handler pointer owner needs a pointer-free Copy layout: {reason}"),
                )
            })?;
            outer
                .state
                .require_present(&Place {
                    root: target.root,
                    fields: vec![],
                })
                .map_err(|_| error(source, "handler pointer owner is not fully initialized"))?;
            super::super::memory::bound_type(ty, &mut self.fuel)
                .map_err(|reason| error(source, reason))?;
            let leaf = owned_field_type(self.package, ty, &target.fields, &mut self.fuel, source)?;
            if !matches!(leaf, Term::Primitive(_))
                || !nir::program::intrinsic_pointer_value(&leaf)
                || (self.delivery)(&leaf).map_err(|reason| error(source, reason))? != Delivery::Copy
            {
                return Err(error(
                    source,
                    "handler pointer requires a live local Copy scalar target",
                ));
            }
            outer
                .state
                .require_present(target)
                .map_err(|_| error(source, "handler pointer target is unavailable"))?;
        }
        Ok(())
    }

    pub(super) fn handler_resources(
        &mut self,
        expression: &Expr,
        handlers: &[nir::program::EffectHandler],
        outer: &Flow,
        depth: usize,
    ) -> Result<
        (
            bool,
            Vec<std::sync::Arc<HandlerReturnValue>>,
            BTreeSet<(LocalId, bool)>,
            Vec<EffectResultInput>,
        ),
        Error,
    > {
        let mut result_inputs = vec![];
        let mut installation_may_reject = false;
        let mut installation_returns = Vec::new();
        let mut installation_loop_targets = BTreeSet::new();
        let installation = expression.source;
        let mut addressed_roots = None;
        for (handler_index, handler) in handlers.iter().enumerate() {
            let mut completions = vec![];
            if handler.abi != nir::effects::Abi::Direct
                && !(handler.abi == nir::effects::Abi::DirectPassing
                    && nir::effects::scalar_passing_signature(&handler.signature))
            {
                return Err(error(
                    installation,
                    "handler slot ownership requires an installation resource protocol",
                ));
            }
            let Term::Function { parameters, result } = &handler.signature else {
                return Err(error(installation, "handler lacks an operation signature"));
            };
            // Void carries no owned slot payload; all represented scalar
            // results need an actual Copy proof before repeated delivery.
            if handler.abi == nir::effects::Abi::DirectPassing
                && **result != Term::Primitive(nir::Primitive::Void)
                && (self.delivery)(result).map_err(|reason| error(installation, reason))?
                    != Delivery::Copy
            {
                return Err(error(
                    installation,
                    "scalar effect slot requires checked Copy permission",
                ));
            }
            // This tuple is the internal operation argument vector, not a
            // replacement for the language's normal-form Tuple constructor.
            for parameter in parameters {
                crate::associated::charge_projection_copy(&parameter.ty, &mut self.fuel)
                    .map_err(|reason| error(installation, reason))?;
            }
            crate::associated::charge_projection_copy(result, &mut self.fuel)
                .map_err(|reason| error(installation, reason))?;
            let input = Term::Tuple(parameters.iter().map(|p| p.ty.clone()).collect());
            for (arm_index, arm) in handler.arms.iter().enumerate() {
                if self.fuel == 0 || depth > 128 {
                    return Err(error(installation, "handler resource budget exhausted"));
                }
                self.fuel -= 1;
                // Traverse the complete arm before retaining a cloned report,
                // including unreachable branches and nested closure bodies.
                let free = nir::free_bindings::collect_arm(arm, &mut self.fuel)
                    .map_err(|reason| error(installation, reason))?;
                let mut captures = Vec::new();
                let mut pointer_captures = BTreeMap::new();
                for binding in &free {
                    let place = Place {
                        root: root(*binding),
                        fields: vec![],
                    };
                    outer.state.require_present(&place).map_err(|_| {
                        error(
                            installation,
                            "handler lexical capture is unavailable at installation",
                        )
                    })?;
                    let ty = outer.types.get(&place.root).ok_or_else(|| {
                        error(
                            installation,
                            "handler lexical capture has no checked storage type",
                        )
                    })?;
                    if let Term::Pointer(subject) = ty {
                        if !super::super::memory::effect_data_layout(
                            self.package,
                            result,
                            &mut self.fuel,
                            0,
                        )
                        .map_err(|reason| error(installation, reason))?
                        {
                            return Err(error(
                                installation,
                                "handler borrowed capture result requires a data-only contract",
                            ));
                        }
                        if (self.delivery)(ty).map_err(|reason| error(installation, reason))?
                            != Delivery::Copy
                        {
                            return Err(error(
                                installation,
                                "handler pointer capture requires Copy evidence",
                            ));
                        }
                        let targets = outer.pointers.get(&place).ok_or_else(|| {
                            error(
                                installation,
                                "handler pointer capture requires known provenance",
                            )
                        })?;
                        self.stable_handler_pointees(outer, targets, installation)?;
                        for target in targets {
                            let ty = &outer.types[&target.root];
                            super::super::memory::bound_type(ty, &mut self.fuel)
                                .map_err(|reason| error(installation, reason))?;
                            if owned_field_type(
                                self.package,
                                ty,
                                &target.fields,
                                &mut self.fuel,
                                installation,
                            )? != **subject
                            {
                                return Err(error(
                                    installation,
                                    "handler pointer target type mismatch",
                                ));
                            }
                        }
                        if !outer.stable_handler_slots.contains(&place.root) {
                            if let Some(changed) = &outer.pointer_slot_mutations {
                                if changed.contains(&place.root) {
                                    return Err(error(
                                        installation,
                                        "handler pointer slot rebinding or address exposure requires a memory contract",
                                    ));
                                }
                            } else {
                                let function = self.exit_contract.ok_or_else(|| {
                                    error(
                                        installation,
                                        "handler pointer slot lacks its complete lexical scope",
                                    )
                                })?;
                                // Include all exits, contracts and defaults of
                                // a function, not just the path to installation.
                                nir::visit::expressions(function,&mut self.fuel,|node,_| {
                                    if mutated_pointer_slot(node)==Some(place.root) {
                                        return Err("handler pointer slot rebinding or address exposure requires a memory contract".into());
                                    }
                                    Ok(())
                                }).map_err(|reason|error(installation,reason))?;
                            }
                        }
                        pointer_captures.insert(*binding, targets.clone());
                    } else {
                        if (self.delivery)(ty).map_err(|reason| error(installation, reason))?
                            != Delivery::Copy
                        {
                            return Err(error(
                                installation,
                                "handler lexical capture requires scalar Copy evidence; affine captures need request-time ownership",
                            ));
                        }
                        self.require_capture_value_layout(ty, installation, 0)?;
                    }
                    captures.push((*binding, ty.clone()));
                }
                if outer.memory_exposed && !free.is_empty() {
                    // Scalar Copy captures preserve ownership whether or not
                    // this arm also captures a pointer. All exposed targets
                    // still have to satisfy the same stable-storage contract.
                    let scalar_roots: BTreeSet<_> = captures
                        .iter()
                        .filter(|(_, ty)| !matches!(ty, Term::Pointer(_)))
                        .map(|(binding, _)| root(*binding))
                        .collect();
                    for targets in outer.pointers.values() {
                        self.stable_handler_pointees(outer, targets, installation)?;
                        // A lexical scalar capture uses the unaliased capture
                        // contract. An unrelated exposed scalar is harmless,
                        // but aliasing this capture needs a different contract.
                        if targets
                            .iter()
                            .any(|target| scalar_roots.contains(&target.root))
                        {
                            return Err(error(
                                installation,
                                "handler lexical capture address exposure requires a memory contract",
                            ));
                        }
                    }
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
                                        *fuel = fuel
                                            .checked_sub(captures.len())
                                            .ok_or("handler capture traversal budget exhausted")?;
                                        pending.extend(
                                            captures
                                                .iter()
                                                .filter_map(|capture| capture.value.as_deref()),
                                        );
                                        return Ok(false);
                                    }
                                    K::Call {
                                        callee, arguments, ..
                                    } => {
                                        *fuel = fuel
                                            .checked_sub(arguments.len() + 1)
                                            .ok_or("handler call traversal budget exhausted")?;
                                        pending.push(callee);
                                        // Defaults observe the callee parameter environment.
                                        pending.extend(
                                            arguments
                                                .iter()
                                                .filter(|arg| !arg.is_default)
                                                .map(|arg| &arg.value),
                                        );
                                        return Ok(false);
                                    }
                                    _ => {}
                                }
                                Ok(true)
                            })
                            .map_err(|reason| error(installation, reason))?;
                        }
                        addressed_roots = Some(addressed);
                    }
                    if free
                        .iter()
                        .any(|binding| addressed_roots.as_ref().unwrap().contains(&root(*binding)))
                    {
                        return Err(error(
                            installation,
                            "handler lexical capture address exposure requires a memory contract",
                        ));
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
                let outer_completions = std::mem::take(&mut self.handler_completions);
                let mut frame = Flow {
                    pointer_slot_mutations: Some(self.pointer_slot_mutations(expression)?),
                    live: true,
                    ..Default::default()
                };
                // Shared captures are observed at request time, not copied at
                // installation. They remain owned by the installing scope.
                let checked = self.in_temporary_scope(
                    arm.body.source,
                    TemporaryBoundary::Match,
                    &mut frame,
                    |this, frame| {
                        for (binding, ty) in &captures {
                            this.declare(frame, *binding, ty, installation)?;
                            // Captures refer to installer storage. An outer provider
                            // can mutate this same slot while this arm is suspended.
                            if outer.effect_mutable_roots.contains(&root(*binding)) {
                                frame.effect_mutable_roots.insert(root(*binding));
                            }
                            this.persistent_roots.insert(root(*binding));
                        }
                        for (binding, targets) in &pointer_captures {
                            for target in targets {
                                if !frame.types.contains_key(&target.root) {
                                    let ty = &outer.types[&target.root];
                                    let shape = (this.shape)(ty)
                                        .map_err(|reason| error(installation, reason))?;
                                    frame
                                        .state
                                        .declare(target.root, &shape, true, &mut this.fuel)
                                        .map_err(|reason| {
                                            error(
                                                installation,
                                                format!("handler borrowed scalar: {reason:?}"),
                                            )
                                        })?;
                                    // Borrowed storage is not owned by the handler
                                    // and never enters its cleanup order.
                                    frame.types.insert(target.root, ty.clone());
                                }
                            }
                            // The full lexical installer has already excluded
                            // rebinding/address exposure of this slot. Nested
                            // handlers may reuse that proof, never synthesize it.
                            frame.stable_handler_slots.insert(root(*binding));
                            frame
                                .effect_borrows
                                .extend(targets.iter().map(|target| target.root));
                            frame.pointers.insert(
                                Place {
                                    root: root(*binding),
                                    fields: vec![],
                                },
                                targets.clone(),
                            );
                        }
                        frame.memory_exposed |= !pointer_captures.is_empty();
                        let (pattern, mut guards) = outer_guards(&arm.pattern);
                        guards.extend(arm.guard.as_ref());
                        this.bind_pattern(
                            pattern,
                            &input,
                            &BTreeMap::new(),
                            frame,
                            arm.body.source,
                            &mut vec![],
                            depth + 1,
                        )?;
                        for guard in guards {
                            this.observe_guard(guard, frame, depth + 1)?;
                            if constant_bool(guard, 0) == Some(false) {
                                frame.live = false;
                                return Ok(());
                            }
                        }
                        let origins = this.evaluate_value_origins(&arm.body, frame, depth + 1)?;
                        if frame.live {
                            this.check_return_origins(frame, &origins, arm.body.source)?;
                            this.capture_handler_completion(frame, &origins, arm.body.source)?;
                            this.exit(frame, arm.body.source, CleanupReason::HandlerEnd)?;
                        }
                        Ok(())
                    },
                );
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
                completions.extend(std::mem::replace(
                    &mut self.handler_completions,
                    outer_completions,
                ));
                checked?;
                self.fuel = self
                    .fuel
                    .checked_sub(invocation.lexical_returns.len())
                    .ok_or_else(|| {
                        error(installation, "handler return summary budget exhausted")
                    })?;
                installation_returns.extend(invocation.lexical_returns.iter().cloned());
                self.fuel = self
                    .fuel
                    .checked_sub(invocation.lexical_loop_exits.len())
                    .ok_or_else(|| error(installation, "handler loop summary budget exhausted"))?;
                installation_loop_targets.extend(invocation.lexical_loop_exits.iter().copied());
                installation_may_reject |= handler.abi == nir::effects::Abi::DirectPassing
                    && !invocation.failed_exits.is_empty();
                self.report.handlers.push(HandlerReport {
                    installation,
                    handler: handler_index,
                    arm: arm_index,
                    definition: arm.clone(),
                    result: (**result).clone(),
                    invocation,
                });
            }
            let mut complete = handler.fallback.is_none();
            if let Some(slot) = handler.fallback {
                for input in &self.effect_result_inputs {
                    input.charge(&mut self.fuel, installation)?;
                    if input.slot == slot
                        && input.operation == handler.operation
                        && input.abi == handler.abi
                        && input.signature == handler.signature
                    {
                        completions.extend(input.values.iter().cloned());
                        complete = true;
                        break;
                    }
                }
            }
            if complete {
                for argument in &handler.operation.arguments {
                    crate::associated::charge_projection_copy(argument, &mut self.fuel)
                        .map_err(|e| error(installation, e))?;
                }
                self.fuel = self
                    .fuel
                    .checked_sub(completions.len() + 1)
                    .ok_or_else(|| {
                        error(installation, "handler result premise budget exhausted")
                    })?;
                result_inputs.push(EffectResultInput {
                    slot: handler_index,
                    operation: handler.operation.clone(),
                    abi: handler.abi,
                    signature: handler.signature.clone(),
                    values: completions,
                });
            }
        }
        Ok((
            installation_may_reject,
            installation_returns,
            installation_loop_targets,
            result_inputs,
        ))
    }
}

impl<'a, F: FnMut(&Term) -> Result<Delivery, String>, G: FnMut(&Term) -> Result<Shape, String>>
    Analyzer<'a, F, G>
{
    fn check_effect_memory_context(&mut self, expr: &Expr, flow: &Flow) -> Result<(), Error> {
        if !flow.memory_exposed {
            return Ok(());
        }
        let K::EffectRequest {
            operation,
            abi,
            signature,
            ..
        } = &expr.kind
        else {
            unreachable!()
        };
        let mut established = false;
        for input in &self.effect_memory_inputs {
            self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or_else(|| error(expr.source, "effect memory premise budget exhausted"))?;
            if &input.operation == operation && input.abi == *abi && input.signature == **signature
            {
                established = true;
                break;
            }
        }
        if !established {
            if self.defer_unknown_calls {
                // A hypothetical closure body has no installer yet. Retain an
                // obligation; actual invocation must establish memory premises.
                self.report.deferred_calls.push(expr.source);
                return Ok(());
            }
            return Err(error(
                expr.source,
                "effect request with pointer dependencies requires a memory contract",
            ));
        }
        for targets in flow.pointers.values() {
            self.stable_handler_pointees(flow, targets, expr.source)?;
        }
        Ok(())
    }
    pub(super) fn authorize_effect_borrow(
        &mut self,
        caller: &Flow,
        targets: &BTreeSet<Place>,
        inner: &mut Flow,
        binding: LocalId,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.stable_handler_pointees(caller, targets, source)?;
        let borrowed = inner
            .pointers
            .get(&Place {
                root: root(binding),
                fields: vec![],
            })
            .ok_or_else(|| error(source, "closure borrowed frame is missing"))?;
        self.pointer_work(borrowed, source)?;
        inner
            .effect_borrows
            .extend(borrowed.iter().map(|place| place.root));
        Ok(())
    }
    pub(super) fn copy_effect_memory_inputs(
        &mut self,
        source: SourceRange,
    ) -> Result<Vec<super::super::memory::EffectInput>, Error> {
        self.fuel = self
            .fuel
            .checked_sub(self.effect_memory_inputs.len())
            .ok_or_else(|| error(source, "effect memory premise copy budget exhausted"))?;
        for input in &self.effect_memory_inputs {
            super::super::memory::bound_type(&input.signature, &mut self.fuel)
                .map_err(|e| error(source, e))?;
            for argument in &input.operation.arguments {
                super::super::memory::bound_type(argument, &mut self.fuel)
                    .map_err(|e| error(source, e))?;
            }
        }
        Ok(self.effect_memory_inputs.clone())
    }
    /// Called only after all installation arms have passed resource analysis.
    /// A data-only row with established outer entries can mutate checked captures but cannot
    /// change pointer provenance, invalidate affine owners or return aliases.
    pub(super) fn installed_effect_memory_inputs(
        &mut self,
        handlers: &[nir::program::EffectHandler],
        outer: &[nir::effects::Operation],
        flow: &Flow,
        source: SourceRange,
    ) -> Result<Vec<super::super::memory::EffectInput>, Error> {
        let previous = self.copy_effect_memory_inputs(source)?;
        let mut inputs = vec![];
        // Resolve the complete outer row against the incoming installation,
        // before adding local providers. A local operation of the same name
        // must never manufacture evidence for its own fallback.
        for operation in outer {
            let mut found = None;
            for input in &previous {
                self.fuel = self
                    .fuel
                    .checked_sub(1)
                    .ok_or_else(|| error(source, "outer effect memory premise budget exhausted"))?;
                if &input.operation == operation {
                    super::super::memory::bound_type(&input.signature, &mut self.fuel)
                        .map_err(|e| error(source, e))?;
                    for argument in &input.operation.arguments {
                        super::super::memory::bound_type(argument, &mut self.fuel)
                            .map_err(|e| error(source, e))?;
                    }
                    found = Some(input.clone());
                    break;
                }
            }
            let Some(input) = found else {
                return Ok(vec![]);
            };
            inputs.push(input);
        }
        for handler in handlers {
            self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or_else(|| error(source, "effect memory installation budget exhausted"))?;
            let Term::Function { parameters, result } = &handler.signature else {
                return Ok(vec![]);
            };
            let scalar = |ty: &Term| {
                matches!(ty, Term::Primitive(_))
                    && (nir::program::intrinsic_pointer_value(ty)
                        || *ty == Term::Primitive(nir::Primitive::Void))
            };
            if let Some(slot) = handler.fallback {
                self.fuel = self
                    .fuel
                    .checked_sub(inputs.len())
                    .ok_or_else(|| error(source, "fallback memory premise budget exhausted"))?;
                if outer.get(slot) != Some(&handler.operation)
                    || !inputs.iter().any(|input| {
                        input.operation == handler.operation
                            && input.abi == handler.abi
                            && input.signature == handler.signature
                    })
                {
                    return Ok(vec![]);
                }
            }
            if !matches!(
                handler.abi,
                nir::effects::Abi::Direct | nir::effects::Abi::DirectPassing
            ) || handler.abi == nir::effects::Abi::DirectPassing
                && (!parameters.is_empty() || !scalar(result))
            {
                return Ok(vec![]);
            }
            if !super::super::memory::effect_data_layout(self.package, result, &mut self.fuel, 0)
                .map_err(|reason| error(source, reason))?
            {
                return Ok(vec![]);
            }
            for parameter in parameters {
                if !super::super::memory::effect_data_layout(
                    self.package,
                    &parameter.ty,
                    &mut self.fuel,
                    0,
                )
                .map_err(|reason| error(source, reason))?
                {
                    return Ok(vec![]);
                }
            }
            crate::associated::charge_projection_copy(&handler.signature, &mut self.fuel)
                .map_err(|e| error(source, e))?;
            for argument in &handler.operation.arguments {
                crate::associated::charge_projection_copy(argument, &mut self.fuel)
                    .map_err(|e| error(source, e))?;
            }
            inputs.push(super::super::memory::EffectInput {
                operation: handler.operation.clone(),
                abi: handler.abi,
                signature: handler.signature.clone(),
            });
        }
        if flow.memory_exposed {
            for targets in flow.pointers.values() {
                self.stable_handler_pointees(flow, targets, source)?;
            }
        }
        Ok(inputs)
    }
}

impl<'a, F: FnMut(&Term) -> Result<Delivery, String>, G: FnMut(&Term) -> Result<Shape, String>>
    Analyzer<'a, F, G>
{
    pub(super) fn copy_effect_result_inputs(
        &mut self,
        source: SourceRange,
    ) -> Result<Vec<EffectResultInput>, Error> {
        for input in &self.effect_result_inputs {
            input.charge(&mut self.fuel, source)?;
        }
        Ok(self.effect_result_inputs.clone())
    }
    pub(super) fn capture_handler_completion(
        &mut self,
        flow: &Flow,
        origins: &ValueOrigins,
        source: SourceRange,
    ) -> Result<(), Error> {
        if !origins.is_empty() {
            return Err(error(
                source,
                "handler completion needs a pointer provenance contract",
            ));
        }
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
                .checked_sub(path.len() + 1)
                .ok_or_else(|| error(source, "handler completion budget exhausted"))?;
            fields.insert(
                path.clone(),
                export_callable_evidence(value, &mut self.fuel, 0)?,
            );
        }
        let variants = copy_enum_domains(&flow.value_variants, &mut self.fuel, source)?;
        self.handler_completions
            .push(std::sync::Arc::new(HandlerReturnValue {
                source,
                callable,
                fields,
                variants,
            }));
        Ok(())
    }
    fn restore_effect_result(&mut self, expression: &Expr, flow: &mut Flow) -> Result<bool, Error> {
        let K::EffectRequest {
            operation,
            abi,
            signature,
            allowed_index,
            ..
        } = &expression.kind
        else {
            unreachable!()
        };
        let source = expression.source;
        let mut values = None;
        for input in &self.effect_result_inputs {
            input.charge(&mut self.fuel, source)?;
            if input.slot == *allowed_index
                && &input.operation == operation
                && input.abi == *abi
                && input.signature == **signature
            {
                values = Some(input.values.clone());
                break;
            }
        }
        let Some(values) = values else {
            return Ok(true);
        };
        if values.is_empty() {
            return Ok(false);
        }
        let mut first = true;
        for value in values {
            let callable = value
                .callable
                .as_ref()
                .map(|e| restore_callable_evidence(e, &mut self.fuel, 0))
                .transpose()?;
            let mut fields = BTreeMap::new();
            for (path, evidence) in &value.fields {
                self.fuel = self
                    .fuel
                    .checked_sub(path.len() + 1)
                    .ok_or_else(|| error(source, "effect result field budget exhausted"))?;
                fields.insert(
                    path.clone(),
                    restore_callable_evidence(evidence, &mut self.fuel, 0)?,
                );
            }
            if first {
                flow.value_callable = callable;
                flow.value_callables = fields;
                flow.value_variants = copy_enum_domains(&value.variants, &mut self.fuel, source)?;
                first = false;
            } else {
                flow.value_callable = match (&flow.value_callable, callable) {
                    (Some(previous), Some(next)) => {
                        merge_callable_provenance(previous, &next, &mut self.fuel, source, 0)?
                    }
                    _ => None,
                };
                flow.value_callables = join_guarded_callables(
                    &flow.value_callables,
                    &fields,
                    &flow.value_variants,
                    &value.variants,
                    &mut self.fuel,
                    source,
                )?;
                flow.value_variants = join_enum_domains(
                    &flow.value_variants,
                    &value.variants,
                    &mut self.fuel,
                    source,
                )?;
            }
        }
        Ok(true)
    }
}

/// Operations delivering callable storage require a call-specific resource
/// context, including when the function itself returns an ordinary scalar.
pub fn effect_result_slots(
    package: &Package,
    signature: &Term,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Vec<usize>, Error> {
    let Term::Function { result, .. } = signature.callable_signature() else {
        return Ok(vec![]);
    };
    let Term::Computation { effects, .. } = result.as_ref() else {
        return Ok(vec![]);
    };
    let mut slots = vec![];
    for (slot, operation) in effects.iter().enumerate() {
        *fuel = fuel
            .checked_sub(package.effects.len() + 1)
            .ok_or_else(|| error(source, "effect result row budget exhausted"))?;
        let declaration = package
            .effects
            .iter()
            .find(|e| e.declaration == operation.declaration)
            .ok_or_else(|| error(source, "effect result operation is missing"))?;
        crate::associated::charge_projection_copy(&declaration.signature, fuel)
            .map_err(|e| error(source, e))?;
        for arg in &operation.arguments {
            crate::associated::charge_projection_copy(arg, fuel).map_err(|e| error(source, e))?;
        }
        let signature = declaration
            .signature
            .instantiate_types(&declaration.type_parameters, &operation.arguments)
            .map_err(|e| error(source, e.to_string()))?;
        let Term::Function { result, .. } = signature else {
            return Err(error(source, "effect result signature missing"));
        };
        if contains_callable_value(Some(package), &result, fuel, source, 0)? {
            slots.push(slot);
        }
    }
    Ok(slots)
}
impl<'a, F: FnMut(&Term) -> Result<Delivery, String>, G: FnMut(&Term) -> Result<Shape, String>>
    Analyzer<'a, F, G>
{
    pub(super) fn effect_inputs_for(
        &mut self,
        signature: &Term,
        source: SourceRange,
    ) -> Result<Vec<EffectResultInput>, Error> {
        let Term::Function { result, .. } = signature.callable_signature() else {
            return Ok(vec![]);
        };
        let Term::Computation { effects, .. } = result.as_ref() else {
            return Ok(vec![]);
        };
        let mut selected = vec![];
        for input in &self.effect_result_inputs {
            input.charge(&mut self.fuel, source)?;
            if effects.get(input.slot) == Some(&input.operation) {
                selected.push(input.clone());
            }
        }
        Ok(selected)
    }
}
