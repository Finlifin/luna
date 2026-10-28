//! Conservative normal-completion queries on checked NIR.
use nir::program::{self as n, ExprKind as K};
use nir::{Primitive as P, Term};
use std::collections::BTreeSet;
fn p(value: P) -> Term {
    Term::Primitive(value)
}
fn is_never(ty: &Term) -> bool {
    *ty == p(P::Never)
}

fn boolean_inner(expr: &n::Expr, depth: usize, fuel: &mut usize) -> Option<bool> {
    *fuel = fuel.checked_sub(1)?;
    if depth >= 64 {
        return None;
    }
    match &expr.kind {
        K::Literal(n::Literal::Bool(value)) => Some(*value),
        K::Not(value) => Some(!boolean_inner(value, depth + 1, fuel)?),
        K::And(left, right) => {
            Some(boolean_inner(left, depth + 1, fuel)? && boolean_inner(right, depth + 1, fuel)?)
        }
        K::Or(left, right) => {
            Some(boolean_inner(left, depth + 1, fuel)? || boolean_inner(right, depth + 1, fuel)?)
        }
        _ => None,
    }
}

/// Only side-effect-free literal Boolean trees are evaluated here. Exhaustion
/// means unknown, never a guessed value or skipped runtime computation.
pub fn boolean(expr: &n::Expr) -> Option<bool> {
    boolean_inner(expr, 0, &mut 4096)
}

/// Both source if and ordered when lowering use the same completion rule.
pub fn conditional_never(condition: &n::Expr, yes: &Term, no: &Term) -> bool {
    is_never(&condition.ty)
        || match boolean(condition) {
            Some(true) => is_never(yes),
            Some(false) => is_never(no),
            None => is_never(yes) && is_never(no),
        }
}

// Only inert patterns are classified. Executable guards/and-is patterns
// remain on the full visitor path, including their control transfers.
pub fn pattern_matches(
    pattern: &n::Pattern,
    value: &n::Expr,
    fuel: &mut usize,
    depth: usize,
) -> Option<bool> {
    *fuel = fuel.checked_sub(1)?;
    if depth >= 64 {
        return None;
    }
    match pattern {
        n::Pattern::Wild => Some(true),
        n::Pattern::Binding { inner, .. } | n::Pattern::Remap { pattern: inner, .. } => {
            pattern_matches(inner, value, fuel, depth + 1)
        }
        n::Pattern::Literal(n::Literal::Bool(expected)) => {
            Some(boolean_inner(value, 0, fuel)? == *expected)
        }
        n::Pattern::Literal(expected) => {
            let K::Literal(actual) = &value.kind else {
                return None;
            };
            literal_matches(expected, actual, fuel)
        }
        n::Pattern::Tuple(patterns) => {
            let K::Tuple(values) = &value.kind else {
                return None;
            };
            fields_match(patterns, values, fuel, depth + 1)
        }
        n::Pattern::Record {
            declaration,
            fields,
        } => {
            let K::Record {
                declaration: actual,
                fields: values,
            } = &value.kind
            else {
                return None;
            };
            if declaration != actual {
                return None;
            }
            fields_match(fields, values, fuel, depth + 1)
        }
        n::Pattern::AnonymousRecord { names, fields } => {
            let K::AnonymousRecord { fields: values } = &value.kind else {
                return None;
            };
            let Term::Record(types) = &value.ty else {
                return None;
            };
            if names.len() != types.len() {
                return None;
            }
            for (name, (actual, _)) in names.iter().zip(types) {
                *fuel = fuel.checked_sub(name.len().max(actual.len()).saturating_add(1))?;
                if name != actual {
                    return None;
                }
            }
            fields_match(fields, values, fuel, depth + 1)
        }
        n::Pattern::Variant {
            declaration,
            index,
            fields,
        } => {
            let K::Variant {
                declaration: actual,
                index: tag,
                fields: values,
            } = &value.kind
            else {
                return None;
            };
            if declaration != actual {
                return None;
            }
            if index != tag {
                return Some(false);
            }
            fields_match(fields, values, fuel, depth + 1)
        }
        n::Pattern::Unit if matches!(value.kind, K::Unit) => Some(true),
        n::Pattern::Or(patterns) => {
            for pattern in patterns {
                if pattern_matches(pattern, value, fuel, depth + 1)? {
                    return Some(true);
                }
            }
            Some(false)
        }
        _ => None,
    }
}

