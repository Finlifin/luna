//! Finite local template storage. Connected locals share target numbers, so
//! assignment copies the current choice while independent aliases stay snapshots.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Candidate {
    ty: Option<Term>,
    declarations: usize,
    assigned: bool,
    dependencies: BTreeSet<n::LocalId>,
    targets: Vec<n::Expr>,
}

fn staged(ty: &Term) -> bool {
    matches!(ty.callable_signature(), Term::Function {parameters,..}
        if parameters.iter().any(|p|p.sort != nir::Sort::Runtime))
}
fn charge(value: &n::Expr, fuel: &mut usize) -> Result<(), String> {
    let mut remaining = *fuel as u64;
    nir::copy_budget::charge_expression(value, &mut remaining)
        .map_err(|_| "mutable template copy budget exhausted")?;
    *fuel = remaining as usize;
    Ok(())
}
fn sources(value: &n::Expr, candidate: &mut Candidate, fuel: &mut usize) -> Result<bool, String> {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("mutable template source budget exhausted")?;
        if value.ty == Term::Primitive(nir::Primitive::Never) {
            continue;
        }
        match &value.kind {
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => {
                charge(value, fuel)?;
                candidate.targets.push(value.clone());
            }
            K::Local(binding) => {
                candidate.dependencies.insert(*binding);
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
            } => {
                pending.push(then_branch);
                pending.push(other);
            }
            K::Match { arms, .. } if !arms.is_empty() => {
                pending.extend(arms.iter().map(|arm| &arm.body))
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

pub(super) fn lower(
    expression: &mut n::Expr,
    next: &mut u32,
    expansion_sources: &mut Vec<n::ExpansionSource>,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut excluded = BTreeSet::new();
    let mut candidates = BTreeMap::<n::LocalId, Candidate>::new();
    iir::visit::expression_pruned(expression, fuel, |expr, fuel| {
        if matches!(
            expr.kind,
            K::AddressOf(_) | K::Closure { .. } | K::TemplateEqual(..)
        ) {
            excluded.extend(nir::free_bindings::collect(expr, fuel)?);
            return Ok(false);
        }
        let (binding, value, declaration) = match &expr.kind {
            K::Let { binding, value } if staged(&value.ty) => (*binding, value.as_ref(), true),
            K::Assign { place, value } if staged(&value.ty) => {
                if !place.fields.is_empty() {
                    excluded.insert(place.root);
                    return Ok(true);
                }
                (place.root, value.as_ref(), false)
            }
            _ => return Ok(true),
        };
        let candidate = candidates.entry(binding).or_default();
        candidate.declarations += usize::from(declaration);
        candidate.assigned |= !declaration;
        charge(value, fuel)?;
        if candidate.ty.as_ref().is_some_and(|ty| ty != &value.ty)
            || !sources(value, candidate, fuel)?
        {
            excluded.insert(binding);
        }
        candidate.ty = Some(value.ty.clone());
        Ok(true)
    })?;
    let mut neighbors = BTreeMap::<n::LocalId, BTreeSet<n::LocalId>>::new();
    for (binding, candidate) in &candidates {
        for dependency in &candidate.dependencies {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("mutable template dependency budget exhausted")?;
            neighbors.entry(*binding).or_default().insert(*dependency);
            neighbors.entry(*dependency).or_default().insert(*binding);
        }
    }
    let mut visited = BTreeSet::new();
    let mut plans = BTreeMap::new(); // original binding -> (runtime tag binding, target group)
    let mut groups = Vec::<Vec<n::Expr>>::new();
    for root in candidates.keys() {
        if visited.contains(root) {
            continue;
        }
        let mut pending = vec![*root];
        let mut component = Vec::new();
        let mut valid = true;
        let mut assigned = false;
        while let Some(binding) = pending.pop() {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("mutable template component budget exhausted")?;
            if !visited.insert(binding) {
                continue;
            }
            component.push(binding);
            if let Some(edges) = neighbors.get(&binding) {
                pending.extend(edges.iter().copied());
            }
            if let Some(candidate) = candidates.get(&binding) {
                assigned |= candidate.assigned;
                valid &= candidate.declarations == 1
                    && !excluded.contains(&binding)
                    && candidate.ty == candidates[root].ty;
            } else {
                valid = false;
            }
        }
        if !valid || !assigned {
            continue;
        }
        let mut targets = Vec::<n::Expr>::new();
        for binding in &component {
            for target in &candidates[binding].targets {
                let mut found = false;
                for existing in &targets {
                    charge(existing, fuel)?;
                    if existing.kind == target.kind && existing.ty == target.ty {
                        found = true;
                        break;
                    }
                }
                if !found {
                    charge(target, fuel)?;
                    targets.push(target.clone());
                }
            }
        }
        if targets.is_empty() {
            continue;
        }
        let group = groups.len();
        groups.push(targets);
        for binding in component {
            let tag = n::LocalId {
                owner: u32::MAX,
                index: *next,
            };
            *next = next
                .checked_add(1)
                .ok_or("mutable template identity space exhausted")?;
            plans.insert(binding, (tag, group));
        }
    }
    if plans.is_empty() {
        return Ok(());
    }
    let tag_type = Term::Primitive(nir::Primitive::U64);
    iir::visit::expression_mut_pruned(expression, fuel, |expr, fuel| {
        if matches!(expr.kind, K::Closure { .. } | K::AddressOf(_)) {
            return Ok(false);
        }
        match &mut expr.kind {
            K::Let { binding, value } => {
                if let Some((tag, group)) = plans.get(binding) {
                    encode(value, *group, &plans, &groups, fuel, 0)?;
                    *binding = *tag;
                }
            }
            K::Assign { place, value } => {
                if let Some((tag, group)) = plans.get(&place.root) {
                    if !place.fields.is_empty() {
                        return Err("mutable template projection escaped validation".into());
                    }
                    encode(value, *group, &plans, &groups, fuel, 0)?;
                    place.root = *tag;
                }
            }
            K::Local(binding) => {
                if let Some((tag, group)) = plans.get(binding) {
                    let targets = &groups[*group];
                    let mut arms = Vec::new();
                    for (index, target) in targets.iter().enumerate() {
                        charge(target, fuel)?;
                        let mut body = target.clone();
                        crate::program::inlining::freshen_copied_expression(
                            &mut body,
                            next,
                            expansion_sources,
                            fuel,
                        )?;
                        arms.push(n::MatchArm {
                            pattern: if index + 1 == targets.len() {
                                P::Wild
                            } else {
                                P::Literal(n::Literal::UnsignedInteger(index as u128))
                            },
                            guard: None,
                            body,
                        });
                    }
                    let index = u32::try_from(expansion_sources.len())
                        .map_err(|_| "mutable template source space exhausted")?;
                    expansion_sources.push(n::ExpansionSource {
                        original: expr.source,
                        site: expr.source,
                    });
                    expr.kind = K::Match {
                        value: Box::new(n::Expr {
                            source: n::SourceRange::generated(index),
                            ty: tag_type.clone(),
                            kind: K::Local(*tag),
                        }),
                        arms,
                    };
                }
            }
            _ => {}
        }
        Ok(true)
    })
}

fn encode(
    value: &mut n::Expr,
    group: usize,
    plans: &BTreeMap<n::LocalId, (n::LocalId, usize)>,
    groups: &[Vec<n::Expr>],
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or("mutable template encoding budget exhausted")?;
    if depth >= 128 {
        return Err("mutable template encoding depth exhausted".into());
    }
    if value.ty == Term::Primitive(nir::Primitive::Never) {
        return Ok(());
    }
    match &mut value.kind {
        K::Local(binding) => {
            let (tag, actual) = plans
                .get(binding)
                .ok_or("mutable template source is unresolved")?;
            if *actual != group {
                return Err("mutable template target domains disagree".into());
            }
            *binding = *tag;
        }
        K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => {
            let mut index = None;
            for (position, target) in groups[group].iter().enumerate() {
                charge(target, fuel)?;
                if target.kind == value.kind && target.ty == value.ty {
                    index = Some(position);
                    break;
                }
            }
            value.kind = K::Literal(n::Literal::UnsignedInteger(
                index.ok_or("mutable template target missing")? as u128,
            ));
        }
        K::Block {
            result: Some(result),
            ..
        }
        | K::CallPreparation {
            result: Some(result),
            ..
        } => encode(result, group, plans, groups, fuel, depth + 1)?,
        K::If {
            then_branch,
            else_branch: Some(other),
            ..
        } => {
            encode(then_branch, group, plans, groups, fuel, depth + 1)?;
            encode(other, group, plans, groups, fuel, depth + 1)?;
        }
        K::Match { arms, .. } => {
            for arm in arms {
                encode(&mut arm.body, group, plans, groups, fuel, depth + 1)?;
            }
        }
        _ => return Err("mutable template selection escaped validation".into()),
    }
    value.ty = Term::Primitive(nir::Primitive::U64);
    Ok(())
}
