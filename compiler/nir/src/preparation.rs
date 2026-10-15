//! Preparation dependencies shared by phase validation and materialization.
use crate::program::{Expr, ExprKind as K, LocalId};
use crate::{Sort, Term};
use std::collections::{BTreeMap, BTreeSet};
fn charge(fuel: &mut usize, count: usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(count)
        .ok_or("preparation dependency budget exhausted")?;
    Ok(())
}
pub fn references(expr: &Expr, fuel: &mut usize) -> Result<BTreeSet<LocalId>, String> {
    let mut result = BTreeSet::new();
    crate::visit::expression(expr, fuel, |expr, _| {
        match &expr.kind {
            K::Local(binding) => {
                result.insert(*binding);
            }
            K::Closure { captures, .. } => {
                for capture in captures {
                    if capture.value.is_none() {
                        result.insert(capture.binding);
                    }
                }
            }
            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } => {
                result.insert(place.root);
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(result)
}

pub fn inputs(
    expr: &Expr,
    fuel: &mut usize,
) -> Result<Option<(BTreeSet<usize>, Vec<(usize, LocalId, Term)>)>, String> {
    let K::CallPreparation {
        statements,
        result: Some(call),
    } = &expr.kind
    else {
        return Ok(None);
    };
    let K::Call {
        callee, arguments, ..
    } = &call.kind
    else {
        return Ok(None);
    };
    let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
        return Ok(None);
    };
    let mut needed = BTreeSet::new();
    for argument in arguments {
        let parameter = parameters
            .get(argument.parameter)
            .ok_or("preparation parameter index out of range")?;
        if parameter.sort != Sort::Runtime && !argument.is_default {
            needed.extend(references(&argument.value, fuel)?);
        }
    }
    if needed.is_empty() {
        return Ok(None);
    };
    let mut owners = BTreeMap::new();
    let mut bindings = Vec::new();
    for (index, statement) in statements.iter().enumerate() {
        charge(fuel, 1)?;
        let mut fields = Vec::new();
        match &statement.kind {
            K::Let { binding, value } => fields.push((*binding, value.ty.clone())),
            K::LetPattern {
                pattern: crate::program::Pattern::Tuple(patterns),
                value,
                guard: None,
                otherwise: None,
            } => {
                let Term::Tuple(types) = &value.ty else {
                    continue;
                };
                if patterns.len() != types.len() {
                    return Err("preparation tuple arity mismatch".into());
                }
                for (pattern, ty) in patterns.iter().zip(types) {
                    let crate::program::Pattern::Binding { binding, inner } = pattern else {
                        return Err("unsupported preparation binding".into());
                    };
                    if !matches!(inner.as_ref(), crate::program::Pattern::Wild) {
                        return Err("unsupported preparation subpattern".into());
                    }
                    fields.push((*binding, ty.clone()));
                }
            }
            _ => continue,
        }
        for (binding, ty) in fields {
            if owners.insert(binding, index).is_some() {
                return Err("duplicate preparation binding".into());
            }
            bindings.push((index, binding, ty));
        }
    }
    let mut indices = BTreeSet::new();
    while let Some(binding) = needed.pop_first() {
        charge(fuel, 1)?;
        let Some(&index) = owners.get(&binding) else {
            continue;
        };
        if !indices.insert(index) {
            continue;
        }
        for reference in references(&statements[index], fuel)? {
            if let Some(&producer) = owners.get(&reference) {
                if producer == index {
                    continue;
                }
                if producer > index {
                    return Err("preparation reads a later producer".into());
                }
                needed.insert(reference);
            }
        }
    }
    if indices.is_empty() {
        return Ok(None);
    };
    // Export only values used outside the selected computation. Intermediate
    // closures or aggregates can stay private to the comptime invocation.
    let mut exported = references(call, fuel)?;
    for (index, statement) in statements.iter().enumerate() {
        if !indices.contains(&index) {
            exported.extend(references(statement, fuel)?);
        }
    }
    bindings.retain(|(index, binding, _)| indices.contains(index) && exported.contains(binding));
    Ok(Some((indices, bindings)))
}