fn fields_match(
    patterns: &[n::Pattern],
    values: &[n::Expr],
    fuel: &mut usize,
    depth: usize,
) -> Option<bool> {
    if patterns.len() != values.len() {
        return None;
    }
    for (pattern, value) in patterns.iter().zip(values) {
        if !pattern_matches(pattern, value, fuel, depth)? {
            return Some(false);
        }
    }
    Some(true)
}

fn literal_matches(expected: &n::Literal, actual: &n::Literal, fuel: &mut usize) -> Option<bool> {
    use n::Literal as L;
    Some(match (expected, actual) {
        (L::SignedInteger(a), L::SignedInteger(b)) => a == b,
        (L::UnsignedInteger(a), L::UnsignedInteger(b)) => a == b,
        (L::Char(a), L::Char(b)) => a == b,
        (L::String(a), L::String(b)) | (L::Symbol(a), L::Symbol(b)) => {
            *fuel = fuel.checked_sub(a.len().max(b.len()))?;
            a == b
        }
        (L::Float16(a), L::Float16(b)) => comptime_runtime::float16::Float16Format::Binary16
            .compare(*a, *b)
            .is_some_and(|o| o.is_eq()),
        (L::BFloat16(a), L::BFloat16(b)) => comptime_runtime::float16::Float16Format::BFloat16
            .compare(*a, *b)
            .is_some_and(|o| o.is_eq()),
        (L::Float32(a), L::Float32(b)) => f32::from_bits(*a) == f32::from_bits(*b),
        (L::Float64(a), L::Float64(b)) => f64::from_bits(*a) == f64::from_bits(*b),
        (
            L::BitVec {
                width: a,
                words: aw,
            },
            L::BitVec {
                width: b,
                words: bw,
            },
        ) => {
            *fuel = fuel.checked_sub(aw.len().max(bw.len()))?;
            a == b && aw == bw
        }
        // Unbounded Integer/Float spellings need semantic normalization, not
        // string equality. Unknown or mismatched literal forms stay conservative.
        _ => return None,
    })
}

/// A loop completes normally only through a break to its own identity. Calls
/// and unproved branches remain conservative; this is not termination proving.
pub fn loop_may_complete(
    body: &n::Expr,
    identity: n::LocalId,
    fuel: &mut usize,
) -> Result<bool, String> {
    loop_may_transfer(body, identity, false, fuel)
}

pub fn loop_may_continue(
    body: &n::Expr,
    identity: n::LocalId,
    fuel: &mut usize,
) -> Result<bool, String> {
    loop_may_transfer(body, identity, true, fuel)
}

