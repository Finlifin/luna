//! Syntactic match facts shared by ownership and memory analysis.
use nir::program::{Expr, ExprKind as K, Literal, Pattern};

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

pub(super) fn known_pattern(pattern: &Pattern, value: &Expr, depth: usize) -> Option<bool> {
    if depth > 128 {
        return None;
    }
    fn fields(patterns: &[Pattern], values: &[Expr], depth: usize) -> Option<bool> {
        if patterns.len() != values.len() {
            return Some(false);
        }
        let mut unknown = false;
        for (pattern, value) in patterns.iter().zip(values) {
            match known_pattern(pattern, value, depth + 1) {
                Some(false) => return Some(false),
                None => unknown = true,
                _ => (),
            }
        }
        if unknown { None } else { Some(true) }
    }
    match (pattern, &value.kind) {
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
        (Pattern::AnonymousRecord { names, fields: patterns }, K::AnonymousRecord { fields: values }) => {
            let nir::Term::Record(types) = &value.ty else { return Some(false); };
            if names.len() != types.len() || names.iter().zip(types).any(|(name,(expected,_))| name != expected) {
                Some(false)
            } else { fields(patterns, values, depth) }
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

pub(super) fn irrefutable_shape(pattern: &Pattern, depth: usize) -> bool {
    if depth > 128 {
        return false;
    }
    match pattern {
        Pattern::Wild | Pattern::Unit => true,
        Pattern::Binding { inner, .. } => irrefutable_shape(inner, depth + 1),
        Pattern::Tuple(fields) | Pattern::Record { fields, .. } | Pattern::AnonymousRecord { fields, .. } => {
            fields.iter().all(|p| irrefutable_shape(p, depth + 1))
        }
        _ => false,
    }
}
