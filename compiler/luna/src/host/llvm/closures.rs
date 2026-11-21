use super::*;

// State is copied with an owned environment, and borrowed with an invocation.
// Index zero describes the environment product; the remaining paths describe
// its captures, including product subfields. Sum payloads retain whole tokens.
pub(super) fn state_count(fields: &std::sync::Arc<Vec<Ty>>) -> usize {
    fn count(ty: &Ty) -> usize {
        match ty {
            Ty::Record(fields) | Ty::PackedRecord(fields) | Ty::Closure(fields) => {
                state_count(fields)
            }
            _ => 1,
        }
    }
    // repr bounds nesting before constructing these layouts. Count preorder
    // tokens without allocating a Vec for every path at every type emission.
    1 + fields.iter().map(count).sum::<usize>()
}

/// Locate a preorder token without constructing every field path. Charge the
/// preceding subtrees before extending worklists; failure publishes no index.
pub(super) fn state_index(
    fields: &std::sync::Arc<Vec<Ty>>,
    path: &[usize],
    fuel: &mut usize,
) -> Result<usize, String> {
    let root = Ty::Record(fields.clone());
    let mut current = &root;
    let mut index = 0usize;
    for field in path {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("host capture state index budget exhausted")?;
        let (Ty::Record(children) | Ty::PackedRecord(children) | Ty::Closure(children)) = current
        else {
            return Err("host capture state path is not a product".into());
        };
        current = children
            .get(*field)
            .ok_or("host capture state field missing")?;
        *fuel = fuel
            .checked_sub(*field)
            .ok_or("host capture state index budget exhausted")?;
        let mut pending = children[..*field].iter().collect::<Vec<_>>();
        while let Some(node) = pending.pop() {
            index = index
                .checked_add(1)
                .ok_or("host capture state index overflow")?;
            if let Ty::Record(fields) | Ty::PackedRecord(fields) | Ty::Closure(fields) = node {
                *fuel = fuel
                    .checked_sub(fields.len())
                    .ok_or("host capture state index budget exhausted")?;
                pending.extend(fields.iter());
            }
        }
        index = index
            .checked_add(1)
            .ok_or("host capture state index overflow")?;
    }
    Ok(index)
}

pub(super) fn definition<'a>(
    graph: &'a iir::Package,
    schedule: &iir::cleanup::Schedule,
    scope: &iir::cleanup::FunctionSchedule,
    context: &[iir::cleanup::ContextKey],
    fuel: &mut usize,
) -> Result<&'a Expr, String> {
    let mut evidence = None;
    for entry in &schedule.closure_definitions {
        *fuel = fuel
            .checked_sub(1 + context.len())
            .ok_or("host closure definition budget exhausted")?;
        if entry.instance == scope.instance && entry.context == context {
            if evidence.replace(&entry.definition).is_some() {
                return Err("host closure definition evidence is ambiguous".into());
            }
        }
    }
    let evidence = evidence.ok_or("host closure definition evidence missing")?;
    let Some(iir::cleanup::ContextKey::Closure { definition, .. }) = context.last() else {
        return Err("host closure definition context missing".into());
    };
    if evidence.source != *definition || !matches!(evidence.kind, K::Closure { .. }) {
        return Err("host closure definition disagrees with its context".into());
    }
    concrete_definition(graph, evidence, fuel)
}

fn concrete_definition<'a>(
    graph: &'a iir::Package,
    evidence: &Expr,
    fuel: &mut usize,
) -> Result<&'a Expr, String> {
    let mut found = None;
    for instance in &graph.functions {
        nir::visit::expressions(&instance.function, fuel, |expr, fuel| {
            if expr.source == evidence.source && matches!(expr.kind, K::Closure { .. }) {
                // Bound structural comparison by visiting both concrete trees.
                nir::visit::expression(expr, fuel, |_, _| Ok(()))?;
                nir::visit::expression(evidence, fuel, |_, _| Ok(()))?;
                if expr == evidence {
                    found = Some(expr);
                }
            }
            Ok(())
        })?;
    }
    found.ok_or_else(|| "host cleanup closure definition missing from instance graph".into())
}

pub(super) fn schedules_for<'a>(
    graph: &iir::Package,
    schedule: &'a iir::cleanup::Schedule,
    node: &Expr,
    fuel: &mut usize,
) -> Result<Vec<(usize, &'a iir::cleanup::FunctionSchedule)>, String> {
    use iir::cleanup::ContextKey;
    let K::Closure { .. } = &node.kind else {
        return Err("host lambda definition missing".into());
    };
    let mut selected = Vec::new();
    for (index, candidate) in schedule.functions.iter().enumerate() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("host closure schedule budget exhausted")?;
        if !matches!(candidate.context.last(), Some(ContextKey::Closure { definition, .. }) if *definition == node.source)
        {
            continue;
        }
        let concrete = definition(graph, schedule, candidate, &candidate.context, fuel)?;
        if concrete != node {
            continue;
        }
        cleanup::inline_scopes(schedule, candidate, fuel)?;
        for alias in &schedule.aliases {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("host closure schedule budget exhausted")?;
            if alias.instance == candidate.instance && alias.context.starts_with(&candidate.context)
            {
                cleanup::validate_alias(schedule, alias, fuel)?;
            }
        }
        selected.push((index, candidate));
    }
    Ok(selected)
}

