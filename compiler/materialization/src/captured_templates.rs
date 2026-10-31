//! Propagate known identities to calls without erasing the captured value or
//! changing the logical closure environment. Unknown selections stay unresolved.
use super::*;
use std::collections::BTreeMap;
mod pointers;

fn staged(ty: &Term) -> bool {
    !ty.is_comptime_callable()
        && matches!(ty.callable_signature(), Term::Function { parameters, .. }
        if parameters.iter().any(|p| p.sort != nir::Sort::Runtime))
}

fn charge(value: &n::Expr, fuel: &mut usize) -> Result<(), String> {
    let mut remaining = *fuel as u64;
    let result = nir::copy_budget::charge_expression(value, &mut remaining);
    *fuel = remaining as usize;
    result.map_err(|_| "captured template copy budget exhausted".into())
}

fn copied(value: &n::Expr, fuel: &mut usize) -> Result<n::Expr, String> {
    charge(value, fuel)?;
    Ok(value.clone())
}

pub(super) fn storage(
    functions: &[iir::FunctionInstance],
    package: &n::Package,
    dispatch: &[n::DispatchInstance],
    type_cores: &mut nir::TypeInterner,
    fuel: &mut usize,
) -> Result<Vec<iir::template_storage::TemplateStorage>, String> {
    let mut tables: Vec<iir::template_storage::TemplateStorage> = Vec::new();
    for function in functions {
        iir::visit::expressions(&function.function, fuel, |expr, fuel| {
            if !staged(&expr.ty) {
                return Ok(());
            }
            let key = match &expr.kind {
                K::Function(declaration) => iir::InstanceKey {
                    declaration: *declaration,
                    arguments: vec![].into(),
                },
                K::FunctionInstance {
                    declaration,
                    arguments,
                } => {
                    nir::type_computations::charge_term(&expr.ty, fuel)
                        .map_err(|e| e.to_string())?;
                    for argument in arguments {
                        nir::type_computations::charge_term(argument, fuel)
                            .map_err(|e| e.to_string())?;
                    }
                    iir::InstanceKey {
                        declaration: *declaration,
                        arguments: arguments.clone().into(),
                    }
                }
                K::TraitMethod {
                    interface,
                    interface_arguments,
                    subject,
                    member,
                    method_arguments,
                } => {
                    *fuel = fuel
                        .checked_sub(dispatch.len().saturating_add(1))
                        .ok_or("template method selection budget exhausted")?;
                    let selected = dispatch
                        .iter()
                        .find(|selected| {
                            selected.interface == *interface
                                && selected.interface_arguments == *interface_arguments
                                && selected.subject == *subject
                        })
                        .ok_or("template method has no selected implementation")?;
                    *fuel = fuel
                        .checked_sub(selected.methods.len().saturating_add(1))
                        .ok_or("template method selection budget exhausted")?;
                    let method = selected
                        .methods
                        .iter()
                        .find(|method| {
                            method.member == *member && method.method_arguments == *method_arguments
                        })
                        .ok_or("template method has no selected body")?;
                    for argument in &method.arguments {
                        nir::type_computations::charge_term(argument, fuel)
                            .map_err(|e| e.to_string())?;
                    }
                    iir::InstanceKey {
                        declaration: method.body,
                        arguments: method.arguments.clone().into(),
                    }
                }
                _ => return Ok(()),
            };
            for table in &tables {
                *fuel = fuel
                    .checked_sub(table.targets().len().saturating_add(1))
                    .ok_or("captured template lookup budget exhausted")?;
                for target in table.targets() {
                    for argument in target.arguments.iter() {
                        nir::type_computations::charge_term(argument, fuel)
                            .map_err(|e| e.to_string())?;
                    }
                }
                if table.targets().contains(&key) {
                    if table.logical_type() != &expr.ty {
                        return Err("captured template has inconsistent logical types".into());
                    }
                    return Ok(());
                }
            }
            tables.push(iir::template_storage::TemplateStorage::from_package_in(
                expr.ty.clone(),
                vec![key],
                package,
                type_cores,
                fuel,
            )?);
            Ok(())
        })?;
    }
    Ok(tables)
}

