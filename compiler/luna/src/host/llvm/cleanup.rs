use super::*;
use iir::cleanup::{Boundary, FunctionSchedule, Program, Schedule, Step, Target};

/// Accept only a published recursive named-call edge to a concrete ancestor
/// scope for the same callee. No alias chains, cycles or competing scopes.
pub(super) fn validate_alias<'a>(
    schedule: &'a Schedule,
    alias: &iir::cleanup::ContextAlias,
    fuel: &mut usize,
) -> Result<&'a FunctionSchedule, String> {
    use iir::cleanup::ContextKey;
    *fuel = fuel
        .checked_sub(
            alias
                .context
                .len()
                .saturating_add(alias.target.len())
                .saturating_add(1),
        )
        .ok_or("host cleanup alias budget exhausted")?;
    let Some(ContextKey::Function { callee, .. }) = alias.context.last() else {
        return Err("host recursive cleanup aliases require a named callee".into());
    };
    if alias.target.len() >= alias.context.len()
        || !alias.context.starts_with(&alias.target)
        || alias.context.iter().any(|key| {
            !matches!(
                key,
                ContextKey::Function { .. } | ContextKey::Closure { .. }
            )
        })
    {
        return Err("host recursive cleanup aliases require an out-of-line ancestor scope".into());
    }
    let mut edges = 0;
    for other in &schedule.aliases {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("host cleanup alias budget exhausted")?;
        if other.instance == alias.instance && other.context == alias.context {
            edges += 1;
        }
    }
    if edges != 1 {
        return Err("host recursive cleanup aliases have competing edges".into());
    }
    let mut target: Option<&FunctionSchedule> = None;
    for scope in &schedule.functions {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("host cleanup alias budget exhausted")?;
        if scope.instance != alias.instance {
            continue;
        }
        if scope.context == alias.context {
            return Err("host recursive cleanup aliases conflict with a concrete scope".into());
        }
        if scope.context == alias.target {
            if target.is_some() {
                return Err("host recursive cleanup aliases have ambiguous targets".into());
            }
            target = Some(scope);
        }
    }
    let target = target.ok_or("host recursive cleanup aliases have no concrete target")?;
    let target_callee = match target.context.last() {
        None => &target.instance,
        Some(ContextKey::Function { callee, .. }) => callee,
        _ => return Err("host recursive cleanup aliases target a different callable kind".into()),
    };
    if target_callee != callee {
        return Err("host recursive cleanup aliases change callee identity".into());
    }
    Ok(target)
}

fn same_scope(left: &FunctionSchedule, right: &FunctionSchedule) -> bool {
    left.sites == right.sites
        && left.discards == right.discards
        && left.transfers == right.transfers
        && left.copies == right.copies
        && left.pattern_transfers == right.pattern_transfers
        && left.pattern_orders == right.pattern_orders
}

/// Defaults and direct handler arms execute inside the selected machine frame.
/// Compare their complete relative paths before sharing code; an arm's nested
/// defaults (and a default's nested handlers) are part of the same body.
pub(super) fn inline_scopes<'a>(
    schedule: &'a Schedule,
    parent: &FunctionSchedule,
    fuel: &mut usize,
) -> Result<Vec<(&'a [iir::cleanup::ContextKey], &'a FunctionSchedule)>, String> {
    use iir::cleanup::ContextKey;
    let mut scopes = Vec::new();
    for child in &schedule.functions {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("host inline cleanup budget exhausted")?;
        if child.instance != parent.instance
            || child.context.len() <= parent.context.len()
            || !child.context.starts_with(&parent.context)
        {
            continue;
        }
        let relative = &child.context[parent.context.len()..];
        let mut separate = false;
        for key in relative {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("host inline cleanup budget exhausted")?;
            match key {
                ContextKey::Default { .. } | ContextKey::EffectHandler { .. } => {}
                ContextKey::Function { .. } | ContextKey::Closure { .. } => {
                    separate = true;
                    break;
                }
            }
        }
        if !separate {
            *fuel = fuel
                .checked_sub(scopes.len())
                .ok_or("host inline cleanup budget exhausted")?;
            if scopes.iter().any(|(path, _)| *path == relative) {
                return Err("host inline cleanup context is ambiguous".into());
            }
            scopes.push((relative, child));
        }
    }
    Ok(scopes)
}

pub(super) fn same_frame(
    schedule: &Schedule,
    left: &FunctionSchedule,
    right: &FunctionSchedule,
    fuel: &mut usize,
) -> Result<bool, String> {
    same_frame_at(schedule, left, right, fuel, 0)
}

fn same_frame_at(
    schedule: &Schedule,
    left: &FunctionSchedule,
    right: &FunctionSchedule,
    fuel: &mut usize,
    depth: usize,
) -> Result<bool, String> {
    if depth >= 128 {
        return Err("host closure dispatch nesting budget exhausted".into());
    }
    // Sharing also shares recursive dispatch. Until equivalent cyclic plans
    // are proven coinductively, different alias targets are not interchangeable.
    let aliases = |parent: &FunctionSchedule, fuel: &mut usize| {
        let mut found = Vec::new();
        for alias in &schedule.aliases {
            *fuel = fuel
                .checked_sub(1 + alias.context.len())
                .ok_or("host recursive dispatch comparison budget exhausted")?;
            if alias.instance == parent.instance && alias.context.starts_with(&parent.context) {
                found.push((
                    &alias.context[parent.context.len()..],
                    &alias.instance,
                    &alias.target,
                ));
            }
        }
        Ok::<_, String>(found)
    };
    let left_aliases = aliases(left, fuel)?;
    let right_aliases = aliases(right, fuel)?;
    *fuel = fuel
        .checked_sub(
            left_aliases
                .len()
                .saturating_mul(right_aliases.len())
                .saturating_add(1),
        )
        .ok_or("host recursive dispatch comparison budget exhausted")?;
    if left_aliases.len() != right_aliases.len()
        || left_aliases
            .iter()
            .any(|alias| right_aliases.iter().filter(|other| *other == alias).count() != 1)
    {
        return Ok(false);
    }
    let children = |parent: &FunctionSchedule, fuel: &mut usize| {
        let mut result = Vec::new();
        for child in &schedule.functions {
            *fuel = fuel
                .checked_sub(1 + child.context.len())
                .ok_or("host closure dispatch comparison budget exhausted")?;
            if child.instance != parent.instance || !child.context.starts_with(&parent.context) {
                continue;
            }
            let path = &child.context[parent.context.len()..];
            if let Some((last, prefix)) = path.split_last()
                && matches!(
                    last,
                    iir::cleanup::ContextKey::Closure { .. }
                        | iir::cleanup::ContextKey::Function { .. }
                )
                && prefix.iter().all(|key| {
                    !matches!(
                        key,
                        iir::cleanup::ContextKey::Function { .. }
                            | iir::cleanup::ContextKey::Closure { .. }
                    )
                })
            {
                result.push((path, child));
            }
        }
        Ok::<_, String>(result)
    };
    let left_children = children(left, fuel)?;
    let right_children = children(right, fuel)?;
    if left_children.len() != right_children.len() {
        return Ok(false);
    }
    for (path, child) in left_children {
        *fuel = fuel
            .checked_sub(right_children.len().saturating_mul(path.len() + 1))
            .ok_or("host closure dispatch comparison budget exhausted")?;
        let matches = right_children
            .iter()
            .filter(|(other, _)| *other == path)
            .collect::<Vec<_>>();
        if matches.len() != 1 || !same_frame_at(schedule, child, matches[0].1, fuel, depth + 1)? {
            return Ok(false);
        }
    }
    if !same_scope(left, right) {
        return Ok(false);
    }
    let left = inline_scopes(schedule, left, fuel)?;
    let right = inline_scopes(schedule, right, fuel)?;
    if left.len() != right.len() {
        return Ok(false);
    }
    for (path, scope) in left {
        let mut matches = 0;
        for (other_path, other_scope) in &right {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("host inline cleanup comparison budget exhausted")?;
            if path == *other_path {
                matches += 1;
                if !same_scope(scope, other_scope) {
                    return Ok(false);
                }
            }
        }
        if matches != 1 {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Collect each published named invocation scope without conflating its plan
/// with another call. Every scope and recursive edge is validated independently.
pub(super) fn schedules_for<'a>(
    schedule: &'a Schedule,
    key: &iir::InstanceKey,
    fuel: &mut usize,
) -> Result<Vec<(usize, &'a FunctionSchedule)>, String> {
    let mut selected = Vec::new();
    for (index, candidate) in schedule.functions.iter().enumerate() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("host cleanup selection budget exhausted")?;
        let belongs = if candidate.context.is_empty() {
            &candidate.instance == key
        } else {
            matches!(candidate.context.last(), Some(iir::cleanup::ContextKey::Function { callee, .. }) if callee == key)
        };
        if !belongs {
            continue;
        }
        inline_scopes(schedule, candidate, fuel)?;
        if !candidate.context.is_empty() {
            for alias in &schedule.aliases {
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or("host cleanup selection budget exhausted")?;
                if alias.instance == candidate.instance
                    && alias.context.starts_with(&candidate.context)
                {
                    validate_alias(schedule, alias, fuel)?;
                }
            }
        }
        selected.push((index, candidate));
    }
    Ok(selected)
}

pub(super) fn canonical_scope<'a>(
    scopes: &[(usize, &'a FunctionSchedule)],
) -> Option<(usize, &'a FunctionSchedule)> {
    scopes
        .iter()
        .find(|(_, scope)| scope.context.is_empty())
        .or(scopes.first())
        .copied()
}

/// Select a canonical plan only when the complete machine dispatch is reusable.
pub(super) fn select_schedule<'a>(
    schedule: &'a Schedule,
    key: &iir::InstanceKey,
    fuel: &mut usize,
) -> Result<Option<&'a FunctionSchedule>, String> {
    let scopes = schedules_for(schedule, key, fuel)?;
    let Some((_, first)) = scopes.first() else {
        return Ok(None);
    };
    for (_, next) in &scopes[1..] {
        if !same_frame(schedule, first, next, fuel)? {
            return Err("host function requires distinct per-call cleanup bodies".into());
        }
    }
    Ok(Some(first))
}

