//! Guard evidence and binding identities; no provider execution or PTC mutation.
use super::*;

// Evidence about an original retained input, conditional on a guard outcome.
// Unknown is a safe expression with no unique identity; None from the evaluator
// means an unsupported computation that may change the candidate binding.
#[derive(Clone, Copy)]
pub(in crate::program) enum PatternGuard<T> {
    Impossible,
    Unknown,
    Exact(T),
}

impl<T: PartialEq> PatternGuard<T> {
    fn both(self, other: Self) -> Self {
        match (self, other) {
            (Self::Impossible, _) | (_, Self::Impossible) => Self::Impossible,
            (Self::Exact(value), _) | (_, Self::Exact(value)) => Self::Exact(value),
            _ => Self::Unknown,
        }
    }
    fn either(self, other: Self) -> Self {
        match (self, other) {
            (Self::Impossible, other) | (other, Self::Impossible) => other,
            (Self::Exact(left), Self::Exact(right)) if left == right => Self::Exact(left),
            _ => Self::Unknown,
        }
    }
}

pub(in crate::program) fn pattern_guard_evidence<'a, T: PartialEq>(
    expression: &'a n::Expr,
    binding: n::LocalId,
    truth: bool,
    fuel: &mut u64,
    span: Span,
    depth: usize,
    atom: &mut impl FnMut(&'a n::Expr, bool, &mut u64) -> Result<Option<PatternGuard<T>>, Error>,
) -> Result<Option<PatternGuard<T>>, Error> {
    use PatternGuard::{Impossible, Unknown};
    if depth >= 128 {
        return Err(error(span, "type guard evidence depth exhausted"));
    }
    *fuel = fuel
        .checked_sub(1)
        .ok_or_else(|| error(span, "type guard evidence budget exhausted"))?;
    Ok(Some(match &expression.kind {
        K::Literal(n::Literal::Bool(value)) => {
            if *value == truth {
                Unknown
            } else {
                Impossible
            }
        }
        K::Local(_) => Unknown,
        K::Not(value) => {
            return pattern_guard_evidence(value, binding, !truth, fuel, span, depth + 1, atom);
        }
        K::And(left, right) | K::Or(left, right) => {
            // Only this finite, non-mutating subset may combine observations
            // of a binding across operands. Calls, statement blocks and writes remain
            // unsupported, even if one operand appears to identify the type.
            let Some(left) =
                pattern_guard_evidence(left, binding, truth, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(right) =
                pattern_guard_evidence(right, binding, truth, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let both = matches!(expression.kind, K::And(..)) == truth;
            if both {
                left.both(right)
            } else {
                left.either(right)
            }
        }
        K::ScalarEqual(left, right) if left.ty == p(P::Bool) && right.ty == p(P::Bool) => {
            // A literal side fixes the other outcome directly. Do not branch
            // twice over nested `== true` wrappers: their cost must be linear.
            if let K::Literal(n::Literal::Bool(value)) = left.kind {
                return pattern_guard_evidence(
                    right,
                    binding,
                    truth == value,
                    fuel,
                    span,
                    depth + 1,
                    atom,
                );
            }
            if let K::Literal(n::Literal::Bool(value)) = right.kind {
                return pattern_guard_evidence(
                    left,
                    binding,
                    truth == value,
                    fuel,
                    span,
                    depth + 1,
                    atom,
                );
            }
            let Some(left_true) =
                pattern_guard_evidence(left, binding, true, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(left_false) =
                pattern_guard_evidence(left, binding, false, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(right_for_true) =
                pattern_guard_evidence(right, binding, truth, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(right_for_false) =
                pattern_guard_evidence(right, binding, !truth, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            left_true
                .both(right_for_true)
                .either(left_false.both(right_for_false))
        }
        K::Block {
            statements,
            result: Some(result),
        } if statements.is_empty() => {
            return pattern_guard_evidence(result, binding, truth, fuel, span, depth + 1, atom);
        }
        K::If {
            condition,
            then_branch,
            else_branch: Some(else_branch),
        } => {
            let Some(yes_condition) =
                pattern_guard_evidence(condition, binding, true, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(no_condition) =
                pattern_guard_evidence(condition, binding, false, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(yes) =
                pattern_guard_evidence(then_branch, binding, truth, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(no) =
                pattern_guard_evidence(else_branch, binding, truth, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            yes_condition.both(yes).either(no_condition.both(no))
        }
        _ => {
            if let Some(evidence) = atom(expression, truth, fuel)? {
                return Ok(Some(evidence));
            }
            if readonly_guard_comparison(expression, fuel, span)? {
                Unknown
            } else {
                return Ok(None);
            }
        }
    }))
}

// A comparison may constrain a different input/domain than the identity
// currently being recovered. Keep its outcome unknown, but only after proving
// structurally that its operands cannot modify the candidate or call code.
pub(in crate::program) fn readonly_guard_comparison(
    expr: &n::Expr,
    fuel: &mut u64,
    span: Span,
) -> Result<bool, Error> {
    let (left, right) = match &expr.kind {
        K::TypeEqual(left, right)
        | K::ScalarEqual(left, right)
        | K::ScalarLess(left, right)
        | K::ScalarLessEqual(left, right)
        | K::ScalarGreater(left, right)
        | K::ScalarGreaterEqual(left, right)
        | K::BitEqual(left, right) => (left, right),
        K::IntegerBinary { op, left, right }
            if matches!(
                op,
                n::IntegerOp::Equal
                    | n::IntegerOp::NotEqual
                    | n::IntegerOp::Less
                    | n::IntegerOp::LessEqual
                    | n::IntegerOp::Greater
                    | n::IntegerOp::GreaterEqual
            ) =>
        {
            (left, right)
        }
        _ => return Ok(false),
    };
    Ok(readonly_guard_operand(left, fuel, span)? && readonly_guard_operand(right, fuel, span)?)
}

pub(in crate::program) fn readonly_guard_operand(
    expr: &n::Expr,
    fuel: &mut u64,
    span: Span,
) -> Result<bool, Error> {
    let mut pending = vec![(expr, 0usize)];
    while let Some((expr, depth)) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(span, "guard observation budget exhausted"))?;
        if depth >= 128 {
            return Err(error(span, "guard observation depth exhausted"));
        }
        match &expr.kind {
            K::Local(_) | K::Literal(_) | K::TypeValue(_) => {}
            K::IntegerNegate(value)
            | K::FloatNegate(value)
            | K::Not(value)
            | K::BitConvert(value)
            | K::TakeBits(value)
            | K::TextQuery { subject: value, .. }
            | K::IntegerRepresentation(value)
            | K::ComptimeIntegerConvert(value)
            | K::SymbolName(value)
            | K::BitSlice { value, .. } => pending.push((value, depth + 1)),
            K::IntegerBinary { left, right, .. }
            | K::FloatBinary { left, right, .. }
            | K::And(left, right)
            | K::Or(left, right)
            | K::BitConcat(left, right)
            | K::BitEqual(left, right)
            | K::TypeEqual(left, right)
            | K::ScalarEqual(left, right)
            | K::ScalarLess(left, right)
            | K::ScalarLessEqual(left, right)
            | K::ScalarGreater(left, right)
            | K::ScalarGreaterEqual(left, right) => {
                pending.push((right, depth + 1));
                pending.push((left, depth + 1));
            }
            K::Block {
                statements,
                result: Some(value),
            } if statements.is_empty() => pending.push((value, depth + 1)),
            K::If {
                condition,
                then_branch,
                else_branch: Some(no),
            } => {
                pending.push((no, depth + 1));
                pending.push((then_branch, depth + 1));
                pending.push((condition, depth + 1));
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

type PatternTypeGuard<'a> = PatternGuard<&'a Term>;

fn pattern_type_guard<'a>(
    expression: &'a n::Expr,
    binding: n::LocalId,
    truth: bool,
    fuel: &mut u64,
    span: Span,
    depth: usize,
) -> Result<Option<PatternTypeGuard<'a>>, Error> {
    use PatternGuard::{Exact, Unknown};
    pattern_guard_evidence(
        expression,
        binding,
        truth,
        fuel,
        span,
        depth,
        &mut |expression, truth, _| {
            Ok(Some(match &expression.kind {
                K::TypeEqual(left, right)
                    if matches!(left.kind, K::Local(_) | K::TypeValue(_))
                        && matches!(right.kind, K::Local(_) | K::TypeValue(_)) =>
                {
                    if truth {
                        match (&left.kind, &right.kind) {
                            (K::Local(id), K::TypeValue(ty)) | (K::TypeValue(ty), K::Local(id))
                                if *id == binding =>
                            {
                                Exact(ty)
                            }
                            _ => Unknown,
                        }
                    } else {
                        Unknown
                    }
                }
                _ => return Ok(None),
            }))
        },
    )
}

/// A successful pattern fixes its subject to this exact Type value. Wrappers
/// preserve that fact; an alternative contributes only when every arm agrees.
pub(in crate::program) fn matched_type_pattern_value<'a>(
    pattern: &'a n::Pattern,
    fuel: &mut u64,
    span: Span,
    depth: usize,
) -> Result<Option<&'a Term>, Error> {
    if depth >= 128 {
        return Err(error(span, "type pattern evidence depth exhausted"));
    }
    *fuel = fuel
        .checked_sub(1)
        .ok_or_else(|| error(span, "type pattern evidence budget exhausted"))?;
    match pattern {
        n::Pattern::Binding { inner, .. } | n::Pattern::Remap { pattern: inner, .. } => {
            matched_type_pattern_value(inner, fuel, span, depth + 1)
        }
        n::Pattern::Guard { pattern, condition } => {
            if let Some(ty) = matched_type_pattern_value(pattern, fuel, span, depth + 1)? {
                return Ok(Some(ty));
            }
            if let n::Pattern::Binding { binding, inner } = pattern.as_ref() {
                if matches!(inner.as_ref(), n::Pattern::Wild) {
                    if let Some(PatternTypeGuard::Exact(ty)) =
                        pattern_type_guard(condition, *binding, true, fuel, span, depth + 1)?
                    {
                        return Ok(Some(ty));
                    }
                }
            }
            Ok(None)
        }
        n::Pattern::Then { first, value, next } => {
            if let Some(ty) = matched_type_pattern_value(first, fuel, span, depth + 1)? {
                return Ok(Some(ty));
            }
            // A bare binding followed by observing that same binding still
            // observes the original subject. Do not transfer facts from an
            // unrelated expression or through effectful intermediate guards.
            if let (n::Pattern::Binding { binding, inner }, K::Local(id)) =
                (first.as_ref(), &value.kind)
            {
                if *binding == *id && matches!(inner.as_ref(), n::Pattern::Wild) {
                    return matched_type_pattern_value(next, fuel, span, depth + 1);
                }
            }
            Ok(None)
        }
        n::Pattern::Or(arms) => {
            let mut common = None;
            for arm in arms {
                let Some(ty) = matched_type_pattern_value(arm, fuel, span, depth + 1)? else {
                    return Ok(None);
                };
                if common.is_some_and(|value| value != ty) {
                    return Ok(None);
                }
                common = Some(ty);
            }
            Ok(common)
        }
        _ => Ok(None),
    }
}

// Pure Boolean patterns can be complemented without executing a guard or view.
// The resulting ordinary pattern shares coverage and matching with every phase.
pub(in crate::program) fn boolean_pattern_coverage(pattern: &n::Pattern) -> Option<u8> {
    let mut pending = vec![pattern];
    let mut mask = 0;
    let mut fuel = 100_000usize;
    while let Some(pattern) = pending.pop() {
        fuel = fuel.checked_sub(1)?;
        match pattern {
            n::Pattern::Wild => mask |= 3,
            n::Pattern::Literal(n::Literal::Bool(value)) => mask |= if *value { 2 } else { 1 },
            n::Pattern::Binding { inner, .. } => pending.push(inner),
            n::Pattern::Remap { pattern, .. } => pending.push(pattern),
            n::Pattern::Or(patterns) => {
                if patterns.len() > fuel {
                    return None;
                }
                pending.extend(patterns);
            }
            _ => return None,
        }
    }
    Some(mask)
}

pub(in crate::program) fn pattern_binding_ids(
    pattern: &hir::pattern::Pattern<'_>,
) -> Vec<n::LocalId> {
    use hir::pattern::PatternKind;
    let mut pending = vec![pattern];
    let mut bindings = Vec::new();
    while let Some(pattern) = pending.pop() {
        match &pattern.kind {
            PatternKind::Binding(_, _, inner) => {
                bindings.push(local(pattern.hir_id));
                if let Some(inner) = inner {
                    pending.push(inner);
                }
            }
            PatternKind::Then(first, _, next) => {
                pending.push(first);
                pending.push(next);
            }
            PatternKind::BitVec { segments, .. } => {
                for segment in segments {
                    if let hir::pattern::BitSegment::Typed { pattern, .. } = segment {
                        pending.push(pattern);
                    }
                }
            }
            PatternKind::ErrorOk(pattern)
            | PatternKind::ErrorErr(pattern)
            | PatternKind::Guard(pattern, _)
            | PatternKind::OptionSome(pattern) => pending.push(pattern),
            PatternKind::NormalForm(_, arguments) => {
                pending.extend(arguments.iter().map(|argument| &argument.pattern))
            }
            PatternKind::Tuple(fields)
            | PatternKind::List(fields, _)
            | PatternKind::AppTuple(_, fields)
            | PatternKind::Or(fields) => pending.extend(fields.iter()),
            PatternKind::Struct(_, fields, _) => {
                pending.extend(fields.iter().map(|field| &field.pat))
            }
            _ => {}
        }
    }
    bindings
}

pub(in crate::program) fn pattern_bindings(
    pattern: &hir::pattern::Pattern<'_>,
) -> Result<BTreeMap<String, n::LocalId>, Error> {
    use hir::pattern::PatternKind;
    let mut result = BTreeMap::new();
    match &pattern.kind {
        PatternKind::CodeSplice(_, outputs) => {
            for output in outputs {
                if result
                    .insert(output.name.name.to_string(), local(output.hir_id))
                    .is_some()
                {
                    return Err(error(pattern.span, "duplicate pattern binding"));
                }
            }
        }
        PatternKind::Then(first, _, next) => {
            result = pattern_bindings(first)?;
            for (name, id) in pattern_bindings(next)? {
                if result.insert(name, id).is_some() {
                    return Err(error(pattern.span, "duplicate pattern binding"));
                }
            }
        }
        PatternKind::BitVec { segments, .. } => {
            for segment in segments {
                if let hir::pattern::BitSegment::Typed { pattern: inner, .. } = segment {
                    for (name, id) in pattern_bindings(inner)? {
                        if result.insert(name, id).is_some() {
                            return Err(error(pattern.span, "duplicate pattern binding"));
                        }
                    }
                }
            }
        }
        PatternKind::ErrorOk(pattern)
        | PatternKind::ErrorErr(pattern)
        | PatternKind::Guard(pattern, _)
        | PatternKind::OptionSome(pattern) => result = pattern_bindings(pattern)?,
        PatternKind::Binding(_, name, inner) => {
            if let Some(inner) = inner {
                result = pattern_bindings(inner)?;
            }
            if result
                .insert(name.name.to_string(), local(pattern.hir_id))
                .is_some()
            {
                return Err(error(pattern.span, "duplicate pattern binding"));
            }
        }
        PatternKind::Struct(_, fields, _) => {
            for field in *fields {
                for (name, id) in pattern_bindings(&field.pat)? {
                    if result.insert(name, id).is_some() {
                        return Err(error(field.span, "duplicate pattern binding"));
                    }
                }
            }
        }
        PatternKind::NormalForm(_, arguments) => {
            for argument in *arguments {
                for (name, id) in pattern_bindings(&argument.pattern)? {
                    if result.insert(name, id).is_some() {
                        return Err(error(argument.pattern.span, "duplicate pattern binding"));
                    }
                }
            }
        }
        PatternKind::Tuple(fields)
        | PatternKind::List(fields, _)
        | PatternKind::AppTuple(_, fields) => {
            for field in *fields {
                for (name, id) in pattern_bindings(field)? {
                    if result.insert(name, id).is_some() {
                        return Err(error(field.span, "duplicate pattern binding"));
                    }
                }
            }
        }
        PatternKind::Or(alternatives) => {
            if let Some(first) = alternatives.first() {
                result = pattern_bindings(first)?;
            }
        }
        _ => {}
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
