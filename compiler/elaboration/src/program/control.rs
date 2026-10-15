//! Conservative normal-completion checks for structured control flow.
use super::*;

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
pub(super) fn boolean(expr: &n::Expr) -> Option<bool> {
    boolean_inner(expr, 0, &mut 4096)
}

/// Both source if and ordered when lowering use the same completion rule.
pub(super) fn conditional_never(condition: &n::Expr, yes: &Term, no: &Term) -> bool {
    is_never(&condition.ty)
        || match boolean(condition) {
            Some(true) => is_never(yes),
            Some(false) => is_never(no),
            None => is_never(yes) && is_never(no),
        }
}

// Only inert patterns are classified. Executable guards/and-is patterns
// remain on the full visitor path, including their control transfers.
fn pattern_matches(
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
pub(super) fn loop_may_complete(
    body: &n::Expr,
    identity: n::LocalId,
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
                K::Break(target) if *target == identity => {
                    found = true;
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
}