pub(super) fn admitted_direct_program(
    graph: &iir::Package,
    schedule: &Schedule,
    function: &FunctionSchedule,
    program: &Program,
    fuel: &mut usize,
) -> bool {
    use iir::cleanup::ContextKey;
    let base_end = function
        .context
        .iter()
        .rposition(|key| {
            matches!(
                key,
                ContextKey::Function { .. } | ContextKey::Closure { .. }
            )
        })
        .map_or(0, |index| index + 1);
    let inline_context = &function.context[base_end..];
    let base = function.context[..base_end].last();
    let named_context = inline_context.is_empty()
        && (base.is_none() || matches!(base, Some(ContextKey::Function { .. })));
    let closure_node = if let Some(iir::cleanup::ContextKey::Closure { .. }) = base {
        let Ok(node) = closures::definition(
            graph,
            schedule,
            function,
            &function.context[..base_end],
            fuel,
        ) else {
            return false;
        };
        if closures::schedules_for(graph, schedule, node, fuel).is_err() {
            return false;
        }
        Some(node)
    } else {
        None
    };
    let key = if let Some(iir::cleanup::ContextKey::Function { callee, .. }) = base {
        callee
    } else {
        &function.instance
    };
    // Specialized scopes may share a machine body only when their complete
    // direct schedules agree; otherwise a per-call code specialization is needed.
    if named_context && schedules_for(schedule, key, fuel).is_err() {
        return false;
    }
    let conditional = has_condition(&program.steps);
    if conditional && !program.guards.is_empty() {
        return false;
    }
    let Ok(ty) = repr(graph, &program.ty, true) else {
        return false;
    };
    let mut tags = Tags::new();
    for (path, tag) in &program.guards {
        let Some(at) = projected_cleanup_type(graph, Some(&program.ty), &ty, path, &tags) else {
            return false;
        };
        if *tag >= variant_count(&at).unwrap_or(0) {
            return false;
        }
        tags.insert(path.clone(), *tag);
    }
    if !admit_steps(
        graph,
        &ty,
        &program.steps,
        &tags,
        fuel,
        0,
        Some(&program.ty),
    ) {
        return false;
    }
    let Some(original) = graph.functions.iter().find(|f| &f.key == key) else {
        return false;
    };
    let mut instance = original.clone();
    if let Some(node) = closure_node {
        let K::Closure {
            body, parameters, ..
        } = &node.kind
        else {
            return false;
        };
        instance.function.body = body.as_ref().clone();
        instance.function.signature = node.ty.callable_signature().clone();
        instance.function.parameters = parameters.clone();
        instance.function.result_binding = None;
        instance.function.defaults.clear();
        instance.function.preconditions.clear();
        instance.function.postconditions.clear();
        instance.function.entry_snapshots.clear();
    }
    let mut observations = Vec::new();
    // Each key enters exactly one child runtime frame. Pattern computations
    // and guards belong to the selected arm, not to its installation frame.
    for key in inline_context {
        let mut selected = None;
        let mut candidates = 0usize;
        if nir::visit::expressions_in_scope_with_observations(
            &instance.function,
            &observations,
            fuel,
            |expr, fuel| {
                match (key, &expr.kind) {
                    (
                        ContextKey::Default {
                            construction,
                            parameter,
                        },
                        K::Call { arguments, .. },
                    ) if expr.source == *construction => {
                        for argument in arguments
                            .iter()
                            .filter(|a| a.is_default && a.parameter == *parameter)
                        {
                            candidates += 1;
                            selected = Some((argument.value.clone(), Vec::new()));
                        }
                    }
                    (
                        ContextKey::EffectHandler {
                            installation,
                            handler,
                            arm,
                        },
                        K::EffectHandle { handlers, .. },
                    ) if expr.source == *installation => {
                        if let Some(arm) = handlers.get(*handler).and_then(|h| h.arms.get(*arm)) {
                            candidates += 1;
                            let mut roots =
                                nir::visit::into_pattern_observations(arm.pattern.clone(), fuel)?;
                            roots.extend(arm.guard.iter().cloned());
                            selected = Some((arm.body.clone(), roots));
                        }
                    }
                    _ => {}
                }
                Ok(())
            },
        )
        .is_err()
            || candidates != 1
        {
            return false;
        }
        let Some((body, roots)) = selected else {
            return false;
        };
        instance.function.body = body;
        instance.function.defaults.clear();
        instance.function.preconditions.clear();
        instance.function.postconditions.clear();
        instance.function.result_binding = None;
        instance.function.entry_snapshots.clear();
        observations = roots;
    }
    if let Some(discard) = function
        .discards
        .iter()
        .find(|d| std::ptr::eq(&d.program, program))
    {
        if conditional {
            return false;
        }
        let mut found = false;
        let result = nir::visit::expressions_in_scope_with_observations(
            &instance.function,
            &observations,
            fuel,
            |expr, _| {
                found |= expr.source == discard.source
                    && matches!(&expr.kind, K::Discard(value) if value.source == discard.value_source);
                Ok(())
            },
        );
        return result.is_ok() && found;
    }
    function.sites.iter().any(|site| {
        if !site
            .actions
            .iter()
            .any(|a| std::ptr::eq(&a.program, program) && match &a.target {
                Target::Local {..} => true,
                Target::Prepared {construction, slot, value_source} if !conditional => {
                    let result_slot = *slot == 0 && *value_source == *construction
                        && instance.function.result_binding.is_none()
                        && !instance.function.postconditions.is_empty();
                    let mut found = result_slot && *construction == instance.function.body.source;
                    let result = nir::visit::expressions_in_scope_with_observations(&instance.function, &observations, fuel, |expr, _| {
                        if expr.source == *construction {
                            found |= result_slot && (matches!(expr.kind, K::Return(_)) || iir::effects::suspension_boundary(expr));
                            if let K::Tuple(fields) | K::AnonymousRecord {fields} | K::Record {fields, ..} | K::Variant {fields, ..} = &expr.kind {
                                found |= fields.get(*slot).is_some_and(|field| field.source == *value_source);
                            }
                            if let K::OptionalPresent(payload) = &expr.kind {
                                found |= *slot == 0 && payload.source == *value_source;
                            }
                            if let K::Closure {captures, ..} = &expr.kind {
                                found |= captures.get(*slot).is_some_and(|capture| capture.value.as_ref().map_or(expr.source, |value| value.source) == *value_source);
                            }
                            if let K::EffectRequest {arguments, ..} = &expr.kind {
                                found |= arguments.get(*slot).is_some_and(|argument| argument.source == *value_source);
                            }
                            if let K::Call {callee, arguments, ..} = &expr.kind {
                                found |= arguments.iter().any(|argument| argument.parameter == *slot && argument.value.source == *value_source);
                                found |= site.boundary == Boundary::InterfacePostconditionFailure
                                    && site.source == *construction
                                    && matches!(callee.kind,K::TraitMethod {..})
                                    && *slot == arguments.len() && *value_source == expr.source;
                            }
                        }
                        Ok(())
                    });
                    result.is_ok() && found
                }
                Target::Temporary {source, ..} => {
                    let mut retained = false;
                    let result = nir::visit::expressions_in_scope_with_observations(&instance.function, &observations, fuel, |expr, _| {
                        retained |= match &expr.kind {
                            K::Match {arms, ..} => arms.iter().any(|arm| arm.body.source == *source),
                            K::LetPattern {value, ..} => expr.source == *source || value.source == *source,
                            K::Call {callee,..} => callee.source == *source,
                            K::EnumName(value) => value.source == *source,
                            _ => false,
                        };
                        Ok(())
                    });
                    result.is_ok() && retained
                }
                _ => false,
            })
        {
            return false;
        }
        let supported = match site.boundary {
            Boundary::InterfacePreconditionFailure | Boundary::InterfacePostconditionFailure => {
                let mut found=false;
                let result=nir::visit::expressions_in_scope_with_observations(&instance.function, &observations,fuel,|expr,_| {
                    found |= expr.source==site.source && matches!(&expr.kind,K::Call {callee,..}
                        if matches!(callee.kind,K::TraitMethod {..}));
                    Ok(())
                });
                result.is_ok() && found
            }
            Boundary::EffectEscape | Boundary::EffectReturn
            | Boundary::EffectBreak(_) | Boundary::EffectContinue(_) => {
                let mut found = false;
                let result = nir::visit::expressions_in_scope_with_observations(
                    &instance.function, &observations, fuel, |expr, _| {
                        found |= expr.source == site.source && iir::effects::suspension_boundary(expr);
                        Ok(())
                    });
                // Loop target identity and lexical reachability were checked
                // when the published cleanup schedule was constructed.
                result.is_ok() && found
            }
            Boundary::HandlerEnd => matches!(inline_context.last(), Some(ContextKey::EffectHandler { .. }))
                && site.source == instance.function.body.source,
            Boundary::FunctionEnd => (named_context || (closure_node.is_some() && inline_context.is_empty())) && site.source == instance.function.body.source,
            Boundary::DefaultFailure(parameter) => {
                let mut found = false;
                let result = nir::visit::expressions_in_scope_with_observations(&instance.function, &observations, fuel, |expr, _| {
                    found |= expr.source == site.source && matches!(&expr.kind, K::Call {arguments, ..}
                        if arguments.iter().any(|a| a.is_default && a.parameter == parameter));
                    Ok(())
                });
                result.is_ok() && found
            }
            Boundary::PostconditionFailure(origin) => {
                let mut found = origin == instance.function.body.source;
                let result = nir::visit::expressions_in_scope_with_observations(&instance.function, &observations, fuel, |expr, _| {
                    found |= expr.source == origin && (matches!(expr.kind, K::Return(_)) || iir::effects::suspension_boundary(expr));
                    Ok(())
                });
                result.is_ok() && found && instance.function.postconditions.iter()
                    .any(|condition| condition.source == site.source)
            }
            Boundary::PredicateFailure => {
                let mut found = instance.function.preconditions.iter()
                    .any(|condition| condition.source == site.source);
                let result = nir::visit::expressions_in_scope_with_observations(&instance.function, &observations, fuel, |expr, _| {
                    found |= expr.source == site.source && matches!(expr.kind, K::Call { .. } | K::EffectRequest { .. } | K::EffectHandle { .. });
                    found |= matches!(&expr.kind, K::RequirePredicate(condition) | K::EnsurePredicate(condition) | K::CheckPredicate(condition) if condition.source == site.source);
                    Ok(())
                });
                result.is_ok() && found
            }
            Boundary::TemporaryEnd => {
                let mut found = false;
                let result = nir::visit::expressions_in_scope_with_observations(&instance.function, &observations, fuel, |expr, _| {
                    found |= expr.source == site.source
                        && !matches!(expr.kind, K::CallPreparation {..})
                        && !expr.shares_discard_temporary_boundary();
                    Ok(())
                });
                result.is_ok() && found
            }
            Boundary::ScopeEnd
            | Boundary::Return
            | Boundary::Break
            | Boundary::Continue
            | Boundary::Replacement
            | Boundary::EffectResume => {
                let mut found = false;
                let result = nir::visit::expressions_in_scope_with_observations(&instance.function, &observations, fuel, |expr, _| {
                    found |= site.boundary == Boundary::ScopeEnd && matches!(&expr.kind, K::Match {arms, ..} if arms.iter().any(|arm| arm.body.source == site.source));
                    found |= expr.source == site.source
                        && matches!(
                            (&site.boundary, &expr.kind),
                            (Boundary::ScopeEnd, K::Block { .. })
                                | (Boundary::Return, K::Return(_))
                                | (Boundary::Break, K::Break(_))
                                | (Boundary::Continue, K::Continue(_))
                                | (Boundary::Replacement, K::Assign { .. })
                                | (Boundary::EffectResume, K::EffectResume(_))
                        );
                    Ok(())
                });
                result.is_ok() && found
            }
            _ => false,
        };
        supported
    })
}

