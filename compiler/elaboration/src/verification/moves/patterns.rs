//! Separate structural observations from a common successful ownership transfer.
//! This does not rewrite the executable pattern or evaluate candidates early.
use super::{error, Error, Pattern, SourceRange};
use nir::program::LocalId;
use std::collections::{BTreeMap, BTreeSet};

/// Acquisition order survives remapping and includes Copy binders inside an
/// as-binding. It is separate from the compact ownership-transfer expansion.
pub(super) fn binding_orders(pattern: &Pattern, fuel: &mut usize, source: SourceRange, depth: usize)
    -> Result<Vec<Vec<LocalId>>, Error> {
    if depth > 128 { return Err(error(source, "pattern binding order depth exhausted")); }
    charge(fuel, source)?;
    let orders = match pattern {
        Pattern::Guard { pattern, .. } => binding_orders(pattern, fuel, source, depth + 1)?,
        Pattern::Then { first, next, .. } => {
            let first = binding_orders(first, fuel, source, depth + 1)?;
            let next = binding_orders(next, fuel, source, depth + 1)?;
            if first.len().saturating_mul(next.len()) > 256 {
                return Err(error(source, "pattern binding order budget exhausted"));
            }
            let mut result = vec![];
            for before in first { for after in &next {
                *fuel = fuel.checked_sub(before.len().saturating_add(after.len()).saturating_add(1))
                    .ok_or_else(|| error(source, "pattern binding order budget exhausted"))?;
                let mut order = before.clone(); order.extend(after); result.push(order);
            }}
            result
        }
        Pattern::Binding {binding, inner} => {
            let mut orders = binding_orders(inner, fuel, source, depth + 1)?;
            for order in &mut orders { charge(fuel, source)?; order.push(*binding); }
            orders
        }
        Pattern::Remap {pattern, bindings} => {
            let mapping = binding_map(pattern, bindings, fuel, source)?;
            let mut orders = binding_orders(pattern, fuel, source, depth + 1)?;
            for order in &mut orders { for binding in order {
                charge(fuel, source)?; *binding = mapping.get(binding).copied().unwrap_or(*binding);
            }}
            orders
        }
        Pattern::Or(alternatives) => {
            let mut result = vec![];
            for alternative in alternatives {
                for order in binding_orders(alternative, fuel, source, depth + 1)? {
                    if !result.contains(&order) { result.push(order); }
                    if result.len() > 256 { return Err(error(source, "pattern binding order budget exhausted")); }
                }
            }
            result
        }
        Pattern::Tuple(fields) | Pattern::Record {fields,..}
        | Pattern::AnonymousRecord {fields,..} | Pattern::Variant {fields,..} => {
            let mut rows = vec![vec![]];
            for field in fields {
                let choices = binding_orders(field, fuel, source, depth + 1)?;
                if rows.len().saturating_mul(choices.len()) > 256 {
                    return Err(error(source, "pattern binding order budget exhausted"));
                }
                let mut next = vec![];
                for row in rows { for choice in &choices {
                    *fuel = fuel.checked_sub(row.len().saturating_add(choice.len()).saturating_add(1))
                        .ok_or_else(|| error(source, "pattern binding order budget exhausted"))?;
                    let mut order = row.clone(); order.extend(choice); next.push(order);
                }}
                rows = next;
            }
            rows
        }
        Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => vec![vec![]],
        _ => return Err(error(source, "pattern binding order requires candidate computation semantics")),
    };
    Ok(orders)
}