fn collect_pattern_bound_locals(
    pattern: &nir::program::Pattern,
    bound: &mut BTreeSet<LocalId>,
    fuel: &mut usize,
) -> Result<(), String> {
    use nir::program::Pattern as P;
    let mut stack = vec![(pattern, 0usize)];
    while let Some((pattern, depth)) = stack.pop() {
        if depth >= 128 {
            return Err("host closure pattern local scan nesting exhausted".into());
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or("host closure pattern local scan budget exhausted")?;
        match pattern {
            P::Binding { binding, inner } | P::Observe { binding, inner } => {
                bound.insert(*binding);
                stack.push((inner, depth + 1));
            }
            P::CodeEvidence { outputs } => {
                for (id, _) in outputs {
                    bound.insert(*id);
                }
            }
            P::Remap { pattern, bindings } => {
                for (from, to) in bindings {
                    bound.insert(*from);
                    bound.insert(*to);
                }
                stack.push((pattern, depth + 1));
            }
            P::OptionalPresent(inner)
            | P::ErrorResult { inner, .. }
            | P::Guard { pattern: inner, .. } => {
                stack.push((inner, depth + 1));
            }
            P::Then { first, next, .. } => {
                stack.push((next, depth + 1));
                stack.push((first, depth + 1));
            }
            P::AnonymousRecord { fields, .. }
            | P::Record { fields, .. }
            | P::Tuple(fields)
            | P::TypeTuple(fields)
            | P::NominalType { fields, .. }
            | P::NormalForm {
                arguments: fields, ..
            }
            | P::Variant { fields, .. }
            | P::Or(fields) => {
                *fuel = fuel
                    .checked_sub(fields.len())
                    .ok_or("host closure pattern local scan budget exhausted")?;
                for field in fields {
                    stack.push((field, depth + 1));
                }
            }
            P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => {}
        }
    }
    Ok(())
}

fn collect_closure_bound_locals(
    node: &Expr,
    fuel: &mut usize,
) -> Result<BTreeSet<LocalId>, String> {
    let K::Closure {
        parameters,
        captures,
        body,
    } = &node.kind
    else {
        return Err("host closure bound locals scan requires a closure node".into());
    };
    let mut bound = BTreeSet::new();
    for capture in captures {
        bound.insert(capture.binding);
    }
    for parameter in parameters {
        bound.insert(*parameter);
    }
    nir::visit::expression_in_closure_environment(body, fuel, |expr, fuel| {
        match &expr.kind {
            K::Let { binding, .. } => {
                bound.insert(*binding);
            }
            K::LetPattern { pattern, .. } => {
                collect_pattern_bound_locals(pattern, &mut bound, fuel)?;
            }
            K::Match { arms, .. } => {
                for arm in arms {
                    collect_pattern_bound_locals(&arm.pattern, &mut bound, fuel)?;
                }
            }
            K::EffectHandle { handlers, .. } => {
                for handler in handlers {
                    for arm in &handler.arms {
                        collect_pattern_bound_locals(&arm.pattern, &mut bound, fuel)?;
                    }
                }
            }
            K::Call {
                parameter_bindings, ..
            } => {
                bound.extend(parameter_bindings.iter().copied());
            }
            K::Loop { identity, .. } => {
                bound.insert(*identity);
            }
            K::NominalType {
                bindings: Some(bindings),
                ..
            }
            | K::CheckTypeAlias {
                bindings: Some(bindings),
                ..
            } => {
                bound.extend(bindings.iter().copied());
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(bound)
}

fn collect_env_expr_locals(
    expr: &Expr,
    out: &mut BTreeMap<LocalId, Term>,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("host cleanup environment nesting budget exhausted".into());
    }
    *fuel = fuel
        .checked_sub(1)
        .ok_or("host closure external cleanup environment budget exhausted")?;
    match &expr.kind {
        K::Local(binding) => {
            if let Some(existing) = out.get(binding) {
                if existing != &expr.ty {
                    return Err("host closure external cleanup environment type mismatch".into());
                }
            } else {
                out.insert(*binding, expr.ty.clone());
            }
            Ok(())
        }
        K::AddressOf(inner)
        | K::ReadPointer(inner)
        | K::NewtypeConvert(inner)
        | K::UnsafeScope(inner)
        | K::Snapshot(inner) => collect_env_expr_locals(inner, out, fuel, depth + 1),
        K::PointerField { pointer: inner, .. }
        | K::PointerAnonymousField { pointer: inner, .. }
        | K::Field { value: inner, .. }
        | K::AnonymousField { value: inner, .. } => {
            collect_env_expr_locals(inner, out, fuel, depth + 1)
        }
        K::ConstAddress { .. } | K::Literal(_) | K::Unit => Ok(()),
        _ => Err("host cleanup environment argument requires a pure expression".into()),
    }
}

fn collect_step_env_locals(
    steps: &[iir::cleanup::Step],
    out: &mut BTreeMap<LocalId, Term>,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    use iir::cleanup::Step;
    if depth >= 128 {
        return Err("host cleanup step nesting budget exhausted".into());
    }
    for step in steps {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("host closure external cleanup step budget exhausted")?;
        match step {
            Step::Invoke { env_arguments, .. } => {
                for arg in env_arguments {
                    collect_env_expr_locals(arg, out, fuel, 0)?;
                }
            }
            Step::Select { cases } => {
                for case in cases {
                    collect_step_env_locals(&case.steps, out, fuel, depth + 1)?;
                }
            }
            Step::IfConstructed { steps, .. } => {
                collect_step_env_locals(steps, out, fuel, depth + 1)?;
            }
            Step::Switch { branches, .. } => {
                for branch in branches {
                    collect_step_env_locals(branch, out, fuel, depth + 1)?;
                }
            }
            Step::Release { .. } => {}
        }
    }
    Ok(())
}

fn collect_function_schedule_env_locals(
    desc: &iir::cleanup::FunctionSchedule,
    out: &mut BTreeMap<LocalId, Term>,
    fuel: &mut usize,
) -> Result<(), String> {
    for site in &desc.sites {
        for action in &site.actions {
            collect_step_env_locals(&action.program.steps, out, fuel, 0)?;
        }
    }
    for discard in &desc.discards {
        collect_step_env_locals(&discard.program.steps, out, fuel, 0)?;
    }
    Ok(())
}

pub(super) fn schedule_cleanup_env_locals(
    schedule: &iir::cleanup::Schedule,
    instance: &iir::InstanceKey,
    prefix: &[iir::cleanup::ContextKey],
    fuel: &mut usize,
) -> Result<BTreeMap<LocalId, Term>, String> {
    use iir::cleanup::ContextKey;
    let mut locals: BTreeMap<LocalId, Term> = BTreeMap::new();
    for desc in &schedule.functions {
        *fuel = fuel
            .checked_sub(1 + desc.context.len())
            .ok_or("host closure external env scan budget exhausted")?;
        if &desc.instance != instance || !desc.context.starts_with(prefix) {
            continue;
        }
        let suffix = &desc.context[prefix.len()..];
        if suffix
            .iter()
            .any(|key| matches!(key, ContextKey::Function { .. }))
        {
            continue;
        }
        collect_function_schedule_env_locals(desc, &mut locals, fuel)?;
    }
    Ok(locals)
}

pub(super) fn scope_external_cleanup_env_slots(
    graph: &iir::Package,
    schedule: &iir::cleanup::Schedule,
    scope: &iir::cleanup::FunctionSchedule,
    fuel: &mut usize,
) -> Result<Vec<(LocalId, Term)>, String> {
    use iir::cleanup::ContextKey;
    let Some(ContextKey::Closure { call, .. }) = scope.context.last() else {
        return Err("host closure scope context missing".into());
    };
    let parent_context = &scope.context[..scope.context.len() - 1];
    let mut external: BTreeMap<LocalId, Term> = BTreeMap::new();
    for candidate in &schedule.functions {
        *fuel = fuel
            .checked_sub(1 + candidate.context.len())
            .ok_or("host closure external env lookup budget exhausted")?;
        if candidate.instance != scope.instance
            || candidate.context.len() != parent_context.len() + 1
            || !candidate.context.starts_with(parent_context)
            || !matches!(
                candidate.context.last(),
                Some(ContextKey::Closure { call: c, .. }) if c == call
            )
        {
            continue;
        }
        let cand_node = definition(graph, schedule, candidate, &candidate.context, fuel)?;
        let bound = collect_closure_bound_locals(cand_node, fuel)?;
        for desc in &schedule.functions {
            *fuel = fuel
                .checked_sub(1 + desc.context.len())
                .ok_or("host closure external env scan budget exhausted")?;
            if desc.instance != candidate.instance || !desc.context.starts_with(&candidate.context)
            {
                continue;
            }
            let suffix = &desc.context[candidate.context.len()..];
            if suffix
                .iter()
                .any(|key| matches!(key, ContextKey::Function { .. }))
            {
                continue;
            }
            let nested_bound = if suffix
                .iter()
                .any(|key| matches!(key, ContextKey::Closure { .. }))
            {
                let mut combined = bound.clone();
                for (offset, key) in suffix.iter().enumerate() {
                    if matches!(key, ContextKey::Closure { .. }) {
                        let prefix = &desc.context[..candidate.context.len() + offset + 1];
                        let nested_node = definition(graph, schedule, desc, prefix, fuel)?;
                        combined.extend(collect_closure_bound_locals(nested_node, fuel)?);
                    }
                }
                Some(combined)
            } else {
                None
            };
            let desc_bound = nested_bound.as_ref().unwrap_or(&bound);
            let mut desc_locals = BTreeMap::new();
            collect_function_schedule_env_locals(desc, &mut desc_locals, fuel)?;
            for (local, ty) in desc_locals {
                if desc_bound.contains(&local) {
                    continue;
                }
                if let Some(existing) = external.get(&local) {
                    if existing != &ty {
                        return Err(
                            "host closure external cleanup environment type mismatch".into()
                        );
                    }
                } else {
                    external.insert(local, ty);
                }
            }
        }
    }
    Ok(external.into_iter().collect())
}

pub(super) fn lambda_requires_external_cleanup_envs(
    graph: &iir::Package,
    schedule: &iir::cleanup::Schedule,
    node: &Expr,
    fuel: &mut usize,
) -> Result<bool, String> {
    for (_, scope) in schedules_for(graph, schedule, node, fuel)? {
        if !scope_external_cleanup_env_slots(graph, schedule, scope, fuel)?.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

impl Emitter<'_> {
    pub(super) fn call_external_cleanup_env_slots(
        &mut self,
        source: nir::program::SourceRange,
    ) -> Result<Vec<(LocalId, Term)>, String> {
        let (Some(parent), Some(schedule)) = (self.cleanup, self.schedules) else {
            return Ok(vec![]);
        };
        for candidate in &schedule.functions {
            *self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or("host closure dispatch env budget exhausted")?;
            if candidate.instance == parent.instance
                && candidate.context.len() == parent.context.len() + 1
                && candidate.context.starts_with(&parent.context)
                && matches!(
                    candidate.context.last(),
                    Some(iir::cleanup::ContextKey::Closure { call, .. }) if *call == source
                )
            {
                return scope_external_cleanup_env_slots(
                    self.graph, schedule, candidate, self.fuel,
                );
            }
        }
        Ok(vec![])
    }

    /// Code identity stays stored in the closure. Only this invocation selects
    /// a machine body using its already checked resource context.
    pub(super) fn specialize_closure_call(
        &mut self,
        source: nir::program::SourceRange,
        code: Value,
    ) -> Result<Value, String> {
        let (Some(parent), Some(schedule)) = (self.cleanup, self.schedules) else {
            return Ok(code);
        };
        let mut routes: BTreeMap<usize, (usize, &iir::cleanup::FunctionSchedule)> = BTreeMap::new();
        for (index, candidate) in schedule.functions.iter().enumerate() {
            *self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or("host closure dispatch budget exhausted")?;
            if candidate.instance != parent.instance
                || candidate.context.len() != parent.context.len() + 1
                || !candidate.context.starts_with(&parent.context)
                || !matches!(candidate.context.last(), Some(iir::cleanup::ContextKey::Closure {call, ..}) if *call == source)
            {
                continue;
            }
            let node = definition(
                self.graph,
                schedule,
                candidate,
                &candidate.context,
                self.fuel,
            )?;
            // Equal concrete definitions can occur in multiple graph instances.
            // Route each registered code identity, not merely the last match.
            for instance in &self.graph.functions {
                nir::visit::expressions(&instance.function, self.fuel, |expr, fuel| {
                    if expr.source == node.source && matches!(expr.kind, K::Closure { .. }) {
                        nir::visit::expression(expr, fuel, |_, _| Ok(()))?;
                        nir::visit::expression(node, fuel, |_, _| Ok(()))?;
                        if expr == node {
                            let lambda = *self
                                .lambdas
                                .get(&(expr as *const Expr as usize))
                                .ok_or("host closure dispatch identity missing")?;
                            if let Some((_, previous)) = routes.get(&lambda) {
                                if !cleanup::same_frame(schedule, previous, candidate, fuel)? {
                                    return Err(
                                        "host closure call has ambiguous cleanup contexts".into()
                                    );
                                }
                            } else {
                                routes.insert(lambda, (index, candidate));
                            }
                        }
                    }
                    Ok(())
                })?;
            }
        }
        let mut selected = code.clone();
        for (lambda, (context, _)) in routes {
            let matches = self.instruction(
                Ty::Bool,
                format!("icmp eq ptr {}, @flurry_lambda_{lambda}", code.text),
            );
            selected = self.instruction(
                Ty::FunctionPointer,
                format!(
                    "select i1 {}, ptr @flurry_lambda_{lambda}_scope_{context}, ptr {}",
                    matches.text, selected.text
                ),
            );
        }
        Ok(selected)
    }

    pub(super) fn has_checked_closure_call(
        &mut self,
        source: nir::program::SourceRange,
    ) -> Result<bool, String> {
        let parent = self
            .cleanup
            .ok_or("host closure call lacks a cleanup scope")?;
        let schedule = self
            .schedules
            .ok_or("host closure call lacks a cleanup schedule")?;
        for candidate in &schedule.functions {
            *self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or("host closure call budget exhausted")?;
            if candidate.instance == parent.instance
                && candidate.context.len() == parent.context.len() + 1
                && candidate.context.starts_with(&parent.context)
                && matches!(candidate.context.last(), Some(iir::cleanup::ContextKey::Closure {call, ..}) if *call == source)
            {
                return Ok(true);
            }
        }
        // A complete acquired parent frame can exclude this call (for example
        // Some's map arm when the actual input is known None). The caller must
        // emit a fail-closed unreachable edge, never an unchecked invocation.
        Ok(false)
    }

    pub(super) fn check_closure_receiver(
        &mut self,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
    ) -> Result<(), String> {
        let mut base = callee;
        let mut path = Vec::new();
        while let K::Field { value, index, .. } | K::AnonymousField { value, index, .. } =
            &base.kind
        {
            path.push(*index);
            base = value;
        }
        let K::Local(root) = base.kind else {
            return Ok(());
        };
        path.reverse();
        for argument in arguments {
            nir::visit::expression_in_closure_environment(
                &argument.value,
                self.fuel,
                |expr, _| {
                    if let K::Assign { place, .. } = &expr.kind {
                        if place.root == root
                            && place
                                .fields
                                .iter()
                                .map(|(_, index)| *index)
                                .zip(&path)
                                .all(|(left, right)| left == *right)
                        {
                            return Err(
                                "host closure receiver is replaced during argument evaluation"
                                    .into(),
                            );
                        }
                    }
                    Ok(())
                },
            )?;
        }
        Ok(())
    }

    pub(super) fn closure(&mut self, expr: &Expr, depth: usize) -> Result<Value, String> {
        let K::Closure { captures, .. } = &expr.kind else {
            unreachable!()
        };
        let index = if let Some(index) = self.lambdas.get(&(expr as *const Expr as usize)) {
            index
        } else {
            // Lexical handlers carry copied syntax with shared runtime slots.
            // Resolve a copy only when its complete instantiated definition is
            // identical; a source position alone cannot identify generic code.
            let original = concrete_definition(self.graph, expr, self.fuel)?;
            if original != expr {
                return Err("host copied lambda differs from its registered definition".into());
            }
            self.lambdas
                .get(&(original as *const Expr as usize))
                .ok_or("host lambda body is not registered")?
        };
        let symbol = format!("@flurry_lambda_{index}");
        let ty = repr(self.graph, &expr.ty, self.checked)?;
        if captures.is_empty() && ty == Ty::FunctionPointer {
            return Ok(Value { ty, text: symbol });
        }
        let Ty::Closure(fields) = &ty else {
            return Err("host capture environment layout missing".into());
        };
        if captures.len() != fields.len() {
            return Err("host capture environment arity mismatch".into());
        }
        let mut value = self.instruction(
            ty.clone(),
            format!("insertvalue {} zeroinitializer, ptr {symbol}, 0", ty.llvm()),
        );
        let states = state_count(fields);
        value = self.instruction(
            ty.clone(),
            format!(
                "insertvalue {} {}, [{states} x i1] [{}], 2",
                ty.llvm(),
                value.text,
                vec!["i1 true"; states].join(", ")
            ),
        );
        for (index, (capture, expected)) in captures.iter().zip(fields.iter()).enumerate() {
            let captured = if let Some(value) = &capture.value {
                self.expr(value, depth)?
            } else {
                let slot = self
                    .locals
                    .get(&capture.binding)
                    .cloned()
                    .ok_or("host capture binding missing")?;
                self.load(&slot)
            };
            if self.terminated {
                return Ok(Value::unit());
            }
            if captured.ty != *expected {
                return Err("host capture representation mismatch".into());
            }
            self.retain_prepared_field(
                expr.source,
                index,
                capture
                    .value
                    .as_ref()
                    .map_or(expr.source, |value| value.source),
                &captured,
            )?;
            if captured.ty != Ty::Unit {
                value = self.instruction(
                    ty.clone(),
                    format!(
                        "insertvalue {} {}, {} {}, 1, {index}",
                        ty.llvm(),
                        value.text,
                        captured.ty.llvm(),
                        captured.text
                    ),
                );
            }
        }
        self.finish_prepared_construction(expr.source)?;
        Ok(value)
    }

    pub(super) fn bind_environment(
        &mut self,
        node: &Expr,
        inputs: &mut Vec<String>,
    ) -> Result<(), String> {
        let K::Closure { captures, .. } = &node.kind else {
            unreachable!()
        };
        if captures.is_empty() {
            return Ok(());
        }
        let ty = repr(self.graph, &node.ty, self.checked)?;
        let Ty::Closure(fields) = &ty else {
            return Err("host lambda environment missing".into());
        };
        if fields.len() != captures.len() {
            return Err("host lambda capture count mismatch".into());
        }
        inputs.push("ptr %environment".into());
        for (index, (capture, field)) in captures.iter().zip(fields.iter()).enumerate() {
            let name = self.fresh();
            self.line(format!(
                "%{name} = getelementptr {}, ptr %environment, i32 0, i32 1, i32 {index}",
                ty.llvm()
            ));
            self.locals.insert(
                capture.binding,
                Slot {
                    align: matches!(field, Ty::PackedRecord(_)).then_some(1),
                    ty: field.clone(),
                    pointer: format!("%{name}"),
                },
            );
            let mut paths = vec![];
            cleanup::product_paths(field, &mut vec![], &mut paths);
            let environment = Slot {
                ty: ty.clone(),
                pointer: "%environment".into(),
                align: None,
            };
            for path in paths {
                let full = std::iter::once(index)
                    .chain(path.iter().copied())
                    .collect::<Vec<_>>();
                let flag = self
                    .embedded_live_flag(&environment, &full)?
                    .ok_or("host capture state layout missing")?;
                self.live.insert((capture.binding, path), flag);
            }
        }
        Ok(())
    }

    // Only resolve already-evaluated lexical places; never repeat callee effects.
    fn closure_place(&mut self, expr: &Expr, depth: usize) -> Result<Option<Slot>, String> {
        if depth >= 128 {
            return Err("host closure place nesting exhausted".into());
        }
        match &expr.kind {
            K::Local(id) => Ok(self.locals.get(id).cloned()),
            K::Field { value, index, .. } | K::AnonymousField { value, index, .. } => {
                let Some(slot) = self.closure_place(value, depth + 1)? else {
                    return Ok(None);
                };
                let (Ty::Record(fields) | Ty::PackedRecord(fields)) = &slot.ty else {
                    return Err("host closure field layout missing".into());
                };
                let ty = fields
                    .get(*index)
                    .cloned()
                    .ok_or("host closure field out of range")?;
                let align = if slot.align == Some(1)
                    || matches!(slot.ty, Ty::PackedRecord(_))
                    || matches!(ty, Ty::PackedRecord(_))
                {
                    Some(1)
                } else {
                    slot.align
                };
                let name = self.fresh();
                self.line(format!(
                    "%{name} = getelementptr {}, ptr {}, i32 0, i32 {index}",
                    slot.ty.llvm(),
                    slot.pointer
                ));
                Ok(Some(Slot {
                    ty,
                    pointer: format!("%{name}"),
                    align,
                }))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn closure_callee(
        &mut self,
        expr: &Expr,
        value: Value,
    ) -> Result<(Value, Option<Slot>), String> {
        if value.ty == Ty::FunctionPointer {
            return Ok((value, None));
        }
        if !matches!(value.ty, Ty::Closure(_)) {
            return Err("host callee has no runtime callable representation".into());
        }
        // The invocation must mutate exactly the environment later inspected by
        // residual cleanup. A second copy would retain stale capture tokens.
        let slot = if let Some(slot) = self.retain_temporary(expr.source, &value)? {
            slot
        } else if let Some(slot) = self.closure_place(expr, 0)? {
            slot
        } else {
            let slot = self.slot(value.ty.clone());
            self.store(&slot, &value)?;
            slot
        };
        let code = self.instruction(
            Ty::FunctionPointer,
            format!("extractvalue {} {}, 0", value.ty.llvm(), value.text),
        );
        Ok((code, Some(slot)))
    }
}

#[cfg(test)]
mod state_count_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn counts_and_indices_match_preorder_for_nested_environments() {
        let fields = Arc::new(vec![
            Ty::Bool,
            Ty::Record(Arc::new(vec![Ty::Unit, Ty::Pointer])),
            Ty::Closure(Arc::new(vec![
                Ty::Record(Arc::new(vec![])),
                Ty::Int(32, false),
            ])),
        ]);
        let expected = [
            vec![],
            vec![0],
            vec![1],
            vec![1, 0],
            vec![1, 1],
            vec![2],
            vec![2, 0],
            vec![2, 1],
        ];
        assert_eq!(state_count(&fields), expected.len());
        for (index, path) in expected.iter().enumerate() {
            assert_eq!(state_index(&fields, path, &mut 100).unwrap(), index);
        }
        let sum = Ty::ErrorResult(Arc::new(
            ErrorResultLayout::new(vec![(None, Ty::Record(fields.clone()))], 64).unwrap(),
        ));
        assert_eq!(state_count(&Arc::new(vec![sum])), 2);
        assert_eq!(state_count(&Arc::new(vec![])), 1);
        let mut nested = Ty::Bool;
        for _ in 0..100 {
            nested = Ty::Record(Arc::new(vec![nested]));
        }
        assert_eq!(state_count(&Arc::new(vec![nested])), 102);
    }

    #[test]
    fn scope_external_cleanup_env_slots_merges_siblings_descendants_and_excludes_locals() {
        use iir::cleanup::{
            Action, Boundary, ClosureDefinition, ContextKey, DiscardCleanup, FunctionSchedule,
            Program, Schedule, Site, Step, Target,
        };
        use nir::program::{Capture, FunctionDeclaration, FunctionPhase, SourceRange};

        let src = |start| SourceRange {
            start,
            end: start + 1,
        };
        let u64_ty = Term::Primitive(P::U64);
        let ptr_u64 = Term::Pointer(Box::new(u64_ty.clone()));
        let decl = nir::DeclarationId {
            package: 0,
            local: 1,
        };
        let instance = iir::InstanceKey {
            declaration: decl,
            arguments: vec![].into(),
        };
        let call_src = src(100);
        let cap_local = LocalId { owner: 1, index: 0 };
        let param_local = LocalId { owner: 1, index: 1 };
        let inner_let = LocalId { owner: 1, index: 2 };
        let ext_a = LocalId {
            owner: nir::program::IMPLICIT_RUNTIME_ENV_OWNER,
            index: 4,
        };
        let ext_b = LocalId { owner: 9, index: 7 };

        let make_closure = |def_src: SourceRange, body_let: LocalId| Expr {
            source: def_src,
            ty: Term::Closure {
                environment: vec![u64_ty.clone()],
                signature: Box::new(Term::Function {
                    parameters: vec![nir::Parameter {
                        sort: Sort::Runtime,
                        ty: u64_ty.clone(),
                    }],
                    result: Box::new(Term::Primitive(P::Void)),
                }),
            },
            kind: K::Closure {
                parameters: vec![param_local],
                captures: vec![Capture {
                    binding: cap_local,
                    ty: u64_ty.clone(),
                    value: None,
                }],
                body: std::sync::Arc::new(Expr {
                    source: src(200),
                    ty: Term::Primitive(P::Void),
                    kind: K::Let {
                        binding: body_let,
                        value: Box::new(Expr {
                            source: src(201),
                            ty: u64_ty.clone(),
                            kind: K::Literal(Literal::UnsignedInteger(0)),
                        }),
                    },
                }),
            },
        };

        let def_a_src = src(10);
        let def_b_src = src(20);
        let closure_a = make_closure(def_a_src, inner_let);
        let closure_b = make_closure(def_b_src, LocalId { owner: 2, index: 0 });
        let wrapper_body = Expr {
            source: src(50),
            ty: Term::Primitive(P::Void),
            kind: K::Block {
                statements: vec![closure_a.clone()],
                result: Some(Box::new(closure_b.clone())),
            },
        };
        let graph = iir::Package {
            functions: vec![iir::FunctionInstance {
                key: instance.clone(),
                function: Function {
                    interface: FunctionDeclaration {
                        source: src(0),
                        declaration: decl,
                        name: "caller".into(),
                        phase: FunctionPhase::Runtime,
                        inline: false,
                        pure: false,
                        is_unsafe: false,
                        type_parameters: vec![],
                        parameters: vec![],
                        env_parameters: vec![],
                        defaults: vec![],
                        preconditions: vec![],
                        postconditions: vec![],
                        result_binding: None,
                        result_type_binder: None,
                        result_guarantees: vec![],
                        entry_snapshots: vec![],
                        capabilities: vec![],
                        requirements: vec![],
                        signature: Term::Function {
                            parameters: vec![],
                            result: Box::new(Term::Primitive(P::Void)),
                        },
                    },
                    body: wrapper_body,
                    runtime_deliveries: Default::default(),
                },
            }],
            ..Default::default()
        };

        let ctx_a = vec![ContextKey::Closure {
            call: call_src,
            definition: def_a_src,
        }];
        let ctx_a_desc = vec![
            ContextKey::Closure {
                call: call_src,
                definition: def_a_src,
            },
            ContextKey::Default {
                construction: src(250),
                parameter: 0,
            },
        ];
        let ctx_b = vec![ContextKey::Closure {
            call: call_src,
            definition: def_b_src,
        }];
        let invoke_step = |local: LocalId, ty: Term| Step::Invoke {
            path: vec![],
            subject: u64_ty.clone(),
            method: instance.clone(),
            env_arguments: vec![Expr {
                source: src(300),
                ty,
                kind: K::Local(local),
            }],
        };

        let schedule_a = FunctionSchedule {
            instance: instance.clone(),
            context: ctx_a.clone(),
            sites: vec![Site {
                source: src(200),
                boundary: Boundary::FunctionEnd,
                actions: vec![Action {
                    target: Target::Local {
                        binding: cap_local,
                        fields: vec![],
                    },
                    program: Program {
                        ty: u64_ty.clone(),
                        guards: vec![],
                        steps: vec![
                            invoke_step(cap_local, u64_ty.clone()),
                            invoke_step(param_local, u64_ty.clone()),
                            invoke_step(inner_let, u64_ty.clone()),
                            invoke_step(ext_a, ptr_u64.clone()),
                        ],
                    },
                }],
            }],
            discards: vec![],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
        };
        let schedule_a_desc = FunctionSchedule {
            instance: instance.clone(),
            context: ctx_a_desc,
            sites: vec![],
            discards: vec![DiscardCleanup {
                source: src(260),
                value_source: src(261),
                program: Program {
                    ty: u64_ty.clone(),
                    guards: vec![],
                    steps: vec![Step::IfConstructed {
                        path: vec![],
                        steps: vec![invoke_step(ext_b, u64_ty.clone())],
                    }],
                },
            }],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
        };
        let schedule_b = FunctionSchedule {
            instance: instance.clone(),
            context: ctx_b.clone(),
            sites: vec![],
            discards: vec![],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
        };
        let fn_local_ignored = LocalId { owner: 7, index: 3 };
        let mut ctx_a_fn = ctx_a.clone();
        ctx_a_fn.push(ContextKey::Function {
            call: src(270),
            callee: instance.clone(),
        });
        let schedule_a_fn = FunctionSchedule {
            instance: instance.clone(),
            context: ctx_a_fn,
            sites: vec![],
            discards: vec![DiscardCleanup {
                source: src(271),
                value_source: src(272),
                program: Program {
                    ty: u64_ty.clone(),
                    guards: vec![],
                    steps: vec![invoke_step(fn_local_ignored, u64_ty.clone())],
                },
            }],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
        };

        let schedule = Schedule {
            closure_definitions: vec![
                ClosureDefinition {
                    instance: instance.clone(),
                    context: ctx_a,
                    definition: closure_a.clone(),
                },
                ClosureDefinition {
                    instance: instance.clone(),
                    context: ctx_b,
                    definition: closure_b,
                },
            ],
            borrowed_callable_functions: vec![],
            functions: vec![
                schedule_a.clone(),
                schedule_a_desc,
                schedule_a_fn,
                schedule_b.clone(),
            ],
            aliases: vec![],
        };

        let slots_b =
            scope_external_cleanup_env_slots(&graph, &schedule, &schedule_b, &mut 10_000).unwrap();
        assert_eq!(
            slots_b,
            vec![(ext_b, u64_ty.clone()), (ext_a, ptr_u64.clone())]
        );
        let slots_a =
            scope_external_cleanup_env_slots(&graph, &schedule, &schedule_a, &mut 10_000).unwrap();
        assert_eq!(slots_a, slots_b);
        assert!(
            lambda_requires_external_cleanup_envs(&graph, &schedule, &closure_a, &mut 10_000)
                .unwrap()
        );
    }

    #[test]
    fn schedule_cleanup_env_locals_collects_handler_and_nested_closures_skipping_functions() {
        use iir::cleanup::{
            Action, Boundary, ContextKey, DiscardCleanup, FunctionSchedule, Program, Schedule,
            Site, Step, Target,
        };
        use nir::program::SourceRange;

        let src = |start| SourceRange {
            start,
            end: start + 1,
        };
        let u64_ty = Term::Primitive(P::U64);
        let ptr_u64 = Term::Pointer(Box::new(u64_ty.clone()));
        let instance = iir::InstanceKey {
            declaration: nir::DeclarationId {
                package: 0,
                local: 9,
            },
            arguments: vec![].into(),
        };
        let arm_prefix = vec![ContextKey::EffectHandler {
            installation: src(10),
            handler: 0,
            arm: 1,
        }];
        let mut nested_closure_ctx = arm_prefix.clone();
        nested_closure_ctx.push(ContextKey::Closure {
            call: src(20),
            definition: src(30),
        });
        let mut nested_fn_ctx = nested_closure_ctx.clone();
        nested_fn_ctx.push(ContextKey::Function {
            call: src(40),
            callee: instance.clone(),
        });

        let arm_local = LocalId { owner: 9, index: 1 };
        let nested_closure_local = LocalId { owner: 9, index: 2 };
        let fn_local_ignored = LocalId { owner: 9, index: 3 };

        let invoke_step = |local: LocalId, ty: Term| Step::Invoke {
            path: vec![],
            subject: u64_ty.clone(),
            method: instance.clone(),
            env_arguments: vec![Expr {
                source: src(2),
                ty,
                kind: K::Local(local),
            }],
        };

        let arm_schedule = FunctionSchedule {
            instance: instance.clone(),
            context: arm_prefix.clone(),
            sites: vec![Site {
                source: src(11),
                boundary: Boundary::HandlerEnd,
                actions: vec![Action {
                    target: Target::Local {
                        binding: arm_local,
                        fields: vec![],
                    },
                    program: Program {
                        ty: u64_ty.clone(),
                        guards: vec![],
                        steps: vec![invoke_step(arm_local, u64_ty.clone())],
                    },
                }],
            }],
            discards: vec![],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
        };
        let nested_closure_schedule = FunctionSchedule {
            instance: instance.clone(),
            context: nested_closure_ctx,
            sites: vec![],
            discards: vec![DiscardCleanup {
                source: src(21),
                value_source: src(22),
                program: Program {
                    ty: ptr_u64.clone(),
                    guards: vec![],
                    steps: vec![invoke_step(nested_closure_local, ptr_u64.clone())],
                },
            }],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
        };
        let nested_fn_schedule = FunctionSchedule {
            instance: instance.clone(),
            context: nested_fn_ctx,
            sites: vec![],
            discards: vec![DiscardCleanup {
                source: src(41),
                value_source: src(42),
                program: Program {
                    ty: u64_ty.clone(),
                    guards: vec![],
                    steps: vec![invoke_step(fn_local_ignored, u64_ty.clone())],
                },
            }],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
        };

        let schedule = Schedule {
            closure_definitions: vec![],
            borrowed_callable_functions: vec![],
            functions: vec![arm_schedule, nested_closure_schedule, nested_fn_schedule],
            aliases: vec![],
        };

        let locals =
            schedule_cleanup_env_locals(&schedule, &instance, &arm_prefix, &mut 10_000).unwrap();
        let expected = BTreeMap::from([(arm_local, u64_ty), (nested_closure_local, ptr_u64)]);
        assert_eq!(locals, expected);
    }
}