impl Emitter<'_> {
    /// Find the innermost persistent capture token along a product path.
    pub(super) fn embedded_live_flag(
        &mut self,
        slot: &Slot,
        path: &[usize],
    ) -> Result<Option<Slot>, String> {
        if path.is_empty() {
            return Ok(None);
        }
        *self.fuel = self
            .fuel
            .checked_sub(path.len().saturating_add(1))
            .ok_or("host capture state lookup budget exhausted")?;
        let mut ty = &slot.ty;
        let mut indices = vec![];
        let mut owner = None;
        for (depth, field) in path.iter().copied().enumerate() {
            match ty {
                Ty::Closure(fields) => {
                    owner = Some((fields, depth, indices.len()));
                    indices.push(1);
                    indices.push(field);
                    ty = fields.get(field).ok_or("host capture field missing")?;
                }
                Ty::Record(fields) => {
                    indices.push(field);
                    ty = fields
                        .get(field)
                        .ok_or("host capture product field missing")?;
                }
                _ => return Ok(None),
            }
        }
        let Some((fields, depth, prefix)) = owner else {
            return Ok(None);
        };
        let index = closures::state_index(fields, &path[depth..], self.fuel)?;
        indices.truncate(prefix);
        indices.extend([2, index]);
        let projection = indices
            .iter()
            .map(|i| format!(", i32 {i}"))
            .collect::<String>();
        let pointer = self.instruction(
            Ty::Pointer,
            format!(
                "getelementptr {}, ptr {}, i32 0{projection}",
                slot.ty.llvm(),
                slot.pointer
            ),
        );
        Ok(Some(Slot {
            ty: Ty::Bool,
            pointer: pointer.text,
        }))
    }
    pub(super) fn set_live(
        &mut self,
        binding: LocalId,
        path: &[usize],
        value: bool,
    ) -> Result<(), String> {
        let needed = self.cleanup.is_some_and(|schedule| {
            schedule.sites.iter().any(|site| {
                site.actions.iter().any(|action| {
                    matches!(&action.target, Target::Local {binding: id, ..} if *id == binding)
                        && has_condition(&action.program.steps)
                })
            })
        });
        if !needed && !self.live.contains_key(&(binding, vec![])) {
            return Ok(());
        }
        if !self.live.contains_key(&(binding, vec![])) {
            let ty = self
                .locals
                .get(&binding)
                .ok_or("host live owner storage missing")?
                .ty
                .clone();
            let mut paths = vec![];
            product_paths(&ty, &mut vec![], &mut paths);
            for path in paths {
                let owner = self.locals[&binding].clone();
                let flag = if let Some(flag) = self.embedded_live_flag(&owner, &path)? {
                    flag
                } else {
                    let flag = self.slot(Ty::Bool);
                    self.prologue
                        .push_str(&format!("  store i1 false, ptr {}\n", flag.pointer));
                    flag
                };
                self.live.insert((binding, path), flag);
            }
        }
        if !self.live.contains_key(&(binding, path.to_vec())) {
            return Err("host construction state path is not a product field".into());
        }
        let affected = self
            .live
            .iter()
            .filter(|((id, fields), _)| *id == binding && fields.starts_with(path))
            .map(|((_, fields), flag)| (fields.clone(), flag.clone()))
            .collect::<Vec<_>>();
        for (fields, flag) in affected {
            // A transferred closure value carries its own capture states. Bind
            // or replacement establishes the outer owner, not fresh captures.
            if value {
                let owner = self.locals[&binding].clone();
                if fields.len() > path.len() && self.embedded_live_flag(&owner, &fields)?.is_some()
                {
                    continue;
                }
            }
            self.store(
                &flag,
                &Value {
                    ty: Ty::Bool,
                    text: value.to_string(),
                },
            )?;
        }
        if value {
            // A moved field does not destroy the parent's own token. A moved
            // parent is reconstructed once all immediate fields are live.
            for length in (0..path.len()).rev() {
                let parent = &path[..length];
                let children = self
                    .live
                    .iter()
                    .filter(|((id, fields), _)| {
                        *id == binding && fields.len() == length + 1 && fields.starts_with(parent)
                    })
                    .map(|(_, flag)| flag.clone())
                    .collect::<Vec<_>>();
                let mut complete = Value {
                    ty: Ty::Bool,
                    text: "true".into(),
                };
                for child in children {
                    let child = self.load(&child);
                    complete = self.instruction(
                        Ty::Bool,
                        format!("and i1 {}, {}", complete.text, child.text),
                    );
                }
                let flag = self
                    .live
                    .get(&(binding, parent.to_vec()))
                    .cloned()
                    .ok_or("host parent live flag missing")?;
                let previous = self.load(&flag);
                let constructed = self.instruction(
                    Ty::Bool,
                    format!("or i1 {}, {}", previous.text, complete.text),
                );
                self.store(&flag, &constructed)?;
            }
        }
        Ok(())
    }
    pub(super) fn cleanup_boundary(
        &mut self,
        source: nir::program::SourceRange,
        boundary: Boundary,
    ) -> Result<(), String> {
        let Some(schedule) = self.cleanup else {
            return Ok(());
        };
        for site in &schedule.sites {
            if site.source != source || site.boundary != boundary {
                continue;
            }
            for (position, action) in site.actions.iter().enumerate() {
                let mut group = None;
                if boundary != Boundary::Replacement {
                    for order in &schedule.pattern_orders {
                        let Some(members) = order.alternatives.first() else {
                            continue;
                        };
                        let positions = site.actions.iter().enumerate().filter_map(|(i, a)|
                            matches!(&a.target, Target::Local {binding, fields} if fields.is_empty() && members.contains(binding)).then_some(i)
                        ).collect::<Vec<_>>();
                        if positions.len() < 2 || !positions.contains(&position) {
                            continue;
                        }
                        if group.is_some() {
                            return Err(
                                "host overlapping pattern cleanup order groups are not implemented"
                                    .into(),
                            );
                        }
                        group = Some((order, positions));
                    }
                }
                if let Some((order, positions)) = group {
                    let slot = self
                        .pattern_orders
                        .get(&(order.source.start, order.source.end))
                        .cloned()
                        .ok_or("host selected pattern cleanup order missing")?;
                    let selected = self.load(&slot);
                    let done = self.fresh();
                    let offset = positions.iter().position(|p| *p == position).unwrap();
                    for (index, alternative) in order.alternatives.iter().enumerate() {
                        let mut ranked = positions
                            .iter()
                            .map(|i| {
                                let Target::Local { binding, .. } = &site.actions[*i].target else {
                                    unreachable!()
                                };
                                alternative
                                    .iter()
                                    .position(|id| id == binding)
                                    .map(|rank| (std::cmp::Reverse(rank), *i))
                                    .ok_or("host cleanup order has inconsistent members")
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        ranked.sort_by_key(|entry| entry.0);
                        let condition = self.instruction(
                            Ty::Bool,
                            format!("icmp eq i32 {}, {index}", selected.text),
                        );
                        let yes = self.fresh();
                        let next = self.fresh();
                        self.line(format!(
                            "br i1 {}, label %{yes}, label %{next}",
                            condition.text
                        ));
                        self.label(&yes);
                        self.cleanup_action(&site.actions[ranked[offset].1])?;
                        self.branch(&done);
                        self.label(&next);
                    }
                    self.line("call void @llvm.trap()");
                    self.line("unreachable");
                    self.label(&done);
                } else {
                    self.cleanup_action(action)?;
                }
            }
        }
        Ok(())
    }
    fn cleanup_action(&mut self, action: &iir::cleanup::Action) -> Result<(), String> {
        *self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or("host cleanup lowering budget exhausted")?;
        if !has_invoke(&action.program.steps) {
            return Ok(());
        }
        if let Target::Prepared {
            construction,
            slot,
            value_source,
        } = &action.target
        {
            let prepared = self
                .prepared
                .get(&(construction.start, construction.end, *slot))
                .cloned()
                .ok_or("host prepared construction slot missing")?;
            if prepared.source != *value_source {
                return Err("host prepared value source mismatch".into());
            }
            let active = self.load(&prepared.live);
            self.guard(&active)?;
            self.cleanup_direct_program(&prepared.storage, &action.program, None)?;
            self.store(
                &prepared.live,
                &Value {
                    ty: Ty::Bool,
                    text: "false".into(),
                },
            )?;
            return Ok(());
        }
        if let Target::Temporary { source, optional } = &action.target {
            let key = (source.start, source.end);
            let Some(slot) = self.temporaries.get(&key).cloned() else {
                if *optional {
                    return Ok(());
                }
                return Err("host residual temporary storage missing".into());
            };
            let flag = self
                .temporary_live
                .get(&key)
                .cloned()
                .ok_or("host temporary live flag missing")?;
            let active = self.load(&flag);
            let done = if *optional {
                let yes = self.fresh();
                let done = self.fresh();
                self.line(format!(
                    "br i1 {}, label %{yes}, label %{done}",
                    active.text
                ));
                self.label(&yes);
                Some(done)
            } else {
                self.guard(&active)?;
                None
            };
            self.cleanup_direct_program(&slot, &action.program, Some(&Owner::Temporary(key)))?;
            self.store(
                &flag,
                &Value {
                    ty: Ty::Bool,
                    text: "false".into(),
                },
            )?;
            if let Some(done) = done {
                self.branch(&done);
                self.label(&done);
            }
            return Ok(());
        }
        let Target::Local { binding, fields } = &action.target else {
            return Err("host direct cleanup target missing".into());
        };
        let slot = self
            .locals
            .get(binding)
            .cloned()
            .ok_or("host cleanup local is not live")?;
        let slot = self.cleanup_field_slot(slot, fields)?;
        let owner = Owner::Local {
            binding: *binding,
            path: fields.clone(),
        };
        self.cleanup_direct_program(&slot, &action.program, Some(&owner))?;
        self.set_live(*binding, fields, false)?;
        Ok(())
    }
}

impl Emitter<'_> {
    pub(super) fn cleanup_discard(
        &mut self,
        source: nir::program::SourceRange,
        value_source: nir::program::SourceRange,
        value: &Value,
    ) -> Result<(), String> {
        let Some(schedule) = self.cleanup else {
            return Ok(());
        };
        for discard in &schedule.discards {
            if discard.source != source || discard.value_source != value_source {
                continue;
            }
            if !has_invoke(&discard.program.steps) {
                continue;
            }
            let slot = self.slot(value.ty.clone());
            self.store(&slot, value)?;
            self.cleanup_direct_program(&slot, &discard.program, None)?;
        }
        Ok(())
    }
    fn cleanup_field_slot(&mut self, mut slot: Slot, path: &[usize]) -> Result<Slot, String> {
        for index in path {
            let (ty, indices) = field_layout(&slot.ty, *index, None)
                .ok_or("host cleanup field requires a product layout")?;
            let path = indices
                .iter()
                .map(|i| format!(", i32 {i}"))
                .collect::<String>();
            let pointer = self.instruction(
                Ty::Pointer,
                format!(
                    "getelementptr {}, ptr {}, i32 0{path}",
                    slot.ty.llvm(),
                    slot.pointer
                ),
            );
            slot = Slot {
                ty,
                pointer: pointer.text,
            };
        }
        Ok(slot)
    }
    fn cleanup_direct_program(
        &mut self,
        slot: &Slot,
        program: &Program,
        live: Option<&Owner>,
    ) -> Result<(), String> {
        let mut tags = Tags::new();
        for (path, tag) in &program.guards {
            let at = self.cleanup_tagged_slot(slot.clone(), path, &tags, Some(&program.ty))?;
            let condition = self.cleanup_tag_condition(&at, *tag)?;
            self.guard(&condition)?;
            tags.insert(path.clone(), *tag);
        }
        self.cleanup_steps(slot, &program.steps, &tags, live, Some(&program.ty))
    }
    fn cleanup_tagged_slot(
        &mut self,
        mut slot: Slot,
        path: &[usize],
        tags: &Tags,
        logical: Option<&Term>,
    ) -> Result<Slot, String> {
        let mut logical = logical.cloned();
        for (depth, field) in path.iter().enumerate() {
            let (next, ty, indices) = cleanup_field_layout(
                self.graph,
                logical.as_ref(),
                &slot.ty,
                *field,
                tags.get(&path[..depth]).copied(),
            )
            .ok_or("host cleanup payload path lacks a supported active variant")?;
            logical = next;
            if indices.is_empty() {
                slot.ty = ty;
                continue;
            }
            let indices = indices
                .iter()
                .map(|i| format!(", i32 {i}"))
                .collect::<String>();
            let pointer = self.instruction(
                Ty::Pointer,
                format!(
                    "getelementptr {}, ptr {}, i32 0{indices}",
                    slot.ty.llvm(),
                    slot.pointer
                ),
            );
            slot = Slot {
                ty,
                pointer: pointer.text,
            };
        }
        Ok(slot)
    }
    fn cleanup_tag_condition(&mut self, slot: &Slot, tag: usize) -> Result<Value, String> {
        let value = self.load(slot);
        if matches!(value.ty, Ty::Enum(_)) {
            return self.variant_condition(&value, tag);
        }
        if matches!(value.ty, Ty::Optional(_)) {
            if tag > 1 {
                return Err("host optional cleanup tag out of range".into());
            }
            let actual = self.optional_tag(&value)?;
            return Ok(self.instruction(Ty::Bool, format!("icmp eq i1 {}, {tag}", actual.text)));
        }
        if matches!(value.ty, Ty::ErrorResult(_)) {
            let actual = self.instruction(
                Ty::Int(32, false),
                format!("extractvalue {} {}, 0", value.ty.llvm(), value.text),
            );
            return Ok(self.instruction(Ty::Bool, format!("icmp eq i32 {}, {tag}", actual.text)));
        }
        Err("host cleanup tag requires enum, optional or error result".into())
    }
    fn cleanup_steps(
        &mut self,
        slot: &Slot,
        steps: &[Step],
        tags: &Tags,
        live: Option<&Owner>,
        logical: Option<&Term>,
    ) -> Result<(), String> {
        for step in steps {
            *self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or("host cleanup lowering budget exhausted")?;
            if let Step::Release { path, .. } = step {
                if let Some(owner) = live {
                    match owner {
                        Owner::Local {
                            binding,
                            path: base,
                        } => {
                            let fields = base.iter().chain(path).copied().collect::<Vec<_>>();
                            // Sum payloads are released inside the active branch,
                            // but only the sum's whole-value token is tracked.
                            let owner_ty = &self
                                .locals
                                .get(binding)
                                .ok_or("host cleanup owner storage missing")?
                                .ty;
                            if product_projection(owner_ty, &fields).is_some() {
                                self.set_live(*binding, &fields, false)?;
                            }
                        }
                        Owner::Temporary(key) => {
                            let flags = self
                                .temporary_fields
                                .iter()
                                .filter(|((source, fields), _)| {
                                    source == key && fields.starts_with(path)
                                })
                                .map(|(_, flag)| flag.clone())
                                .collect::<Vec<_>>();
                            for flag in flags {
                                self.store(
                                    &flag,
                                    &Value {
                                        ty: Ty::Bool,
                                        text: "false".into(),
                                    },
                                )?;
                            }
                        }
                    }
                }
                continue;
            }
            if let Step::IfConstructed { path, steps } = step {
                let embedded = self.embedded_live_flag(slot, path)?;
                let owner = live.ok_or("host conditional cleanup owner missing")?;
                let flag = embedded
                    .or_else(|| match owner {
                        Owner::Local {
                            binding,
                            path: base,
                        } => {
                            let fields = base.iter().chain(path).copied().collect::<Vec<_>>();
                            self.live.get(&(*binding, fields)).cloned()
                        }
                        Owner::Temporary(key) => {
                            self.temporary_fields.get(&(*key, path.clone())).cloned()
                        }
                    })
                    .ok_or("host conditional cleanup field state missing")?;
                let condition = self.load(&flag);
                let yes = self.fresh();
                let done = self.fresh();
                self.line(format!(
                    "br i1 {}, label %{yes}, label %{done}",
                    condition.text
                ));
                self.label(&yes);
                self.cleanup_steps(slot, steps, tags, live, logical)?;
                self.branch(&done);
                self.label(&done);
                continue;
            }
            if let Step::Select { cases } = step {
                // Finish selection before invoking user code. Inner guards are
                // reached only when their enclosing payload is active.
                let selected = self.slot(Ty::Int(32, false));
                self.store(
                    &selected,
                    &Value {
                        ty: Ty::Int(32, false),
                        text: "-1".into(),
                    },
                )?;
                for (index, case) in cases.iter().enumerate() {
                    let next = self.fresh();
                    let mut candidate_tags = tags.clone();
                    for (path, tag) in &case.guards {
                        let at =
                            self.cleanup_tagged_slot(slot.clone(), path, &candidate_tags, logical)?;
                        let condition = self.cleanup_tag_condition(&at, *tag)?;
                        let yes = self.fresh();
                        self.line(format!(
                            "br i1 {}, label %{yes}, label %{next}",
                            condition.text
                        ));
                        self.label(&yes);
                        candidate_tags.insert(path.clone(), *tag);
                    }
                    let previous = self.load(&selected);
                    let unique =
                        self.instruction(Ty::Bool, format!("icmp eq i32 {}, -1", previous.text));
                    self.guard(&unique)?;
                    self.store(
                        &selected,
                        &Value {
                            ty: Ty::Int(32, false),
                            text: index.to_string(),
                        },
                    )?;
                    self.branch(&next);
                    self.label(&next);
                }
                let index = self.load(&selected);
                let exists = self.instruction(Ty::Bool, format!("icmp ne i32 {}, -1", index.text));
                self.guard(&exists)?;
                let done = self.fresh();
                for (candidate, case) in cases.iter().enumerate() {
                    let matches = self
                        .instruction(Ty::Bool, format!("icmp eq i32 {}, {candidate}", index.text));
                    let yes = self.fresh();
                    let next = self.fresh();
                    self.line(format!(
                        "br i1 {}, label %{yes}, label %{next}",
                        matches.text
                    ));
                    self.label(&yes);
                    let mut tags = tags.clone();
                    tags.extend(case.guards.iter().cloned());
                    self.cleanup_steps(slot, &case.steps, &tags, live, logical)?;
                    self.branch(&done);
                    self.label(&next);
                }
                self.line("call void @llvm.trap()");
                self.line("unreachable");
                self.label(&done);
                continue;
            }
            if let Step::Switch { path, branches } = step {
                let at = self.cleanup_tagged_slot(slot.clone(), path, tags, logical)?;
                let done = self.fresh();
                for (tag, branch) in branches.iter().enumerate() {
                    let condition = self.cleanup_tag_condition(&at, tag)?;
                    let yes = self.fresh();
                    let no = self.fresh();
                    self.line(format!(
                        "br i1 {}, label %{yes}, label %{no}",
                        condition.text
                    ));
                    self.label(&yes);
                    let mut tags = tags.clone();
                    tags.insert(path.clone(), tag);
                    self.cleanup_steps(slot, branch, &tags, live, logical)?;
                    self.branch(&done);
                    self.label(&no);
                }
                self.line("call void @llvm.trap()");
                self.line("unreachable");
                self.label(&done);
                continue;
            }

            let Step::Invoke {
                method,
                subject,
                path,
            } = step
            else {
                continue;
            };
            let (actual, _) = cleanup_projection(self.graph, logical, &slot.ty, path, tags)
                .ok_or("host cleanup subject projection is invalid")?;
            if actual.as_ref().is_some_and(|actual| actual != subject) {
                return Err("host cleanup logical subject mismatch".into());
            }
            let slot = self.cleanup_tagged_slot(slot.clone(), path, tags, logical)?;
            if slot.ty != repr(self.graph, subject, true)? {
                return Err("host cleanup subject representation mismatch".into());
            }
            let (index, function) = self
                .graph
                .functions
                .iter()
                .enumerate()
                .find(|(_, f)| &f.key == method)
                .ok_or("host destructor instance missing")?;
            let Term::Function { parameters, result } = &function.function.signature else {
                return Err("host destructor signature missing".into());
            };
            if parameters.len() != 1
                || parameters[0].ty != Term::Pointer(Box::new(subject.clone()))
                || parameters[0].sort != Sort::Runtime
                || repr(self.graph, &parameters[0].ty, true)? != Ty::Pointer
                || repr(self.graph, result, true)? != Ty::Unit
            {
                return Err(
                    "host destructor ABI requires a subject pointer and void result".into(),
                );
            }
            // Cleanup is a separate invocation while an outer return may be
            // pending. Its own effectful work must not consume that request.
            let pending_return = if self.failure_result.is_some() {
                let saved =
                    self.instruction(Ty::Pointer, "load ptr, ptr @flurry_effect_control".into());
                self.line("store ptr null, ptr @flurry_effect_control");
                Some(saved)
            } else {
                None
            };
            if let Some(schedule) = self.schedules {
                // Production body emission independently requires acquired
                // evidence. Here reject incompatible plans for this canonical
                // destructor call (standalone emitter fixtures may supply stubs).
                select_schedule(schedule, method, self.fuel)?;
            }
            self.line(format!("call void @flurry_{index}(ptr {})", slot.pointer));
            if let Some(saved) = pending_return {
                let pending =
                    self.instruction(Ty::Pointer, "load ptr, ptr @flurry_effect_control".into());
                let settled =
                    self.instruction(Ty::Bool, format!("icmp eq ptr {}, null", pending.text));
                self.guard(&settled)?;
                self.line(format!(
                    "store ptr {}, ptr @flurry_effect_control",
                    saved.text
                ));
            }
            if self.failure_result.is_some() {
                // A failure inside Drop terminates cleanup rather than running
                // the rest of the frame with a half-destroyed subject.
                self.line("call void @flurry_check_failure()");
            }
        }
        Ok(())
    }
}

