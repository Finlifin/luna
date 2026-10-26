//! Syntactic match facts shared by ownership and memory analysis.
use nir::program::{Expr, ExprKind as K, Literal, Pattern};

/// A syntactic, budgeted proof that evaluation cannot reach the following
/// statement. This says nothing about rejection, ownership or loop completion.
pub(super) fn cannot_complete(expr: &Expr, fuel: &mut usize, depth: usize) -> Result<bool, String> {
    if depth >= 128 {
        return Err("control-flow observation nesting budget exhausted".into());
    }
    *fuel = fuel
        .checked_sub(1)
        .ok_or("control-flow observation budget exhausted")?;
    if expr.ty == nir::Term::Primitive(nir::Primitive::Never) {
        return Ok(true);
    }
    match &expr.kind {
        K::Return(_) | K::Break(_) | K::Continue(_) => Ok(true),
        K::UnsafeScope(value)
        | K::Discard(value)
        | K::Let { value, .. }
        | K::Assign { value, .. } => cannot_complete(value, fuel, depth + 1),
        K::Block { statements, result } | K::CallPreparation { statements, result } => {
            for statement in statements {
                if cannot_complete(statement, fuel, depth + 1)? {
                    return Ok(true);
                }
            }
            result
                .as_deref()
                .map(|value| cannot_complete(value, fuel, depth + 1))
                .unwrap_or(Ok(false))
        }
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            if cannot_complete(condition, fuel, depth + 1)? {
                return Ok(true);
            }
            match constant_bool(condition, 0) {
                Some(true) => cannot_complete(then_branch, fuel, depth + 1),
                Some(false) => else_branch
                    .as_deref()
                    .map(|value| cannot_complete(value, fuel, depth + 1))
                    .unwrap_or(Ok(false)),
                None => Ok(cannot_complete(then_branch, fuel, depth + 1)?
                    && else_branch
                        .as_deref()
                        .map(|value| cannot_complete(value, fuel, depth + 1))
                        .unwrap_or(Ok(false))?),
            }
        }
        _ => Ok(false),
    }
}

pub(super) fn constant_bool(expr: &Expr, depth: usize) -> Option<bool> {
    if depth > 128 {
        return None;
    }
    match &expr.kind {
        K::Literal(Literal::Bool(value)) => Some(*value),
        K::Not(value) => Some(!constant_bool(value, depth + 1)?),
        K::And(left, right) => {
            Some(constant_bool(left, depth + 1)? && constant_bool(right, depth + 1)?)
        }
        K::Or(left, right) => {
            Some(constant_bool(left, depth + 1)? || constant_bool(right, depth + 1)?)
        }
        _ => None,
    }
}

pub(super) fn pattern_observes(pattern: &Pattern, depth: usize) -> bool {
    if depth > 128 {
        return true;
    }
    match pattern {
        Pattern::CodeEvidence { .. } => true,
        Pattern::Observe { .. } | Pattern::Then { .. } | Pattern::Guard { .. } => true,
        Pattern::OptionalPresent(inner)
        | Pattern::ErrorResult { inner, .. }
        | Pattern::Binding { inner, .. }
        | Pattern::Remap { pattern: inner, .. } => pattern_observes(inner, depth + 1),
        Pattern::Tuple(fields)
        | Pattern::TypeTuple(fields)
        | Pattern::Or(fields)
        | Pattern::NominalType { fields, .. }
        | Pattern::Record { fields, .. }
        | Pattern::AnonymousRecord { fields, .. }
        | Pattern::Variant { fields, .. }
        | Pattern::NormalForm {
            arguments: fields, ..
        } => fields
            .iter()
            .any(|field| pattern_observes(field, depth + 1)),
        Pattern::OptionalAbsent | Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => false,
    }
}

