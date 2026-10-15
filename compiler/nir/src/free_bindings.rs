//! Lexical value dependencies of a checked expression. Binding scopes here
//! follow interpretation; type-level bound slots are handled by substitution.
use crate::program::{Expr, ExprKind as K, LocalId, NormalFormArgument as A, Pattern as P};
use std::collections::BTreeSet;
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
fn read(id: LocalId, scope: &Scope, free: &mut Scope) {
    if !scope.contains(&id) {
        free.insert(id);
    }
}
pub fn collect(expr: &Expr, fuel: &mut usize) -> Result<Scope, String> {
    let mut free = Scope::new();
    scan(expr, &Scope::new(), &mut free, fuel, 0)?;
    Ok(free)
}
/// Lexical dependencies of one handler/match arm, excluding its own pattern binders.
pub fn collect_arm(arm: &crate::program::MatchArm, fuel: &mut usize) -> Result<Scope, String> {
    let mut free = Scope::new();
    arm_scan(arm, &Scope::new(), &mut free, fuel, 0)?;
    Ok(free)
}
fn scan(
    expr: &Expr,
    scope: &Scope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("capture analysis depth exhausted".into());
    }
    crate::visit::expression_pruned(expr, fuel, |node, fuel| {
        match &node.kind {
            K::Local(id) => read(*id, scope, free),
            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } => {
                read(place.root, scope, free)
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let mut inner = fork(scope, fuel)?;
                for value in statements {
                    statement(value, &mut inner, free, fuel, depth + 1)?;
                }
                if let Some(value) = result {
                    scan(value, &inner, free, fuel, depth + 1)?;
                }
                return Ok(false);
            }
            K::LetPattern { .. } => {
                statement(node, &mut fork(scope, fuel)?, free, fuel, depth + 1)?;
                return Ok(false);
            }
            K::Match { value, arms } => {
                scan(value, scope, free, fuel, depth + 1)?;
                for arm in arms {
                    arm_scan(arm, scope, free, fuel, depth + 1)?;
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
                        scan(value, scope, free, fuel, depth + 1)?;
                    } else {
                        read(capture.binding, scope, free);
                    }
                    inner.insert(capture.binding);
                }
                let mut missing = Scope::new();
                scan(body, &inner, &mut missing, fuel, depth + 1)?;
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
                scan(callee, scope, free, fuel, depth + 1)?;
                let mut defaults = Scope::new();
                for argument in arguments {
                    if argument.is_default {
                        let mut missing = Scope::new();
                        scan(&argument.value, &defaults, &mut missing, fuel, depth + 1)?;
                        if !missing.is_empty() {
                            return Err("call default references an unavailable parameter".into());
                        }
                    } else {
                        scan(&argument.value, scope, free, fuel, depth + 1)?;
                    }
                    if let Some(binding) = parameter_bindings.get(argument.parameter) {
                        defaults.insert(*binding);
                    }
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
                            statement(&preparation.expression, &mut inner, free, fuel, depth + 1)?;
                        }
                        normal(&arguments[*slot], &inner, free, fuel, depth + 1)?;
                        inner.insert(plan.bindings[*slot]);
                    }
                    for preparation in preparations {
                        statement(&preparation.expression, &mut inner, free, fuel, depth + 1)?;
                    }
                } else {
                    for argument in arguments {
                        normal(argument, scope, free, fuel, depth + 1)?;
                    }
                }
                return Ok(false);
            }
            K::EffectHandle {
                computation,
                handlers,
                ..
            } => {
                scan(computation, scope, free, fuel, depth + 1)?;
                for handler in handlers {
                    for arm in &handler.arms {
                        arm_scan(arm, scope, free, fuel, depth + 1)?;
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
) -> Result<(), String> {
    charge(fuel, 1)?;
    match &expr.kind {
        K::Let { binding, value } => {
            scan(value, scope, free, fuel, depth + 1)?;
            scope.insert(*binding);
        }
        K::LetPattern {
            pattern,
            value,
            guard,
            otherwise,
        } => {
            scan(value, scope, free, fuel, depth + 1)?;
            let mut success = fork(scope, fuel)?;
            pattern_scan(pattern, &mut success, free, fuel, depth + 1)?;
            if let Some(guard) = guard {
                scan(guard, &success, free, fuel, depth + 1)?;
            }
            if let Some(otherwise) = otherwise {
                scan(otherwise, scope, free, fuel, depth + 1)?;
            }
            *scope = success;
        }
        _ => scan(expr, scope, free, fuel, depth + 1)?,
    }
    Ok(())
}
fn arm_scan(
    arm: &crate::program::MatchArm,
    scope: &Scope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    let mut inner = fork(scope, fuel)?;
    pattern_scan(&arm.pattern, &mut inner, free, fuel, depth + 1)?;
    if let Some(guard) = &arm.guard {
        scan(guard, &inner, free, fuel, depth + 1)?;
    }
    scan(&arm.body, &inner, free, fuel, depth + 1)
}
fn pattern_scan(
    pattern: &P,
    scope: &mut Scope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("capture pattern depth exhausted".into());
    }
    charge(fuel, 1)?;
    match pattern {
        P::Binding { binding, inner } => {
            pattern_scan(inner, scope, free, fuel, depth + 1)?;
            scope.insert(*binding);
        }
        P::Then { first, value, next } => {
            pattern_scan(first, scope, free, fuel, depth + 1)?;
            scan(value, scope, free, fuel, depth + 1)?;
            pattern_scan(next, scope, free, fuel, depth + 1)?;
        }
        P::Guard { pattern, condition } => {
            pattern_scan(pattern, scope, free, fuel, depth + 1)?;
            scan(condition, scope, free, fuel, depth + 1)?;
        }
        P::Remap { pattern, bindings } => {
            pattern_scan(pattern, scope, free, fuel, depth + 1)?;
            charge(fuel, bindings.len())?;
            for (from, _) in bindings {
                read(*from, scope, free);
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
                pattern_scan(pattern, &mut branch, free, fuel, depth + 1)?;
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
        P::Record { fields, .. }
        | P::AnonymousRecord { fields, .. }
        | P::Variant { fields, .. }
        | P::NormalForm {
            arguments: fields, ..
        }
        | P::Tuple(fields)
        | P::TypeTuple(fields) => {
            for field in fields {
                pattern_scan(field, scope, free, fuel, depth + 1)?;
            }
        }
        P::Wild | P::Unit | P::Literal(_) => {}
    }
    Ok(())
}
fn normal(
    argument: &A,
    scope: &Scope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("capture normal-input depth exhausted".into());
    }
    charge(fuel, 1)?;
    match argument {
        A::Value(value) | A::TypeListSpread(value) => scan(value, scope, free, fuel, depth + 1)?,
        A::TypeList { items, .. } | A::Tuple { items, .. } => {
            for item in items {
                normal(item, scope, free, fuel, depth + 1)?;
            }
        }
        A::Block {
            statements, result, ..
        } => {
            let mut inner = fork(scope, fuel)?;
            for value in statements {
                statement(value, &mut inner, free, fuel, depth + 1)?;
            }
            if let Some(value) = result {
                normal(value, &inner, free, fuel, depth + 1)?;
            }
        }
        A::If {
            condition,
            then_input,
            else_input,
            ..
        } => {
            scan(condition, scope, free, fuel, depth + 1)?;
            normal(then_input, scope, free, fuel, depth + 1)?;
            normal(else_input, scope, free, fuel, depth + 1)?;
        }
        A::Match { value, arms, .. } => {
            scan(value, scope, free, fuel, depth + 1)?;
            for arm in arms {
                let mut inner = fork(scope, fuel)?;
                pattern_scan(&arm.pattern, &mut inner, free, fuel, depth + 1)?;
                if let Some(guard) = &arm.guard {
                    scan(guard, &inner, free, fuel, depth + 1)?;
                }
                normal(&arm.body, &inner, free, fuel, depth + 1)?;
            }
        }
        A::Integer { .. } | A::Unit { .. } => {}
    }
    Ok(())
}
