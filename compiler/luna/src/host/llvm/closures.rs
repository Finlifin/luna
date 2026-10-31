use super::*;

// State is copied with an owned environment, and borrowed with an invocation.
// Index zero describes the environment product; the remaining paths describe
// its captures, including product subfields. Sum payloads retain whole tokens.
pub(super) fn state_count(fields: &std::sync::Arc<Vec<Ty>>) -> usize {
    fn count(ty: &Ty) -> usize {
        match ty {
            Ty::Record(fields) | Ty::Closure(fields) => state_count(fields),
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
        let (Ty::Record(children) | Ty::Closure(children)) = current else {
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
            if let Ty::Record(fields) | Ty::Closure(fields) = node {
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

impl Emitter<'_> {
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
                    ty: field.clone(),
                    pointer: format!("%{name}"),
                },
            );
            let mut paths = vec![];
            cleanup::product_paths(field, &mut vec![], &mut paths);
            let environment = Slot {
                ty: ty.clone(),
                pointer: "%environment".into(),
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
                let Ty::Record(fields) = &slot.ty else {
                    return Err("host closure field layout missing".into());
                };
                let ty = fields
                    .get(*index)
                    .cloned()
                    .ok_or("host closure field out of range")?;
                let name = self.fresh();
                self.line(format!(
                    "%{name} = getelementptr {}, ptr {}, i32 0, i32 {index}",
                    slot.ty.llvm(),
                    slot.pointer
                ));
                Ok(Some(Slot {
                    ty,
                    pointer: format!("%{name}"),
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
        let sum = Ty::ErrorResult(Arc::new(vec![(None, Ty::Record(fields.clone()))]));
        assert_eq!(state_count(&Arc::new(vec![sum])), 2);
        assert_eq!(state_count(&Arc::new(vec![])), 1);
        let mut nested = Ty::Bool;
        for _ in 0..100 {
            nested = Ty::Record(Arc::new(vec![nested]));
        }
        assert_eq!(state_count(&Arc::new(vec![nested])), 102);
    }
}