pub(super) fn has_alternatives(pattern: &Pattern, fuel: &mut usize, source: SourceRange, depth: usize)
    -> Result<bool, Error> {
    if depth > 128 { return Err(error(source, "pattern candidate depth exhausted")); }
    charge(fuel, source)?;
    match pattern {
        Pattern::Or(_) => Ok(true),
        Pattern::Then { first, .. } => has_alternatives(first, fuel, source, depth + 1),
        // An as-binding owns the whole position. Its inner alternatives may
        // only copy, and already use the compact Copy candidate analysis.
        Pattern::Binding { .. } => Ok(false),
        Pattern::Remap { pattern: inner, .. } | Pattern::Guard { pattern: inner, .. } =>
            has_alternatives(inner, fuel, source, depth + 1),
        Pattern::Tuple(fields) | Pattern::Record { fields, .. }
        | Pattern::AnonymousRecord { fields, .. } | Pattern::Variant { fields, .. } => {
            for field in fields {
                if has_alternatives(field, fuel, source, depth + 1)? { return Ok(true); }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

/// Enumerate structural success cases for ownership analysis only. Executable
/// matching keeps the original ordered decision tree and evaluates it once.
pub(super) fn alternatives(pattern: &Pattern, fuel: &mut usize, source: SourceRange, depth: usize)
    -> Result<Vec<Pattern>, Error> {
    if depth > 128 { return Err(error(source, "pattern candidate depth exhausted")); }
    charge(fuel, source)?;
    let result = match pattern {
        Pattern::Or(items) => {
            let mut result = vec![];
            for item in items {
                result.extend(alternatives(item, fuel, source, depth + 1)?);
                if result.len() > 256 { return Err(error(source, "pattern candidate budget exhausted")); }
            }
            result
        }
        Pattern::Binding { .. } => vec![pattern.clone()],
        // The next pattern observes a separately computed scalar in the
        // currently supported ownership path. Its Copy alternatives remain
        // compact and never alter the original subject's residual layout.
        Pattern::Then { first, value, next } => alternatives(first, fuel, source, depth + 1)?
            .into_iter().map(|first| Pattern::Then {
                first: Box::new(first), value: value.clone(), next: next.clone(),
            }).collect(),
        Pattern::Guard { pattern, condition } => alternatives(pattern, fuel, source, depth + 1)?
            .into_iter().map(|pattern| Pattern::Guard {
                pattern: Box::new(pattern), condition: condition.clone(),
            }).collect(),
        Pattern::Remap { pattern, bindings } => alternatives(pattern, fuel, source, depth + 1)?
            .into_iter().map(|pattern| Pattern::Remap { pattern: Box::new(pattern), bindings: bindings.clone() }).collect(),
        Pattern::Tuple(fields) | Pattern::Record { fields, .. }
        | Pattern::AnonymousRecord { fields, .. } | Pattern::Variant { fields, .. } => {
            let mut rows = vec![vec![]];
            for field in fields {
                let choices = alternatives(field, fuel, source, depth + 1)?;
                if rows.len().saturating_mul(choices.len()) > 256 {
                    return Err(error(source, "pattern candidate budget exhausted"));
                }
                let mut next = vec![];
                for row in rows {
                    for choice in &choices {
                        // Charge recursive structure before cloning a row.
                        // Inner as-bindings may retain Copy alternatives with
                        // different paths; do not demand one transfer plan.
                        for item in row.iter().chain(std::iter::once(choice)) {
                            exports(item, fuel, source, depth + 1)?;
                        }
                        let mut candidate = row.clone(); candidate.push(choice.clone()); next.push(candidate);
                    }
                }
                rows = next;
            }
            rows.into_iter().map(|fields| match pattern {
                Pattern::Tuple(_) => Pattern::Tuple(fields),
                Pattern::Record { declaration, .. } => Pattern::Record { declaration: *declaration, fields },
                Pattern::AnonymousRecord { names, .. } => Pattern::AnonymousRecord { names: names.clone(), fields },
                Pattern::Variant { declaration, index, .. } => Pattern::Variant { declaration: *declaration, index: *index, fields },
                _ => unreachable!(),
            }).collect()
        }
        _ => vec![pattern.clone()],
    };
    Ok(result)
}

pub(super) fn common_plan(
    pattern: &Pattern,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<(Pattern, Pattern), Error> {
    if depth > 128 {
        return Err(error(source, "or-pattern transfer depth exhausted"));
    }
    charge(fuel, source)?;
    let pair = match pattern {
        Pattern::Wild | Pattern::Literal(_) => (Pattern::Wild, Pattern::Wild),
        Pattern::Unit => (Pattern::Unit, Pattern::Wild),
        Pattern::Binding { binding, inner } => {
            let (observation, transfer) = common_plan(inner, fuel, source, depth + 1)?;
            (observation, Pattern::Binding { binding: *binding, inner: Box::new(transfer) })
        }
        Pattern::Remap { pattern, bindings } => {
            let (observation, mut transfer) = common_plan(pattern, fuel, source, depth + 1)?;
            remap(&mut transfer, bindings, fuel, source)?;
            (observation, transfer)
        }
        Pattern::Or(alternatives) => {
            let mut observations = Vec::new();
            let mut common = None;
            for alternative in alternatives {
                let (observation, transfer) = common_plan(alternative, fuel, source, depth + 1)?;
                if common.as_ref().is_some_and(|previous| previous != &transfer) {
                    return Err(error(source, "or-pattern alternatives require conditional ownership transfer plans"));
                }
                common = Some(transfer);
                observations.push(observation);
            }
            (Pattern::Or(observations), common.unwrap_or(Pattern::Wild))
        }
        Pattern::Tuple(fields)
        | Pattern::Record { fields, .. }
        | Pattern::AnonymousRecord { fields, .. }
        | Pattern::Variant { fields, .. } => {
            let mut observations = Vec::new();
            let mut transfers = Vec::new();
            for field in fields {
                let (observation, transfer) = common_plan(field, fuel, source, depth + 1)?;
                observations.push(observation);
                transfers.push(transfer);
            }
            let rebuild = |fields| match pattern {
                Pattern::Tuple(_) => Pattern::Tuple(fields),
                Pattern::Record { declaration, .. } => Pattern::Record { declaration: *declaration, fields },
                Pattern::AnonymousRecord { names, .. } => Pattern::AnonymousRecord { names: names.clone(), fields },
                Pattern::Variant { declaration, index, .. } => Pattern::Variant { declaration: *declaration, index: *index, fields },
                _ => unreachable!(),
            };
            let transfer = if transfers.iter().all(|field| matches!(field, Pattern::Wild)) {
                Pattern::Wild
            } else {
                rebuild(transfers)
            };
            (rebuild(observations), transfer)
        }
        // Never erase computations, nested guards, or type-pattern protocols.
        // They need candidate-local evaluation and failure-path accounting.
        _ => return Err(error(source, "or-pattern requires candidate computation and failure semantics")),
    };
    Ok(pair)
}

fn charge(fuel: &mut usize, source: SourceRange) -> Result<(), Error> {
    *fuel = fuel.checked_sub(1).ok_or_else(|| error(source, "or-pattern transfer budget exhausted"))?;
    Ok(())
}

/// Validate candidate exports independently of extraction paths and delivery.
pub(super) fn binding_map(
    pattern: &Pattern,
    bindings: &[(LocalId, LocalId)],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<BTreeMap<LocalId, LocalId>, Error> {
    let names = exports(pattern, fuel, source, 0)?;
    checked_map(&names, bindings, fuel, source)
}

fn checked_map(
    names: &BTreeSet<LocalId>, bindings: &[(LocalId, LocalId)],
    fuel: &mut usize, source: SourceRange,
) -> Result<BTreeMap<LocalId, LocalId>, Error> {
    let mut mapping = BTreeMap::new();
    for (from, to) in bindings {
        charge(fuel, source)?;
        if !names.contains(from) || mapping.insert(*from, *to).is_some() {
            return Err(error(source, "invalid copied pattern remap source"));
        }
    }
    let mut targets = BTreeSet::new();
    for name in names {
        charge(fuel, source)?;
        if !targets.insert(mapping.get(name).copied().unwrap_or(*name)) {
            return Err(error(source, "copied pattern remap merges bindings"));
        }
    }
    Ok(mapping)
}

fn exports(pattern: &Pattern, fuel: &mut usize, source: SourceRange, depth: usize)
    -> Result<BTreeSet<LocalId>, Error> {
    if depth > 128 { return Err(error(source, "copied pattern binding depth exhausted")); }
    charge(fuel, source)?;
    let mut names = BTreeSet::new();
    match pattern {
        Pattern::Guard { pattern, .. } => return exports(pattern, fuel, source, depth + 1),
        Pattern::Then { first, next, .. } => {
            names = exports(first, fuel, source, depth + 1)?;
            for name in exports(next, fuel, source, depth + 1)? {
                charge(fuel, source)?;
                if !names.insert(name) { return Err(error(source, "duplicate computed pattern binding")); }
            }
        }
        Pattern::Binding { binding, inner } => {
            names = exports(inner, fuel, source, depth + 1)?;
            if !names.insert(*binding) { return Err(error(source, "duplicate copied pattern binding")); }
        }
        Pattern::Remap { pattern, bindings } => {
            let inner = exports(pattern, fuel, source, depth + 1)?;
            let mapping = checked_map(&inner, bindings, fuel, source)?;
            names = inner.into_iter().map(|name| mapping.get(&name).copied().unwrap_or(name)).collect();
        }
        Pattern::Or(alternatives) => {
            let mut common = None;
            for alternative in alternatives {
                let candidate = exports(alternative, fuel, source, depth + 1)?;
                if common.as_ref().is_some_and(|previous| previous != &candidate) {
                    return Err(error(source, "copied pattern alternatives export different bindings"));
                }
                common = Some(candidate);
            }
            names = common.unwrap_or_default();
        }
        Pattern::Tuple(fields) | Pattern::Record { fields, .. }
        | Pattern::AnonymousRecord { fields, .. } | Pattern::Variant { fields, .. } => {
            for field in fields {
                for name in exports(field, fuel, source, depth + 1)? {
                    charge(fuel, source)?;
                    if !names.insert(name) { return Err(error(source, "duplicate copied pattern binding")); }
                }
            }
        }
        Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => (),
        _ => return Err(error(source, "copied pattern requires candidate computation and failure semantics")),
    }
    Ok(names)
}

fn remap(
    pattern: &mut Pattern,
    bindings: &[(LocalId, LocalId)],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<(), Error> {
    let mut mapping = BTreeMap::new();
    for (from, to) in bindings {
        charge(fuel, source)?;
        if mapping.insert(*from, *to).is_some() {
            return Err(error(source, "duplicate or-pattern remap source"));
        }
    }
    let mut found = BTreeSet::new();
    let mut exported = BTreeSet::new();
    let mut pending = vec![pattern];
    while let Some(pattern) = pending.pop() {
        charge(fuel, source)?;
        match pattern {
            Pattern::Binding { binding, inner } => {
                if !found.insert(*binding) {
                    return Err(error(source, "duplicate or-pattern input binding"));
                }
                // Simultaneous substitution: swaps must not chain through a
                // binding which has already been renamed in this traversal.
                *binding = mapping.get(binding).copied().unwrap_or(*binding);
                if !exported.insert(*binding) {
                    return Err(error(source, "or-pattern remap merges owned bindings"));
                }
                pending.push(inner);
            }
            Pattern::Tuple(fields) | Pattern::Record { fields, .. }
            | Pattern::AnonymousRecord { fields, .. } | Pattern::Variant { fields, .. } => pending.extend(fields),
            Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => (),
            _ => return Err(error(source, "noncanonical or-pattern transfer plan")),
        }
    }
    if mapping.keys().any(|from| !found.contains(from)) {
        return Err(error(source, "or-pattern remap refers to a missing binding"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
    fn id(index: u32) -> LocalId { LocalId { owner: 1, index } }
    fn binding(index: u32) -> Pattern {
        Pattern::Binding { binding: id(index), inner: Box::new(Pattern::Wild) }
    }

    #[test]
    fn sequential_candidates_keep_computed_exports_separate_and_ordered() {
        let value = Box::new(nir::program::Expr {
            source: SOURCE, ty: nir::Term::Primitive(nir::Primitive::Bool),
            kind: nir::program::ExprKind::Local(id(1)),
        });
        let pattern = Pattern::Then {
            first: Box::new(Pattern::Or(vec![
                Pattern::Tuple(vec![binding(1),binding(2)]),
                Pattern::Tuple(vec![binding(2),binding(1)]),
            ])), value: value.clone(), next: Box::new(binding(3)),
        };
        let before=pattern.clone();
        assert!(has_alternatives(&pattern,&mut 1000,SOURCE,0).unwrap());
        let candidates=alternatives(&pattern,&mut 1000,SOURCE,0).unwrap();
        assert_eq!(candidates.len(),2);
        for candidate in candidates {
            assert!(matches!(candidate,Pattern::Then {value:ref kept,..} if kept==&value));
            assert_eq!(exports(&candidate,&mut 1000,SOURCE,0).unwrap(),BTreeSet::from([id(1),id(2),id(3)]));
        }
        assert_eq!(binding_orders(&pattern,&mut 1000,SOURCE,0).unwrap(),vec![vec![id(1),id(2),id(3)],vec![id(2),id(1),id(3)]]);
        let duplicate=Pattern::Then { first:Box::new(binding(1)),value,next:Box::new(binding(1)) };
        assert!(exports(&duplicate,&mut 1000,SOURCE,0).is_err());
        assert!(alternatives(&pattern,&mut 1,SOURCE,0).is_err());
        assert_eq!(pattern,before);
    }

    #[test]
    fn guarded_candidates_preserve_computations_exports_and_order() {
        let condition = Box::new(nir::program::Expr {
            source: SOURCE, ty: nir::Term::Primitive(nir::Primitive::Bool),
            kind: nir::program::ExprKind::Local(id(1)),
        });
        let pattern = Pattern::Guard {
            pattern: Box::new(Pattern::Or(vec![
                Pattern::Tuple(vec![binding(1),binding(2)]),
                Pattern::Tuple(vec![binding(2),binding(1)]),
            ])), condition: condition.clone(),
        };
        let before=pattern.clone();
        assert!(has_alternatives(&pattern,&mut 1000,SOURCE,0).unwrap());
        let candidates=alternatives(&pattern,&mut 1000,SOURCE,0).unwrap();
        assert_eq!(candidates.len(),2);
        for candidate in candidates {
            assert!(matches!(candidate,Pattern::Guard {condition: ref kept,..} if kept==&condition));
            assert_eq!(exports(&candidate,&mut 1000,SOURCE,0).unwrap(),BTreeSet::from([id(1),id(2)]));
        }
        assert_eq!(binding_orders(&pattern,&mut 1000,SOURCE,0).unwrap(),vec![vec![id(1),id(2)],vec![id(2),id(1)]]);
        assert!(alternatives(&pattern,&mut 1,SOURCE,0).is_err());
        assert_eq!(pattern,before);
    }

    #[test]
    fn binding_order_products_are_bounded_and_recover_after_exhaustion() {
        let choice = |index| Pattern::Or(vec![
            Pattern::Tuple(vec![binding(index), binding(index + 1)]),
            Pattern::Tuple(vec![binding(index + 1), binding(index)]),
        ]);
        let large = Pattern::Tuple((0..9).map(|index| choice(index * 2)).collect());
        assert!(binding_orders(&large, &mut 100_000, SOURCE, 0).is_err());
        let small = Pattern::Binding {binding: id(4), inner: Box::new(Pattern::Tuple(vec![choice(0),choice(2)]))};
        let original = small.to_string();
        assert!(binding_orders(&small, &mut 1, SOURCE, 0).is_err());
        let orders = binding_orders(&small, &mut 1000, SOURCE, 0).unwrap();
        assert_eq!(orders.len(), 4);
        assert_eq!(orders[0], vec![id(0),id(1),id(2),id(3),id(4)]);
        assert_eq!(orders[3], vec![id(1),id(0),id(3),id(2),id(4)]);
        assert_eq!(small.to_string(), original);
    }

    #[test]
    fn conditional_candidates_are_bounded_and_do_not_rewrite_the_pattern() {
        let choice = Pattern::Or(vec![Pattern::Wild, Pattern::Unit]);
        let source = Pattern::Tuple(vec![choice.clone(); 9]);
        let before = source.to_string();
        assert!(alternatives(&source, &mut 100_000, SOURCE, 0).is_err());
        assert_eq!(source.to_string(), before);
        let small = Pattern::Tuple(vec![choice.clone(); 2]);
        assert!(alternatives(&small, &mut 1, SOURCE, 0).is_err());
        assert_eq!(alternatives(&small, &mut 1000, SOURCE, 0).unwrap().len(), 4);
        // These inner choices copy from an intact owner; they do not multiply
        // the affine ownership plans for the whole binding.
        let whole = Pattern::Binding { binding: id(1), inner: Box::new(source) };
        assert!(!has_alternatives(&whole, &mut 10, SOURCE, 0).unwrap());
    }

    #[test]
    fn affine_expansion_preserves_inner_copy_choices_of_whole_bindings() {
        let inner = Pattern::Or(vec![
            Pattern::Tuple(vec![binding(1), Pattern::Wild]),
            Pattern::Tuple(vec![Pattern::Wild, binding(1)]),
        ]);
        let whole = Pattern::Binding { binding: id(2), inner: Box::new(inner) };
        let source = Pattern::Tuple(vec![Pattern::Or(vec![binding(3), binding(3)]), whole.clone()]);
        let cases = alternatives(&source, &mut 1000, SOURCE, 0).unwrap();
        assert_eq!(cases, vec![Pattern::Tuple(vec![binding(3), whole]); 2]);
    }

    #[test]
    fn binding_remaps_are_simultaneous_and_candidate_local() {
        let source = Pattern::Or(vec![
            Pattern::Tuple(vec![binding(1), binding(2)]),
            Pattern::Remap {
                pattern: Box::new(Pattern::Tuple(vec![binding(2), binding(1)])),
                bindings: vec![(id(1), id(2)), (id(2), id(1))],
            },
        ]);
        let before = source.to_string();
        let (_, plan) = common_plan(&source, &mut 100, SOURCE, 0).unwrap();
        assert_eq!(plan, Pattern::Tuple(vec![binding(1), binding(2)]));
        assert_eq!(source.to_string(), before);
    }

    #[test]
    fn remap_cannot_invent_or_merge_owned_bindings() {
        for bindings in [
            vec![(id(3), id(4))],
            vec![(id(1), id(3)), (id(1), id(4))],
            vec![(id(1), id(3)), (id(2), id(3))],
            vec![(id(1), id(2))],
        ] {
            let source = Pattern::Remap {
                pattern: Box::new(Pattern::Tuple(vec![binding(1), binding(2)])),
                bindings,
            };
            assert!(common_plan(&source, &mut 100, SOURCE, 0).is_err());
        }
    }

    #[test]
    fn equal_exports_do_not_erase_candidate_computations() {
        let condition = nir::program::Expr {
            source: SOURCE,
            ty: nir::Term::Primitive(nir::Primitive::Bool),
            kind: nir::program::ExprKind::Literal(nir::program::Literal::Bool(true)),
        };
        for computation in [
            Pattern::Guard { pattern: Box::new(binding(1)), condition: Box::new(condition.clone()) },
            Pattern::Then { first: Box::new(binding(1)), value: Box::new(condition), next: Box::new(Pattern::Wild) },
        ] {
            let source = Pattern::Or(vec![computation, binding(1)]);
            let failure = common_plan(&source, &mut 100, SOURCE, 0).unwrap_err();
            assert!(failure.reason.contains("candidate computation"));
        }
    }

    #[test]
    fn exhausted_plan_does_not_poison_a_later_attempt() {
        let source = Pattern::Or(vec![binding(1), Pattern::Remap {
            pattern: Box::new(binding(2)), bindings: vec![(id(2), id(1))],
        }]);
        let before = source.to_string();
        assert!(common_plan(&source, &mut 2, SOURCE, 0).is_err());
        assert!(common_plan(&source, &mut 100, SOURCE, 129).is_err());
        assert_eq!(common_plan(&source, &mut 100, SOURCE, 0).unwrap().1, binding(1));
        assert_eq!(source.to_string(), before);
    }

    #[test]
    fn copied_remap_validates_all_candidates_without_requiring_equal_paths() {
        let source = Pattern::Or(vec![
            Pattern::Tuple(vec![binding(1), Pattern::Wild]),
            Pattern::Remap {
                pattern: Box::new(Pattern::Tuple(vec![Pattern::Wild, binding(2)])),
                bindings: vec![(id(2), id(1))],
            },
        ]);
        assert_eq!(binding_map(&source, &[(id(1), id(3))], &mut 100, SOURCE).unwrap(),
            BTreeMap::from([(id(1), id(3))]));
        assert!(binding_map(&source, &[(id(2), id(3))], &mut 100, SOURCE).is_err());
        assert!(binding_map(&source, &[], &mut 1, SOURCE).is_err());
        let incompatible = Pattern::Or(vec![binding(1), binding(2)]);
        assert!(binding_map(&incompatible, &[], &mut 100, SOURCE).is_err());
        let duplicate = Pattern::Tuple(vec![binding(1), binding(1)]);
        assert!(binding_map(&duplicate, &[], &mut 100, SOURCE).is_err());
    }
}