pub(super) fn forward(
    expression: &mut n::Expr,
    next: &mut u32,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut unavailable = BTreeSet::new();
    let mut has_capture = false;
    // Escaped addresses need alias analysis before a finite domain is sound.
    iir::visit::expression_pruned(expression, fuel, |expr, fuel| {
        if let K::Closure { captures, .. } = &expr.kind {
            has_capture |= captures.iter().any(|capture| staged(&capture.ty));
        }
        if let K::Call { callee, .. } = &expr.kind {
            has_capture |= staged(&callee.ty) && matches!(callee.kind, K::ReadPointer(_));
        }
        if let K::Assign { place, .. } = &expr.kind {
            if !place.fields.is_empty() {
                unavailable.insert(place.root);
            }
        }
        Ok(true)
    })?;
    if !has_capture {
        return Ok(());
    }
    let pointers = pointers::analyze(expression, &mut unavailable, fuel)?;
    scope(
        expression,
        &BTreeMap::new(),
        &unavailable,
        &pointers,
        next,
        fuel,
        0,
    )
}

type Targets = BTreeMap<n::LocalId, Vec<n::Expr>>;

fn copy_targets(values: &[n::Expr], fuel: &mut usize) -> Result<Vec<n::Expr>, String> {
    values.iter().map(|value| copied(value, fuel)).collect()
}

// Discover a finite result domain without replaying its selection or effects.
fn targets(
    value: &n::Expr,
    known: &Targets,
    pointers: &pointers::Aliases,
    fuel: &mut usize,
) -> Result<Option<Vec<n::Expr>>, String> {
    let mut pending = vec![value];
    let mut result: Vec<n::Expr> = Vec::new();
    while let Some(value) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("captured template domain budget exhausted")?;
        if value.ty == Term::Primitive(nir::Primitive::Never) {
            continue;
        }
        match &value.kind {
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => {
                let mut duplicate = false;
                for existing in &result {
                    // Charge both identity payloads before comparing them.
                    charge(existing, fuel)?;
                    charge(value, fuel)?;
                    duplicate |= existing.kind == value.kind && existing.ty == value.ty;
                }
                if !duplicate {
                    result.push(copied(value, fuel)?);
                }
            }
            K::Local(binding) => {
                let Some(values) = known.get(binding) else {
                    return Ok(None);
                };
                pending.extend(values);
            }
            K::ReadPointer(pointer) => {
                let Some(binding) = pointers::target(pointer, pointers) else {
                    return Ok(None);
                };
                let Some(values) = known.get(&binding) else {
                    return Ok(None);
                };
                pending.extend(values);
            }
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
            } => pending.extend([other.as_ref(), then_branch.as_ref()]),
            K::Match { arms, .. } if !arms.is_empty() => {
                pending.extend(arms.iter().map(|arm| &arm.body))
            }
            _ => return Ok(None),
        }
    }
    Ok((!result.is_empty()).then_some(result))
}

fn dispatch(callee: &n::Expr, targets: &[n::Expr], fuel: &mut usize) -> Result<n::Expr, String> {
    let Some(last) = targets.last() else {
        return Err("empty captured template domain".into());
    };
    let mut result = copied(last, fuel)?;
    if targets.len() == 1 {
        // Knowing the identity does not authorize using a moved capture. Keep
        // an observational read even when no runtime choice remains.
        let observation = n::Expr {
            source: callee.source,
            ty: Term::Primitive(nir::Primitive::Bool),
            kind: K::TemplateEqual(
                Box::new(copied(callee, fuel)?),
                Box::new(copied(last, fuel)?),
            ),
        };
        return Ok(n::Expr {
            source: callee.source,
            ty: callee.ty.clone(),
            kind: K::Block {
                statements: vec![n::Expr {
                    source: callee.source,
                    ty: Term::Primitive(nir::Primitive::Void),
                    kind: K::Discard(Box::new(observation)),
                }],
                result: Some(Box::new(result)),
            },
        });
    }
    for target in targets[..targets.len() - 1].iter().rev() {
        let condition = n::Expr {
            source: callee.source,
            ty: Term::Primitive(nir::Primitive::Bool),
            kind: K::TemplateEqual(
                Box::new(copied(callee, fuel)?),
                Box::new(copied(target, fuel)?),
            ),
        };
        result = n::Expr {
            source: callee.source,
            ty: callee.ty.clone(),
            kind: K::If {
                condition: Box::new(condition),
                then_branch: Box::new(copied(target, fuel)?),
                else_branch: Some(Box::new(result)),
            },
        };
    }
    result.source = callee.source;
    Ok(result)
}