pub(super) fn known_pattern(pattern: &Pattern, value: &Expr, depth: usize) -> Option<bool> {
    if depth > 128 {
        return None;
    }
    fn fields(patterns: &[Pattern], values: &[Expr], depth: usize) -> Option<bool> {
        if patterns.len() != values.len() {
            return Some(false);
        }
        let mut unknown = false;
        let mut observed_unknown = false;
        for (pattern, value) in patterns.iter().zip(values) {
            match known_pattern(pattern, value, depth + 1) {
                // A preceding unknown pattern may execute a computed view or
                // guard. A later mismatch cannot authorize skipping that work.
                Some(false) => return if observed_unknown { None } else { Some(false) },
                None => {
                    unknown = true;
                    observed_unknown |= pattern_observes(pattern, depth + 1);
                }
                _ => (),
            }
        }
        if unknown { None } else { Some(true) }
    }
    match (pattern, &value.kind) {
        (Pattern::OptionalAbsent, K::OptionalAbsent) => Some(true),
        (Pattern::OptionalAbsent, K::OptionalPresent(_))
        | (Pattern::OptionalPresent(_), K::OptionalAbsent) => Some(false),
        (Pattern::OptionalPresent(inner), K::OptionalPresent(payload)) => {
            known_pattern(inner, payload, depth + 1)
        }
        (Pattern::Wild, _) => Some(true),
        (Pattern::Remap { pattern, .. }, _) => known_pattern(pattern, value, depth + 1),
        (Pattern::Or(alternatives), _) => {
            let mut unknown = false;
            for alternative in alternatives {
                match known_pattern(alternative, value, depth + 1) {
                    Some(true) => return Some(true),
                    None => unknown = true,
                    Some(false) => (),
                }
            }
            if unknown { None } else { Some(false) }
        }
        (Pattern::Binding { inner, .. }, _) => known_pattern(inner, value, depth + 1),
        (Pattern::Literal(Literal::Bool(expected)), _) => {
            constant_bool(value, depth + 1).map(|actual| actual == *expected)
        }
        (Pattern::Unit, K::Unit) => Some(true),
        (Pattern::Tuple(patterns), K::Tuple(values)) => fields(patterns, values, depth),
        (
            Pattern::AnonymousRecord {
                names,
                fields: patterns,
            },
            K::AnonymousRecord { fields: values },
        ) => {
            let nir::Term::Record(types) = &value.ty else {
                return Some(false);
            };
            if names.len() != types.len()
                || names
                    .iter()
                    .zip(types)
                    .any(|(name, (expected, _))| name != expected)
            {
                Some(false)
            } else {
                fields(patterns, values, depth)
            }
        }
        (
            Pattern::Record {
                declaration,
                fields: patterns,
            },
            K::Record {
                declaration: actual,
                fields: values,
            },
        ) => {
            if declaration != actual {
                Some(false)
            } else {
                fields(patterns, values, depth)
            }
        }
        (
            Pattern::Variant {
                declaration,
                index,
                fields: patterns,
            },
            K::Variant {
                declaration: actual,
                index: tag,
                fields: values,
            },
        ) => {
            if declaration != actual || index != tag {
                Some(false)
            } else {
                fields(patterns, values, depth)
            }
        }
        _ => None,
    }
}

pub(super) fn outer_guards(mut pattern: &Pattern) -> (&Pattern, Vec<&Expr>) {
    let mut guards = vec![];
    while let Pattern::Guard {
        pattern: inner,
        condition,
    } = pattern
    {
        guards.push(condition.as_ref());
        pattern = inner;
    }
    guards.reverse();
    (pattern, guards)
}

pub(super) fn bounded_outer_guards<'a>(
    mut pattern: &'a Pattern,
    fuel: &mut usize,
) -> Result<(&'a Pattern, Vec<&'a Expr>), String> {
    let mut guards = vec![];
    while let Pattern::Guard {
        pattern: inner,
        condition,
    } = pattern
    {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("pattern guard budget exhausted")?;
        guards.push(condition.as_ref());
        pattern = inner;
    }
    guards.reverse();
    Ok((pattern, guards))
}

pub(super) fn irrefutable_shape(pattern: &Pattern, depth: usize) -> bool {
    if depth > 128 {
        return false;
    }
    match pattern {
        Pattern::Wild | Pattern::Unit => true,
        Pattern::Binding { inner, .. } => irrefutable_shape(inner, depth + 1),
        Pattern::Tuple(fields)
        | Pattern::Record { fields, .. }
        | Pattern::AnonymousRecord { fields, .. } => {
            fields.iter().all(|p| irrefutable_shape(p, depth + 1))
        }
        _ => false,
    }
}

#[cfg(test)]
mod optional_tests {
    use super::*;

    #[test]
    fn optional_tests_are_refutable_even_when_their_payload_is_wild() {
        assert!(!irrefutable_shape(&Pattern::OptionalAbsent, 0));
        assert!(!irrefutable_shape(
            &Pattern::OptionalPresent(Box::new(Pattern::Wild)),
            0
        ));
        assert!(irrefutable_shape(&Pattern::Wild, 0));
    }
}