fn loop_may_transfer(
    body: &n::Expr,
    identity: n::LocalId,
    continuing: bool,
    fuel: &mut usize,
) -> Result<bool, String> {
    let mut pending = vec![body];
    let mut found = false;
    while let Some(root) = pending.pop() {
        if found {
            break;
        }
        nir::visit::expression_pruned(root, fuel, |expr, fuel| {
            if found {
                return Ok(false);
            }
            match &expr.kind {
                K::Break(target) | K::Continue(target)
                    if *target == identity && matches!(expr.kind, K::Continue(_)) == continuing =>
                {
                    found = true;
                    return Ok(false);
                }
                K::Tuple(fields)
                | K::TypeList(fields)
                | K::Record { fields, .. }
                | K::Object { fields, .. }
                | K::AnonymousRecord { fields }
                | K::Variant { fields, .. } => {
                    *fuel = fuel
                        .checked_sub(fields.len())
                        .ok_or("loop completion budget exhausted")?;
                    for field in fields {
                        pending.push(field);
                        if is_never(&field.ty) {
                            break;
                        }
                    }
                    return Ok(false);
                }
                K::Closure { captures, .. } => {
                    *fuel = fuel
                        .checked_sub(captures.len())
                        .ok_or("loop completion budget exhausted")?;
                    pending.extend(
                        captures
                            .iter()
                            .filter_map(|capture| capture.value.as_deref()),
                    );
                    return Ok(false);
                }
                K::Call {
                    callee, arguments, ..
                } => {
                    *fuel = fuel
                        .checked_sub(arguments.len().saturating_add(1))
                        .ok_or("loop completion budget exhausted")?;
                    pending.push(callee);
                    if !is_never(&callee.ty) {
                        for argument in arguments.iter().filter(|argument| !argument.is_default) {
                            pending.push(&argument.value);
                            if is_never(&argument.value.ty) {
                                break;
                            }
                        }
                    }
                    return Ok(false);
                }
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    *fuel = fuel
                        .checked_sub(statements.len().saturating_add(1))
                        .ok_or("loop completion budget exhausted")?;
                    let mut live = true;
                    for statement in statements {
                        pending.push(statement);
                        if is_never(&statement.ty) {
                            live = false;
                            break;
                        }
                    }
                    if live {
                        pending.extend(result.as_deref());
                    }
                    return Ok(false);
                }
                K::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    pending.push(condition);
                    if !is_never(&condition.ty) {
                        match boolean(condition) {
                            Some(true) => pending.push(then_branch),
                            Some(false) => pending.extend(else_branch.as_deref()),
                            None => {
                                pending.push(then_branch);
                                pending.extend(else_branch.as_deref());
                            }
                        }
                    }
                    return Ok(false);
                }
                K::Match { value, arms } => {
                    // The subject runs before every pattern and guard. Even
                    // executable/unknown patterns are unreachable if it exits.
                    if is_never(&value.ty) {
                        pending.push(value);
                        return Ok(false);
                    }
                    *fuel = fuel
                        .checked_sub(arms.len().saturating_add(1))
                        .ok_or("loop completion budget exhausted")?;
                    // Classify before pruning. If any pattern needs execution,
                    // keep the original visitor, which traverses those expressions.
                    let mut pattern_fuel = 4096;
                    let decisions: Option<Vec<_>> = arms
                        .iter()
                        .map(|arm| pattern_matches(&arm.pattern, value, &mut pattern_fuel, 0))
                        .collect();
                    if let Some(decisions) = decisions {
                        pending.push(value);
                        if !is_never(&value.ty) {
                            for (arm, matches) in arms.iter().zip(decisions) {
                                if !matches {
                                    continue;
                                }
                                if let Some(guard) = &arm.guard {
                                    pending.push(guard);
                                    if is_never(&guard.ty) {
                                        break;
                                    }
                                    match boolean(guard) {
                                        Some(false) => continue,
                                        Some(true) => {
                                            pending.push(&arm.body);
                                            break;
                                        }
                                        None => pending.push(&arm.body),
                                    }
                                } else {
                                    pending.push(&arm.body);
                                    break;
                                }
                            }
                        }
                        return Ok(false);
                    }
                }
                K::ScalarEqual(left, right)
                | K::TypeEqual(left, right)
                | K::BitEqual(left, right)
                | K::BitConcat(left, right)
                | K::IntegerBinary { left, right, .. }
                | K::FloatBinary { left, right, .. } => {
                    pending.push(left);
                    if !is_never(&left.ty) {
                        pending.push(right);
                    }
                    return Ok(false);
                }
                K::And(left, right) | K::Or(left, right) => {
                    pending.push(left);
                    let skip = if matches!(expr.kind, K::And(..)) {
                        Some(false)
                    } else {
                        Some(true)
                    };
                    if !is_never(&left.ty) && boolean(left) != skip {
                        pending.push(right);
                    }
                    return Ok(false);
                }
                _ => {}
            }
            Ok(true)
        })?;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal(value: bool) -> n::Expr {
        n::Expr {
            ty: p(P::Bool),
            source: n::SourceRange { start: 0, end: 0 },
            kind: K::Literal(n::Literal::Bool(value)),
        }
    }

    #[test]
    fn literal_classification_uses_runtime_float_equality_and_bounded_storage() {
        use n::Literal as L;
        let zero = L::Float64(0.0f64.to_bits());
        let negative_zero = L::Float64((-0.0f64).to_bits());
        let nan = L::Float64(f64::NAN.to_bits());
        assert_eq!(literal_matches(&zero, &negative_zero, &mut 100), Some(true));
        assert_eq!(literal_matches(&nan, &nan, &mut 100), Some(false));
        for (zero, negative_zero, nan) in [
            (L::Float16(0), L::Float16(0x8000), L::Float16(0x7e01)),
            (L::BFloat16(0), L::BFloat16(0x8000), L::BFloat16(0x7fc1)),
        ] {
            assert_eq!(literal_matches(&zero, &negative_zero, &mut 100), Some(true));
            assert_eq!(literal_matches(&nan, &nan, &mut 100), Some(false));
        }
        let text = L::String("hello".into());
        assert_eq!(literal_matches(&text, &text, &mut 4), None);
        assert_eq!(literal_matches(&text, &text, &mut 5), Some(true));
        let integer = L::Integer("01".into());
        assert_eq!(literal_matches(&integer, &integer, &mut 100), None);
    }

    #[test]
    fn structural_classification_keeps_unknown_fields_and_executable_patterns() {
        let value = n::Expr {
            ty: Term::Tuple(vec![p(P::Bool)]),
            source: literal(true).source,
            kind: K::Tuple(vec![literal(true)]),
        };
        let pattern = n::Pattern::Tuple(vec![n::Pattern::Literal(n::Literal::Bool(true))]);
        assert_eq!(pattern_matches(&pattern, &value, &mut 100, 0), Some(true));
        assert_eq!(pattern_matches(&pattern, &value, &mut 1, 0), None);
        let guarded = n::Pattern::Tuple(vec![n::Pattern::Guard {
            pattern: Box::new(n::Pattern::Wild),
            condition: Box::new(literal(true)),
        }]);
        assert_eq!(pattern_matches(&guarded, &value, &mut 100, 0), None);
        let mut unknown = value.clone();
        unknown.kind = K::Tuple(vec![n::Expr {
            kind: K::Local(n::LocalId { owner: 1, index: 0 }),
            ..literal(true)
        }]);
        assert_eq!(pattern_matches(&pattern, &unknown, &mut 100, 0), None);
    }

    #[test]
    fn boolean_budget_is_conservative_and_short_circuits() {
        let mut expr = literal(true);
        for _ in 0..65 {
            expr = n::Expr {
                ty: p(P::Bool),
                source: expr.source,
                kind: K::Not(Box::new(expr)),
            };
        }
        assert_eq!(boolean(&expr), None);
        let short = n::Expr {
            ty: p(P::Bool),
            source: expr.source,
            kind: K::And(Box::new(literal(false)), Box::new(expr)),
        };
        assert_eq!(boolean(&short), Some(false));
        assert_eq!(boolean_inner(&literal(true), 0, &mut 0), None);
        assert_eq!(boolean(&literal(true)), Some(true));
    }

    #[test]
    fn loop_exit_budget_does_not_turn_exhaustion_into_divergence() {
        let identity = n::LocalId { owner: 1, index: 2 };
        let body = n::Expr {
            ty: p(P::Never),
            source: n::SourceRange { start: 0, end: 0 },
            kind: K::Break(identity),
        };
        assert!(loop_may_complete(&body, identity, &mut 0).is_err());
        assert_eq!(loop_may_complete(&body, identity, &mut 10), Ok(true));
        assert_eq!(
            loop_may_complete(&body, n::LocalId { owner: 1, index: 3 }, &mut 10),
            Ok(false)
        );
    }

    #[test]
    fn divergent_match_subject_excludes_pattern_guard_and_arm_transfers() {
        let identity = n::LocalId { owner: 1, index: 2 };
        let node = |kind| n::Expr {
            ty: p(P::Never),
            source: n::SourceRange { start: 0, end: 0 },
            kind,
        };
        for (subject, breaks, continues) in [
            (node(K::Return(Some(Box::new(literal(true))))), false, false),
            (node(K::Break(identity)), true, false),
            (node(K::Continue(identity)), false, true),
        ] {
            // Executable patterns deliberately prevent literal classification.
            // Neither their effects nor the subsequent guard/body can run.
            for unreachable in [K::Break(identity), K::Continue(identity)] {
                let body = node(K::Match {
                    value: Box::new(subject.clone()),
                    arms: vec![n::MatchArm {
                        pattern: n::Pattern::Guard {
                            pattern: Box::new(n::Pattern::Wild),
                            condition: Box::new(node(unreachable.clone())),
                        },
                        guard: Some(node(unreachable.clone())),
                        body: node(unreachable),
                    }],
                });
                assert_eq!(loop_may_complete(&body, identity, &mut 100), Ok(breaks));
                assert_eq!(loop_may_continue(&body, identity, &mut 100), Ok(continues));
                assert!(loop_may_complete(&body, identity, &mut 0).is_err());
            }
        }
    }
}

