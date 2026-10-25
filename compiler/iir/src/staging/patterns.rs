//! Matching completed immutable data during staged input preparation.
//! Candidate bindings remain private until a pattern and its guard succeed.
use super::*;
use nir::program::Pattern;
use std::collections::BTreeMap;

pub(super) fn bindings(
    pattern: &Pattern,
    output: &mut BTreeSet<LocalId>,
    fuel: &mut usize,
) -> Result<bool, String> {
    let mut pending = vec![(pattern, 0)];
    while let Some((pattern, depth)) = pending.pop() {
        charge(fuel, 1)?;
        if depth >= 128 {
            return Err("staged pattern depth exceeded".into());
        }
        match pattern {
            Pattern::Wild | Pattern::Unit => {}
            Pattern::Literal(
                Literal::Bool(_)
                | Literal::Char(_)
                | Literal::String(_)
                | Literal::Symbol(_)
                | Literal::Float(_),
            ) => {}
            Pattern::Binding { binding, inner } => {
                output.insert(*binding);
                pending.push((inner, depth + 1));
            }
            Pattern::ErrorResult { inner, .. } => pending.push((inner, depth + 1)),
            Pattern::Guard { pattern, .. } => pending.push((pattern, depth + 1)),
            Pattern::Then { first, next, .. } => {
                pending.push((first, depth + 1));
                pending.push((next, depth + 1));
            }
            Pattern::Remap { pattern, bindings } => {
                charge(fuel, bindings.len())?;
                output.extend(bindings.iter().map(|(_, to)| *to));
                pending.push((pattern, depth + 1));
            }
            Pattern::Tuple(fields)
            | Pattern::Record { fields, .. }
            | Pattern::Variant { fields, .. }
            | Pattern::AnonymousRecord { fields, .. }
            | Pattern::Or(fields) => {
                charge(fuel, fields.len())?;
                pending.extend(fields.iter().map(|field| (field, depth + 1)));
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

fn copy_locals<'layout>(
    heads: impl Into<Layouts<'layout>>,
    locals: &BTreeMap<LocalId, Expr>,
    fuel: &mut usize,
    depth: usize,
) -> Result<BTreeMap<LocalId, Expr>, String> {
    let heads = heads.into();
    for value in locals.values() {
        constant(heads, value, fuel, depth)?;
    }
    Ok(locals.clone())
}

#[inline(never)]
fn matches<'layout>(
    heads: impl Into<Layouts<'layout>>,
    pattern: &Pattern,
    value: &Expr,
    locals: &mut BTreeMap<LocalId, Expr>,
    fuel: &mut usize,
    depth: usize,
) -> Result<bool, String> {
    let heads = heads.into();
    charge(fuel, 1)?;
    if depth >= 128 {
        return Err("staged pattern depth exceeded".into());
    }
    let pairs = match pattern {
        Pattern::Wild => return Ok(true),
        Pattern::ErrorResult { label, inner } => {
            return match error_values::payload(value, label.as_ref(), fuel)? {
                Some(payload) => matches(heads, inner, payload, locals, fuel, depth + 1),
                None => Ok(false),
            };
        }
        Pattern::Unit => return Ok(matches!(value.kind, K::Unit)),
        Pattern::Literal(literal) => {
            if !matches!(
                literal,
                Literal::Bool(_)
                    | Literal::Char(_)
                    | Literal::String(_)
                    | Literal::Symbol(_)
                    | Literal::Float(_)
            ) {
                return Err(
                    "staged literal pattern requires supported scalar matching semantics".into(),
                );
            }
            if let Literal::Float(spelling) = literal {
                charge(fuel, spelling.len())?;
            }
            let expected = Expr {
                source: value.source,
                ty: value.ty.clone(),
                kind: K::Literal(literal.clone()),
            };
            let expected = constant(heads, &expected, fuel, depth + 1)?;
            let actual = constant(heads, value, fuel, depth + 1)?;
            if let (Term::Float(expected), Term::Float(actual)) = (&expected, &actual) {
                let mut budget = comptime_runtime::Budget {
                    steps: *fuel,
                    max_limbs: 4096,
                };
                let equal = actual.compare(expected, &mut budget);
                *fuel = budget.steps;
                return equal.map(|order| order.is_eq()).map_err(|e| e.to_string());
            }
            return Ok(expected == actual);
        }
        Pattern::Binding { binding, inner } => {
            if !matches(heads, inner, value, locals, fuel, depth + 1)? {
                return Ok(false);
            }
            constant(heads, value, fuel, depth + 1)?;
            if locals.insert(*binding, value.clone()).is_some() {
                return Err("duplicate staged match binding".into());
            }
            return Ok(true);
        }
        Pattern::Guard { pattern, condition } => {
            if !matches(heads, pattern, value, locals, fuel, depth + 1)? {
                return Ok(false);
            }
            let result = fold_local_input(heads, condition, locals, fuel, depth + 1)?;
            return match constant(heads, &result, fuel, depth + 1)? {
                Term::Bool(value) => Ok(value),
                _ => Err("staged match guard is not Boolean".into()),
            };
        }
        Pattern::Then {
            first,
            value: computed,
            next,
        } => {
            if !matches(heads, first, value, locals, fuel, depth + 1)? {
                return Ok(false);
            }
            let computed = fold_local_input(heads, computed, locals, fuel, depth + 1)?;
            return matches(heads, next, &computed, locals, fuel, depth + 1);
        }
        Pattern::Or(alternatives) => {
            for alternative in alternatives {
                let mut candidate = copy_locals(heads, locals, fuel, depth + 1)?;
                if matches(heads, alternative, value, &mut candidate, fuel, depth + 1)? {
                    *locals = candidate;
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        Pattern::Remap { pattern, bindings } => {
            charge(fuel, locals.len().saturating_add(bindings.len()))?;
            let ambient: BTreeSet<_> = locals.keys().copied().collect();
            if !matches(heads, pattern, value, locals, fuel, depth + 1)? {
                return Ok(false);
            }
            let mut sources = BTreeSet::new();
            let mut targets = BTreeSet::new();
            let mut values = Vec::new();
            for (from, to) in bindings {
                if ambient.contains(from) || !sources.insert(*from) || !targets.insert(*to) {
                    return Err("invalid staged match remap".into());
                }
                let value = locals
                    .get(from)
                    .ok_or("missing staged match remap source")?;
                constant(heads, value, fuel, depth + 1)?;
                values.push((*to, value.clone()));
            }
            if targets
                .iter()
                .any(|to| locals.contains_key(to) && !sources.contains(to))
            {
                return Err("staged match remap merges bindings".into());
            }
            for from in sources {
                locals.remove(&from);
            }
            locals.extend(values);
            return Ok(true);
        }
        Pattern::Tuple(fields) => {
            let K::Tuple(values) = &value.kind else {
                return Err("staged tuple pattern domain mismatch".into());
            };
            (fields, values)
        }
        Pattern::Record {
            declaration,
            fields,
        } => {
            let (
                K::Record {
                    declaration: actual,
                    fields: values,
                },
                Term::Nominal {
                    declaration: owner, ..
                },
            ) = (&value.kind, &value.ty)
            else {
                return Err("staged record pattern domain mismatch".into());
            };
            if declaration != actual || declaration != owner {
                return Err("staged record pattern identity mismatch".into());
            }
            (fields, values)
        }
        Pattern::Variant {
            declaration,
            index,
            fields,
        } => {
            let (
                K::Variant {
                    declaration: actual,
                    index: selected,
                    fields: values,
                },
                Term::Nominal {
                    declaration: owner, ..
                },
            ) = (&value.kind, &value.ty)
            else {
                return Err("staged enum pattern domain mismatch".into());
            };
            if declaration != actual || declaration != owner {
                return Err("staged enum pattern identity mismatch".into());
            }
            if index != selected {
                return Ok(false);
            }
            (fields, values)
        }
        Pattern::AnonymousRecord { names, fields } => {
            let (K::AnonymousRecord { fields: values }, Term::Record(types)) =
                (&value.kind, &value.ty)
            else {
                return Err("staged structural pattern domain mismatch".into());
            };
            if names.len() != types.len() || names.len() != fields.len() {
                return Err("staged record pattern arity mismatch".into());
            }
            for (name, (expected, _)) in names.iter().zip(types) {
                charge(fuel, name.len().saturating_add(expected.len()))?;
                if name != expected {
                    return Err("staged record pattern label mismatch".into());
                }
            }
            (fields, values)
        }
        _ => return Err("staged data pattern requires unsupported matching".into()),
    };
    if pairs.0.len() != pairs.1.len() {
        return Err("staged pattern arity mismatch".into());
    }
    for (pattern, value) in pairs.0.iter().zip(pairs.1) {
        if !matches(heads, pattern, value, locals, fuel, depth + 1)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn fold<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    if depth >= 128 {
        return Err("staged match depth exceeded".into());
    }
    charge(fuel, 1)?;
    let K::Match { value, arms } = &expr.kind else {
        unreachable!()
    };
    let value = fold_input(heads, value, fuel, depth + 1)?;
    constant(heads, &value, fuel, depth + 1)?;
    for arm in arms {
        let mut locals = BTreeMap::new();
        if !matches(heads, &arm.pattern, &value, &mut locals, fuel, depth + 1)? {
            continue;
        }
        if let Some(guard) = &arm.guard {
            let guard = fold_local_input(heads, guard, &locals, fuel, depth + 1)?;
            match constant(heads, &guard, fuel, depth + 1)? {
                Term::Bool(false) => continue,
                Term::Bool(true) => {}
                _ => return Err("staged match guard is not Boolean".into()),
            }
        }
        let mut result = fold_local_input(heads, &arm.body, &locals, fuel, depth + 1)?;
        if result.ty != expr.ty {
            return Err("staged match result type mismatch".into());
        }
        constant(heads, &result, fuel, depth)?;
        result.source = expr.source;
        return Ok(result);
    }
    Err("staged match is not exhaustive for its input".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::{MatchArm, SourceRange};
    fn boolean(value: bool) -> Expr {
        Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(Literal::Bool(value)),
        }
    }
    fn bind(binding: LocalId) -> Pattern {
        Pattern::Binding {
            binding,
            inner: Box::new(Pattern::Wild),
        }
    }
    fn pair() -> Expr {
        Expr {
            source: boolean(true).source,
            ty: Term::Tuple(vec![boolean(true).ty; 2]),
            kind: K::Tuple(vec![boolean(true), boolean(false)]),
        }
    }

    #[test]
    fn exact_float_literal_patterns_compare_numeric_values_with_bounded_work() {
        for (pattern, input, expected) in [
            ("0.0", "-0.0", true),
            (
                "1.0000000000000000000000000000001",
                "10.000000000000000000000000000001e-1",
                true,
            ),
            (
                "1.0000000000000000000000000000001",
                "1.0000000000000000000000000000002",
                false,
            ),
        ] {
            let literal = Pattern::Literal(Literal::Float(pattern.into()));
            let value = Expr {
                source: boolean(true).source,
                ty: Term::Primitive(Primitive::Float),
                kind: K::Literal(Literal::Float(input.into())),
            };
            assert!(matches(&[], &literal, &value, &mut BTreeMap::new(), &mut 0, 0).is_err());
            assert_eq!(
                matches(&[], &literal, &value, &mut BTreeMap::new(), &mut 10000, 0).unwrap(),
                expected
            );
            let mut bindings_set = BTreeSet::new();
            assert!(bindings(&literal, &mut bindings_set, &mut 1000).unwrap());
            assert!(bindings_set.is_empty());
        }
        let wrong = Expr {
            source: boolean(true).source,
            ty: Term::Primitive(Primitive::F64),
            kind: K::Literal(Literal::Float64(0)),
        };
        assert!(
            matches(
                &[],
                &Pattern::Literal(Literal::Float("0.0".into())),
                &wrong,
                &mut BTreeMap::new(),
                &mut 1000,
                0
            )
            .is_err()
        );
        let invalid = Expr {
            source: boolean(true).source,
            ty: Term::Primitive(Primitive::Float),
            kind: K::Literal(Literal::Float("0.0".into())),
        };
        assert!(
            matches(
                &[],
                &Pattern::Literal(Literal::Float("NaN".into())),
                &invalid,
                &mut BTreeMap::new(),
                &mut 1000,
                0
            )
            .is_err()
        );
    }

    #[test]
    fn candidates_rollback_and_cannot_remap_ambient_bindings() {
        let first = LocalId { owner: 1, index: 0 };
        let second = LocalId { owner: 1, index: 1 };
        let ambient = LocalId { owner: 1, index: 2 };
        let pattern = Pattern::Or(vec![
            Pattern::Tuple(vec![bind(first), Pattern::Literal(Literal::Bool(true))]),
            Pattern::Tuple(vec![Pattern::Literal(Literal::Bool(true)), bind(second)]),
        ]);
        let mut locals = BTreeMap::from([(ambient, boolean(true))]);
        assert!(matches(&[], &pattern, &pair(), &mut locals, &mut 1000, 0).unwrap());
        assert_eq!(locals.len(), 2);
        assert_eq!(locals.get(&second), Some(&boolean(false)));
        assert!(!locals.contains_key(&first));
        let invalid = Pattern::Remap {
            pattern: Box::new(Pattern::Wild),
            bindings: vec![(ambient, first)],
        };
        assert!(matches(&[], &invalid, &pair(), &mut locals, &mut 1000, 0).is_err());
        let swap = Pattern::Remap {
            pattern: Box::new(Pattern::Tuple(vec![bind(first), bind(second)])),
            bindings: vec![(first, second), (second, first)],
        };
        let mut swapped = BTreeMap::new();
        assert!(matches(&[], &swap, &pair(), &mut swapped, &mut 1000, 0).unwrap());
        assert_eq!(swapped.get(&first), Some(&boolean(false)));
        assert_eq!(swapped.get(&second), Some(&boolean(true)));
    }

    #[test]
    fn unmatched_guards_and_unselected_bodies_are_not_evaluated() {
        let missing = Expr {
            source: boolean(true).source,
            ty: boolean(true).ty,
            kind: K::Local(LocalId { owner: 9, index: 9 }),
        };
        let expression = Expr {
            source: boolean(true).source,
            ty: boolean(true).ty,
            kind: K::Match {
                value: Box::new(pair()),
                arms: vec![
                    MatchArm {
                        pattern: Pattern::Tuple(vec![
                            Pattern::Literal(Literal::Bool(false)),
                            Pattern::Wild,
                        ]),
                        guard: Some(missing.clone()),
                        body: missing.clone(),
                    },
                    MatchArm {
                        pattern: Pattern::Wild,
                        guard: None,
                        body: boolean(true),
                    },
                    MatchArm {
                        pattern: Pattern::Wild,
                        guard: None,
                        body: missing,
                    },
                ],
            },
        };
        assert!(fold(&[], &expression, &mut 1, 0).is_err());
        assert_eq!(fold(&[], &expression, &mut 1000, 0).unwrap(), boolean(true));
        let mut wrong = expression.clone();
        let K::Match { arms, .. } = &mut wrong.kind else {
            unreachable!()
        };
        arms.remove(1);
        assert!(fold(&[], &wrong, &mut 1000, 0).is_err());
        assert_eq!(fold(&[], &expression, &mut 1000, 0).unwrap(), boolean(true));
    }
}
