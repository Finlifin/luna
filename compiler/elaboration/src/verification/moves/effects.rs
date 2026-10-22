//! Owned effect inputs and the suspended invocation's full-frame exit plan.
//! Direct handler arms have independent input and cleanup scopes. Copy scalar
//! products and scalar-payload enums remain owned by the installer. Stable pointers to scalar leaves of
//! pointer-free Copy storage use borrowed frames and conditional memory premises;
//! other lexical captures still require request-time ownership/provenance evidence.
use super::*;

fn mutated_pointer_slot(node: &Expr, fuel: &mut usize) -> Result<Option<Place>, String> {
    Ok(match &node.kind {
        K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } => {
            *fuel = fuel
                .checked_sub(place.fields.len() + 1)
                .ok_or("handler mutation path budget exhausted")?;
            Some(Place {
                root: root(place.root),
                fields: place.fields.iter().map(|(_, index)| *index).collect(),
            })
        }
        // Address exposure retains the existing whole-object restriction: an
        // escaping alias needs a separate shared-storage contract.
        K::AddressOf(value) => expression_place(value).map(|place| Place {
            root: place.root,
            fields: vec![],
        }),
        _ => None,
    })
}

fn check_capture_slot_mutation(
    mutation: &Place,
    captured: PlaceId,
    layout: &[super::super::memory::PointerInputLeaf],
    fuel: &mut usize,
) -> Result<(), String> {
    if mutation.root != captured {
        return Ok(());
    }
    for leaf in layout {
        *fuel = fuel
            .checked_sub(
                mutation
                    .fields
                    .len()
                    .saturating_add(leaf.storage_fields.len())
                    .saturating_add(1),
            )
            .ok_or("handler pointer overlap budget exhausted")?;
        if mutation.fields.starts_with(&leaf.storage_fields)
            || leaf.storage_fields.starts_with(&mutation.fields)
        {
            return Err(
                "handler pointer slot rebinding or address exposure requires a memory contract"
                    .into(),
            );
        }
    }
    Ok(())
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

    // Generic borrowed fields share physical storage across enum alternatives.
    // Recover only correlations justified by an unchanged original parameter;
    // reconstructed/rebound values must retain their ordinary conservative set.
    pub(super) fn guarded_request_targets(
        &mut self,
        argument: &Expr,
        leaf: &super::super::memory::PointerInputLeaf,
        targets: &BTreeSet<Place>,
        flow: &Flow,
        evaluated: bool,
    ) -> Result<BTreeSet<Place>, Error> {
        self.pointer_work(targets, argument.source)?;
        if leaf.guards.is_empty() {
            return Ok(targets.clone());
        }
        let qualified = if evaluated {
            self.copy_value_pointer_guards(&flow.value_pointer_guards, argument.source)?
        } else {
            self.value_pointer_guards(argument, flow, argument.source)?
        };
        if let Some(entries) = qualified.get(&leaf.storage_fields) {
            let mut kept = BTreeSet::new();
            for target in targets {
                let mut excluded = false;
                if let Some(conditions) = entries.get(target) {
                    for (path, tag) in conditions {
                        for (requested, selected) in &leaf.guards {
                            self.fuel = self
                                .fuel
                                .checked_sub(path.len() + requested.len() + 1)
                                .ok_or_else(|| {
                                    error(
                                        argument.source,
                                        "guarded request candidate budget exhausted",
                                    )
                                })?;
                            if path == requested && tag != selected {
                                excluded = true;
                            }
                        }
                    }
                }
                if !excluded {
                    kept.insert(target.clone());
                }
            }
            if kept.is_empty() {
                return Err(error(
                    argument.source,
                    "effect variant payload has no compatible established origin",
                ));
            }
            return Ok(kept);
        }
        let Some(place) = expression_place(argument) else {
            return Ok(targets.clone());
        };
        let Some(function) = self.exit_contract else {
            return Ok(targets.clone());
        };
        let Some(parameter) = function
            .parameters
            .iter()
            .position(|binding| root(*binding) == place.root)
        else {
            return Ok(targets.clone());
        };
        let mut changed = false;
        nir::visit::expressions(function, &mut self.fuel, |node, fuel| {
            if mutated_pointer_slot(node, fuel)?.is_some_and(|mutation| mutation.root == place.root)
            {
                changed = true;
            }
            Ok(())
        })
        .map_err(|reason| error(argument.source, reason))?;
        if changed {
            return Ok(targets.clone());
        }
        let Term::Function { parameters, .. } = function.signature.callable_signature() else {
            return Ok(targets.clone());
        };
        let source_layout = super::super::memory::pointer_input_layout(
            self.package,
            &parameters[parameter].ty,
            &mut self.fuel,
        )
        .map_err(|reason| error(argument.source, reason))?;
        let mut kept = BTreeSet::new();
        for target in targets {
            let mut excluded = false;
            if target.fields.is_empty() {
                if let Some(BorrowedRoot::Parameter {
                    parameter: source_parameter,
                    input_fields,
                    ..
                }) = self.report.borrowed_roots.get(&target.root)
                {
                    if *source_parameter == parameter {
                        for source_leaf in &source_layout {
                            self.fuel = self
                                .fuel
                                .checked_sub(
                                    source_leaf.input_fields.len() + input_fields.len() + 1,
                                )
                                .ok_or_else(|| {
                                    error(
                                        argument.source,
                                        "handler guarded origin budget exhausted",
                                    )
                                })?;
                            if &source_leaf.input_fields != input_fields {
                                continue;
                            }
                            for (source_path, source_tag) in &source_leaf.guards {
                                for (requested_path, requested_tag) in &leaf.guards {
                                    self.fuel = self
                                        .fuel
                                        .checked_sub(
                                            source_path.len()
                                                + place.fields.len()
                                                + requested_path.len()
                                                + 1,
                                        )
                                        .ok_or_else(|| {
                                            error(
                                                argument.source,
                                                "handler variant correlation budget exhausted",
                                            )
                                        })?;
                                    if source_path.iter().copied().eq(place
                                        .fields
                                        .iter()
                                        .chain(requested_path)
                                        .copied())
                                        && source_tag != requested_tag
                                    {
                                        excluded = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if !excluded {
                kept.insert(target.clone());
            }
        }
        if kept.is_empty() {
            return Err(error(
                argument.source,
                "effect variant payload has no compatible established origin",
            ));
        }
        Ok(kept)
    }

    pub(super) fn pointer_slot_mutations(
        &mut self,
        scope: &Expr,
    ) -> Result<std::sync::Arc<BTreeSet<Place>>, Error> {
        let mut changed = BTreeSet::new();
        nir::visit::expression(scope, &mut self.fuel, |node, fuel| {
            if let Some(slot) = mutated_pointer_slot(node, fuel)? {
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
        let capture_pointers = self.capture_pointer_snapshot(&escaped, source)?;
        let capture_guards =
            self.capture_pointer_guard_snapshot(&escaped, &capture_pointers, source)?;
        self.report
            .effect_capture_pointer_guard_exits
            .push(capture_guards);
        let capture_domains = self.capture_enum_snapshot(&escaped, source)?;
        self.report.effect_capture_enum_exits.push(capture_domains);
        self.report
            .effect_capture_pointer_exits
            .push(capture_pointers);
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
    ) -> Result<ValueOrigins, Error> {
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
            || !(expr.ty == **result
                || expr.ty == Term::Primitive(nir::Primitive::Never)
                    && arguments
                        .iter()
                        .any(|argument| argument.ty == Term::Primitive(nir::Primitive::Never)))
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
        let K::EffectRequest {
            operation,
            allowed_index,
            ..
        } = &expr.kind
        else {
            unreachable!()
        };
        let mut forwarded = None;
        let mut copy_parameters = BTreeSet::new();
        let mut parameter_accesses = BTreeSet::new();
        for input in &self.effect_result_inputs {
            input.charge(&mut self.fuel, expr.source)?;
            if input.slot == *allowed_index
                && &input.operation == operation
                && input.abi == abi
                && &input.signature == signature
            {
                forwarded = input.forwarded_parameters.clone();
                copy_parameters = input.copy_parameters.clone();
                parameter_accesses = input.parameter_accesses.clone();
                break;
            }
        }
        let mut returned_origins = BTreeMap::new();
        let mut returned_guards: Option<ValuePointerGuards> = None;
        let mut returned_domains: Option<EnumDomains> = None;
        // Input preparation retains already evaluated inputs until the request
        // takes ownership. An exit in a later input still cleans earlier ones.
        let mut input_values = Vec::new();
        let mut accessed_targets = Vec::new();
        self.pending_frame(expr.source, |this| {
            for (slot, argument) in arguments.iter().enumerate() {
                let origins = this.evaluate_value_origins(argument, flow, depth + 1)?;
                if !flow.live {
                    return Ok(());
                }
                let discarded_copy = !forwarded
                    .as_ref()
                    .is_some_and(|indices| indices.contains(&slot))
                    && copy_parameters.contains(&slot)
                    && super::super::memory::effect_transfer_layout(
                        this.package,
                        &argument.ty,
                        &mut this.fuel,
                    )
                    .map_err(|reason| error(expr.source, reason))?;
                if !origins.is_empty()
                    && !discarded_copy
                    && (!forwarded
                        .as_ref()
                        .is_some_and(|indices| indices.contains(&slot))
                        || !super::super::memory::effect_transfer_layout(
                            this.package,
                            result,
                            &mut this.fuel,
                        )
                        .map_err(|reason| error(expr.source, reason))?
                        || argument.ty != **result)
                {
                    return Err(error(
                        expr.source,
                        "effect request with pointer dependencies requires a memory contract",
                    ));
                }
                let symbolic_origins = if parameter_accesses
                    .iter()
                    .any(|access| access.parameter == slot)
                {
                    this.memory_input_targets(
                        &argument.ty,
                        &origins,
                        &flow.value_variants,
                        expr.source,
                    )?
                } else {
                    BTreeMap::new()
                };
                for access in &parameter_accesses {
                    this.fuel = this
                        .fuel
                        .checked_sub(access.input_fields.len() + 1)
                        .ok_or_else(|| {
                            error(expr.source, "effect parameter target budget exhausted")
                        })?;
                    if access.parameter != slot {
                        continue;
                    }
                    let leaf = super::super::memory::pointer_input_layout(
                        this.package,
                        &argument.ty,
                        &mut this.fuel,
                    )
                    .map_err(|reason| error(expr.source, reason))?
                    .into_iter()
                    .find(|leaf| leaf.input_fields == access.input_fields)
                    .ok_or_else(|| {
                        error(expr.source, "effect parameter access path is not a pointer")
                    })?;
                    let targets = symbolic_origins.get(&access.input_fields).ok_or_else(|| {
                        error(
                            expr.source,
                            "effect parameter access needs actual pointer origins",
                        )
                    })?;
                    // Empty symbolic origins are produced only for a field
                    // excluded by the prepared argument's tag evidence.
                    if targets.is_empty() {
                        continue;
                    }
                    let targets =
                        this.guarded_request_targets(argument, &leaf, targets, flow, true)?;
                    accessed_targets.push((targets, leaf.pointee));
                }
                this.check_effect_memory_context(expr, flow)?;
                if forwarded
                    .as_ref()
                    .is_some_and(|indices| indices.contains(&slot))
                {
                    returned_guards = Some(if let Some(previous) = &returned_guards {
                        this.join_result_pointer_guards(
                            previous,
                            &returned_origins,
                            &flow.value_pointer_guards,
                            &origins,
                            expr.source,
                            Some((
                                completion_type(&expr.ty),
                                returned_domains.as_ref().expect("forwarded result domains"),
                                &flow.value_variants,
                            )),
                        )?
                    } else {
                        this.copy_value_pointer_guards(&flow.value_pointer_guards, expr.source)?
                    });
                    returned_domains = Some(if let Some(previous) = &returned_domains {
                        this.join_value_origins(
                            completion_type(&expr.ty),
                            &mut returned_origins,
                            previous,
                            &origins,
                            &flow.value_variants,
                            expr.source,
                        )?;
                        join_enum_domains(
                            previous,
                            &flow.value_variants,
                            &mut this.fuel,
                            expr.source,
                        )?
                    } else {
                        for (path, targets) in &origins {
                            this.fuel = this.fuel.checked_sub(path.len() + 1).ok_or_else(|| {
                                error(expr.source, "effect origin snapshot budget exhausted")
                            })?;
                            this.pointer_work(targets, expr.source)?;
                        }
                        returned_origins = origins.clone();
                        copy_enum_domains(&flow.value_variants, &mut this.fuel, expr.source)?
                    });
                }
                input_values.push(this.handler_value_evidence(flow, argument.source)?);
                this.remember_pending_origins(argument, slot, flow, origins)?;
            }
            Ok(())
        })?;
        if !flow.live {
            return Ok(BTreeMap::new());
        }
        for (targets, pointee) in accessed_targets {
            self.stable_handler_pointees(flow, &targets, expr.source)?;
            for target in &targets {
                let ty = flow
                    .types
                    .get(&target.root)
                    .ok_or_else(|| error(expr.source, "effect parameter target type missing"))?;
                if owned_field_type(
                    self.package,
                    ty,
                    &target.fields,
                    &mut self.fuel,
                    expr.source,
                )? != pointee
                {
                    return Err(error(
                        expr.source,
                        "effect parameter target pointee mismatch",
                    ));
                }
            }
        }
        self.check_effect_memory_context(expr, flow)?;
        flow.value_callable = None;
        flow.value_callables.clear();
        flow.value_variants.clear();
        let may_resume = self.restore_effect_result(expr, flow, &input_values)?;
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
        flow.value_pointer_guards = if flow.live {
            returned_guards.unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        Ok(if flow.live {
            returned_origins
        } else {
            BTreeMap::new()
        })
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
            let mut local_accesses = BTreeSet::new();
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
                let mut capture_domains = BTreeMap::new();
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
                    let layout = super::super::memory::pointer_input_layout(
                        self.package,
                        ty,
                        &mut self.fuel,
                    )
                    .map_err(|reason| error(installation, reason))?;
                    if !layout.is_empty() {
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
                        let domains = if let Some(domains) = outer.enum_domains.get(&place.root) {
                            copy_enum_domains(domains, &mut self.fuel, installation)?
                        } else {
                            EnumDomains::new()
                        };
                        super::super::memory::bound_type(ty, &mut self.fuel)
                            .map_err(|reason| error(installation, reason))?;
                        let capture = Expr {
                            ty: ty.clone(),
                            source: installation,
                            kind: K::Local(*binding),
                        };
                        let mut stable_domains = EnumDomains::new();
                        for leaf in &layout {
                            let mut position = 0;
                            while position + 1 < leaf.input_fields.len() {
                                self.fuel = self.fuel.checked_sub(1).ok_or_else(|| {
                                    error(installation, "handler capture tag budget exhausted")
                                })?;
                                if leaf.input_fields[position] == VARIANT_PATH {
                                    let path = &leaf.input_fields[..position];
                                    if let Some(tags) = domains.get(path) {
                                        self.fuel = self
                                            .fuel
                                            .checked_sub(path.len() + tags.len() + 1)
                                            .ok_or_else(|| {
                                                error(
                                                    installation,
                                                    "handler capture tag copy budget exhausted",
                                                )
                                            })?;
                                        stable_domains.insert(path.to_vec(), tags.clone());
                                    }
                                    position += 2;
                                } else {
                                    position += 1;
                                }
                            }
                            if enum_path_excluded(&leaf.input_fields, &domains) {
                                continue;
                            }
                            let field_place = Place {
                                root: place.root,
                                fields: leaf.storage_fields.clone(),
                            };
                            let targets = outer.pointers.get(&field_place).ok_or_else(|| {
                                error(
                                    installation,
                                    "handler pointer capture requires known provenance",
                                )
                            })?;
                            let targets = self
                                .guarded_request_targets(&capture, leaf, targets, outer, false)?;
                            self.stable_handler_pointees(outer, &targets, installation)?;
                            for target in &targets {
                                let ty = &outer.types[&target.root];
                                super::super::memory::bound_type(ty, &mut self.fuel)
                                    .map_err(|reason| error(installation, reason))?;
                                if owned_field_type(
                                    self.package,
                                    ty,
                                    &target.fields,
                                    &mut self.fuel,
                                    installation,
                                )? != leaf.pointee
                                {
                                    return Err(error(
                                        installation,
                                        "handler pointer target type mismatch",
                                    ));
                                }
                            }
                            pointer_captures
                                .entry(field_place)
                                .or_insert_with(BTreeSet::new)
                                .extend(targets);
                        }
                        capture_domains.insert(*binding, stable_domains);
                        if !outer.stable_handler_slots.contains(&place.root) {
                            if let Some(changed) = &outer.pointer_slot_mutations {
                                for mutation in changed
                                    .range(place.clone()..)
                                    .take_while(|mutation| mutation.root == place.root)
                                {
                                    check_capture_slot_mutation(
                                        mutation,
                                        place.root,
                                        &layout,
                                        &mut self.fuel,
                                    )
                                    .map_err(|reason| error(installation, reason))?;
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
                                nir::visit::expressions(function, &mut self.fuel, |node, fuel| {
                                    if let Some(mutation) = mutated_pointer_slot(node, fuel)? {
                                        check_capture_slot_mutation(
                                            &mutation, place.root, &layout, fuel,
                                        )?;
                                    }
                                    Ok(())
                                })
                                .map_err(|reason| error(installation, reason))?;
                            }
                        }
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
                    self.fuel = self
                        .fuel
                        .checked_sub(pointer_captures.len().saturating_add(captures.len()))
                        .ok_or_else(|| {
                            error(installation, "handler capture root budget exhausted")
                        })?;
                    let pointer_roots: BTreeSet<_> =
                        pointer_captures.keys().map(|place| place.root).collect();
                    let scalar_roots: BTreeSet<_> = captures
                        .iter()
                        .filter(|(binding, _)| !pointer_roots.contains(&root(*binding)))
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
                let parameter_summary = super::super::memory::direct_handler_accesses(
                    self.package,
                    handler,
                    arm,
                    &captures,
                    &mut self.fuel,
                )
                .map_err(|reason| error(installation, reason))?;
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
                            if let Some(domains)=capture_domains.get(binding) {
                                let domains=copy_enum_domains(domains,&mut this.fuel,installation)?;
                                this.install_enum_domains(frame,&Place {root:root(*binding),fields:vec![]},domains,installation)?;
                            }
                        }
                        for (place, targets) in &pointer_captures {
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
                            frame.stable_handler_slots.insert(place.root);
                            frame
                                .effect_borrows
                                .extend(targets.iter().map(|target| target.root));
                            frame.pointers.insert(place.clone(),targets.clone());
                            if let Some(qualified)=outer.pointer_guards.get(place) {
                                let mut retained=PointerTargetGuards::new();
                                for (target,guards) in qualified {
                                    this.charge_pointer_conditions(guards,installation)?;
                                    if targets.contains(target) {retained.insert(target.clone(),guards.clone());}
                                }
                                if !retained.is_empty() {frame.pointer_guards.insert(place.clone(),retained);}
                            }
                        }
                        frame.memory_exposed |= !pointer_captures.is_empty();
                        let (pattern, mut guards) = outer_guards(&arm.pattern);
                        guards.extend(arm.guard.as_ref());
                        let mut parameter_origins=BTreeMap::new();
                        let mut parameter_guards=ValuePointerGuards::new();
                        if let Some(summary) = &parameter_summary {
                            let mut borrowed = BTreeSet::new();
                            for access in &summary.accesses {
                                this.fuel=this.fuel.checked_sub(access.input_fields.len()+1)
                                    .ok_or_else(||error(installation,"handler input field budget exhausted"))?;
                                if !borrowed.insert((access.parameter,access.input_fields.clone())) {continue;}
                                let parameter=&parameters[access.parameter].ty;
                                let leaf=super::super::memory::pointer_input_layout(this.package,parameter,&mut this.fuel)
                                    .map_err(|reason|error(installation,reason))?.into_iter()
                                    .find(|leaf|leaf.input_fields==access.input_fields)
                                    .ok_or_else(||error(installation,"handler input field is not a pointer"))?;
                                if (this.delivery)(&leaf.pointee).map_err(|reason|error(installation,reason))?!=Delivery::Copy
                                    || (this.delivery)(parameter).map_err(|reason|error(installation,reason))?!=Delivery::Copy {
                                    return Err(error(installation,"effect parameter access requires Copy input and scalar storage"));
                                }
                                let mut borrowed_root=PlaceId {owner:u64::MAX-1,local:u32::try_from(borrowed.len()-1)
                                    .map_err(|_|error(installation,"effect borrowed field index overflow"))?};
                                // Captured pointers may already refer to virtual
                                // roots from the installer's borrowed parameters.
                                while frame.types.contains_key(&borrowed_root) {
                                    this.fuel=this.fuel.checked_sub(1).ok_or_else(||error(installation,"effect borrowed root budget exhausted"))?;
                                    borrowed_root.local=borrowed_root.local.checked_add(1)
                                        .ok_or_else(||error(installation,"effect borrowed root index overflow"))?;
                                }
                                let shape=(this.shape)(&leaf.pointee).map_err(|reason|error(installation,reason))?;
                                frame.state.declare(borrowed_root,&shape,true,&mut this.fuel)
                                    .map_err(|reason|error(installation,format!("effect parameter borrowed field: {reason:?}")))?;
                                frame.types.insert(borrowed_root,leaf.pointee.clone());
                                let mut path=vec![access.parameter];
                                path.extend(leaf.storage_fields);
                                // Distinct variants can share a physical slot.
                                // Keep all candidates rather than overwriting
                                // an earlier branch's conditional source.
                                if !leaf.guards.is_empty() {
                                    this.charge_pointer_conditions(&leaf.guards,installation)?;
                                    let guards=leaf.guards.into_iter().map(|(fields,tag)| {
                                        (std::iter::once(access.parameter).chain(fields).collect(),tag)
                                    }).collect();
                                    parameter_guards.entry(path.clone()).or_default()
                                        .insert(Place {root:borrowed_root,fields:vec![]},guards);
                                }
                                parameter_origins.entry(path).or_insert_with(BTreeSet::new)
                                    .insert(Place {root:borrowed_root,fields:vec![]});
                                frame.memory_exposed=true;
                                frame.effect_borrows.insert(borrowed_root);
                                this.report.borrowed_roots.insert(borrowed_root,BorrowedRoot::Parameter {
                                    parameter:access.parameter,input_fields:access.input_fields.clone(),pointee:leaf.pointee,
                                });
                            }
                        }
                        this.bind_pattern_with_pointer_guards(
                            pattern,
                            &input,
                            &parameter_origins,
                            &parameter_guards,
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
                if let Some(summary) = parameter_summary {
                    local_accesses.extend(summary.accesses);
                }
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
            let mut fallback_parameter = None;
            let mut parameter_accesses = local_accesses;
            if let Some(slot) = handler.fallback {
                for input in &self.effect_result_inputs {
                    input.charge(&mut self.fuel, installation)?;
                    if input.slot == slot
                        && input.operation == handler.operation
                        && input.abi == handler.abi
                        && input.signature == handler.signature
                    {
                        completions.extend(input.values.iter().cloned());
                        fallback_parameter = input.forwarded_parameters.clone();
                        parameter_accesses.extend(input.parameter_accesses.iter().cloned());
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
                let mut copy_parameters = BTreeSet::new();
                for (index, parameter) in parameters.iter().enumerate() {
                    self.fuel = self.fuel.checked_sub(1).ok_or_else(|| {
                        error(installation, "effect copy premise budget exhausted")
                    })?;
                    if (self.delivery)(&parameter.ty)
                        .map_err(|reason| error(installation, reason))?
                        == Delivery::Copy
                    {
                        copy_parameters.insert(index);
                    }
                }
                result_inputs.push(EffectResultInput {
                    parameter_accesses,
                    copy_parameters,
                    slot: handler_index,
                    operation: handler.operation.clone(),
                    abi: handler.abi,
                    signature: handler.signature.clone(),
                    values: completions,
                    forwarded_parameters: {
                        let local = super::super::effect_transfer::forwarded_parameters(
                            handler,
                            &mut self.fuel,
                        )
                        .map_err(|reason| error(installation, reason))?;
                        if handler.fallback.is_some() {
                            super::super::effect_transfer::with_fallback(
                                local,
                                fallback_parameter.as_ref(),
                                &mut self.fuel,
                            )
                            .map_err(|reason| error(installation, reason))?
                        } else {
                            local
                        }
                    },
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
            input
                .charge_parameter_accesses(&mut self.fuel)
                .map_err(|reason| error(expr.source, reason))?;
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
        let ty = inner
            .types
            .get(&root(binding))
            .ok_or_else(|| error(source, "closure borrowed binding type is missing"))?;
        let (pointer_depth, _) =
            super::super::memory::pointer_representation(self.package, ty, &mut self.fuel)
                .map_err(|reason| error(source, reason))?
                .ok_or_else(|| {
                    error(
                        source,
                        "closure borrowed binding has no pointer representation",
                    )
                })?;
        let borrowed = inner
            .pointers
            .get(&Place {
                root: root(binding),
                fields: vec![0; pointer_depth],
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
            input
                .charge_parameter_accesses(&mut self.fuel)
                .map_err(|reason| error(source, reason))?;
            self.fuel = self
                .fuel
                .checked_sub(
                    input
                        .forwarded_parameters
                        .as_ref()
                        .map_or(0, BTreeSet::len)
                        .saturating_add(input.copy_parameters.len()),
                )
                .ok_or_else(|| error(source, "effect transfer premise budget exhausted"))?;
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
    /// An established row may mutate stable captures. A proven identity transfer
    /// retains candidate operand origins; other inputs require data-only layout
    /// or checked Copy permission before their aliases can be ignored.
    pub(super) fn installed_effect_memory_inputs(
        &mut self,
        handlers: &[nir::program::EffectHandler],
        outer: &[nir::effects::Operation],
        results: &[EffectResultInput],
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
        for (handler_index, handler) in handlers.iter().enumerate() {
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
            let mut forwarded =
                super::super::effect_transfer::forwarded_parameters(handler, &mut self.fuel)
                    .map_err(|reason| error(source, reason))?;
            if let Some(slot) = handler.fallback {
                self.fuel = self
                    .fuel
                    .checked_sub(inputs.len())
                    .ok_or_else(|| error(source, "fallback memory premise budget exhausted"))?;
                let previous = inputs.iter().find(|input| {
                    input.operation == handler.operation
                        && input.abi == handler.abi
                        && input.signature == handler.signature
                });
                forwarded = super::super::effect_transfer::with_fallback(
                    forwarded,
                    previous.and_then(|input| input.forwarded_parameters.as_ref()),
                    &mut self.fuel,
                )
                .map_err(|reason| error(source, reason))?;
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
            let identity_transfer = (super::super::memory::effect_transfer_layout(
                self.package,
                result,
                &mut self.fuel,
            )
            .map_err(|reason| error(source, reason))?
                || super::super::memory::effect_data_layout(
                    self.package,
                    result,
                    &mut self.fuel,
                    0,
                )
                .map_err(|reason| error(source, reason))?)
                && forwarded.as_ref().is_some_and(|indices| {
                    !indices.is_empty()
                        && indices
                            .iter()
                            .all(|slot| parameters.get(*slot).is_some_and(|p| p.ty == **result))
                });
            if !identity_transfer
                && !super::super::memory::effect_data_layout(
                    self.package,
                    result,
                    &mut self.fuel,
                    0,
                )
                .map_err(|reason| error(source, reason))?
            {
                return Ok(vec![]);
            }
            let mut copy_parameters = BTreeSet::new();
            for (slot, parameter) in parameters.iter().enumerate() {
                if identity_transfer
                    && forwarded
                        .as_ref()
                        .is_some_and(|indices| indices.contains(&slot))
                {
                    continue;
                }
                if (self.delivery)(&parameter.ty).map_err(|reason| error(source, reason))?
                    == Delivery::Copy
                    && super::super::memory::effect_transfer_layout(
                        self.package,
                        &parameter.ty,
                        &mut self.fuel,
                    )
                    .map_err(|reason| error(source, reason))?
                {
                    copy_parameters.insert(slot);
                    continue;
                }
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
            let mut parameter_accesses = BTreeSet::new();
            for result in results {
                result.charge(&mut self.fuel, source)?;
                if result.slot == handler_index
                    && result.operation == handler.operation
                    && result.abi == handler.abi
                    && result.signature == handler.signature
                {
                    parameter_accesses.extend(result.parameter_accesses.iter().cloned());
                }
            }
            inputs.push(super::super::memory::EffectInput {
                parameter_accesses,
                copy_parameters,
                forwarded_parameters: forwarded,
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
        let value = self.handler_value_evidence(flow, source)?;
        self.handler_completions.push(value);
        Ok(())
    }
    fn handler_value_evidence(
        &mut self,
        flow: &Flow,
        source: SourceRange,
    ) -> Result<std::sync::Arc<HandlerReturnValue>, Error> {
        self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or_else(|| error(source, "handler value evidence budget exhausted"))?;
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
        Ok(std::sync::Arc::new(HandlerReturnValue {
            source,
            callable,
            fields,
            variants,
        }))
    }
    fn restore_effect_result(
        &mut self,
        expression: &Expr,
        flow: &mut Flow,
        arguments: &[std::sync::Arc<HandlerReturnValue>],
    ) -> Result<bool, Error> {
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
                values = Some(if let Some(parameters) = &input.forwarded_parameters {
                    let mut values = Vec::new();
                    for parameter in parameters {
                        values.push(
                            arguments
                                .get(*parameter)
                                .ok_or_else(|| {
                                    error(source, "effect forwarded parameter is out of range")
                                })?
                                .clone(),
                        );
                    }
                    values
                } else {
                    input.values.clone()
                });
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
    let Term::Function { parameters, result } = signature.callable_signature() else {
        return Ok(vec![]);
    };
    let Term::Computation { effects, .. } = result.as_ref() else {
        return Ok(vec![]);
    };
    let mut pointer_context =
        contains_pointer_value(Some(package), completion_type(result), fuel, source, 0)?;
    for parameter in parameters {
        pointer_context |= contains_pointer_value(Some(package), &parameter.ty, fuel, source, 0)?;
    }
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
            .instantiate_parameters(
                &declaration
                    .parameter_binders()
                    .map_err(|e| error(source, e.to_string()))?,
                &operation.arguments,
            )
            .map_err(|e| error(source, e.to_string()))?;
        let Term::Function { parameters, result } = signature else {
            return Err(error(source, "effect result signature missing"));
        };
        let mut operation_context = contains_effect_value(Some(package), &result, fuel, source, 0)?;
        for parameter in &parameters {
            operation_context |=
                contains_pointer_value(Some(package), &parameter.ty, fuel, source, 0)?;
        }
        if pointer_context || operation_context {
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