fn scope(
    expression: &mut n::Expr,
    inherited: &Targets,
    unavailable: &BTreeSet<n::LocalId>,
    pointers: &pointers::Aliases,
    next: &mut u32,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("captured template scope depth exhausted".into());
    }
    let mut known = BTreeMap::new();
    for (binding, values) in inherited {
        known.insert(*binding, copy_targets(values, fuel)?);
    }
    let mut declarations = Vec::new();
    iir::visit::expression_pruned(expression, fuel, |expr, fuel| {
        if matches!(expr.kind, K::Closure { .. }) {
            return Ok(false);
        }
        let source = match &expr.kind {
            K::Let { binding, value } => Some((*binding, value)),
            K::Assign { place, value } if place.fields.is_empty() => Some((place.root, value)),
            K::WritePointer { pointer, value } => {
                pointers::target(pointer, pointers).map(|binding| (binding, value))
            }
            _ => None,
        };
        if let Some((binding, value)) = source {
            if staged(&value.ty) && !unavailable.contains(&binding) {
                declarations.push((binding, copied(value, fuel)?));
            }
        }
        Ok(true)
    })?;
    loop {
        let mut changed = false;
        for (binding, value) in &declarations {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("captured template alias budget exhausted")?;
            if let Some(values) = targets(value, &known, pointers, fuel)? {
                let domain = known.entry(*binding).or_default();
                for value in values {
                    let mut found = false;
                    for existing in domain.iter() {
                        charge(existing, fuel)?;
                        charge(&value, fuel)?;
                        found |= existing.kind == value.kind && existing.ty == value.ty;
                    }
                    if !found {
                        domain.push(value);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    // Growing domains discovers cyclic aliases with concrete seeds. A producer
    // we cannot resolve poisons its whole cell and every dependent alias; never
    // keep only the subset of assignments that happened to be recognizable.
    loop {
        let mut removed = false;
        for (binding, value) in &declarations {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("captured template domain validation budget exhausted")?;
            if known.contains_key(binding) && targets(value, &known, pointers, fuel)?.is_none() {
                known.remove(binding);
                removed = true;
            }
        }
        if !removed {
            break;
        }
    }
    iir::visit::expression_mut_pruned(expression, fuel, |expr, fuel| {
        if let K::Closure { captures, body, .. } = &mut expr.kind {
            let mut captured = BTreeMap::new();
            for capture in captures {
                if unavailable.contains(&capture.binding) {
                    continue;
                }
                let values = match capture.value.as_deref() {
                    None => known
                        .get(&capture.binding)
                        .map(|values| copy_targets(values, fuel))
                        .transpose()?,
                    Some(value) if staged(&value.ty) => targets(value, &known, pointers, fuel)?,
                    _ => None,
                };
                if let Some(values) = values {
                    captured.insert(capture.binding, values);
                }
            }
            if std::sync::Arc::strong_count(body) > 1 {
                *body = std::sync::Arc::new(copied(body, fuel)?);
            }
            scope(
                std::sync::Arc::make_mut(body),
                &captured,
                unavailable,
                pointers,
                next,
                fuel,
                depth + 1,
            )?;
            return Ok(false);
        }
        if let K::Call { callee, .. } = &mut expr.kind {
            if let K::Local(binding) = callee.kind {
                if let Some(values) = known.get(&binding) {
                    **callee = dispatch(callee, values, fuel)?;
                }
            } else if matches!(callee.kind, K::ReadPointer(_)) && staged(&callee.ty) {
                if let Some(values) = targets(callee, &known, pointers, fuel)? {
                    let binding = n::LocalId {
                        owner: u32::MAX,
                        index: *next,
                    };
                    *next = next
                        .checked_add(1)
                        .ok_or("template callee identity space exhausted")?;
                    let value = copied(callee, fuel)?;
                    let local = n::Expr {
                        source: callee.source,
                        ty: callee.ty.clone(),
                        kind: K::Local(binding),
                    };
                    let selected = dispatch(&local, &values, fuel)?;
                    callee.kind = K::Block {
                        statements: vec![n::Expr {
                            source: callee.source,
                            ty: Term::Primitive(nir::Primitive::Void),
                            kind: K::Let {
                                binding,
                                value: Box::new(value),
                            },
                        }],
                        result: Some(Box::new(selected)),
                    };
                }
            }
        }
        Ok(true)
    })
}
