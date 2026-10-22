//! Finite equality partitions of the open Symbol domain, including free runtime
//! symbols. Fresh atoms distinguish unnamed values; no program is invoked.
use nir::program::{Expr, ExprKind as K, Literal, LocalId, Pattern};
use nir::{Primitive, Term};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

fn spend(fuel: &mut usize, work: usize) -> Result<(), ()> {
    *fuel = fuel.checked_sub(work).ok_or(())?;
    Ok(())
}
fn labels<'a>(
    pattern: &'a Pattern,
    out: &mut BTreeSet<&'a str>,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), ()> {
    spend(fuel, 1)?;
    if depth >= 128 {
        return Err(());
    }
    match pattern {
        Pattern::Literal(Literal::Symbol(name)) => {
            spend(fuel, name.len().saturating_mul(out.len().saturating_add(1)))?;
            out.insert(name);
        }
        Pattern::Binding { inner, .. }
        | Pattern::Observe { inner, .. }
        | Pattern::Remap { pattern: inner, .. } => labels(inner, out, fuel, depth + 1)?,
        Pattern::Guard { pattern, condition } => {
            labels(pattern, out, fuel, depth + 1)?;
            expression_labels(condition, out, fuel, depth + 1)?;
        }
        Pattern::Then { first, value, next } => {
            labels(first, out, fuel, depth + 1)?;
            expression_labels(value, out, fuel, depth + 1)?;
            labels(next, out, fuel, depth + 1)?;
        }
        Pattern::Or(fields)
        | Pattern::Tuple(fields)
        | Pattern::AnonymousRecord { fields, .. }
        | Pattern::Record { fields, .. }
        | Pattern::Variant { fields, .. } => {
            for field in fields {
                labels(field, out, fuel, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn expression_labels<'a>(
    expr: &'a Expr,
    out: &mut BTreeSet<&'a str>,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), ()> {
    if depth >= 128 {
        return Err(());
    }
    nir::visit::expression(expr, fuel, |expr, fuel| {
        let name = match &expr.kind {
            K::Literal(Literal::Symbol(name)) => Some(name.as_str()),
            K::ComptimeValue(Term::Symbol(name)) => Some(name.as_ref()),
            _ => None,
        };
        if let Some(name) = name {
            spend(fuel, name.len().saturating_mul(out.len().saturating_add(1)))
                .map_err(|_| "symbol label budget".to_string())?;
            out.insert(name);
        }
        if let K::Match { arms, .. } = &expr.kind {
            for arm in arms {
                labels(&arm.pattern, out, fuel, depth + 1)
                    .map_err(|_| "symbol label budget".to_string())?;
            }
        }
        Ok(())
    })
    .map_err(|_| ())
}

#[cfg(test)]
pub(crate) fn cells<'a>(
    patterns: impl IntoIterator<Item = &'a Pattern>,
    fuel: &mut usize,
) -> Result<Vec<Option<&'a str>>, ()> {
    let mut names = BTreeSet::new();
    for pattern in patterns {
        labels(pattern, &mut names, fuel, 0)?;
    }
    spend(fuel, names.len().saturating_add(1))?;
    Ok(std::iter::once(None)
        .chain(names.into_iter().map(Some))
        .collect())
}

/// Equality partitions quantify external runtime symbols as well as the subject.
/// Fresh atoms represent distinct names outside the finite literal vocabulary.
pub(crate) struct Case<'a> {
    subject: Atom<'a>,
    env: Environment<'a>,
}
pub(crate) fn cases<'a>(
    patterns: impl IntoIterator<Item = &'a Pattern>,
    fuel: &mut usize,
) -> Result<Vec<Case<'a>>, ()> {
    let mut names = BTreeSet::new();
    let mut inputs = BTreeSet::new();
    let mut booleans = BTreeSet::new();
    for pattern in patterns {
        labels(pattern, &mut names, fuel, 0)?;
        let free = nir::free_bindings::collect_pattern(pattern, fuel).map_err(|_| ())?;
        for observation in nir::visit::pattern_observations(pattern, fuel).map_err(|_| ())? {
            nir::visit::expression(observation, fuel, |expr, _| {
                if let K::Local(id) = &expr.kind {
                    if expr.ty == Term::Primitive(Primitive::Symbol) && free.contains(id) {
                        inputs.insert(*id);
                    } else if expr.ty == Term::Primitive(Primitive::Bool) && free.contains(id) {
                        booleans.insert(*id);
                    }
                }
                Ok(())
            })
            .map_err(|_| ())?;
        }
    }
    if inputs.len() >= 128 {
        return Err(());
    }
    let names: Vec<_> = names.into_iter().collect();
    let inputs: Vec<_> = inputs.into_iter().collect();
    let mut out = Vec::new();
    enumerate(&names, &inputs, &mut Vec::new(), 0, &mut out, fuel)?;
    for binding in booleans {
        let mut expanded = Vec::new();
        for mut case in out {
            spend(fuel, 2)?;
            let mut other = snapshot(&case.env, fuel)?;
            other.insert(binding, Value::Bool(false));
            case.env.insert(binding, Value::Bool(true));
            expanded.push(Case {
                subject: case.subject,
                env: other,
            });
            expanded.push(case);
        }
        out = expanded;
    }
    Ok(out)
}
fn enumerate<'a>(
    names: &[&'a str],
    inputs: &[LocalId],
    values: &mut Vec<Atom<'a>>,
    fresh: usize,
    out: &mut Vec<Case<'a>>,
    fuel: &mut usize,
) -> Result<(), ()> {
    spend(fuel, 1)?;
    if values.len() == inputs.len() + 1 {
        spend(fuel, values.len())?;
        out.push(Case {
            subject: values[0],
            env: inputs
                .iter()
                .copied()
                .zip(values[1..].iter().copied().map(Value::Symbol))
                .collect(),
        });
        return Ok(());
    }
    for atom in names
        .iter()
        .copied()
        .map(Atom::Named)
        .chain((0..=fresh).map(Atom::Fresh))
    {
        values.push(atom);
        let next = if atom == Atom::Fresh(fresh) {
            fresh + 1
        } else {
            fresh
        };
        enumerate(names, inputs, values, next, out, fuel)?;
        values.pop();
    }
    Ok(())
}
pub(crate) fn covers_case<'a>(
    pattern: &'a Pattern,
    case: &Case<'a>,
    fuel: &mut usize,
) -> Result<bool, ()> {
    let mut env = snapshot(&case.env, fuel)?;
    Ok(matches!(
        matches(pattern, Value::Symbol(case.subject), &mut env, fuel, 0)?,
        Some(true)
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Atom<'a> {
    Named(&'a str),
    Fresh(usize),
}
#[derive(Clone)]
enum Value<'a> {
    Symbol(Atom<'a>),
    Bool(bool),
    Unit,
    Tuple(Rc<[Value<'a>]>),
    Record(&'a [(String, Term)], Rc<[Value<'a>]>),
    Nominal(nir::DeclarationId, Option<usize>, Rc<[Value<'a>]>),
}
type Environment<'a> = BTreeMap<LocalId, Value<'a>>;
fn snapshot<'a>(env: &Environment<'a>, fuel: &mut usize) -> Result<Environment<'a>, ()> {
    spend(fuel, env.len())?;
    Ok(env.clone())
}
/// Rebuild only the modified path. Aliases retain their immutable old values.
fn assigned<'a>(
    old: Value<'a>,
    path: &[(nir::program::FieldOwner, usize)],
    value: Value<'a>,
    fuel: &mut usize,
    depth: usize,
) -> Result<Option<Value<'a>>, ()> {
    spend(fuel, 1)?;
    if depth >= 128 {
        return Err(());
    }
    let Some(((owner, index), rest)) = path.split_first() else {
        return Ok(Some(value));
    };
    let (nir::program::FieldOwner::Anonymous, Value::Record(layout, fields)) = (owner, old) else {
        return Ok(None);
    };
    let Some(field) = fields.get(*index) else {
        return Ok(None);
    };
    let Some(replacement) = assigned(field.clone(), rest, value, fuel, depth + 1)? else {
        return Ok(None);
    };
    spend(fuel, fields.len())?;
    let mut updated = fields.to_vec();
    updated[*index] = replacement;
    Ok(Some(Value::Record(layout, updated.into())))
}
macro_rules! known {
    ($value:expr) => {
        match $value? {
            Some(value) => value,
            None => return Ok(None),
        }
    };
}
fn boolean(value: Value<'_>) -> Option<bool> {
    if let Value::Bool(value) = value {
        Some(value)
    } else {
        None
    }
}
fn literal(value: &Literal) -> Option<Value<'_>> {
    match value {
        Literal::Symbol(name) => Some(Value::Symbol(Atom::Named(name))),
        Literal::Bool(value) => Some(Value::Bool(*value)),
        _ => None,
    }
}
fn equal(left: Value<'_>, right: Value<'_>, fuel: &mut usize) -> Result<bool, ()> {
    if let Value::Symbol(Atom::Named(name)) = &left {
        spend(fuel, name.len())?;
    }
    if let Value::Symbol(Atom::Named(name)) = &right {
        spend(fuel, name.len())?;
    }
    match (left, right) {
        (Value::Symbol(a), Value::Symbol(b)) => Ok(a == b),
        (Value::Bool(a), Value::Bool(b)) => Ok(a == b),
        (Value::Unit, Value::Unit) => Ok(true),
        (Value::Tuple(_) | Value::Record(..) | Value::Nominal(..), _)
        | (_, Value::Tuple(_) | Value::Record(..) | Value::Nominal(..)) => Err(()),
        _ => Ok(false),
    }
}

#[cfg(test)]
pub(crate) fn covers(pattern: &Pattern, cell: Option<&str>, fuel: &mut usize) -> Result<bool, ()> {
    Ok(matches!(
        matches(
            pattern,
            Value::Symbol(cell.map_or(Atom::Fresh(0), Atom::Named)),
            &mut BTreeMap::new(),
            fuel,
            0
        )?,
        Some(true)
    ))
}
fn matches<'a>(
    pattern: &'a Pattern,
    value: Value<'a>,
    env: &mut Environment<'a>,
    fuel: &mut usize,
    depth: usize,
) -> Result<Option<bool>, ()> {
    spend(fuel, 1)?;
    if depth >= 128 {
        return Err(());
    }
    match pattern {
        Pattern::Record {
            declaration,
            fields,
        }
        | Pattern::Variant {
            declaration,
            fields,
            ..
        } => {
            let Value::Nominal(actual, variant, values) = value else {
                return Ok(None);
            };
            if *declaration != actual {
                return Ok(None);
            }
            let expected = match pattern {
                Pattern::Variant { index, .. } => Some(*index),
                _ => None,
            };
            if variant != expected {
                return Ok(Some(false));
            }
            if fields.len() != values.len() {
                return Ok(None);
            }
            spend(fuel, fields.len())?;
            for (field, value) in fields.iter().zip(values.iter()) {
                if !known!(matches(field, value.clone(), env, fuel, depth + 1)) {
                    return Ok(Some(false));
                }
            }
            Ok(Some(true))
        }
        Pattern::AnonymousRecord { names, fields } => {
            let Value::Record(layout, values) = value else {
                return Ok(None);
            };
            if names.len() != layout.len() || fields.len() != values.len() {
                return Ok(None);
            }
            spend(fuel, names.len())?;
            for (name, (expected, _)) in names.iter().zip(layout) {
                spend(fuel, name.len().saturating_add(expected.len()))?;
                if name != expected {
                    return Ok(None);
                }
            }
            for (field, value) in fields.iter().zip(values.iter()) {
                if !known!(matches(field, value.clone(), env, fuel, depth + 1)) {
                    return Ok(Some(false));
                }
            }
            Ok(Some(true))
        }
        Pattern::Tuple(fields) => {
            let Value::Tuple(values) = value else {
                return Ok(None);
            };
            if fields.len() != values.len() {
                return Ok(Some(false));
            }
            spend(fuel, fields.len())?;
            for (field, value) in fields.iter().zip(values.iter()) {
                if !known!(matches(field, value.clone(), env, fuel, depth + 1)) {
                    return Ok(Some(false));
                }
            }
            Ok(Some(true))
        }
        Pattern::Wild => Ok(Some(true)),
        Pattern::Unit => Ok(Some(matches!(value, Value::Unit))),
        Pattern::Literal(expected) => match literal(expected) {
            Some(expected) => Ok(Some(equal(value, expected, fuel)?)),
            None => Ok(None),
        },
        Pattern::Binding { binding, inner } => {
            if !known!(matches(inner, value.clone(), env, fuel, depth + 1)) {
                return Ok(Some(false));
            }
            env.insert(*binding, value);
            Ok(Some(true))
        }
        Pattern::Observe { binding, inner } => {
            let old = env.insert(*binding, value.clone());
            let result = matches(inner, value.clone(), env, fuel, depth + 1);
            if let Some(old) = old {
                env.insert(*binding, old);
            } else {
                env.remove(binding);
            }
            result
        }
        Pattern::Remap { pattern, bindings } => {
            if !known!(matches(pattern, value, env, fuel, depth + 1)) {
                return Ok(Some(false));
            }
            let original = snapshot(env, fuel)?;
            for (from, to) in bindings {
                spend(fuel, 1)?;
                let Some(value) = original.get(from) else {
                    return Ok(None);
                };
                env.insert(*to, value.clone());
            }
            Ok(Some(true))
        }
        Pattern::Or(patterns) => {
            for pattern in patterns {
                let mut candidate = snapshot(env, fuel)?;
                if known!(matches(
                    pattern,
                    value.clone(),
                    &mut candidate,
                    fuel,
                    depth + 1
                )) {
                    *env = candidate;
                    return Ok(Some(true));
                }
            }
            Ok(Some(false))
        }
        Pattern::Guard { pattern, condition } => {
            if !known!(matches(pattern, value, env, fuel, depth + 1)) {
                return Ok(Some(false));
            }
            Ok(boolean(known!(expression(condition, env, fuel, depth + 1))))
        }
        Pattern::Then {
            first,
            value: computed,
            next,
        } => {
            if !known!(matches(first, value, env, fuel, depth + 1)) {
                return Ok(Some(false));
            }
            let computed = known!(expression(computed, env, fuel, depth + 1));
            matches(next, computed, env, fuel, depth + 1)
        }
        _ => Ok(None),
    }
}
fn expression<'a>(
    expr: &'a Expr,
    env: &Environment<'a>,
    fuel: &mut usize,
    depth: usize,
) -> Result<Option<Value<'a>>, ()> {
    spend(fuel, 1)?;
    if depth >= 128 {
        return Err(());
    }
    match &expr.kind {
        K::Unit => Ok(Some(Value::Unit)),
        K::Literal(value) => Ok(literal(value)),
        K::ComptimeValue(Term::Symbol(name)) if expr.ty == Term::Primitive(Primitive::Symbol) => {
            Ok(Some(Value::Symbol(Atom::Named(name))))
        }
        K::ComptimeValue(Term::Bool(value)) if expr.ty == Term::Primitive(Primitive::Bool) => {
            Ok(Some(Value::Bool(*value)))
        }
        K::Local(id) => Ok(env.get(id).cloned()),
        K::Record {
            declaration,
            fields,
        }
        | K::Variant {
            declaration,
            fields,
            ..
        } => {
            spend(fuel, fields.len())?;
            let mut values = Vec::with_capacity(fields.len());
            for field in fields {
                values.push(known!(expression(field, env, fuel, depth + 1)));
            }
            let variant = match &expr.kind {
                K::Variant { index, .. } => Some(*index),
                _ => None,
            };
            Ok(Some(Value::Nominal(*declaration, variant, values.into())))
        }
        K::Field {
            value,
            declaration,
            index,
        } => {
            let Value::Nominal(actual, None, fields) =
                known!(expression(value, env, fuel, depth + 1))
            else {
                return Ok(None);
            };
            if *declaration != actual {
                return Ok(None);
            }
            Ok(fields.get(*index).cloned())
        }
        K::AnonymousRecord { fields } => {
            let Term::Record(layout) = &expr.ty else {
                return Ok(None);
            };
            if layout.len() != fields.len() {
                return Ok(None);
            }
            spend(fuel, fields.len())?;
            let mut values = Vec::with_capacity(fields.len());
            for field in fields {
                values.push(known!(expression(field, env, fuel, depth + 1)));
            }
            Ok(Some(Value::Record(layout, values.into())))
        }
        K::AnonymousField { value, name, index } => {
            let Value::Record(layout, values) = known!(expression(value, env, fuel, depth + 1))
            else {
                return Ok(None);
            };
            let Some((expected, _)) = layout.get(*index) else {
                return Ok(None);
            };
            spend(fuel, name.len().saturating_add(expected.len()))?;
            if name != expected {
                return Ok(None);
            }
            Ok(values.get(*index).cloned())
        }
        K::Tuple(fields) => {
            spend(fuel, fields.len())?;
            let mut values = Vec::with_capacity(fields.len());
            for field in fields {
                values.push(known!(expression(field, env, fuel, depth + 1)));
            }
            Ok(Some(Value::Tuple(values.into())))
        }
        K::Not(inner) => Ok(boolean(known!(expression(inner, env, fuel, depth + 1)))
            .map(|value| Value::Bool(!value))),
        K::And(left, right) | K::Or(left, right) => {
            let Some(left) = boolean(known!(expression(left, env, fuel, depth + 1))) else {
                return Ok(None);
            };
            if matches!(expr.kind, K::And(..)) && !left || matches!(expr.kind, K::Or(..)) && left {
                return Ok(Some(Value::Bool(left)));
            }
            Ok(boolean(known!(expression(right, env, fuel, depth + 1))).map(Value::Bool))
        }
        K::ScalarEqual(left, right) => {
            let left = known!(expression(left, env, fuel, depth + 1));
            let right = known!(expression(right, env, fuel, depth + 1));
            Ok(Some(Value::Bool(equal(left, right, fuel)?)))
        }
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let Some(condition) = boolean(known!(expression(condition, env, fuel, depth + 1)))
            else {
                return Ok(None);
            };
            if condition {
                expression(then_branch, env, fuel, depth + 1)
            } else if let Some(other) = else_branch {
                expression(other, env, fuel, depth + 1)
            } else {
                Ok(Some(Value::Unit))
            }
        }
        K::Match { value, arms } => {
            let value = known!(expression(value, env, fuel, depth + 1));
            for arm in arms {
                let mut candidate = snapshot(env, fuel)?;
                if !known!(matches(
                    &arm.pattern,
                    value.clone(),
                    &mut candidate,
                    fuel,
                    depth + 1
                )) {
                    continue;
                }
                if let Some(guard) = &arm.guard {
                    let Some(guard) =
                        boolean(known!(expression(guard, &candidate, fuel, depth + 1)))
                    else {
                        return Ok(None);
                    };
                    if !guard {
                        continue;
                    }
                }
                return expression(&arm.body, &candidate, fuel, depth + 1);
            }
            Ok(None)
        }
        K::Block { statements, result } => {
            let mut local = snapshot(env, fuel)?;
            let mut owned = BTreeSet::new();
            for statement in statements {
                spend(fuel, 1)?;
                let statement = match &statement.kind {
                    K::Discard(value) => value.as_ref(),
                    _ => statement,
                };
                match &statement.kind {
                    K::Let { binding, value } => {
                        let value = known!(expression(value, &local, fuel, depth + 1));
                        local.insert(*binding, value);
                        owned.insert(*binding);
                    }
                    K::Assign { place, value } if owned.contains(&place.root) => {
                        let value = known!(expression(value, &local, fuel, depth + 1));
                        let Some(old) = local.get(&place.root).cloned() else {
                            return Ok(None);
                        };
                        let value = known!(assigned(old, &place.fields, value, fuel, depth + 1));
                        local.insert(place.root, value);
                    }
                    K::LetPattern {
                        pattern,
                        value,
                        guard,
                        otherwise: _,
                    } => {
                        let value = known!(expression(value, &local, fuel, depth + 1));
                        let mut candidate = snapshot(&local, fuel)?;
                        if !known!(matches(
                            pattern,
                            value.clone(),
                            &mut candidate,
                            fuel,
                            depth + 1
                        )) {
                            return Ok(None);
                        }
                        if let Some(guard) = guard {
                            if boolean(known!(expression(guard, &candidate, fuel, depth + 1)))
                                != Some(true)
                            {
                                return Ok(None);
                            }
                        }
                        // The checked pattern introduces fresh lexical identities.
                        // Its outputs may be assigned locally; outer values may not.
                        spend(fuel, candidate.len())?;
                        owned.extend(
                            candidate
                                .keys()
                                .filter(|id| !local.contains_key(id))
                                .copied(),
                        );
                        local = candidate;
                    }
                    K::Discard(value) => {
                        known!(expression(value, &local, fuel, depth + 1));
                    }
                    _ => {
                        known!(expression(statement, &local, fuel, depth + 1));
                    }
                }
            }
            if let Some(result) = result {
                expression(result, &local, fuel, depth + 1)
            } else {
                Ok(Some(Value::Unit))
            }
        }
        // Calls, external writes, and unfamiliar computations remain unknown, including
        // under negation. Nothing here performs required comptime invocation.
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::SourceRange;
    use nir::{Primitive, Term};
    fn expr(ty: Primitive, kind: K) -> Expr {
        Expr {
            ty: Term::Primitive(ty),
            kind,
            source: SourceRange { start: 0, end: 0 },
        }
    }
    fn predicate(name: &str, negated: bool) -> Expr {
        let binding = LocalId { owner: 0, index: 0 };
        let equality = expr(
            Primitive::Bool,
            K::ScalarEqual(
                Box::new(expr(Primitive::Symbol, K::Local(binding))),
                Box::new(expr(
                    Primitive::Symbol,
                    K::Literal(Literal::Symbol(name.into())),
                )),
            ),
        );
        if negated {
            expr(Primitive::Bool, K::Not(Box::new(equality)))
        } else {
            equality
        }
    }
    fn guarded(condition: Expr) -> Pattern {
        Pattern::Guard {
            pattern: Box::new(Pattern::Binding {
                binding: LocalId { owner: 0, index: 0 },
                inner: Box::new(Pattern::Wild),
            }),
            condition: Box::new(condition),
        }
    }
    #[test]
    fn finite_cells_agree_with_equality_boolean_oracle() {
        for a in [false, true] {
            for b in [false, true] {
                for conjunction in [false, true] {
                    let left = Box::new(predicate("a", a));
                    let right = Box::new(predicate("b", b));
                    let pattern = guarded(expr(
                        Primitive::Bool,
                        if conjunction {
                            K::And(left, right)
                        } else {
                            K::Or(left, right)
                        },
                    ));
                    assert_eq!(
                        cells([&pattern], &mut 10_000).unwrap(),
                        vec![None, Some("a"), Some("b")]
                    );
                    for cell in [None, Some("a"), Some("b"), Some("unmentioned")] {
                        let left = (cell == Some("a")) ^ a;
                        let right = (cell == Some("b")) ^ b;
                        assert_eq!(
                            covers(&pattern, cell, &mut 10_000).unwrap(),
                            if conjunction {
                                left && right
                            } else {
                                left || right
                            }
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn budget_exhaustion_does_not_produce_coverage_or_poison_recovery() {
        let pattern = guarded(predicate("a", true));
        assert!(cells([&pattern], &mut 0).is_err());
        assert!(covers(&pattern, None, &mut 1).is_err());
        assert!(covers(&pattern, None, &mut 1000).unwrap());
        assert!(!covers(&pattern, Some("a"), &mut 1000).unwrap());
    }
}

#[cfg(test)]
mod completed_tests {
    use super::*;
    use nir::program::SourceRange;
    fn node(ty: Primitive, kind: K) -> Expr {
        Expr {
            ty: Term::Primitive(ty),
            kind,
            source: SourceRange { start: 0, end: 0 },
        }
    }
    #[test]
    fn completed_symbols_refine_partitions_before_proving_complements() {
        let binding = LocalId { owner: 0, index: 0 };
        let equality = node(
            Primitive::Bool,
            K::ScalarEqual(
                Box::new(node(Primitive::Symbol, K::Local(binding))),
                Box::new(node(
                    Primitive::Symbol,
                    K::ComptimeValue(Term::Symbol("ready".into())),
                )),
            ),
        );
        let positive = Pattern::Guard {
            pattern: Box::new(Pattern::Binding {
                binding,
                inner: Box::new(Pattern::Wild),
            }),
            condition: Box::new(equality.clone()),
        };
        let negative = Pattern::Guard {
            pattern: Box::new(Pattern::Binding {
                binding,
                inner: Box::new(Pattern::Wild),
            }),
            condition: Box::new(node(Primitive::Bool, K::Not(Box::new(equality)))),
        };
        assert_eq!(
            cells([&negative], &mut 1000).unwrap(),
            vec![None, Some("ready")]
        );
        let empty = BTreeMap::new();
        let ty = Term::Primitive(Primitive::Symbol);
        assert_eq!(
            crate::coverage::check(&empty, &ty, &[&negative], 10000),
            crate::coverage::Coverage::Missing
        );
        assert_eq!(
            crate::coverage::check(&empty, &ty, &[&positive, &negative], 10000),
            crate::coverage::Coverage::Exhaustive
        );
    }
    #[test]
    fn completed_evidence_does_not_authorize_opaque_or_mistyped_values() {
        let bad = node(Primitive::Symbol, K::ComptimeValue(Term::Bool(true)));
        assert!(
            expression(&bad, &BTreeMap::new(), &mut 1000, 0)
                .unwrap()
                .is_none()
        );
        let opaque = node(
            Primitive::Symbol,
            K::ComptimeValue(Term::NormalForm {
                head: nir::DeclarationId {
                    package: 0,
                    local: 1,
                },
                sort: nir::Sort::ComptimeValue,
                arguments: vec![],
            }),
        );
        assert!(
            expression(&opaque, &BTreeMap::new(), &mut 1000, 0)
                .unwrap()
                .is_none()
        );
        let yes = node(Primitive::Bool, K::ComptimeValue(Term::Bool(true)));
        assert!(matches!(
            expression(&yes, &BTreeMap::new(), &mut 1000, 0).unwrap(),
            Some(Value::Bool(true))
        ));
    }
}

#[cfg(test)]
mod partition_tests {
    use super::*;
    #[test]
    fn boolean_partitions_preserve_independence_and_budget_failure() {
        use nir::program::SourceRange;
        let flags: Vec<_> = (0..8).map(|index| LocalId { owner: 1, index }).collect();
        let patterns: Vec<_> = flags
            .iter()
            .map(|id| Pattern::Guard {
                pattern: Box::new(Pattern::Wild),
                condition: Box::new(Expr {
                    ty: Term::Primitive(Primitive::Bool),
                    source: SourceRange { start: 0, end: 0 },
                    kind: K::Local(*id),
                }),
            })
            .collect();
        assert!(cases(patterns.iter(), &mut 200).is_err());
        let partitions = cases(patterns.iter(), &mut 100_000).unwrap();
        assert_eq!(partitions.len(), 256);
        let mut assignments = BTreeSet::new();
        for case in &partitions {
            let bits: Vec<_> = patterns
                .iter()
                .map(|pattern| covers_case(pattern, case, &mut 1000).unwrap())
                .collect();
            assignments.insert(bits);
        }
        assert_eq!(assignments.len(), 256);
        assert!(assignments.contains(&vec![false; 8]));
        assert!(assignments.contains(&vec![true; 8]));
    }
    #[test]
    fn equality_partitions_include_aliasing_and_distinct_fresh_names() {
        let inputs = [
            LocalId { owner: 0, index: 1 },
            LocalId { owner: 0, index: 2 },
        ];
        let mut cases = Vec::new();
        enumerate(
            &["fixed"],
            &inputs,
            &mut Vec::new(),
            0,
            &mut cases,
            &mut 10000,
        )
        .unwrap();
        // Subject, two inputs, and a distinguished literal: Bell(4) = 15.
        assert_eq!(cases.len(), 15);
        let signature = |values: [Atom<'_>; 4]| {
            let mut result = Vec::new();
            for i in 0..4 {
                for j in 0..i {
                    result.push(values[i] == values[j]);
                }
            }
            result
        };
        let actual: BTreeSet<_> = cases
            .iter()
            .map(|case| {
                signature([
                    case.subject,
                    match &case.env[&inputs[0]] {
                        Value::Symbol(atom) => *atom,
                        _ => unreachable!(),
                    },
                    match &case.env[&inputs[1]] {
                        Value::Symbol(atom) => *atom,
                        _ => unreachable!(),
                    },
                    Atom::Named("fixed"),
                ])
            })
            .collect();
        let mut expected = BTreeSet::new();
        let domain = [
            Atom::Named("fixed"),
            Atom::Named("x"),
            Atom::Named("y"),
            Atom::Named("z"),
        ];
        for a in domain {
            for b in domain {
                for c in domain {
                    expected.insert(signature([a, b, c, Atom::Named("fixed")]));
                }
            }
        }
        assert_eq!(actual, expected);
        let mut partial = Vec::new();
        assert!(
            enumerate(
                &["fixed"],
                &inputs,
                &mut Vec::new(),
                0,
                &mut partial,
                &mut 10
            )
            .is_err()
        );
        let mut recovered = Vec::new();
        enumerate(
            &["fixed"],
            &inputs,
            &mut Vec::new(),
            0,
            &mut recovered,
            &mut 10000,
        )
        .unwrap();
        assert_eq!(recovered.len(), 15);
    }
}

#[cfg(test)]
mod record_tests {
    use super::*;
    use nir::program::SourceRange;
    #[test]
    fn record_writes_are_persistent_bounded_and_reject_wrong_paths() {
        use nir::program::FieldOwner;
        let layout = vec![("key".into(), Term::Primitive(Primitive::Symbol))];
        let original = Value::Record(&layout, vec![Value::Symbol(Atom::Named("ready"))].into());
        let path = [(FieldOwner::Anonymous, 0)];
        assert!(
            assigned(
                original.clone(),
                &path,
                Value::Symbol(Atom::Named("other")),
                &mut 1,
                0
            )
            .is_err()
        );
        let changed = assigned(
            original.clone(),
            &path,
            Value::Symbol(Atom::Named("other")),
            &mut 1000,
            0,
        )
        .unwrap()
        .unwrap();
        let Value::Record(_, before) = original.clone() else {
            unreachable!()
        };
        let Value::Record(_, after) = changed else {
            unreachable!()
        };
        assert!(matches!(before[0], Value::Symbol(Atom::Named("ready"))));
        assert!(matches!(after[0], Value::Symbol(Atom::Named("other"))));
        for path in [
            vec![(FieldOwner::Anonymous, 1)],
            vec![(FieldOwner::Anonymous, 0), (FieldOwner::Anonymous, 0)],
            vec![(
                FieldOwner::Nominal(nir::DeclarationId {
                    package: 0,
                    local: 0,
                }),
                0,
            )],
        ] {
            assert!(
                assigned(original.clone(), &path, Value::Bool(true), &mut 1000, 0)
                    .unwrap()
                    .is_none()
            );
        }
    }
    #[test]
    fn record_projection_checks_labels_and_positions_with_budget() {
        let source = SourceRange { start: 0, end: 0 };
        let record = Expr {
            source,
            ty: Term::Record(vec![
                ("key".into(), Term::Primitive(Primitive::Symbol)),
                ("enabled".into(), Term::Primitive(Primitive::Bool)),
            ]),
            kind: K::AnonymousRecord {
                fields: vec![
                    Expr {
                        source,
                        ty: Term::Primitive(Primitive::Symbol),
                        kind: K::Literal(Literal::Symbol("ready".into())),
                    },
                    Expr {
                        source,
                        ty: Term::Primitive(Primitive::Bool),
                        kind: K::Literal(Literal::Bool(true)),
                    },
                ],
            },
        };
        for (name, index, valid) in [
            ("key", 0, true),
            ("key", 1, false),
            ("missing", 0, false),
            ("key", 2, false),
        ] {
            let projection = Expr {
                source,
                ty: Term::Primitive(Primitive::Symbol),
                kind: K::AnonymousField {
                    value: Box::new(record.clone()),
                    name: name.into(),
                    index,
                },
            };
            let result = expression(&projection, &BTreeMap::new(), &mut 1000, 0).unwrap();
            assert_eq!(
                matches!(result, Some(Value::Symbol(Atom::Named("ready")))),
                valid
            );
            assert!(expression(&projection, &BTreeMap::new(), &mut 1, 0).is_err());
        }
        let value = expression(&record, &BTreeMap::new(), &mut 1000, 0)
            .unwrap()
            .unwrap();
        let wrong = Pattern::AnonymousRecord {
            names: vec!["enabled".into(), "key".into()],
            fields: vec![Pattern::Wild, Pattern::Wild],
        };
        assert_eq!(
            matches(&wrong, value, &mut BTreeMap::new(), &mut 1000, 0).unwrap(),
            None
        );
    }
}

#[cfg(test)]
mod nominal_tests {
    use super::*;
    #[test]
    fn nominal_observations_require_declaration_and_variant_identity() {
        let first = nir::DeclarationId {
            package: 1,
            local: 1,
        };
        let other = nir::DeclarationId {
            package: 2,
            local: 1,
        };
        let pattern = Pattern::Variant {
            declaration: first,
            index: 0,
            fields: vec![Pattern::Literal(Literal::Bool(true))],
        };
        for (declaration, variant, expected) in [
            (first, Some(0), Some(true)),
            (first, Some(1), Some(false)),
            (other, Some(0), None),
        ] {
            let value = Value::Nominal(declaration, variant, vec![Value::Bool(true)].into());
            assert_eq!(
                matches(&pattern, value, &mut BTreeMap::new(), &mut 1000, 0).unwrap(),
                expected
            );
        }
        let source = nir::program::SourceRange { start: 0, end: 0 };
        let record = Expr {
            source,
            ty: Term::Nominal {
                declaration: first,
                arguments: vec![],
            },
            kind: K::Record {
                declaration: first,
                fields: vec![Expr {
                    source,
                    ty: Term::Primitive(Primitive::Bool),
                    kind: K::Literal(Literal::Bool(true)),
                }],
            },
        };
        let wrong = Expr {
            source,
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Field {
                value: Box::new(record),
                declaration: other,
                index: 0,
            },
        };
        assert!(
            expression(&wrong, &BTreeMap::new(), &mut 1000, 0)
                .unwrap()
                .is_none()
        );
    }
}