/// Loop desugaring wraps condition tests, exits and scopes in one source span.
/// Give those synthetic nodes separate identities for runtime cleanup, keeping
/// the explicit shared boundary of a discarded Void expression intact.
pub fn freshen_loop_sites(
    body: &mut n::Expr,
    original: n::SourceRange,
    sources: &mut Vec<n::ExpansionSource>,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut shared_sites = BTreeSet::new();
    nir::visit::expression_mut_pruned(body, fuel, |node, fuel| {
        if node.source != original {
            return Ok(shared_sites.contains(&(node.source.start, node.source.end)));
        }
        let index = u32::try_from(sources.len()).map_err(|_| "loop source space exhausted")?;
        sources.push(n::ExpansionSource {
            original,
            site: original,
        });
        let fresh = n::SourceRange::generated(index);
        let mut current = node;
        loop {
            let shared_discard = current.shares_discard_temporary_boundary();
            current.source = fresh;
            if !shared_discard {
                break;
            }
            shared_sites.insert((fresh.start, fresh.end));
            *fuel = fuel
                .checked_sub(1)
                .ok_or("loop discard source budget exhausted")?;
            let K::Discard(value) = &mut current.kind else {
                unreachable!()
            };
            current = value;
        }
        Ok(true)
    })
}

#[cfg(test)]
mod synthetic_loop_sites_tests {
    use super::*;
    #[test]
    fn loop_sites_preserve_shared_discard_chains_and_source_provenance() {
        let original = n::SourceRange { start: 3, end: 40 };
        let node = |kind| n::Expr {
            source: original,
            ty: p(P::Void),
            kind,
        };
        let statement = node(K::Discard(Box::new(node(K::Discard(Box::new(node(
            K::Unit,
        )))))));
        let body = node(K::Block {
            statements: vec![statement],
            result: None,
        });
        assert!(freshen_loop_sites(&mut body.clone(), original, &mut vec![], &mut 1).is_err());
        let mut body = body;
        let mut sources = vec![];
        freshen_loop_sites(&mut body, original, &mut sources, &mut 1000).unwrap();
        let K::Block { statements, .. } = &body.kind else {
            unreachable!()
        };
        let first = &statements[0];
        let K::Discard(second) = &first.kind else {
            unreachable!()
        };
        let K::Discard(value) = &second.kind else {
            unreachable!()
        };
        assert_ne!(body.source, first.source);
        assert_eq!(first.source, second.source);
        assert_eq!(second.source, value.source);
        assert!(first.shares_discard_temporary_boundary());
        assert!(second.shares_discard_temporary_boundary());
        assert_eq!(sources.len(), 2);
        assert!(
            sources
                .iter()
                .all(|source| source.original == original && source.site == original)
        );
    }
}
