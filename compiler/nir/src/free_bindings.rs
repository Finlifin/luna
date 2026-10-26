//! Lexical value dependencies of a checked expression. Binding scopes here
//! follow interpretation; type-level bound slots are handled by substitution.
use crate::program::{Expr, ExprKind as K, LocalId, NormalFormArgument as A, Pattern as P};
use std::collections::{BTreeMap, BTreeSet};
type Dependencies = BTreeMap<LocalId, Scope>;
type Scope = BTreeSet<LocalId>;
fn charge(fuel: &mut usize, amount: usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(amount)
        .ok_or("capture analysis budget exhausted")?;
    Ok(())
}
fn fork(scope: &Scope, fuel: &mut usize) -> Result<Scope, String> {
    charge(fuel, scope.len())?;
    Ok(scope.clone())
}
fn read(
    id: LocalId,
    scope: &Scope,
    free: &mut Scope,
    dependencies: &Dependencies,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    charge(fuel, 1)?;
    if scope.contains(&id) {
        return Ok(());
    }
    if depth >= 128 {
        return Err("capture dependency depth exhausted".into());
    }
    if let Some(inputs) = dependencies.get(&id) {
        for input in inputs {
            read(*input, scope, free, dependencies, fuel, depth + 1)?;
        }
    } else {
        free.insert(id);
    }
    Ok(())
}
pub fn collect(expr: &Expr, fuel: &mut usize) -> Result<Scope, String> {
    collect_with_dependencies(expr, &Dependencies::new(), fuel)
}
/// Runtime dependencies of pattern observations, excluding lexical binders.
pub fn collect_pattern(pattern: &P, fuel: &mut usize) -> Result<Scope, String> {
    let mut free = Scope::new();
    pattern_scan(
        pattern,
        &mut Scope::new(),
        &mut free,
        fuel,
        0,
        &Dependencies::new(),
    )?;
    Ok(free)
}
/// As `collect`, with symbolic reads expanded in their lexical scope.
pub fn collect_with_dependencies(
    expr: &Expr,
    dependencies: &Dependencies,
    fuel: &mut usize,
) -> Result<Scope, String> {
    let mut free = Scope::new();
    scan(expr, &Scope::new(), &mut free, fuel, 0, dependencies)?;
    Ok(free)
}
/// Lexical dependencies of one handler/match arm, excluding its own pattern binders.
pub fn collect_arm(arm: &crate::program::MatchArm, fuel: &mut usize) -> Result<Scope, String> {
    collect_arm_with_dependencies(arm, &Dependencies::new(), fuel)
}
/// Expand symbolic code dependencies at each lexical use, so a guard's own
/// pattern bindings are excluded while outer runtime dependencies remain free.
pub fn collect_arm_with_dependencies(
    arm: &crate::program::MatchArm,
    dependencies: &Dependencies,
    fuel: &mut usize,
) -> Result<Scope, String> {
    let mut free = Scope::new();
    arm_scan(arm, &Scope::new(), &mut free, fuel, 0, dependencies)?;
    Ok(free)
}
fn scan(
    expr: &Expr,
    scope: &Scope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
    dependencies: &Dependencies,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("capture analysis depth exhausted".into());
    }
    crate::visit::expression_pruned(expr, fuel, |node, fuel| {
        match &node.kind {
            K::Local(id) => read(*id, scope, free, dependencies, fuel, depth)?,
            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } => {
                read(place.root, scope, free, dependencies, fuel, depth)?
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let mut inner = fork(scope, fuel)?;
                for value in statements {
                    statement(value, &mut inner, free, fuel, depth + 1, dependencies)?;
                }
                if let Some(value) = result {
                    scan(value, &inner, free, fuel, depth + 1, dependencies)?;
                }
                return Ok(false);
            }
            K::LetPattern { .. } => {
                statement(
                    node,
                    &mut fork(scope, fuel)?,
                    free,
                    fuel,
                    depth + 1,
                    dependencies,
                )?;
                return Ok(false);
            }
            K::Match { value, arms } => {
                scan(value, scope, free, fuel, depth + 1, dependencies)?;
                for arm in arms {
                    arm_scan(arm, scope, free, fuel, depth + 1, dependencies)?;
                }
                return Ok(false);
            }
            K::Closure {
                captures,
                parameters,
                body,
            } => {
                charge(fuel, captures.len().saturating_add(parameters.len()))?;
                let mut inner: Scope = parameters.iter().copied().collect();
                for capture in captures {
                    if let Some(value) = &capture.value {
                        scan(value, scope, free, fuel, depth + 1, dependencies)?;
                    } else {
                        read(capture.binding, scope, free, dependencies, fuel, depth)?;
                    }
                    inner.insert(capture.binding);
                }
                let mut missing = Scope::new();
                scan(body, &inner, &mut missing, fuel, depth + 1, dependencies)?;
                if !missing.is_empty() {
                    return Err("closure body references an undeclared capture".into());
                }
                return Ok(false);
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } => {
                scan(callee, scope, free, fuel, depth + 1, dependencies)?;
                let mut defaults = Scope::new();
                for argument in arguments {
                    if argument.is_default {
                        let mut missing = Scope::new();
                        scan(
                            &argument.value,
                            &defaults,
                            &mut missing,
                            fuel,
                            depth + 1,
                            dependencies,
                        )?;
                        if !missing.is_empty() {
                            return Err("call default references an unavailable parameter".into());
                        }
                    } else {
                        scan(&argument.value, scope, free, fuel, depth + 1, dependencies)?;
                    }
                    if let Some(binding) = parameter_bindings.get(argument.parameter) {
                        defaults.insert(*binding);
                    }
                }
                return Ok(false);
            }
            K::NominalType {
                preparations,
                arguments,
                order,
                bindings,
                ..
            }
            | K::CheckTypeAlias {
                preparations,
                arguments,
                order,
                bindings,
                ..
            } => {
                charge(fuel, arguments.len())?;
                if order
                    .as_ref()
                    .is_some_and(|order| order.len() != arguments.len())
                    || bindings.as_ref().is_some_and(|bindings| {
                        bindings.len() != arguments.len()
                            || bindings.iter().collect::<BTreeSet<_>>().len() != bindings.len()
                    })
                {
                    return Err("invalid nominal input plan".into());
                }
                if node.kind.nominal_local_bindings().is_none() {
                    return Err("invalid nominal input bindings".into());
                }
                let mut preparations = preparations.iter().peekable();
                let mut inner = fork(scope, fuel)?;
                let mut seen = BTreeSet::new();
                for position in 0..arguments.len() {
                    while preparations.peek().is_some_and(|p| p.before == position) {
                        statement(
                            &preparations.next().unwrap().expression,
                            &mut inner,
                            free,
                            fuel,
                            depth + 1,
                            dependencies,
                        )?;
                    }
                    let slot = order.as_ref().map_or(position, |order| order[position]);
                    if slot >= arguments.len() || !seen.insert(slot) {
                        return Err("invalid nominal input order".into());
                    }
                    normal(
                        &arguments[slot],
                        &inner,
                        free,
                        fuel,
                        depth + 1,
                        dependencies,
                    )?;
                    if let Some(bindings) = bindings {
                        inner.insert(bindings[slot]);
                    }
                }
                for preparation in preparations {
                    statement(
                        &preparation.expression,
                        &mut inner,
                        free,
                        fuel,
                        depth + 1,
                        dependencies,
                    )?;
                }
                if let K::CheckTypeAlias {
                    body: Some(body), ..
                } = &node.kind
                {
                    scan(body, &inner, free, fuel, depth + 1, dependencies)?;
                }
                return Ok(false);
            }
            K::NormalFormType {
                arguments, plan, ..
            } => {
                charge(fuel, arguments.len())?;
                let mut inner = fork(scope, fuel)?;
                if let Some(plan) = plan {
                    if !plan.valid(arguments.len()) {
                        return Err("invalid normal-form input plan".into());
                    }
                    let mut preparations = plan.preparations.iter().peekable();
                    for (position, slot) in plan.order.iter().enumerate() {
                        while preparations.peek().is_some_and(|p| p.before == position) {
                            let preparation = preparations.next().unwrap();
                            statement(
                                &preparation.expression,
                                &mut inner,
                                free,
                                fuel,
                                depth + 1,
                                dependencies,
                            )?;
                        }
                        normal(
                            &arguments[*slot],
                            &inner,
                            free,
                            fuel,
                            depth + 1,
                            dependencies,
                        )?;
                        inner.insert(plan.bindings[*slot]);
                    }
                    for preparation in preparations {
                        statement(
                            &preparation.expression,
                            &mut inner,
                            free,
                            fuel,
                            depth + 1,
                            dependencies,
                        )?;
                    }
                } else {
                    for argument in arguments {
                        normal(argument, scope, free, fuel, depth + 1, dependencies)?;
                    }
                }
                return Ok(false);
            }
            K::EffectHandle {
                computation,
                handlers,
                ..
            } => {
                scan(computation, scope, free, fuel, depth + 1, dependencies)?;
                for handler in handlers {
                    for arm in &handler.arms {
                        arm_scan(arm, scope, free, fuel, depth + 1, dependencies)?;
                    }
                }
                return Ok(false);
            }
            _ => {}
        }
        Ok(true)
    })
}
fn statement(
    expr: &Expr,
    scope: &mut Scope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
    dependencies: &Dependencies,
) -> Result<(), String> {
    charge(fuel, 1)?;
    match &expr.kind {
        K::Let { binding, value } => {
            scan(value, scope, free, fuel, depth + 1, dependencies)?;
            scope.insert(*binding);
        }
        K::LetPattern {
            pattern,
            value,
            guard,
            otherwise,
        } => {
            scan(value, scope, free, fuel, depth + 1, dependencies)?;
            let mut success = fork(scope, fuel)?;
            pattern_scan(pattern, &mut success, free, fuel, depth + 1, dependencies)?;
            if let Some(guard) = guard {
                scan(guard, &success, free, fuel, depth + 1, dependencies)?;
            }
            if let Some(otherwise) = otherwise {
                scan(otherwise, scope, free, fuel, depth + 1, dependencies)?;
            }
            *scope = success;
        }
        _ => scan(expr, scope, free, fuel, depth + 1, dependencies)?,
    }
    Ok(())
}
fn arm_scan(
    arm: &crate::program::MatchArm,
    scope: &Scope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
    dependencies: &Dependencies,
) -> Result<(), String> {
    let mut inner = fork(scope, fuel)?;
    pattern_scan(
        &arm.pattern,
        &mut inner,
        free,
        fuel,
        depth + 1,
        dependencies,
    )?;
    if let Some(guard) = &arm.guard {
        scan(guard, &inner, free, fuel, depth + 1, dependencies)?;
    }
    scan(&arm.body, &inner, free, fuel, depth + 1, dependencies)
}
fn pattern_scan(
    pattern: &P,
    scope: &mut Scope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
    dependencies: &Dependencies,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("capture pattern depth exhausted".into());
    }
    charge(fuel, 1)?;
    match pattern {
        P::CodeEvidence { outputs } => {
            charge(fuel, outputs.len())?;
            scope.extend(outputs.iter().map(|(id, _)| *id));
        }

        P::Observe { binding, inner } => {
            let inserted = scope.insert(*binding);
            pattern_scan(inner, scope, free, fuel, depth + 1, dependencies)?;
            if inserted {
                scope.remove(binding);
            }
        }

        P::Binding { binding, inner } => {
            pattern_scan(inner, scope, free, fuel, depth + 1, dependencies)?;
            scope.insert(*binding);
        }
        P::Then { first, value, next } => {
            pattern_scan(first, scope, free, fuel, depth + 1, dependencies)?;
            scan(value, scope, free, fuel, depth + 1, dependencies)?;
            pattern_scan(next, scope, free, fuel, depth + 1, dependencies)?;
        }
        P::Guard { pattern, condition } => {
            pattern_scan(pattern, scope, free, fuel, depth + 1, dependencies)?;
            scan(condition, scope, free, fuel, depth + 1, dependencies)?;
        }
        P::Remap { pattern, bindings } => {
            pattern_scan(pattern, scope, free, fuel, depth + 1, dependencies)?;
            charge(fuel, bindings.len())?;
            for (from, _) in bindings {
                read(*from, scope, free, dependencies, fuel, depth)?;
            }
            let targets: Scope = bindings.iter().map(|(_, to)| *to).collect();
            for (from, _) in bindings {
                if !targets.contains(from) {
                    scope.remove(from);
                }
            }
            scope.extend(targets);
        }
        P::Or(patterns) => {
            let mut common = None;
            for pattern in patterns {
                let mut branch = fork(scope, fuel)?;
                pattern_scan(pattern, &mut branch, free, fuel, depth + 1, dependencies)?;
                common = Some(match common {
                    None => branch,
                    Some(previous) => (&previous as &Scope)
                        .intersection(&branch)
                        .copied()
                        .collect(),
                });
            }
            if let Some(common) = common {
                *scope = common;
            }
        }
        P::OptionalPresent(inner) | P::ErrorResult { inner, .. } => {
            pattern_scan(inner, scope, free, fuel, depth + 1, dependencies)?
        }
        P::NominalType { fields, .. }
        | P::Record { fields, .. }
        | P::AnonymousRecord { fields, .. }
        | P::Variant { fields, .. }
        | P::NormalForm {
            arguments: fields, ..
        }
        | P::Tuple(fields)
        | P::TypeTuple(fields) => {
            for field in fields {
                pattern_scan(field, scope, free, fuel, depth + 1, dependencies)?;
            }
        }
        P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => {}
    }
    Ok(())
}
fn normal(
    argument: &A,
    scope: &Scope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
    dependencies: &Dependencies,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("capture normal-input depth exhausted".into());
    }
    charge(fuel, 1)?;
    match argument {
        A::Value(value) | A::TypeListSpread(value) => {
            scan(value, scope, free, fuel, depth + 1, dependencies)?
        }
        A::TypeList { items, .. } | A::Tuple { items, .. } => {
            for item in items {
                normal(item, scope, free, fuel, depth + 1, dependencies)?;
            }
        }
        A::Block {
            statements, result, ..
        } => {
            let mut inner = fork(scope, fuel)?;
            for value in statements {
                statement(value, &mut inner, free, fuel, depth + 1, dependencies)?;
            }
            if let Some(value) = result {
                normal(value, &inner, free, fuel, depth + 1, dependencies)?;
            }
        }
        A::If {
            condition,
            then_input,
            else_input,
            ..
        } => {
            scan(condition, scope, free, fuel, depth + 1, dependencies)?;
            normal(then_input, scope, free, fuel, depth + 1, dependencies)?;
            normal(else_input, scope, free, fuel, depth + 1, dependencies)?;
        }
        A::Match { value, arms, .. } => {
            scan(value, scope, free, fuel, depth + 1, dependencies)?;
            for arm in arms {
                let mut inner = fork(scope, fuel)?;
                pattern_scan(
                    &arm.pattern,
                    &mut inner,
                    free,
                    fuel,
                    depth + 1,
                    dependencies,
                )?;
                if let Some(guard) = &arm.guard {
                    scan(guard, &inner, free, fuel, depth + 1, dependencies)?;
                }
                normal(&arm.body, &inner, free, fuel, depth + 1, dependencies)?;
            }
        }
        A::Integer { .. } | A::Unit { .. } => {}
    }
    Ok(())
}
