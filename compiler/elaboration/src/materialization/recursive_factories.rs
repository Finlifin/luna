//! Finite factory target analysis. Recursive execution stays in runtime helper
//! functions returning shared tags; only the callable choice is defunctionalized.
use super::*;

#[derive(Clone)]
pub(super) struct Selection {
    helper: Id,
    targets: std::sync::Arc<Vec<n::Expr>>,
}

fn key(callee: &n::Expr) -> Option<iir::InstanceKey> {
    match &callee.kind {
        K::Function(declaration) => Some(iir::InstanceKey {
            declaration: *declaration,
            arguments: vec![],
        }),
        K::FunctionInstance {
            declaration,
            arguments,
        } => Some(iir::InstanceKey {
            declaration: *declaration,
            arguments: arguments.clone(),
        }),
        _ => None,
    }
}
fn work(fuel: &mut usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or("recursive factory target budget exhausted")?;
    Ok(())
}
// Inspect completing result alternatives; statements/conditions remain intact.
fn alternatives(
    value: &n::Expr,
    targets: &mut Vec<n::Expr>,
    locals: &std::collections::BTreeSet<n::LocalId>,
    fuel: &mut usize,
) -> Result<bool, String> {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        work(fuel)?;
        if value.ty == Term::Primitive(nir::Primitive::Never) {
            continue;
        }
        match &value.kind {
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => {
                let mut exists = false;
                for target in targets.iter() {
                    template_factories::charge(target, fuel)?;
                    if target.kind == value.kind && target.ty == value.ty {
                        exists = true;
                        break;
                    }
                }
                if !exists {
                    template_factories::charge(value, fuel)?;
                    targets.push(value.clone());
                }
            }
            K::Call { callee, .. } if key(callee).is_some() => {}
            K::Local(binding) if locals.contains(binding) => {}
            K::Block {
                result: Some(result),
                ..
            }
            | K::CallPreparation {
                result: Some(result),
                ..
            } => pending.push(result),
            K::If {
                then_branch,
                else_branch: Some(other),
                ..
            } => {
                pending.push(other);
                pending.push(then_branch);
            }
            K::Match { arms, .. } if !arms.is_empty() => {
                pending.extend(arms.iter().rev().map(|arm| &arm.body))
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}
struct Node {
    key: iir::InstanceKey,
    function: n::Function,
    edges: Vec<iir::InstanceKey>,
    locals: std::collections::BTreeSet<n::LocalId>,
}

fn charge_function(function: &n::Function, fuel: &mut usize) -> Result<(), String> {
    iir::visit::expressions(function, fuel, |expr, fuel| {
        nir::type_computations::charge_term(&expr.ty, fuel).map_err(|error| error.to_string())
    })?;
    nir::type_computations::charge_term(&function.signature, fuel)
        .map_err(|error| error.to_string())
}

impl Builder<'_> {
    pub(super) fn lower_recursive_factory_call(
        &mut self,
        expression: &mut n::Expr,
    ) -> Result<bool, String> {
        let K::Call { callee, .. } = &expression.kind else {
            return Ok(false);
        };
        if !Self::returns_staged_callable(&callee.ty) {
            return Ok(false);
        }
        template_factories::charge(callee, &mut self.fuel)?;
        let Some(root) = key(callee) else {
            return Ok(false);
        };
        if !self.recursive_factories.contains_key(&root) {
            if !self.prepare_recursive_factories(&root, &callee.ty)? {
                return Ok(false);
            }
        }
        let selection = self.recursive_factories[&root].clone();
        template_factories::charge(expression, &mut self.fuel)?;
        let result = expression.ty.clone();
        let mut candidate = expression.clone();
        let K::Call {
            callee,
            parameter_bindings,
            ..
        } = &mut candidate.kind
        else {
            unreachable!()
        };
        let helper = &self.generated_functions[&selection.helper];
        callee.kind = K::Function(selection.helper);
        callee.ty = helper.signature.clone();
        *parameter_bindings = helper.parameters.clone();
        candidate.ty = Term::Primitive(nir::Primitive::U64);
        // The helper call and the dispatch are separate runtime boundaries.
        // Retain the original location through the expansion source table.
        let source = u32::try_from(self.quote_sources.len())
            .map_err(|_| "recursive factory source space exhausted")?;
        self.quote_sources.push(n::ExpansionSource {
            original: candidate.source,
            site: candidate.source,
        });
        candidate.source = n::SourceRange::generated(source);
        *expression =
            template_factories::dispatch(candidate, &result, &selection.targets, &mut self.fuel)?;
        expression.source = self.quote_sources[source as usize].original;
        Ok(true)
    }

    fn prepare_recursive_factories(
        &mut self,
        root: &iir::InstanceKey,
        signature: &Term,
    ) -> Result<bool, String> {
        let Term::Function { result, .. } = signature.callable_signature() else {
            return Ok(false);
        };
        let result = result.as_ref();
        let mut pending = VecDeque::from([root.clone()]);
        let mut nodes = Vec::<Node>::new();
        let mut indices = HashMap::new();
        let mut targets = Vec::new();
        while let Some(key) = pending.pop_front() {
            work(&mut self.fuel)?;
            if indices.contains_key(&key) {
                continue;
            }
            let Some(template) = self
                .package
                .functions
                .iter()
                .find(|f| f.declaration == key.declaration)
            else {
                return Ok(false);
            };
            if template.phase != n::FunctionPhase::Runtime {
                return Ok(false);
            }
            charge_function(template, &mut self.fuel)?;
            let mut function = template
                .instantiate_parameters(
                    &self.input_binders(&template.type_parameters),
                    &key.arguments,
                )
                .map_err(|e| e.to_string())?;
            let Term::Function {
                parameters,
                result: output,
            } = function.signature.callable_signature()
            else {
                return Ok(false);
            };
            if output.as_ref() != result
                || parameters.iter().any(|p| {
                    p.sort != nir::Sort::Runtime
                        || matches!(p.ty.callable_signature(),Term::Function {parameters,..}
                    if parameters.iter().any(|p|p.sort!=nir::Sort::Runtime))
                })
            {
                return Ok(false);
            }
            function.type_parameters.clear();
            dead_tails::prune(&mut function.body, &mut self.fuel)?;
            mutable_callbacks::lower(&mut function.body, &mut self.quote_cursor, &mut self.fuel)?;
            staged::forward_template_aliases(
                &mut function.body,
                &mut self.quote_cursor,
                &mut self.quote_sources,
                &mut self.fuel,
            )?;
            let mut locals = std::collections::BTreeSet::new();
            let mut blocked = std::collections::BTreeSet::new();
            iir::visit::expression_in_closure_environment(
                &function.body,
                &mut self.fuel,
                |expr, fuel| {
                    if let K::Let { binding, value } = &expr.kind {
                        if &value.ty == result && !locals.insert(*binding) {
                            blocked.insert(*binding);
                        }
                    }
                    if matches!(expr.kind, K::AddressOf(_) | K::Closure { .. }) {
                        blocked.extend(nir::free_bindings::collect(expr, fuel)?);
                    }
                    Ok(())
                },
            )?;
            if locals.iter().any(|binding| blocked.contains(binding)) {
                return Ok(false);
            }
            let mut supported =
                alternatives(&function.body, &mut targets, &locals, &mut self.fuel)?;
            let mut edges = Vec::new();
            iir::visit::expression_in_closure_environment(
                &function.body,
                &mut self.fuel,
                |expr, fuel| {
                    if let K::Return(Some(value)) = &expr.kind {
                        supported &= alternatives(value, &mut targets, &locals, fuel)?;
                    }
                    if let K::Let { binding, value } = &expr.kind {
                        if locals.contains(binding) {
                            supported &= alternatives(value, &mut targets, &locals, fuel)?;
                        }
                    }
                    if let K::Assign { place, value } = &expr.kind {
                        if locals.contains(&place.root) {
                            supported &= place.fields.is_empty()
                                && alternatives(value, &mut targets, &locals, fuel)?;
                        } else if &value.ty == result {
                            // An external/projected slot has a different representation.
                            supported = false;
                        }
                    }
                    if let K::Call { callee, .. } = &expr.kind {
                        if matches!(callee.ty.callable_signature(),Term::Function {result:output,..} if output.as_ref()==result)
                        {
                            template_factories::charge(callee, fuel)?;
                            if let Some(dependency) = self::key(callee) {
                                if !edges.contains(&dependency) {
                                    edges.push(dependency);
                                }
                            } else {
                                supported = false;
                            }
                        }
                    }
                    Ok(())
                },
            )?;
            if !supported {
                return Ok(false);
            }
            pending.extend(edges.iter().cloned());
            indices.insert(key.clone(), nodes.len());
            nodes.push(Node {
                key,
                function,
                edges,
                locals,
            });
        }
        // Acyclic producers use the established expansion path. Kahn's walk
        // distinguishes a shared dependency from a true recursion cycle.
        let mut outgoing = Vec::with_capacity(nodes.len());
        let mut callers = vec![Vec::new(); nodes.len()];
        let mut leaves = VecDeque::new();
        for (index, node) in nodes.iter().enumerate() {
            work(&mut self.fuel)?;
            outgoing.push(node.edges.len());
            if node.edges.is_empty() {
                leaves.push_back(index);
            }
            for edge in &node.edges {
                work(&mut self.fuel)?;
                let target = *indices
                    .get(edge)
                    .ok_or("recursive factory dependency missing")?;
                callers[target].push(index);
            }
        }
        let mut removed = 0;
        while let Some(index) = leaves.pop_front() {
            work(&mut self.fuel)?;
            removed += 1;
            for caller in &callers[index] {
                work(&mut self.fuel)?;
                outgoing[*caller] -= 1;
                if outgoing[*caller] == 0 {
                    leaves.push_back(*caller);
                }
            }
        }
        if removed == nodes.len() || targets.is_empty() {
            return Ok(false);
        }
        let targets = std::sync::Arc::new(targets);
        let mut identities = Vec::new();
        for _ in &nodes {
            loop {
                work(&mut self.fuel)?;
                let id = Id {
                    package: u64::MAX - 6,
                    local: self.quote_cursor,
                };
                self.quote_cursor = self
                    .quote_cursor
                    .checked_add(1)
                    .ok_or("recursive factory identity space exhausted")?;
                if !self.package.functions.iter().any(|f| f.declaration == id)
                    && !self.generated_functions.contains_key(&id)
                {
                    identities.push(id);
                    break;
                }
            }
        }
        let tag_type = Term::Primitive(nir::Primitive::U64);
        let mut generated = Vec::new();
        for (index, node) in nodes.iter().enumerate() {
            charge_function(&node.function, &mut self.fuel)?;
            let mut function = node.function.clone();
            iir::visit::expression_mut_pruned(&mut function.body, &mut self.fuel, |expr, fuel| {
                if matches!(expr.kind, K::Closure { .. }) {
                    return Ok(false);
                }
                if let K::Let { binding, value } = &mut expr.kind {
                    if node.locals.contains(binding) {
                        encode(value, &nodes, &identities, &targets, fuel, 0)?;
                    }
                }
                if let K::Assign { place, value } = &mut expr.kind {
                    if node.locals.contains(&place.root) {
                        if !place.fields.is_empty() {
                            return Err(
                                "recursive factory projected assignment escaped validation".into(),
                            );
                        }
                        encode(value, &nodes, &identities, &targets, fuel, 0)?;
                    }
                }
                Ok(true)
            })?;
            rewrite_returns(
                &mut function.body,
                &nodes,
                &identities,
                &targets,
                &mut self.fuel,
                0,
            )?;
            encode(
                &mut function.body,
                &nodes,
                &identities,
                &targets,
                &mut self.fuel,
                0,
            )?;
            // Result/initializer positions now contain tags. Remaining callable
            // reads reconstruct dispatch at the use site, without reevaluating
            // the recursive factory that produced the stored choice.
            iir::visit::expression_mut_pruned(&mut function.body, &mut self.fuel, |expr, fuel| {
                if matches!(expr.kind, K::Closure { .. }) {
                    return Ok(false);
                }
                if let K::Local(binding) = expr.kind {
                    if node.locals.contains(&binding) && &expr.ty == result {
                        let mut tag = expr.clone();
                        tag.ty = tag_type.clone();
                        *expr = template_factories::dispatch(tag, result, &targets, fuel)?;
                        return Ok(false);
                    }
                }
                Ok(true)
            })?;
            function.signature = function
                .signature
                .with_callable_result(tag_type.clone())
                .unwrap();
            if let Some(binding) = function.result_binding {
                for predicate in &mut function.postconditions {
                    let mut valid = true;
                    iir::visit::expression_pruned(predicate, &mut self.fuel, |expr, fuel| {
                        if matches!(expr.kind, K::Closure { .. } | K::AddressOf(_)) {
                            valid &= !nir::free_bindings::collect(expr, fuel)?.contains(&binding);
                            return Ok(false);
                        }
                        Ok(true)
                    })?;
                    if !valid {
                        return Ok(false);
                    }
                    iir::visit::expression_mut_pruned(predicate, &mut self.fuel, |expr, fuel| {
                        if matches!(expr.kind,K::Local(id) if id==binding) {
                            *expr = template_factories::dispatch(
                                n::Expr {
                                    source: expr.source,
                                    ty: tag_type.clone(),
                                    kind: K::Local(binding),
                                },
                                result,
                                &targets,
                                fuel,
                            )?;
                            return Ok(false);
                        }
                        Ok(true)
                    })?;
                }
            }
            function.declaration = identities[index];
            function.name = format!("{}::template-target", function.name);
            function.inline = false;
            generated.push(function);
        }
        for (node, function) in nodes.into_iter().zip(generated) {
            let helper = function.declaration;
            self.generated_functions.insert(helper, function);
            self.recursive_factories.insert(
                node.key,
                Selection {
                    helper,
                    targets: targets.clone(),
                },
            );
        }
        Ok(true)
    }
}

fn encode(
    value: &mut n::Expr,
    nodes: &[Node],
    identities: &[Id],
    targets: &[n::Expr],
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    work(fuel)?;
    if depth >= 128 {
        return Err("recursive factory result depth exhausted".into());
    }
    if value.ty == Term::Primitive(nir::Primitive::Never) {
        return Ok(());
    }
    match &mut value.kind {
        K::Local(binding) if nodes.iter().any(|node| node.locals.contains(binding)) => {}
        K::Call {
            callee,
            parameter_bindings,
            ..
        } => {
            let key = key(callee).ok_or("recursive factory callee is unresolved")?;
            let index = nodes
                .iter()
                .position(|node| node.key == key)
                .ok_or("recursive factory callee was not analyzed")?;
            callee.kind = K::Function(identities[index]);
            callee.ty = nodes[index]
                .function
                .signature
                .clone()
                .with_callable_result(Term::Primitive(nir::Primitive::U64))
                .unwrap();
            *parameter_bindings = nodes[index].function.parameters.clone();
        }
        K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => {
            let mut selected = None;
            for (index, target) in targets.iter().enumerate() {
                template_factories::charge(target, fuel)?;
                if value.kind == target.kind && value.ty == target.ty {
                    selected = Some(index);
                    break;
                }
            }
            value.kind = K::Literal(n::Literal::UnsignedInteger(
                selected.ok_or("recursive factory target missing")? as u128,
            ));
        }
        K::Block {
            result: Some(result),
            ..
        }
        | K::CallPreparation {
            result: Some(result),
            ..
        } => encode(result, nodes, identities, targets, fuel, depth + 1)?,
        K::If {
            then_branch,
            else_branch: Some(other),
            ..
        } => {
            encode(then_branch, nodes, identities, targets, fuel, depth + 1)?;
            encode(other, nodes, identities, targets, fuel, depth + 1)?;
        }
        K::Match { arms, .. } => {
            for arm in arms {
                encode(&mut arm.body, nodes, identities, targets, fuel, depth + 1)?;
            }
        }
        _ => return Err("recursive factory result escaped validation".into()),
    }
    value.ty = Term::Primitive(nir::Primitive::U64);
    Ok(())
}
fn rewrite_returns(
    body: &mut n::Expr,
    nodes: &[Node],
    identities: &[Id],
    targets: &[n::Expr],
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("recursive factory capture depth exhausted".into());
    }
    iir::visit::expression_mut_pruned(body, fuel, |expr, fuel| {
        if let K::Closure { captures, .. } = &mut expr.kind {
            for capture in captures {
                if let Some(value) = &mut capture.value {
                    rewrite_returns(value, nodes, identities, targets, fuel, depth + 1)?;
                }
            }
            return Ok(false);
        }
        if let K::Return(Some(value)) = &mut expr.kind {
            encode(value, nodes, identities, targets, fuel, 0)?;
        }
        Ok(true)
    })
}
