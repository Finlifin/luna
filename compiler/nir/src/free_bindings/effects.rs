//! Conservative invalidation of local observations across checked code.
use crate::program::{self as n, ExprKind as K};
use std::collections::{BTreeMap, BTreeSet};

/// Access to selected bindings in the enclosing invocation frame.
pub struct Access {
    pub readonly: BTreeSet<n::LocalId>,
    pub writable: BTreeSet<n::LocalId>,
}

pub fn access(
    expr: &n::Expr,
    select: impl Fn(n::LocalId) -> bool,
    fuel: &mut usize,
) -> Result<Access, crate::visit::Error> {
    let mut selected = super::collect(expr, fuel)?;
    super::charge(fuel, selected.len())?;
    selected.retain(|id| select(*id));
    let mut retained: BTreeMap<_, ()> = selected.iter().copied().map(|id| (id, ())).collect();
    retain_unmodified(expr, &mut retained, fuel)?;
    let (readonly, writable) = selected
        .into_iter()
        .partition(|id| retained.contains_key(id));
    Ok(Access { readonly, writable })
}

pub fn remove_pattern_bindings<T>(
    pattern: &n::Pattern,
    bindings: &mut BTreeMap<n::LocalId, T>,
    fuel: &mut usize,
) -> Result<(), crate::visit::Error> {
    let mut pending = vec![pattern];
    while let Some(pattern) = pending.pop() {
        *fuel = fuel.checked_sub(1).ok_or(crate::visit::Error::Budget(
            "binding invalidation budget exhausted",
        ))?;
        match pattern {
            n::Pattern::Binding { binding, inner } => {
                bindings.remove(binding);
                pending.push(inner);
            }
            n::Pattern::Remap {
                pattern,
                bindings: remap,
            } => {
                *fuel = fuel
                    .checked_sub(remap.len())
                    .ok_or(crate::visit::Error::Budget(
                        "binding invalidation budget exhausted",
                    ))?;
                for (_, to) in remap {
                    bindings.remove(to);
                }
                pending.push(pattern);
            }
            n::Pattern::OptionalPresent(inner)
            | n::Pattern::ErrorResult { inner, .. }
            | n::Pattern::Observe { inner, .. }
            | n::Pattern::Guard { pattern: inner, .. } => pending.push(inner),
            n::Pattern::Then { first, next, .. } => {
                pending.push(first);
                pending.push(next);
            }
            n::Pattern::Tuple(fields)
            | n::Pattern::TypeTuple(fields)
            | n::Pattern::Or(fields)
            | n::Pattern::NominalType { fields, .. }
            | n::Pattern::Record { fields, .. }
            | n::Pattern::AnonymousRecord { fields, .. }
            | n::Pattern::Variant { fields, .. }
            | n::Pattern::NormalForm {
                arguments: fields, ..
            } => {
                *fuel = fuel
                    .checked_sub(fields.len())
                    .ok_or(crate::visit::Error::Budget(
                        "binding invalidation budget exhausted",
                    ))?;
                pending.extend(fields);
            }
            n::Pattern::OptionalAbsent
            | n::Pattern::Wild
            | n::Pattern::Unit
            | n::Pattern::Literal(_)
            | n::Pattern::CodeEvidence { .. } => {}
        }
    }
    Ok(())
}

pub fn retain_unmodified<T>(
    expr: &n::Expr,
    bindings: &mut BTreeMap<n::LocalId, T>,
    fuel: &mut usize,
) -> Result<(), crate::visit::Error> {
    if bindings.is_empty() {
        return Ok(());
    }
    crate::visit::expression_checked(expr, fuel, |node, fuel| {
        if bindings.is_empty() {
            return Ok(());
        }
        match &node.kind {
            K::Assign { place, .. } | K::BitUpdate { place, .. } | K::BitAssign { place, .. } => {
                bindings.remove(&place.root);
            }
            K::AddressOf(place_expr) => {
                let mut root = place_expr.as_ref();
                while let K::Field { value, .. } | K::AnonymousField { value, .. } = &root.kind {
                    *fuel = fuel.checked_sub(1).ok_or(crate::visit::Error::Budget(
                        "binding invalidation budget exhausted",
                    ))?;
                    root = value;
                }
                if let K::Local(id) = root.kind {
                    bindings.remove(&id);
                } else {
                    bindings.clear();
                }
            }
            K::WritePointer { .. } | K::BitWritePointer { .. } => {
                bindings.clear();
            }
            K::Call { arguments, .. } => {
                for arg in arguments {
                    if arg.passing == n::ArgumentPassing::Value {
                        continue;
                    }
                    let mut root = &arg.value;
                    while let K::Field { value, .. } | K::AnonymousField { value, .. } = &root.kind
                    {
                        *fuel = fuel.checked_sub(1).ok_or(crate::visit::Error::Budget(
                            "binding invalidation budget exhausted",
                        ))?;
                        root = value;
                    }
                    if let K::Local(id) = root.kind {
                        bindings.remove(&id);
                    } else {
                        bindings.clear();
                        break;
                    }
                }
            }
            K::Let { binding, .. } => {
                bindings.remove(binding);
            }
            K::LetPattern { pattern, .. } => {
                remove_pattern_bindings(pattern, bindings, fuel)?;
            }
            K::Closure { captures, .. } => {
                for capture in captures {
                    if capture.value.is_none() {
                        bindings.remove(&capture.binding);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    })
}
