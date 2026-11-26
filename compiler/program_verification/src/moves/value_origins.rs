//! Value provenance follows evaluation order and retains temporary ownership.
//! Effect installation, representation transport, and recursive control nodes
//! have separate frames so unrelated operation state does not accumulate.
use super::*;

impl<F: FnMut(&Term) -> Result<Delivery, String>, G: FnMut(&Term) -> Result<Shape, String>>
    Analyzer<'_, F, G>
{
    pub(super) fn evaluate_value_origins_inner(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        if !flow.live {
            return Ok(BTreeMap::new());
        }
        if self.fuel == 0 || depth > 128 {
            return Err(error(expr.source, "value origin budget exhausted"));
        }
        self.fuel -= 1;
        if matches!(expr.kind, K::EffectHandle { .. }) {
            return self.installed_effect_origins(expr, flow, depth);
        }
        if let K::EffectRequest {
            abi,
            signature,
            arguments,
            ..
        } = &expr.kind
        {
            return self.effect_request_resources(
                expr,
                *abi,
                signature,
                arguments,
                flow,
                depth + 1,
            );
        }
        // Transparent evaluation boundaries must run before pointer handling:
        // a run can deliver a borrowed pointer or a closure with provenance.
        match &expr.kind {
            K::EffectComputation { effects, body } => {
                // Term::computation canonicalizes an empty row to its result.
                // The explicit boundary still carries evaluation/provenance.
                if effects.is_empty()
                    && (expr.ty == body.ty || body.ty == Term::Primitive(nir::Primitive::Never))
                {
                    return self.evaluate_value_origins(body, flow, depth + 1);
                }
                let Term::Computation {
                    effects: declared,
                    result,
                } = &expr.ty
                else {
                    return Err(error(
                        expr.source,
                        "computation boundary requires a computation type",
                    ));
                };
                if effects != declared
                    || (body.ty != **result && body.ty != Term::Primitive(nir::Primitive::Never))
                {
                    return Err(error(
                        expr.source,
                        "computation boundary disagrees with its completion type",
                    ));
                }
                return self.evaluate_value_origins(body, flow, depth + 1);
            }
            K::EffectRun { computation, slots } => {
                let Term::Computation { effects, result } = &computation.ty else {
                    return Err(error(expr.source, "effect run requires a computation"));
                };
                if expr.ty != **result {
                    return Err(error(
                        expr.source,
                        "effect run disagrees with its completion type",
                    ));
                }
                let previous = self.copy_effect_result_inputs(expr.source)?;
                let mut routed = vec![];
                if slots.len() != effects.len() {
                    return Err(error(expr.source, "effect result routing arity mismatch"));
                }
                for (slot, (operation, outer_slot)) in effects.iter().zip(slots).enumerate() {
                    for input in &previous {
                        input.charge(&mut self.fuel, expr.source)?;
                        if input.slot == *outer_slot && &input.operation == operation {
                            let mut input = input.clone();
                            input.slot = slot;
                            routed.push(input);
                            break;
                        }
                    }
                }
                self.effect_result_inputs = routed;
                let result = self.evaluate_value_origins(computation, flow, depth + 1);
                self.effect_result_inputs = previous;
                return result;
            }
            _ => {}
        }
        if matches!(expr.kind, K::NewtypeConvert(_)) {
            return self.converted_value_origins(expr, flow, depth);
        }
        if let K::ErrorConvert {
            conversion:
                nir::error_results::Conversion::Embed | nir::error_results::Conversion::Product,
            value,
        } = &expr.kind
        {
            return self.error_transport_origins(expr, value, flow, depth);
        }
        if let K::UnsafeScope(value) = &expr.kind {
            if value.ty != expr.ty {
                return Err(error(expr.source, "unsafe scope changes its body type"));
            }
            return self.evaluate_value_origins(value, flow, depth + 1);
        }
        if let K::Snapshot(value) = &expr.kind {
            if value.ty != expr.ty
                || (self.delivery)(&expr.ty).map_err(|e| error(expr.source, e))? != Delivery::Copy
            {
                return Err(error(
                    expr.source,
                    "entry snapshot requires checked Copy permission",
                ));
            }
            return self.evaluate_value_origins(value, flow, depth + 1);
        }
        if matches!(expr.kind, K::Call { .. } | K::ReadPointer(_))
            && !matches!(completion_type(&expr.ty), Term::Pointer(_))
        {
            if let Some((pointer_depth, _)) =
                self.pointer_representation(completion_type(&expr.ty), expr.source)?
            {
                let targets = self.evaluate_pointer_value(expr, flow, depth + 1)?;
                flow.value_pointer_guards.clear();
                return Ok(targets
                    .map(|targets| BTreeMap::from([(vec![0; pointer_depth], targets)]))
                    .unwrap_or_default());
            }
        }
        if matches!(completion_type(&expr.ty), Term::Pointer(_)) {
            let targets = self.evaluate_pointer_value(expr, flow, depth + 1)?;
            flow.value_pointer_guards.clear();
            return Ok(targets
                .map(|targets| BTreeMap::from([(vec![], targets)]))
                .unwrap_or_default());
        }
        match &expr.kind {
            K::ReadPointer(pointer)
                if !self.pointer_input_layout(&expr.ty, expr.source)?.is_empty() =>
            {
                let origins = self.read_aggregate_origins(expr, pointer, flow, depth + 1)?;
                self.capture_envs_for_type(completion_type(&expr.ty), flow, false, expr.source)?;
                Ok(origins)
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
                env_arguments,
            } if self.package.is_some()
                && matches!(
                    completion_type(&expr.ty),
                    Term::Tuple(_)
                        | Term::Record(_)
                        | Term::Nominal { .. }
                        | Term::NormalForm { .. }
                        | Term::Closure { .. }
                        | Term::ErrorResult { .. }
                        | Term::Optional(_)
                )
                && self.storage_inputs_are_closed(completion_type(&expr.ty), expr.source)?
                && !self
                    .pointer_input_layout(completion_type(&expr.ty), expr.source)?
                    .is_empty() =>
            {
                let (origins, guards) = self.evaluate_named_pointer_aggregate(
                    expr,
                    callee,
                    arguments,
                    env_arguments,
                    parameter_bindings,
                    flow,
                    depth,
                )?;
                flow.value_pointer_guards = guards;
                self.capture_envs_for_type(completion_type(&expr.ty), flow, false, expr.source)?;
                Ok(origins)
            }
            K::Closure { .. } => {
                flow.constructed_capture_origins.clear();
                flow.value_captured_envs.clear();
                self.expr(expr, flow, depth + 1)?;
                self.capture_envs_for_type(completion_type(&expr.ty), flow, true, expr.source)?;
                Ok(std::mem::take(&mut flow.constructed_capture_origins))
            }
            K::TraitObjectType { .. } => self.trait_object_type_origins(expr, flow, depth),
            K::DynCast(cast) => self.dyn_cast_origins(expr, cast, flow, depth),
            K::DynCall(call) => self.dyn_call_origins(expr, call, flow, depth),
            K::AnonymousField { .. } | K::Field { .. } if expression_place(expr).is_none() => {
                self.project_temporary(expr, flow, depth + 1)
            }
            K::NamedField { value, name } => {
                self.named_field_origins(expr, value, name, flow, depth)
            }
            K::EnumVariantFieldPtr {
                pointer,
                variant,
                field,
            } => self.enum_variant_field_ptr_origins(expr, pointer, variant, field, flow, depth),
            K::ConstructStruct { fields } => self.evaluate_value_origins(fields, flow, depth + 1),
            K::ConstructEnum { variant, fields } => {
                self.expr(variant, flow, depth + 1)?;
                self.evaluate_value_origins(fields, flow, depth + 1)
            }
            K::TupleAppend { tuple, element } => {
                self.tuple_append_origins(tuple, element, flow, depth)
            }
            K::Match { value, arms } => self.in_temporary_scope(
                expr.source,
                TemporaryBoundary::Match,
                flow,
                |this, flow| this.match_arms(value, arms, flow, expr.source, depth + 1),
            ),
            K::Tuple(_)
            | K::AnonymousRecord { .. }
            | K::Record { .. }
            | K::OptionalPresent(_)
            | K::OptionalAbsent
            | K::Variant { .. }
            | K::ErrorConvert { .. } => self.constructed_value_origins(expr, flow, depth),
            K::Block { .. } | K::CallPreparation { .. } => {
                self.block_value_origins(expr, flow, depth)
            }
            K::If {
                else_branch: Some(_),
                ..
            } => self.conditional_value_origins(expr, flow, depth),
            _ => {
                let mut origins = BTreeMap::new();
                let mut leaves = BTreeMap::new();
                let place_source = expression_place(expr);
                let place_envs = if let Some(source) = &place_source {
                    for (slot, callable) in &flow.callable_fields {
                        self.fuel = self
                            .fuel
                            .checked_sub(slot.fields.len().saturating_add(1))
                            .ok_or_else(|| error(expr.source, "callable field budget exhausted"))?;
                        if slot.root == source.root && slot.fields.starts_with(&source.fields) {
                            leaves.insert(
                                slot.fields[source.fields.len()..].to_vec(),
                                callable.clone(),
                            );
                        }
                    }
                    for (slot, targets) in &flow.pointers {
                        self.fuel = self
                            .fuel
                            .checked_sub(slot.fields.len().saturating_add(1))
                            .ok_or_else(|| error(expr.source, "value origin budget exhausted"))?;
                        if slot.root == source.root && slot.fields.starts_with(&source.fields) {
                            self.pointer_work(targets, expr.source)?;
                            origins.insert(
                                slot.fields[source.fields.len()..].to_vec(),
                                targets.clone(),
                            );
                        }
                    }
                    flow.state
                        .captured_envs_under(source, &mut self.fuel)
                        .map_err(|e| error(expr.source, format!("captured env extract: {e:?}")))?
                } else {
                    BTreeMap::new()
                };
                flow.value_captured_envs.clear();
                self.expr(expr, flow, depth + 1)?;
                if place_source.is_some() {
                    flow.value_callables = leaves;
                    flow.value_captured_envs = place_envs;
                } else {
                    flow.value_pointer_guards.clear();
                    self.capture_envs_for_type(
                        completion_type(&expr.ty),
                        flow,
                        false,
                        expr.source,
                    )?;
                }
                Ok(origins)
            }
        }
    }

    #[inline(never)]
    fn installed_effect_origins(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        if let K::EffectHandle {
            computation,
            handlers,
            outer,
        } = &expr.kind
        {
            let (may_reject, return_values, loop_targets, mut result_inputs) =
                self.handler_resources(expr, handlers, flow, depth + 1)?;
            let may_return = !return_values.is_empty();
            if may_reject {
                // Providers run before any subject argument is prepared. Their
                // failure therefore unwinds the installer's current owners.
                let mut rejected = flow.clone();
                self.exit(&mut rejected, expr.source, CleanupReason::PredicateFailure)?;
            }
            if may_return && self.handler_result.is_some() {
                for value in &return_values {
                    self.check_handler_return_origins(flow, &value.origins, expr.source)?;
                }
                self.fuel = self
                    .fuel
                    .checked_sub(return_values.len())
                    .ok_or_else(|| error(expr.source, "handler return summary budget exhausted"))?;
                self.report
                    .lexical_returns
                    .extend(return_values.iter().cloned());
            }
            for mut input in self.copy_effect_result_inputs(expr.source)? {
                if outer.get(input.slot) == Some(&input.operation) {
                    input.slot = input
                        .slot
                        .checked_add(handlers.len())
                        .ok_or_else(|| error(expr.source, "effect result slot overflow"))?;
                    result_inputs.push(input);
                }
            }
            let memory_inputs = self.installed_effect_memory_inputs(
                handlers,
                outer,
                &result_inputs,
                flow,
                expr.source,
            )?;
            self.fuel = self
                .fuel
                .checked_sub(flow.effect_mutable_roots.len())
                .ok_or_else(|| {
                    error(
                        expr.source,
                        "handler capture mutation scope budget exhausted",
                    )
                })?;
            let previous_mutable = flow.effect_mutable_roots.clone();
            for handler in handlers {
                for arm in &handler.arms {
                    let captures = nir::free_bindings::collect_arm(arm, &mut self.fuel)
                        .map_err(|reason| error(expr.source, reason))?;
                    for binding in captures {
                        let id = root(binding);
                        flow.effect_mutable_roots.insert(id);
                        flow.enum_domains.remove(&id);
                    }
                }
            }
            let previous_memory = std::mem::replace(&mut self.effect_memory_inputs, memory_inputs);
            let previous_results = std::mem::replace(&mut self.effect_result_inputs, result_inputs);
            let previous = self.effect_return_enabled;
            let return_floor = self.effect_return_values.len();
            self.effect_return_values.extend(return_values);
            self.fuel = self
                .fuel
                .checked_sub(
                    self.effect_loop_targets
                        .len()
                        .saturating_add(loop_targets.len()),
                )
                .ok_or_else(|| error(expr.source, "effect loop target budget exhausted"))?;
            let previous_loops = self.effect_loop_targets.clone();
            self.effect_loop_targets
                .extend(loop_targets.iter().copied());
            for &(target, is_break) in &loop_targets {
                if self.handler_result.is_some()
                    && !self.loops.iter().any(|frame| frame.identity == target)
                {
                    self.report.lexical_loop_exits.insert((target, is_break));
                }
            }
            self.effect_return_enabled |= may_return;
            let result = (|| {
                if (may_return || !loop_targets.is_empty())
                    && handlers
                        .iter()
                        .any(|h| h.abi == nir::effects::Abi::DirectPassing)
                {
                    if self.handler_result.is_some() {
                        self.effect_escape_checkpoint(flow, expr.source)?;
                    } else {
                        if may_return {
                            self.effect_return_checkpoint(flow, expr.source, depth + 1)?;
                        }
                        self.effect_loop_checkpoints(flow, expr.source)?;
                    }
                }
                self.evaluate_value_origins(computation, flow, depth + 1)
            })();
            self.effect_return_enabled = previous;
            self.effect_return_values.truncate(return_floor);
            self.effect_loop_targets = previous_loops;
            flow.effect_mutable_roots = previous_mutable;
            self.effect_memory_inputs = previous_memory;
            self.effect_result_inputs = previous_results;
            return result;
        }
        unreachable!("installed_effect_origins dispatcher")
    }

    #[inline(never)]
    fn converted_value_origins(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        if let K::NewtypeConvert(value) = &expr.kind {
            let package = self.package.ok_or_else(|| {
                error(expr.source, "newtype conversion requires nominal metadata")
            })?;
            let conversion = nir::newtypes::conversion(
                &package.newtypes,
                &value.ty,
                &expr.ty,
                &[],
                &mut self.fuel,
            )
            .map_err(|reason| error(expr.source, reason.to_string()))?
            .ok_or_else(|| error(expr.source, "invalid newtype conversion edge"))?;
            let origins = self.evaluate_value_origins(value, flow, depth + 1)?;
            if !flow.live {
                return Ok(origins);
            }
            // These are private representation paths, not source projections.
            // Evaluate the complete operand once before moving its evidence.
            fn remap<T>(
                values: BTreeMap<Vec<usize>, T>,
                direction: nir::newtypes::Direction,
                fuel: &mut usize,
                source: SourceRange,
            ) -> Result<BTreeMap<Vec<usize>, T>, Error> {
                let mut result = BTreeMap::new();
                for (mut path, value) in values {
                    *fuel = fuel
                        .checked_sub(path.len().saturating_add(1))
                        .ok_or_else(|| error(source, "newtype origin budget exhausted"))?;
                    match direction {
                        nir::newtypes::Direction::Wrap => path.insert(0, 0),
                        nir::newtypes::Direction::Unwrap => {
                            if path.first() != Some(&0) {
                                return Err(error(source, "invalid newtype representation origin"));
                            }
                            path.remove(0);
                        }
                    }
                    result.insert(path, value);
                }
                Ok(result)
            }
            let direction = conversion.direction();
            let origins = remap(origins, direction, &mut self.fuel, expr.source)?;
            let mut guards = remap(
                std::mem::take(&mut flow.value_pointer_guards),
                direction,
                &mut self.fuel,
                expr.source,
            )?;
            for entries in guards.values_mut() {
                for conditions in entries.values_mut() {
                    self.charge_pointer_conditions(conditions, expr.source)?;
                    for (path, _) in conditions {
                        match direction {
                            nir::newtypes::Direction::Wrap => path.insert(0, 0),
                            nir::newtypes::Direction::Unwrap => {
                                if path.first() != Some(&0) {
                                    return Err(error(
                                        expr.source,
                                        "invalid newtype pointer guard",
                                    ));
                                }
                                path.remove(0);
                            }
                        }
                    }
                }
            }
            flow.value_pointer_guards = guards;
            flow.value_callables = remap(
                std::mem::take(&mut flow.value_callables),
                direction,
                &mut self.fuel,
                expr.source,
            )?;
            flow.value_variants = remap(
                std::mem::take(&mut flow.value_variants),
                direction,
                &mut self.fuel,
                expr.source,
            )?;
            flow.value_callable = flow.value_callables.get(&vec![]).cloned();
            return Ok(origins);
        }
        unreachable!("converted_value_origins dispatcher")
    }

    #[inline(never)]
    fn constructed_value_origins(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        match &expr.kind {
            K::Tuple(_)
            | K::AnonymousRecord { .. }
            | K::Record { .. }
            | K::OptionalPresent(_)
            | K::OptionalAbsent
            | K::Variant { .. }
            | K::ErrorConvert { .. } => self.pending_frame(expr.source, |this| {
                let (fields, variant) = crate::memory::owned_constructor(expr, &mut this.fuel)
                    .map_err(|reason| error(expr.source, reason))?;
                let mut origins = BTreeMap::new();
                let mut callables = BTreeMap::new();
                let mut domains = EnumDomains::new();
                let mut pointer_guards = ValuePointerGuards::new();
                let mut captured_envs = BTreeMap::new();
                if let Some(tag) = variant {
                    domains.insert(vec![], BTreeSet::from([tag]));
                }
                for (index, field) in fields.iter().enumerate() {
                    let children = this.evaluate_value_origins(field, flow, depth + 1)?;
                    if !flow.live {
                        return Ok(BTreeMap::new());
                    }
                    let child_envs = flow.value_captured_envs.clone();
                    for (path, envs) in child_envs {
                        this.fuel = this
                            .fuel
                            .checked_sub(path.len().saturating_add(1))
                            .ok_or_else(|| {
                                error(expr.source, "constructed captured env budget exhausted")
                            })?;
                        let mut nested = vec![index];
                        nested.extend(path);
                        captured_envs.insert(nested, envs);
                    }
                    for (mut slot, mut entries) in std::mem::take(&mut flow.value_pointer_guards) {
                        this.fuel = this
                            .fuel
                            .checked_sub(slot.len().saturating_add(1))
                            .ok_or_else(|| {
                                error(expr.source, "constructed pointer guard budget exhausted")
                            })?;
                        slot.insert(0, index);
                        for conditions in entries.values_mut() {
                            this.charge_pointer_conditions(conditions, expr.source)?;
                            for (path, _) in conditions {
                                path.insert(0, index);
                            }
                        }
                        pointer_guards.insert(slot, entries);
                    }
                    for (path, tags) in std::mem::take(&mut flow.value_variants) {
                        this.fuel = this
                            .fuel
                            .checked_sub(path.len().saturating_add(tags.len()).saturating_add(3))
                            .ok_or_else(|| {
                                error(expr.source, "enum domain construction budget exhausted")
                            })?;
                        let mut nested = match variant {
                            Some(tag) => vec![VARIANT_PATH, tag, index],
                            _ => vec![index],
                        };
                        nested.extend(path);
                        domains.insert(nested, tags);
                    }
                    for (path, callable) in std::mem::take(&mut flow.value_callables) {
                        let qualifier = variant;
                        let prefix_len = if qualifier.is_some() { 3 } else { 1 };
                        this.fuel = this
                            .fuel
                            .checked_sub(path.len().saturating_add(prefix_len))
                            .ok_or_else(|| {
                                error(expr.source, "callable origin budget exhausted")
                            })?;
                        let mut nested = match qualifier {
                            Some(variant) => vec![VARIANT_PATH, variant, index],
                            None => vec![index],
                        };
                        nested.extend(path);
                        callables.insert(nested, callable);
                    }
                    this.remember_pending_origins(field, index, flow, children.clone())?;
                    for (path, targets) in children {
                        this.fuel = this
                            .fuel
                            .checked_sub(path.len().saturating_add(1))
                            .ok_or_else(|| error(expr.source, "value origin budget exhausted"))?;
                        let mut nested = vec![index];
                        nested.extend(path);
                        if let Some(tag) = variant {
                            this.pointer_work(&targets, expr.source)?;
                            let entries = pointer_guards.entry(nested.clone()).or_default();
                            for target in &targets {
                                entries
                                    .entry(target.clone())
                                    .or_default()
                                    .push((vec![], tag));
                            }
                        }
                        origins.insert(nested, targets);
                    }
                }
                flow.value_callables = callables;
                flow.value_variants = domains;
                flow.value_pointer_guards = pointer_guards;
                flow.value_captured_envs = captured_envs;
                this.capture_envs_for_type(completion_type(&expr.ty), flow, true, expr.source)?;
                Ok(origins)
            }),
            _ => unreachable!("constructed_value_origins dispatcher"),
        }
    }

    #[inline(never)]
    fn block_value_origins(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        match &expr.kind {
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let outer = flow.roots.clone();
                for statement in statements {
                    if matches!(expr.kind, K::CallPreparation { .. }) {
                        self.prepared_binding(statement, flow, depth + 1)?;
                    } else {
                        self.full_expression(statement, flow, depth + 1)?;
                    }
                }
                let origins = if let Some(result) = result {
                    self.evaluate_value_origins(result, flow, depth + 1)?
                } else {
                    flow.value_callable = None;
                    flow.value_callables.clear();
                    flow.value_variants.clear();
                    flow.value_captured_envs.clear();
                    BTreeMap::new()
                };
                if flow.live {
                    if result.is_some()
                        && self.temporary_scope.boundary == TemporaryBoundary::FunctionResult
                        && self.temporary_scope.source == expr.source
                    {
                        self.finish_temporaries(flow, expr.source, self.temporary_scope_id)?;
                    }
                    let locals: Vec<_> = flow
                        .order
                        .iter()
                        .rev()
                        .filter(|id| !outer.contains(id))
                        .copied()
                        .collect();
                    self.check_scope_origins(&origins, &locals, expr.source)?;
                    for id in locals {
                        self.retire(flow, id, expr.source)?;
                    }
                }
                Ok(origins)
            }
            _ => unreachable!("block_value_origins dispatcher"),
        }
    }

    #[inline(never)]
    fn conditional_value_origins(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        match &expr.kind {
            K::If {
                condition,
                then_branch,
                else_branch: Some(other),
            } => {
                self.in_temporary_scope(
                    condition.source,
                    TemporaryBoundary::Condition,
                    flow,
                    |this, flow| this.expr(condition, flow, depth + 1),
                )?;
                if !flow.live {
                    return Ok(BTreeMap::new());
                }
                if let Some(value) = constant_bool(condition, 0) {
                    return self.evaluate_value_origins(
                        if value { then_branch } else { other },
                        flow,
                        depth + 1,
                    );
                }
                let mut right = flow.clone();
                let mut left_origins = self.evaluate_value_origins(then_branch, flow, depth + 1)?;
                let right_origins = self.evaluate_value_origins(other, &mut right, depth + 1)?;
                match (flow.live, right.live) {
                    (true, true) => {
                        let result_guards = self.join_result_pointer_guards(
                            &flow.value_pointer_guards,
                            &left_origins,
                            &right.value_pointer_guards,
                            &right_origins,
                            expr.source,
                            Some((
                                completion_type(&expr.ty),
                                &flow.value_variants,
                                &right.value_variants,
                            )),
                        )?;
                        self.join_value_origins(
                            completion_type(&expr.ty),
                            &mut left_origins,
                            &flow.value_variants,
                            &right_origins,
                            &right.value_variants,
                            expr.source,
                        )?;
                        self.join_flow(flow, &right, expr.source)?;
                        flow.value_pointer_guards = result_guards;
                        Ok(left_origins)
                    }
                    (false, true) => {
                        *flow = right;
                        Ok(right_origins)
                    }
                    (true, false) => Ok(left_origins),
                    (false, false) => Ok(BTreeMap::new()),
                }
            }
            _ => unreachable!("conditional_value_origins dispatcher"),
        }
    }

    #[inline(never)]
    fn trait_object_type_origins(
        &mut self,
        expr: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        let K::TraitObjectType { arguments, .. } = &expr.kind else {
            unreachable!("trait_object_type_origins dispatcher");
        };
        for arg in arguments {
            self.expr(arg, flow, depth + 1)?;
            if !flow.live {
                return Ok(BTreeMap::new());
            }
        }
        Ok(BTreeMap::new())
    }

    #[inline(never)]
    fn dyn_cast_origins(
        &mut self,
        expr: &Expr,
        cast: &nir::program::DynCast,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        let mut origins = BTreeMap::new();
        let value_origins = self.evaluate_value_origins(&cast.value, flow, depth + 1)?;
        if !flow.live {
            return Ok(BTreeMap::new());
        }
        for (slot, env_arg) in cast.env_arguments.iter().enumerate() {
            let child_origins = self.evaluate_value_origins(env_arg, flow, depth + 1)?;
            if !flow.live {
                return Ok(BTreeMap::new());
            }
            for (mut path, targets) in child_origins {
                self.fuel = self
                    .fuel
                    .checked_sub(path.len().saturating_add(1))
                    .ok_or_else(|| error(expr.source, "dyn env origin budget exhausted"))?;
                path.insert(0, slot);
                origins.insert(path, targets);
            }
        }
        if cast.by_pointer {
            if let Some(targets) = value_origins.get(&vec![]) {
                self.pointer_work(targets, expr.source)?;
                let mut collected = targets.clone();
                for target in targets {
                    for (slot, nested) in &flow.pointers {
                        if slot.root == target.root && slot.fields.starts_with(&target.fields) {
                            self.pointer_work(nested, expr.source)?;
                            collected.extend(nested.iter().cloned());
                        }
                    }
                }
                origins.insert(vec![], collected);
            }
        } else {
            let mut nested = BTreeSet::new();
            for targets in value_origins.values() {
                self.pointer_work(targets, expr.source)?;
                nested.extend(targets.iter().cloned());
            }
            if !nested.is_empty() {
                origins.insert(vec![], nested);
            }
        }
        Ok(origins)
    }

    #[inline(never)]
    fn dyn_call_origins(
        &mut self,
        expr: &Expr,
        call: &nir::program::DynCall,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        let mut flat_origins = BTreeSet::new();
        let receiver_origins = if call.by_value_receiver {
            let origins = self.evaluate_value_origins(&call.receiver, flow, depth + 1)?;
            for targets in origins.values() {
                self.pointer_work(targets, expr.source)?;
                flat_origins.extend(targets.iter().cloned());
            }
            origins
        } else {
            let targets = self.evaluate_pointer_value(&call.receiver, flow, depth + 1)?;
            if !flow.live {
                return Ok(BTreeMap::new());
            }
            let mut env = BTreeMap::new();
            if let Some(targets) = &targets {
                for target in targets {
                    flow.state.require_present(target).map_err(|reason| {
                        error(
                            expr.source,
                            format!(
                                "dynamic method call on unavailable receiver {target}: {reason:?}"
                            ),
                        )
                    })?;
                    let mut saw_pointer_slot = false;
                    for (slot, val) in &flow.pointers {
                        if slot.root == target.root && slot.fields.starts_with(&target.fields) {
                            let rel = slot.fields[target.fields.len()..].to_vec();
                            if rel.is_empty() {
                                saw_pointer_slot = true;
                                for pointed in val {
                                    if flow.types.contains_key(&pointed.root) {
                                        flow.state.require_present(pointed).map_err(|reason| {
                                            error(
                                                expr.source,
                                                format!(
                                                    "dynamic method call dereferences moved or dropped target {pointed}: {reason:?}"
                                                ),
                                            )
                                        })?;
                                    }
                                    flat_origins.insert(pointed.clone());
                                    for (nested_slot, nested_val) in &flow.pointers {
                                        if nested_slot.root == pointed.root
                                            && nested_slot.fields.starts_with(&pointed.fields)
                                        {
                                            self.pointer_work(nested_val, expr.source)?;
                                            flat_origins.extend(nested_val.iter().cloned());
                                        }
                                    }
                                }
                            } else {
                                self.pointer_work(val, expr.source)?;
                                flat_origins.extend(val.iter().cloned());
                                env.entry(rel)
                                    .or_insert_with(BTreeSet::new)
                                    .extend(val.iter().cloned());
                            }
                        }
                    }
                    if !saw_pointer_slot {
                        flat_origins.insert(target.clone());
                    }
                }
                self.pointer_work(&flat_origins, expr.source)?;
            }
            env
        };
        if !flow.live {
            return Ok(BTreeMap::new());
        }
        for arg in &call.arguments {
            let arg_origins = self.evaluate_value_origins(&arg.value, flow, depth + 1)?;
            if !flow.live {
                return Ok(BTreeMap::new());
            }
            for targets in arg_origins.values() {
                self.pointer_work(targets, expr.source)?;
                flat_origins.extend(targets.iter().cloned());
            }
        }
        let mut result_origins = BTreeMap::new();
        let out_ty = completion_type(&expr.ty);
        let returns_dyn = matches!(out_ty, Term::TraitObject { .. })
            || matches!(
                out_ty,
                Term::Closure { signature, .. }
                    if matches!(signature.as_ref(), Term::TraitObject { .. })
            );
        if returns_dyn {
            for (path, targets) in receiver_origins {
                if !path.is_empty() {
                    self.pointer_work(&targets, expr.source)?;
                    result_origins.insert(path, targets);
                }
            }
            if !flat_origins.is_empty() {
                self.pointer_work(&flat_origins, expr.source)?;
                result_origins.insert(vec![], flat_origins);
            }
        } else if !flat_origins.is_empty() {
            if matches!(out_ty, Term::Pointer(_)) {
                self.pointer_work(&flat_origins, expr.source)?;
                result_origins.insert(vec![], flat_origins);
            } else if self.storage_inputs_are_closed(out_ty, expr.source)? {
                for leaf in self.pointer_input_layout(out_ty, expr.source)? {
                    self.pointer_work(&flat_origins, expr.source)?;
                    result_origins.insert(leaf.storage_fields, flat_origins.clone());
                }
            }
        }
        Ok(result_origins)
    }

    #[inline(never)]
    fn named_field_origins(
        &mut self,
        _expr: &Expr,
        value: &Expr,
        name: &str,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        let origins = self.evaluate_value_origins(value, flow, depth + 1)?;
        if !flow.live {
            return Ok(BTreeMap::new());
        }
        if let Some(index) = resolve_concrete_field_index_by_name(self.package, &value.ty, name) {
            let mut projected = BTreeMap::new();
            for (path, targets) in origins {
                if let Some(rest) = path.strip_prefix(&[index][..]) {
                    projected.insert(rest.to_vec(), targets);
                }
            }
            flow.value_pointer_guards = flow
                .value_pointer_guards
                .iter()
                .filter_map(|(slot, entries)| {
                    slot.strip_prefix(&[index][..])
                        .map(|rest| (rest.to_vec(), entries.clone()))
                })
                .collect();
            flow.value_variants = flow
                .value_variants
                .iter()
                .filter_map(|(path, tags)| {
                    path.strip_prefix(&[index][..])
                        .map(|rest| (rest.to_vec(), tags.clone()))
                })
                .collect();
            flow.value_callables = flow
                .value_callables
                .iter()
                .filter_map(|(path, callable)| {
                    path.strip_prefix(&[index][..])
                        .map(|rest| (rest.to_vec(), callable.clone()))
                })
                .collect();
            flow.value_callable = flow.value_callables.get(&vec![]).cloned();
            return Ok(projected);
        }
        flow.value_pointer_guards.clear();
        flow.value_variants.clear();
        flow.value_callables.clear();
        flow.value_callable = None;
        Ok(BTreeMap::new())
    }

    #[inline(never)]
    fn enum_variant_field_ptr_origins(
        &mut self,
        expr: &Expr,
        pointer: &Expr,
        variant: &Expr,
        field: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        let targets = self.evaluate_pointer_value(pointer, flow, depth + 1)?;
        self.expr(variant, flow, depth + 1)?;
        self.expr(field, flow, depth + 1)?;
        if !flow.live {
            return Ok(BTreeMap::new());
        }
        let targets = targets.unwrap_or_default();
        self.pointer_work(&targets, expr.source)?;
        let layout = self.pointer_input_layout(completion_type(&expr.ty), expr.source)?;
        let mut origins = BTreeMap::new();
        for leaf in layout {
            if !leaf.guards.is_empty() {
                let mut guarded = PointerTargetGuards::new();
                self.merge_pointer_conditions(&mut guarded, &targets, &leaf.guards, expr.source)?;
                flow.value_pointer_guards
                    .insert(leaf.storage_fields.clone(), guarded);
            }
            origins.insert(leaf.storage_fields, targets.clone());
        }
        Ok(origins)
    }

    #[inline(never)]
    fn tuple_append_origins(
        &mut self,
        tuple: &Expr,
        element: &Expr,
        flow: &mut Flow,
        depth: usize,
    ) -> Result<ValueOrigins, Error> {
        let prefix_len = resolve_tuple_len(self.package, completion_type(&tuple.ty)).unwrap_or(0);
        let mut origins = self.evaluate_value_origins(tuple, flow, depth + 1)?;
        if !flow.live {
            return Ok(BTreeMap::new());
        }
        let mut guards = std::mem::take(&mut flow.value_pointer_guards);
        let mut domains = std::mem::take(&mut flow.value_variants);
        let mut callables = std::mem::take(&mut flow.value_callables);
        let elem_origins = self.evaluate_value_origins(element, flow, depth + 1)?;
        if !flow.live {
            return Ok(BTreeMap::new());
        }
        for (path, targets) in elem_origins {
            let mut nested = vec![prefix_len];
            nested.extend(path);
            origins.insert(nested, targets);
        }
        for (mut slot, mut entries) in std::mem::take(&mut flow.value_pointer_guards) {
            slot.insert(0, prefix_len);
            for conditions in entries.values_mut() {
                for (path, _) in conditions {
                    path.insert(0, prefix_len);
                }
            }
            guards.insert(slot, entries);
        }
        for (path, tags) in std::mem::take(&mut flow.value_variants) {
            let mut nested = vec![prefix_len];
            nested.extend(path);
            domains.insert(nested, tags);
        }
        for (path, callable) in std::mem::take(&mut flow.value_callables) {
            let mut nested = vec![prefix_len];
            nested.extend(path);
            callables.insert(nested, callable);
        }
        flow.value_pointer_guards = guards;
        flow.value_variants = domains;
        flow.value_callables = callables;
        flow.value_callable = flow.value_callables.get(&vec![]).cloned();
        Ok(origins)
    }
}