type Tags = std::collections::BTreeMap<Vec<usize>, usize>;

pub(super) fn has_invoke(steps: &[Step]) -> bool {
    let mut pending: Vec<_> = steps.iter().collect();
    while let Some(step) = pending.pop() {
        match step {
            Step::Invoke { .. } => return true,
            Step::Switch { branches, .. } => pending.extend(branches.iter().flatten()),
            Step::Select { cases } => pending.extend(cases.iter().flat_map(|c| &c.steps)),
            Step::IfConstructed { steps, .. } => pending.extend(steps),
            Step::Release { .. } => {}
        }
    }
    false
}
fn variant_count(ty: &Ty) -> Option<usize> {
    match ty {
        Ty::Optional(_) => Some(2),
        Ty::Enum(layout) => Some(layout.fields.len()),
        Ty::ErrorResult(variants) => Some(variants.len()),
        _ => None,
    }
}
fn field_layout(ty: &Ty, field: usize, tag: Option<usize>) -> Option<(Ty, Vec<usize>)> {
    match ty {
        Ty::Record(fields) => Some((fields.get(field)?.clone(), vec![field])),
        Ty::Closure(fields) => Some((fields.get(field)?.clone(), vec![1, field])),
        Ty::Enum(layout) if layout.aggregate => {
            let tag = tag?;
            Some((
                layout.fields.get(tag)?.get(field)?.1.clone(),
                vec![tag + 1, field],
            ))
        }
        Ty::Optional(payload) if field == 0 && tag == Some(1) => {
            Some(((**payload).clone(), vec![1]))
        }
        Ty::ErrorResult(variants) if field == 0 => {
            let tag = tag?;
            Some((variants.get(tag)?.1.clone(), vec![tag + 1]))
        }
        _ => None,
    }
}
// Cleanup paths describe language ownership, including newtype representation
// edges. Such an edge consumes a logical field but contributes no LLVM GEP.
fn cleanup_field_layout(
    graph: &iir::Package,
    logical: Option<&Term>,
    physical: &Ty,
    field: usize,
    tag: Option<usize>,
) -> Option<(Option<Term>, Ty, Vec<usize>)> {
    let next = match logical {
        None => None,
        Some(Term::Nominal { .. }) => {
            let instance = graph
                .nominals
                .iter()
                .find(|instance| Some(&instance.ty) == logical)?;
            match &instance.definition {
                iir::NominalDefinition::Newtype(definition) => {
                    if field != 0
                        || repr(graph, &definition.representation, true).ok().as_ref()
                            != Some(physical)
                    {
                        return None;
                    }
                    return Some((
                        Some(definition.representation.clone()),
                        physical.clone(),
                        vec![],
                    ));
                }
                iir::NominalDefinition::Struct(definition) => {
                    Some(definition.fields.get(field)?.1.clone())
                }
                iir::NominalDefinition::Enum(definition) => {
                    Some(definition.variants.get(tag?)?.fields.get(field)?.clone())
                }
            }
        }
        Some(Term::Record(fields)) => Some(fields.get(field)?.1.clone()),
        Some(Term::Tuple(fields)) => Some(fields.get(field)?.clone()),
        Some(ty @ Term::NormalForm { .. }) => Some(
            graph
                .normal_forms
                .iter()
                .find_map(|head| head.tuple_elements(ty))?
                .get(field)?
                .clone(),
        ),
        Some(Term::Closure { environment, .. }) => Some(environment.get(field)?.clone()),
        Some(Term::Optional(payload)) if field == 0 && tag == Some(1) => Some((**payload).clone()),
        Some(Term::ErrorResult { errors, result }) if field == 0 => Some(if tag? == 0 {
            (**result).clone()
        } else {
            errors.get(tag? - 1)?.clone()
        }),
        _ => return None,
    };
    let (ty, indices) = field_layout(physical, field, tag)?;
    if let Some(logical) = &next {
        if repr(graph, logical, true).ok().as_ref() != Some(&ty) {
            return None;
        }
    }
    Some((next, ty, indices))
}
fn cleanup_projection(
    graph: &iir::Package,
    logical: Option<&Term>,
    physical: &Ty,
    path: &[usize],
    tags: &Tags,
) -> Option<(Option<Term>, Ty)> {
    let mut logical = logical.cloned();
    let mut physical = physical.clone();
    for (depth, field) in path.iter().enumerate() {
        let (next, ty, _) = cleanup_field_layout(
            graph,
            logical.as_ref(),
            &physical,
            *field,
            tags.get(&path[..depth]).copied(),
        )?;
        logical = next;
        physical = ty;
    }
    Some((logical, physical))
}
fn projected_cleanup_type(
    graph: &iir::Package,
    logical: Option<&Term>,
    physical: &Ty,
    path: &[usize],
    tags: &Tags,
) -> Option<Ty> {
    cleanup_projection(graph, logical, physical, path, tags).map(|(_, physical)| physical)
}
fn admit_steps(
    graph: &iir::Package,
    ty: &Ty,
    steps: &[Step],
    tags: &Tags,
    fuel: &mut usize,
    depth: usize,
    logical: Option<&Term>,
) -> bool {
    if depth > 128 {
        return false;
    }
    for step in steps {
        let Some(next) = fuel.checked_sub(1) else {
            return false;
        };
        *fuel = next;
        match step {
            // Release is logical ownership bookkeeping, not a memory access;
            // e.g. a newtype field can be erased from the target layout.
            Step::Release { .. } => {}
            Step::Invoke { path, subject, .. } => {
                let Some((actual, at)) = cleanup_projection(graph, logical, ty, path, tags) else {
                    return false;
                };
                if actual.as_ref().is_some_and(|actual| actual != subject)
                    || repr(graph, subject, true).ok().as_ref() != Some(&at)
                {
                    return false;
                }
            }
            Step::IfConstructed { path, steps } => {
                if product_projection(ty, path).is_none()
                    || !admit_steps(graph, ty, steps, tags, fuel, depth + 1, logical)
                {
                    return false;
                }
            }
            Step::Select { cases } => {
                if cases.len() >= u32::MAX as usize {
                    return false;
                }
                for case in cases {
                    let Some(next) = fuel.checked_sub(1) else {
                        return false;
                    };
                    *fuel = next;
                    let mut tags = tags.clone();
                    for (path, tag) in &case.guards {
                        let Some(next) = fuel.checked_sub(1) else {
                            return false;
                        };
                        *fuel = next;
                        let Some(at) = projected_cleanup_type(graph, logical, ty, path, &tags)
                        else {
                            return false;
                        };
                        if *tag >= variant_count(&at).unwrap_or(0) {
                            return false;
                        }
                        tags.insert(path.clone(), *tag);
                    }
                    if !admit_steps(graph, ty, &case.steps, &tags, fuel, depth + 1, logical) {
                        return false;
                    }
                }
            }
            Step::Switch { path, branches } => {
                let Some(at) = projected_cleanup_type(graph, logical, ty, path, tags) else {
                    return false;
                };
                if variant_count(&at) != Some(branches.len()) {
                    return false;
                }
                for (tag, branch) in branches.iter().enumerate() {
                    let mut tags = tags.clone();
                    tags.insert(path.clone(), tag);
                    if !admit_steps(graph, ty, branch, &tags, fuel, depth + 1, logical) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_projection_erases_only_materialized_newtype_representation_edges() {
        let mut graph = iir::Package::default();
        let mut logical = Term::Primitive(P::Bool);
        for local in [1, 2] {
            let declaration = nir::DeclarationId { package: 17, local };
            let ty = Term::Nominal {
                declaration,
                arguments: vec![],
            };
            graph.nominals.push(iir::NominalInstance {
                ty: ty.clone(),
                definition: iir::NominalDefinition::Newtype(nir::newtypes::Newtype {
                    declaration,
                    name: format!("Wrapper{local}"),
                    type_parameters: vec![],
                    parameter_types: vec![],
                    capture_count: 0,
                    requirements: vec![],
                    capabilities: vec![],
                    representation: logical,
                }),
            });
            logical = ty;
        }
        let tags = Tags::new();
        for path in [vec![], vec![0], vec![0, 0]] {
            assert_eq!(
                projected_cleanup_type(&graph, Some(&logical), &Ty::Bool, &path, &tags),
                Some(Ty::Bool)
            );
        }
        for path in [vec![1], vec![0, 1], vec![0, 0, 0]] {
            assert!(
                projected_cleanup_type(&graph, Some(&logical), &Ty::Bool, &path, &tags).is_none()
            );
        }
        assert!(
            cleanup_field_layout(&graph, Some(&logical), &Ty::Int(8, false), 0, None).is_none()
        );
        let unknown = Term::Nominal {
            declaration: nir::DeclarationId {
                package: 18,
                local: 2,
            },
            arguments: vec![],
        };
        assert!(cleanup_field_layout(&graph, Some(&unknown), &Ty::Bool, 0, None).is_none());
        // The erased edge reuses the address and retains the next logical type.
        let (next, physical, indices) =
            cleanup_field_layout(&graph, Some(&logical), &Ty::Bool, 0, None).unwrap();
        assert_eq!(next, Some(graph.nominals[0].ty.clone()));
        assert_eq!(physical, Ty::Bool);
        assert!(indices.is_empty());
        let method = iir::InstanceKey {
            declaration: nir::DeclarationId {
                package: 17,
                local: 99,
            },
            arguments: vec![],
        };
        // Inner and Outer have identical machine layouts. Only the logical
        // subject at the selected representation edge may be retired there.
        for (subject, accepted) in [
            (graph.nominals[0].ty.clone(), true),
            (logical.clone(), false),
            (Term::Primitive(P::Bool), false),
        ] {
            let steps = vec![Step::Invoke {
                method: method.clone(),
                subject,
                path: vec![0],
            }];
            assert_eq!(
                admit_steps(
                    &graph,
                    &Ty::Bool,
                    &steps,
                    &tags,
                    &mut 1000,
                    0,
                    Some(&logical)
                ),
                accepted
            );
        }
    }

    #[test]
    fn recursive_aliases_validate_ancestry_identity_uniqueness_and_budget() {
        use iir::cleanup::{ContextAlias, ContextKey};
        let key = iir::InstanceKey {
            declaration: nir::DeclarationId {
                package: 0,
                local: 1,
            },
            arguments: vec![],
        };
        let source = nir::program::SourceRange { start: 0, end: 1 };
        let scope = FunctionSchedule {
            instance: key.clone(),
            context: vec![],
            sites: vec![],
            discards: vec![],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
        };
        let alias = ContextAlias {
            instance: key.clone(),
            context: vec![ContextKey::Function {
                call: source,
                callee: key.clone(),
            }],
            target: vec![],
        };
        let mut schedule = Schedule {
            closure_definitions: vec![],
            borrowed_callable_functions: vec![],
            functions: vec![scope.clone()],
            aliases: vec![alias.clone()],
        };
        assert!(validate_alias(&schedule, &alias, &mut 0).is_err());
        assert!(validate_alias(&schedule, &alias, &mut 100).is_ok());
        schedule.aliases.push(alias.clone());
        assert!(
            validate_alias(&schedule, &alias, &mut 100)
                .unwrap_err()
                .contains("competing edges")
        );
        schedule.aliases.pop();
        schedule.functions.push(scope.clone());
        assert!(
            validate_alias(&schedule, &alias, &mut 100)
                .unwrap_err()
                .contains("ambiguous targets")
        );
        schedule.functions[1].context = alias.context.clone();
        assert!(
            validate_alias(&schedule, &alias, &mut 100)
                .unwrap_err()
                .contains("conflict")
        );
        schedule.functions.pop();
        schedule.functions[0].instance.declaration.local = 2;
        assert!(
            validate_alias(&schedule, &alias, &mut 100)
                .unwrap_err()
                .contains("no concrete target")
        );
        schedule.functions[0] = scope;
        let mut wrong = alias.clone();
        let ContextKey::Function { callee, .. } = &mut wrong.context[0] else {
            unreachable!()
        };
        callee.declaration.local = 3;
        schedule.aliases[0] = wrong.clone();
        assert!(
            validate_alias(&schedule, &wrong, &mut 100)
                .unwrap_err()
                .contains("callee identity")
        );
        wrong.target = wrong.context.clone();
        schedule.aliases[0] = wrong.clone();
        assert!(
            validate_alias(&schedule, &wrong, &mut 100)
                .unwrap_err()
                .contains("ancestor scope")
        );
        schedule.aliases[0] = alias.clone();
        assert!(validate_alias(&schedule, &alias, &mut 100).is_ok());
    }

    #[test]
    fn canonical_cleanup_reuse_checks_nested_named_dispatch() {
        use iir::cleanup::ContextKey;
        let key = iir::InstanceKey {
            declaration: nir::DeclarationId {
                package: 0,
                local: 1,
            },
            arguments: vec![],
        };
        let nested_key = iir::InstanceKey {
            declaration: nir::DeclarationId {
                package: 0,
                local: 2,
            },
            arguments: vec![],
        };
        let source = nir::program::SourceRange { start: 1, end: 2 };
        let root = FunctionSchedule {
            instance: key.clone(),
            context: vec![],
            sites: vec![],
            discards: vec![],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
        };
        let mut other = root.clone();
        other.instance.declaration.local = 3;
        other.context.push(ContextKey::Function {
            call: source,
            callee: key.clone(),
        });
        let mut first_child = root.clone();
        first_child.context.push(ContextKey::Function {
            call: source,
            callee: nested_key.clone(),
        });
        let mut second_child = other.clone();
        second_child.context.push(ContextKey::Function {
            call: source,
            callee: nested_key,
        });
        let mut schedule = Schedule {
            closure_definitions: vec![],
            borrowed_callable_functions: vec![],
            aliases: vec![],
            functions: vec![root, other, first_child, second_child],
        };
        assert!(
            select_schedule(&schedule, &key, &mut 10000)
                .unwrap()
                .is_some()
        );
        schedule.functions[3]
            .transfers
            .push(iir::cleanup::Transfer {
                source,
                binding: LocalId { owner: 0, index: 0 },
                fields: vec![],
            });
        assert!(
            select_schedule(&schedule, &key, &mut 10000)
                .unwrap_err()
                .contains("distinct per-call")
        );
        schedule.functions[3].transfers.clear();
        assert!(select_schedule(&schedule, &key, &mut 0).is_err());
        assert!(
            select_schedule(&schedule, &key, &mut 10000)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn named_cleanup_reuse_requires_matching_inline_scopes() {
        use iir::cleanup::ContextKey;
        let key = iir::InstanceKey {
            declaration: nir::DeclarationId {
                package: 0,
                local: 1,
            },
            arguments: vec![],
        };
        let source = nir::program::SourceRange { start: 0, end: 1 };
        let scope = FunctionSchedule {
            instance: key.clone(),
            context: vec![],
            sites: vec![],
            discards: vec![],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
        };
        let mut specialized = scope.clone();
        specialized.instance.declaration.local = 2;
        specialized.context.push(ContextKey::Function {
            call: source,
            callee: key.clone(),
        });
        let mut schedule = Schedule {
            closure_definitions: vec![],
            borrowed_callable_functions: vec![],
            functions: vec![scope, specialized.clone()],
            aliases: vec![],
        };
        assert!(select_schedule(&schedule, &key, &mut 0).is_err());
        assert!(
            select_schedule(&schedule, &key, &mut 100)
                .unwrap()
                .is_some()
        );
        // An ownership difference is significant even without destructor calls.
        schedule.functions[1]
            .transfers
            .push(iir::cleanup::Transfer {
                source,
                binding: LocalId { owner: 0, index: 0 },
                fields: vec![],
            });
        assert!(
            select_schedule(&schedule, &key, &mut 100)
                .unwrap_err()
                .contains("distinct per-call")
        );
        schedule.functions[1] = specialized.clone();
        let mut nested = specialized;
        nested.context.push(ContextKey::Default {
            construction: source,
            parameter: 0,
        });
        schedule.functions.push(nested);
        assert!(
            select_schedule(&schedule, &key, &mut 200)
                .unwrap_err()
                .contains("distinct per-call")
        );
        let mut root_default = schedule.functions[0].clone();
        root_default.context.push(ContextKey::Default {
            construction: source,
            parameter: 0,
        });
        schedule.functions.push(root_default.clone());
        assert!(
            select_schedule(&schedule, &key, &mut 200)
                .unwrap()
                .is_some()
        );
        schedule.functions[3].copies.push(iir::cleanup::Transfer {
            source,
            binding: LocalId { owner: 0, index: 0 },
            fields: vec![],
        });
        assert!(
            select_schedule(&schedule, &key, &mut 200)
                .unwrap_err()
                .contains("distinct per-call")
        );
        schedule.functions[3] = root_default.clone();
        schedule.functions.push(root_default);
        assert!(
            select_schedule(&schedule, &key, &mut 200)
                .unwrap_err()
                .contains("ambiguous")
        );
        schedule.functions.truncate(2);
        schedule.aliases.push(iir::cleanup::ContextAlias {
            instance: schedule.functions[1].instance.clone(),
            context: schedule.functions[1].context.clone(),
            target: vec![],
        });
        assert!(
            select_schedule(&schedule, &key, &mut 100)
                .unwrap_err()
                .contains("recursive cleanup aliases")
        );
        schedule.aliases.clear();
        assert!(
            select_schedule(&schedule, &key, &mut 100)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn handler_cleanup_sharing_preserves_arm_identity_and_nested_ownership() {
        use iir::cleanup::ContextKey;
        let key = iir::InstanceKey {
            declaration: nir::DeclarationId {
                package: 0,
                local: 1,
            },
            arguments: vec![],
        };
        let source = nir::program::SourceRange { start: 10, end: 20 };
        let root = FunctionSchedule {
            instance: key.clone(),
            context: vec![],
            sites: vec![],
            discards: vec![],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
        };
        let mut caller = root.clone();
        caller.instance.declaration.local = 2;
        caller.context.push(ContextKey::Function {
            call: source,
            callee: key.clone(),
        });
        let handler = ContextKey::EffectHandler {
            installation: source,
            handler: 0,
            arm: 1,
        };
        let default = ContextKey::Default {
            construction: source,
            parameter: 0,
        };
        for path in [
            vec![handler.clone()],
            vec![handler.clone(), default.clone()],
            vec![default.clone(), handler.clone()],
            vec![handler.clone(), handler.clone()],
        ] {
            let mut left = root.clone();
            left.context.extend(path.clone());
            let mut right = caller.clone();
            right.context.extend(path);
            let mut schedule = Schedule {
                closure_definitions: vec![],
                borrowed_callable_functions: vec![],
                aliases: vec![],
                functions: vec![root.clone(), caller.clone(), left.clone(), right.clone()],
            };
            assert!(
                select_schedule(&schedule, &key, &mut 1000)
                    .unwrap()
                    .is_some()
            );
            // A selected arm's ownership difference forbids sharing even if the
            // enclosing callable and all destructor lists are identical.
            schedule.functions[3]
                .transfers
                .push(iir::cleanup::Transfer {
                    source,
                    binding: LocalId { owner: 0, index: 0 },
                    fields: vec![0],
                });
            assert!(
                select_schedule(&schedule, &key, &mut 1000)
                    .unwrap_err()
                    .contains("distinct per-call")
            );
            schedule.functions[3] = right.clone();
            // Installation, handler index and arm index are all scope identity.
            let at = right
                .context
                .iter()
                .rposition(|k| matches!(k, ContextKey::EffectHandler { .. }))
                .unwrap();
            for replacement in [
                ContextKey::EffectHandler {
                    installation: nir::program::SourceRange { start: 11, end: 20 },
                    handler: 0,
                    arm: 1,
                },
                ContextKey::EffectHandler {
                    installation: source,
                    handler: 1,
                    arm: 1,
                },
                ContextKey::EffectHandler {
                    installation: source,
                    handler: 0,
                    arm: 2,
                },
            ] {
                schedule.functions[3].context[at] = replacement;
                assert!(
                    select_schedule(&schedule, &key, &mut 1000)
                        .unwrap_err()
                        .contains("distinct per-call")
                );
            }
            schedule.functions[3] = right.clone();
            schedule.functions.push(right);
            assert!(
                select_schedule(&schedule, &key, &mut 1000)
                    .unwrap_err()
                    .contains("ambiguous")
            );
            schedule.functions.pop();
            assert!(select_schedule(&schedule, &key, &mut 0).is_err());
            assert!(select_schedule(&schedule, &key, &mut 1000).is_ok());

            // A function or closure called inside the arm owns a separate
            // machine body; its descendants must not enter this comparison.
            for separate in [
                ContextKey::Function {
                    call: source,
                    callee: key.clone(),
                },
                ContextKey::Closure {
                    call: source,
                    definition: source,
                },
            ] {
                let mut child = left.clone();
                child
                    .context
                    .extend([separate, handler.clone(), default.clone()]);
                child.copies.push(iir::cleanup::Transfer {
                    source,
                    binding: LocalId { owner: 0, index: 5 },
                    fields: vec![],
                });
                schedule.functions.push(child);
                assert!(same_frame(&schedule, &root, &caller, &mut 1000).unwrap());
                schedule.functions.pop();
            }
        }
    }

    #[test]
    fn cleanup_admission_requires_unique_scope_local_default_source() {
        use iir::cleanup::{ContextKey, DiscardCleanup};
        let source = |start| nir::program::SourceRange {
            start,
            end: start + 1,
        };
        let declaration = nir::DeclarationId {
            package: 0,
            local: 1,
        };
        let key = iir::InstanceKey {
            declaration,
            arguments: vec![],
        };
        let unit = Expr {
            source: source(1),
            ty: Term::Primitive(P::Void),
            kind: K::Unit,
        };
        let discard = Expr {
            source: source(2),
            ty: unit.ty.clone(),
            kind: K::Discard(Box::new(unit.clone())),
        };
        let call = |value: Expr| Expr {
            source: source(3),
            ty: unit.ty.clone(),
            kind: K::Call {
                callee: Box::new(unit.clone()),
                arguments: vec![nir::program::CallArgument {
                    passing: Default::default(),
                    parameter: 0,
                    value,
                    is_default: true,
                }],
                parameter_bindings: vec![],
            },
        };
        let function = Function {
            phase: nir::program::FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration,
            name: "scope_test".into(),
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(unit.ty.clone()),
            },
            parameters: vec![],
            defaults: vec![],
            body: call(discard.clone()),
        };
        let mut graph = iir::Package {
            functions: vec![iir::FunctionInstance {
                key: key.clone(),
                function,
            }],
            ..Default::default()
        };
        let scope = FunctionSchedule {
            instance: key,
            context: vec![],
            sites: vec![],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
            discards: vec![DiscardCleanup {
                source: source(2),
                value_source: source(1),
                program: Program {
                    ty: Term::Primitive(P::U8),
                    guards: vec![],
                    steps: vec![],
                },
            }],
        };
        let mut schedule = Schedule {
            closure_definitions: vec![],
            functions: vec![scope],
            aliases: vec![],
            borrowed_callable_functions: vec![],
        };
        let admitted = |graph: &iir::Package, schedule: &Schedule, fuel: &mut usize| {
            let scope = &schedule.functions[0];
            admitted_direct_program(graph, schedule, scope, &scope.discards[0].program, fuel)
        };
        // Parent cannot borrow a discard located in a default's resource frame.
        assert!(!admitted(&graph, &schedule, &mut 1000));
        schedule.functions[0].context.push(ContextKey::Default {
            construction: source(3),
            parameter: 0,
        });
        assert!(admitted(&graph, &schedule, &mut 1000));
        assert!(!admitted(&graph, &schedule, &mut 0));
        assert!(admitted(&graph, &schedule, &mut 1000));
        // Source collisions within a scope are not resolved by traversal order.
        let original = graph.functions[0].function.body.clone();
        graph.functions[0].function.body = Expr {
            source: source(4),
            ty: unit.ty.clone(),
            kind: K::Block {
                statements: vec![original.clone(), original.clone()],
                result: None,
            },
        };
        assert!(!admitted(&graph, &schedule, &mut 1000));
        // A nested default with the same construction source is not ambiguous
        // with its parent; it is selected only by another explicit context key.
        graph.functions[0].function.body = call(original);
        assert!(!admitted(&graph, &schedule, &mut 1000));
        let nested = schedule.functions[0].context[0].clone();
        schedule.functions[0].context.push(nested);
        assert!(admitted(&graph, &schedule, &mut 1000));
        graph.functions[0].function.body = call(discard.clone());
        assert!(!admitted(&graph, &schedule, &mut 1000));

        let install = |body: Expr, guard: Option<Expr>| Expr {
            source: source(5),
            ty: unit.ty.clone(),
            kind: K::EffectHandle {
                computation: Box::new(unit.clone()),
                outer: vec![],
                handlers: vec![nir::program::EffectHandler {
                    operation: nir::effects::Operation {
                        declaration,
                        arguments: vec![],
                    },
                    abi: nir::effects::Abi::Direct,
                    signature: Term::Function {
                        parameters: vec![],
                        result: Box::new(unit.ty.clone()),
                    },
                    arms: vec![nir::program::MatchArm {
                        pattern: nir::program::Pattern::Wild,
                        guard,
                        body,
                    }],
                    fallback: None,
                }],
            },
        };
        let handler_key = ContextKey::EffectHandler {
            installation: source(5),
            handler: 0,
            arm: 0,
        };
        schedule.functions[0].context.clear();
        graph.functions[0].function.body = install(discard.clone(), None);
        assert!(!admitted(&graph, &schedule, &mut 1000));
        schedule.functions[0].context.push(handler_key.clone());
        assert!(admitted(&graph, &schedule, &mut 1000));
        // The guard belongs to the arm's frame even though it is not its body.
        graph.functions[0].function.body = install(unit.clone(), Some(discard.clone()));
        assert!(admitted(&graph, &schedule, &mut 1000));
        graph.functions[0].function.body = install(call(discard.clone()), None);
        assert!(!admitted(&graph, &schedule, &mut 1000));
        schedule.functions[0].context.push(ContextKey::Default {
            construction: source(3),
            parameter: 0,
        });
        assert!(admitted(&graph, &schedule, &mut 1000));
        graph.functions[0].function.body = call(install(discard.clone(), None));
        schedule.functions[0].context.reverse();
        assert!(admitted(&graph, &schedule, &mut 1000));
        schedule.functions[0].context = vec![
            ContextKey::Default {
                construction: source(3),
                parameter: 0,
            },
            ContextKey::EffectHandler {
                installation: source(5),
                handler: 0,
                arm: 1,
            },
        ];
        assert!(!admitted(&graph, &schedule, &mut 1000));
        schedule.functions[0].context = vec![handler_key];
        graph.functions[0].function.body = install(discard, None);
        // HandlerEnd belongs only to the selected arm root, not its parent.
        let program = schedule.functions[0].discards[0].program.clone();
        schedule.functions[0].sites.push(iir::cleanup::Site {
            source: source(2),
            boundary: Boundary::HandlerEnd,
            actions: vec![iir::cleanup::Action {
                target: Target::Local {
                    binding: LocalId { owner: 0, index: 0 },
                    fields: vec![],
                },
                program,
            }],
        });
        let admit_site = |graph: &iir::Package, schedule: &Schedule| {
            let scope = &schedule.functions[0];
            admitted_direct_program(
                graph,
                schedule,
                scope,
                &scope.sites[0].actions[0].program,
                &mut 1000,
            )
        };
        assert!(admit_site(&graph, &schedule));
        schedule.functions[0].sites[0].source = source(5);
        assert!(!admit_site(&graph, &schedule));
        schedule.functions[0].sites[0].source = source(2);
        schedule.functions[0].context.clear();
        assert!(!admit_site(&graph, &schedule));
    }

    #[test]
    fn conditional_sum_admission_tracks_whole_values_not_payload_fields() {
        let graph = iir::Package::default();
        let sum = Ty::ErrorResult(std::sync::Arc::new(vec![
            (None, Ty::Bool),
            (Some(Term::Primitive(P::Bool)), Ty::Bool),
        ]));
        let record = Ty::Record(std::sync::Arc::new(vec![sum]));
        let tags = Tags::from([(vec![0], 1)]);
        for (path, accepted) in [(vec![], true), (vec![0], true), (vec![0, 0], false)] {
            let steps = vec![Step::IfConstructed {
                path,
                steps: vec![],
            }];
            assert_eq!(
                admit_steps(&graph, &record, &steps, &tags, &mut 1000, 0, None),
                accepted
            );
        }
    }

    #[test]
    fn native_select_requires_exactly_one_candidate() {
        let graph = iir::Package::default();
        let lambdas = BTreeMap::new();
        let template_identities = callables::TemplateIdentities::default();
        for guards in [vec![0], vec![1], vec![0, 0], vec![1, 0]] {
            let mut declarations = BTreeSet::new();
            let mut fuel = 10_000;
            let mut emitter = Emitter {
                graph: &graph,
                cleanup: None,
                schedules: None,
                resource_unwinds: vec![],
                checked: true,
                function: None,
                closure_body: None,
                failure_result: None,
                effects: vec![],
                effect_resume: None,
                native_return: None,
                loop_destinations: Default::default(),
                outlined_return: None,
                outlined_loops: BTreeMap::new(),
                lambdas: &lambdas,
                template_identities: &template_identities,
                locals: BTreeMap::new(),
                live: BTreeMap::new(),
                temporaries: BTreeMap::new(),
                temporary_live: BTreeMap::new(),
                prepared: BTreeMap::new(),
                receipt_paths: vec![],
                pattern_orders: BTreeMap::new(),
                temporary_fields: BTreeMap::new(),
                loops: BTreeMap::new(),
                prologue: String::new(),
                body: String::new(),
                declarations: &mut declarations,
                text_constants: &mut strings::TextConstants::default(),
                serial: 0,
                terminated: false,
                fuel: &mut fuel,
            };
            let ty = Ty::ErrorResult(std::sync::Arc::new(vec![
                (None, Ty::Bool),
                (Some(Term::Primitive(P::Bool)), Ty::Bool),
            ]));
            let slot = emitter.slot(ty.clone());
            emitter
                .store(
                    &slot,
                    &Value {
                        ty,
                        text: "zeroinitializer".into(),
                    },
                )
                .unwrap();
            let steps = vec![Step::Select {
                cases: guards
                    .iter()
                    .map(|tag| iir::cleanup::VariantCase {
                        guards: vec![(vec![], *tag)],
                        steps: vec![],
                    })
                    .collect(),
            }];
            assert!(admit_steps(
                &graph,
                &slot.ty,
                &steps,
                &Tags::new(),
                &mut 100,
                0,
                None,
            ));
            emitter
                .cleanup_steps(&slot, &steps, &Tags::new(), None, None)
                .unwrap();
            emitter.line("ret i32 0");
            let ir = format!(
                "declare void @llvm.trap()\ndefine i32 @main() {{\nentry:\n{}{}}}\n",
                emitter.prologue, emitter.body
            );
            for optimization in [
                crate::host::OptimizationLevel::O0,
                crate::host::OptimizationLevel::O3,
            ] {
                let toolchain = crate::host::Toolchain {
                    optimization,
                    ..Default::default()
                };
                let output = toolchain.execute_test_ir(&ir).unwrap();
                assert_eq!(
                    output.status.success(),
                    guards.iter().filter(|tag| **tag == 0).count() == 1,
                    "{guards:?}"
                );
            }
        }
    }
}

fn has_condition(steps: &[Step]) -> bool {
    let mut pending: Vec<_> = steps.iter().collect();
    while let Some(step) = pending.pop() {
        match step {
            Step::IfConstructed { .. } => return true,
            Step::Switch { branches, .. } => pending.extend(branches.iter().flatten()),
            Step::Select { cases } => pending.extend(cases.iter().flat_map(|c| &c.steps)),
            _ => {}
        }
    }
    false
}

enum Owner {
    Local { binding: LocalId, path: Vec<usize> },
    Temporary((u32, u32)),
}
pub(super) fn product_paths(ty: &Ty, prefix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    out.push(prefix.clone());
    if let Ty::Record(fields) | Ty::Closure(fields) = ty {
        for (index, field) in fields.iter().enumerate() {
            prefix.push(index);
            product_paths(field, prefix, out);
            prefix.pop();
        }
    }
}
// Products have independent field tokens; sum payloads are opaque to this
// state model. The sum itself (including a sum stored in a product) has a token.
fn product_projection<'a>(mut ty: &'a Ty, path: &[usize]) -> Option<&'a Ty> {
    for index in path {
        let (Ty::Record(fields) | Ty::Closure(fields)) = ty else {
            return None;
        };
        ty = fields.get(*index)?;
    }
    Some(ty)
}

#[derive(Clone)]
pub(super) struct PreparedSlot {
    source: nir::program::SourceRange,
    storage: Slot,
    live: Slot,
}
impl Emitter<'_> {
    pub(super) fn retain_prepared_field(
        &mut self,
        construction: nir::program::SourceRange,
        index: usize,
        source: nir::program::SourceRange,
        value: &Value,
    ) -> Result<(), String> {
        let needed = self.cleanup.is_some_and(|schedule| schedule.sites.iter().flat_map(|site| &site.actions).any(|action|
            matches!(&action.target, Target::Prepared {construction: origin, slot, value_source} if *origin == construction && *slot == index && *value_source == source)
            && has_invoke(&action.program.steps)));
        if !needed {
            return Ok(());
        }
        let storage = self.slot(value.ty.clone());
        self.store(&storage, value)?;
        let live = self.slot(Ty::Bool);
        self.prologue
            .push_str(&format!("  store i1 false, ptr {}\n", live.pointer));
        self.store(
            &live,
            &Value {
                ty: Ty::Bool,
                text: "true".into(),
            },
        )?;
        self.prepared.insert(
            (construction.start, construction.end, index),
            PreparedSlot {
                source,
                storage,
                live,
            },
        );
        Ok(())
    }
    pub(super) fn finish_prepared_construction(
        &mut self,
        construction: nir::program::SourceRange,
    ) -> Result<(), String> {
        let flags = self
            .prepared
            .iter()
            .filter(|((start, end, _), _)| *start == construction.start && *end == construction.end)
            .map(|(_, slot)| slot.live.clone())
            .collect::<Vec<_>>();
        for flag in flags {
            self.store(
                &flag,
                &Value {
                    ty: Ty::Bool,
                    text: "false".into(),
                },
            )?;
        }
        Ok(())
    }
}
