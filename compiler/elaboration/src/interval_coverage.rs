//! Bounded symbolic integer/character coverage. Never invokes user code or PTC.
use comptime_runtime::{Budget, Integer};
use nir::Primitive;
use nir::program::{Expr, ExprKind as K, IntegerOp, Literal, LocalId, Pattern};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Edge {
    NegativeInfinity,
    Finite(Integer),
    PositiveInfinity,
}
#[derive(Clone, Debug)]
pub(crate) struct Interval {
    pub start: Edge,
    pub end: Edge,
}
#[derive(Clone)]
enum Value {
    Subject,
    Constant(Integer),
}
#[derive(Clone)]
struct State {
    ranges: Vec<Interval>,
    bindings: BTreeMap<LocalId, Value>,
}
fn spend(steps: &mut usize, count: usize) -> Result<(), ()> {
    *steps = steps.checked_sub(count).ok_or(())?;
    Ok(())
}
fn integer_work(value: &Integer) -> usize {
    value.bit_len().div_ceil(32).saturating_add(1)
}
fn edge_work(edge: &Edge) -> usize {
    match edge {
        Edge::Finite(value) => integer_work(value),
        _ => 1,
    }
}
fn compare(a: &Edge, b: &Edge, steps: &mut usize) -> Result<std::cmp::Ordering, ()> {
    spend(steps, edge_work(a).saturating_add(edge_work(b)))?;
    Ok(a.cmp(b))
}
fn copy_edge(edge: &Edge, steps: &mut usize) -> Result<Edge, ()> {
    spend(steps, edge_work(edge))?;
    Ok(edge.clone())
}
fn whole() -> Vec<Interval> {
    vec![Interval {
        start: Edge::NegativeInfinity,
        end: Edge::PositiveInfinity,
    }]
}
fn arithmetic<T>(
    steps: &mut usize,
    action: impl FnOnce(&mut Budget) -> Result<T, comptime_runtime::Error>,
) -> Result<T, ()> {
    let mut budget = Budget {
        steps: *steps,
        max_limbs: 4096,
    };
    let result = action(&mut budget);
    *steps = budget.steps;
    result.map_err(|_| ())
}
fn successor(value: &Integer, steps: &mut usize) -> Result<Integer, ()> {
    arithmetic(steps, |budget| value.add(&Integer::from_u128(1), budget))
}
fn literal(value: &Literal, steps: &mut usize) -> Result<Option<Integer>, ()> {
    spend(steps, 1)?;
    Ok(match value {
        Literal::Integer(text) => Some(arithmetic(steps, |budget| {
            Integer::parse_literal(text, budget)
        })?),
        Literal::SignedInteger(value) => Some(Integer::from_i128(*value)),
        Literal::UnsignedInteger(value) => Some(Integer::from_u128(*value)),
        Literal::Char(value) => Some(Integer::from_u128(*value as u128)),
        _ => None,
    })
}
fn value(expr: &Expr, state: &State, steps: &mut usize, depth: usize) -> Result<Option<Value>, ()> {
    spend(steps, 1)?;
    if depth > 128 {
        return Err(());
    }
    match &expr.kind {
        K::Literal(v) => Ok(literal(v, steps)?.map(Value::Constant)),
        K::Local(id) => {
            if let Some(Value::Constant(v)) = state.bindings.get(id) {
                spend(steps, integer_work(v))?;
            }
            Ok(state.bindings.get(id).cloned())
        }
        K::IntegerNegate(expr) => match value(expr, state, steps, depth + 1)? {
            Some(Value::Constant(v)) => Ok(Some(Value::Constant(arithmetic(steps, |budget| {
                Integer::from_u128(0).subtract(&v, budget)
            })?))),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}
fn intersect(a: &[Interval], b: &[Interval], steps: &mut usize) -> Result<Vec<Interval>, ()> {
    let mut out = Vec::new();
    for a in a {
        for b in b {
            let start = if compare(&a.start, &b.start, steps)?.is_lt() {
                &b.start
            } else {
                &a.start
            };
            let end = if compare(&a.end, &b.end, steps)?.is_lt() {
                &a.end
            } else {
                &b.end
            };
            if compare(start, end, steps)?.is_lt() {
                out.push(Interval {
                    start: copy_edge(start, steps)?,
                    end: copy_edge(end, steps)?,
                });
            }
        }
    }
    Ok(out)
}
fn complement(ranges: &[Interval], steps: &mut usize) -> Result<Vec<Interval>, ()> {
    let mut remaining = whole();
    for range in ranges {
        let outside = [
            Interval {
                start: Edge::NegativeInfinity,
                end: copy_edge(&range.start, steps)?,
            },
            Interval {
                start: copy_edge(&range.end, steps)?,
                end: Edge::PositiveInfinity,
            },
        ];
        remaining = intersect(&remaining, &outside, steps)?;
    }
    Ok(remaining)
}
fn copy_state(state: &State, steps: &mut usize) -> Result<State, ()> {
    for range in &state.ranges {
        spend(
            steps,
            edge_work(&range.start).saturating_add(edge_work(&range.end)),
        )?;
    }
    for binding in state.bindings.values() {
        spend(
            steps,
            match binding {
                Value::Subject => 1,
                Value::Constant(v) => integer_work(v),
            },
        )?;
    }
    Ok(state.clone())
}
// Every recursive result is relative to the current input region. This is
// essential when a nested negation appears in only one conditional branch.
fn condition(
    expr: &Expr,
    state: &State,
    steps: &mut usize,
    depth: usize,
) -> Result<Option<Vec<Interval>>, ()> {
    if state.ranges.is_empty() {
        spend(steps, 1)?;
        return Ok(Some(Vec::new()));
    }
    condition_inner(expr, state, steps, depth)?
        .map(|ranges| intersect(&state.ranges, &ranges, steps))
        .transpose()
}
fn condition_inner(
    expr: &Expr,
    state: &State,
    steps: &mut usize,
    depth: usize,
) -> Result<Option<Vec<Interval>>, ()> {
    spend(steps, 1)?;
    if depth > 128 {
        return Err(());
    }
    let (left, right, op) = match &expr.kind {
        K::ScalarLessEqual(left, right) => (left, right, IntegerOp::LessEqual),
        K::ScalarLess(left, right) => (left, right, IntegerOp::Less),
        K::ScalarGreater(left, right) => (left, right, IntegerOp::Greater),
        K::ScalarGreaterEqual(left, right) => (left, right, IntegerOp::GreaterEqual),
        K::ScalarEqual(left, right) => (left, right, IntegerOp::Equal),
        K::IntegerBinary { op, left, right } if op.is_comparison() => (left, right, *op),
        K::Match { value: input, arms } => {
            if !matches!(value(input, state, steps, depth + 1)?, Some(Value::Subject)) {
                return Ok(None);
            }
            let mut remaining = copy_state(state, steps)?;
            let mut truth = Vec::new();
            for arm in arms {
                if remaining.ranges.is_empty() {
                    break;
                }
                if arm.guard.is_some() {
                    return Ok(None);
                }
                let mut complete = true;
                let states = pattern(
                    &arm.pattern,
                    copy_state(&remaining, steps)?,
                    steps,
                    depth + 1,
                    &mut complete,
                )?;
                // Positive coverage may safely under-approximate an unknown
                // guard. Complementing that approximation would be unsound.
                if !complete {
                    return Ok(None);
                }
                let mut matched = Vec::new();
                for state in states {
                    let Some(regions) = condition(&arm.body, &state, steps, depth + 1)? else {
                        return Ok(None);
                    };
                    spend(steps, regions.len().saturating_add(state.ranges.len()))?;
                    truth.extend(regions);
                    matched.extend(state.ranges);
                }
                let outside = complement(&matched, steps)?;
                remaining.ranges = intersect(&remaining.ranges, &outside, steps)?;
            }
            return Ok(remaining.ranges.is_empty().then_some(truth));
        }
        K::Not(inner) => {
            return condition(inner, state, steps, depth + 1)?
                .map(|v| complement(&v, steps))
                .transpose();
        }
        K::And(left, right) | K::Or(left, right) => {
            let Some(mut truth) = condition(left, state, steps, depth + 1)? else {
                return Ok(None);
            };
            let mut pending = copy_state(state, steps)?;
            pending.ranges = if matches!(expr.kind, K::Or(..)) {
                intersect(&state.ranges, &complement(&truth, steps)?, steps)?
            } else {
                std::mem::take(&mut truth)
            };
            let Some(right) = condition(right, &pending, steps, depth + 1)? else {
                return Ok(None);
            };
            spend(steps, right.len())?;
            truth.extend(right);
            return Ok(Some(truth));
        }
        K::If {
            condition: test,
            then_branch,
            else_branch: Some(else_branch),
        } => {
            let Some(selected) = condition(test, state, steps, depth + 1)? else {
                return Ok(None);
            };
            let mut rejected = copy_state(state, steps)?;
            rejected.ranges = intersect(&state.ranges, &complement(&selected, steps)?, steps)?;
            let mut accepted = copy_state(state, steps)?;
            accepted.ranges = selected;
            let Some(mut truth) = condition(then_branch, &accepted, steps, depth + 1)? else {
                return Ok(None);
            };
            let Some(other) = condition(else_branch, &rejected, steps, depth + 1)? else {
                return Ok(None);
            };
            spend(steps, other.len())?;
            truth.extend(other);
            return Ok(Some(truth));
        }
        K::Block {
            statements,
            result: Some(result),
        } => {
            let mut state = copy_state(state, steps)?;
            for statement in statements {
                spend(steps, 1)?;
                let K::Let {
                    binding,
                    value: expr,
                } = &statement.kind
                else {
                    return Ok(None);
                };
                let Some(v) = value(expr, &state, steps, depth + 1)? else {
                    return Ok(None);
                };
                state.bindings.insert(*binding, v);
            }
            return condition(result, &state, steps, depth + 1);
        }
        K::Literal(Literal::Bool(value)) => return Ok(Some(if *value { whole() } else { vec![] })),
        _ => return Ok(None),
    };
    let left = value(left, state, steps, depth + 1)?;
    let right = value(right, state, steps, depth + 1)?;
    let (point, op) = match (left, right) {
        (Some(Value::Subject), Some(Value::Constant(v))) => (v, op),
        (Some(Value::Constant(v)), Some(Value::Subject)) => (
            v,
            match op {
                IntegerOp::Less => IntegerOp::Greater,
                IntegerOp::LessEqual => IntegerOp::GreaterEqual,
                IntegerOp::Greater => IntegerOp::Less,
                IntegerOp::GreaterEqual => IntegerOp::LessEqual,
                _ => op,
            },
        ),
        (Some(Value::Subject), Some(Value::Subject)) => {
            return Ok(Some(
                if matches!(
                    op,
                    IntegerOp::Equal | IntegerOp::LessEqual | IntegerOp::GreaterEqual
                ) {
                    whole()
                } else {
                    vec![]
                },
            ));
        }
        (Some(Value::Constant(a)), Some(Value::Constant(b))) => {
            let nir::Term::Bool(result) = arithmetic(steps, |budget| op.evaluate(&a, &b, budget))?
            else {
                return Ok(None);
            };
            return Ok(Some(if result { whole() } else { vec![] }));
        }
        _ => return Ok(None),
    };
    let ranges = match op {
        IntegerOp::Less => vec![Interval {
            start: Edge::NegativeInfinity,
            end: Edge::Finite(point),
        }],
        IntegerOp::LessEqual => vec![Interval {
            start: Edge::NegativeInfinity,
            end: Edge::Finite(successor(&point, steps)?),
        }],
        IntegerOp::Greater => vec![Interval {
            start: Edge::Finite(successor(&point, steps)?),
            end: Edge::PositiveInfinity,
        }],
        IntegerOp::GreaterEqual => vec![Interval {
            start: Edge::Finite(point),
            end: Edge::PositiveInfinity,
        }],
        IntegerOp::Equal | IntegerOp::NotEqual => {
            let end = Edge::Finite(successor(&point, steps)?);
            let singleton = vec![Interval {
                start: Edge::Finite(point),
                end,
            }];
            if op == IntegerOp::NotEqual {
                complement(&singleton, steps)?
            } else {
                singleton
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(ranges))
}

fn pattern(
    pat: &Pattern,
    mut state: State,
    steps: &mut usize,
    depth: usize,
    complete: &mut bool,
) -> Result<Vec<State>, ()> {
    spend(steps, 1)?;
    if depth > 128 {
        return Err(());
    }
    match pat {
        Pattern::Wild => Ok(vec![state]),
        Pattern::Binding { binding, inner } => {
            let mut states = pattern(inner, state, steps, depth + 1, complete)?;
            for state in &mut states {
                spend(steps, 1)?;
                state.bindings.insert(*binding, Value::Subject);
            }
            Ok(states)
        }
        Pattern::Remap {
            pattern: inner,
            bindings,
        } => {
            let mut states = pattern(inner, state, steps, depth + 1, complete)?;
            for state in &mut states {
                let mut values = Vec::new();
                let mut targets = std::collections::BTreeSet::new();
                for (from, to) in bindings {
                    spend(steps, 1)?;
                    if let Some(v) = state.bindings.get(from) {
                        spend(
                            steps,
                            match v {
                                Value::Subject => 1,
                                Value::Constant(v) => integer_work(v),
                            },
                        )?;
                        values.push((*to, v.clone()));
                        targets.insert(*to);
                    } else {
                        *complete = false;
                    }
                }
                for (id, value) in values {
                    state.bindings.insert(id, value);
                }
                for (from, _) in bindings {
                    spend(steps, 1)?;
                    if !targets.contains(from) {
                        state.bindings.remove(from);
                    }
                }
            }
            Ok(states)
        }
        Pattern::Or(alternatives) => {
            let mut out = Vec::new();
            let mut remaining = state;
            for alternative in alternatives {
                if remaining.ranges.is_empty() {
                    break;
                }
                let next = copy_state(&remaining, steps)?;
                let mut exact = true;
                let states = pattern(alternative, next, steps, depth + 1, &mut exact)?;
                if exact {
                    let mut matched = Vec::new();
                    for state in &states {
                        for range in &state.ranges {
                            matched.push(Interval {
                                start: copy_edge(&range.start, steps)?,
                                end: copy_edge(&range.end, steps)?,
                            });
                        }
                    }
                    remaining.ranges =
                        intersect(&remaining.ranges, &complement(&matched, steps)?, steps)?;
                }
                spend(steps, states.len())?;
                out.extend(states);
                if !exact {
                    // A prior alternative may succeed and export different
                    // bindings. An outer guard cannot retry later alternatives.
                    *complete = false;
                    break;
                }
            }
            Ok(out)
        }
        Pattern::Literal(v) => {
            let Some(v) = literal(v, steps)? else {
                *complete = false;
                return Ok(vec![]);
            };
            let end = successor(&v, steps)?;
            state.ranges = intersect(
                &state.ranges,
                &[Interval {
                    start: Edge::Finite(v),
                    end: Edge::Finite(end),
                }],
                steps,
            )?;
            Ok(vec![state])
        }
        Pattern::Guard {
            pattern: inner,
            condition: expr,
        } => {
            let mut out = Vec::new();
            for mut state in pattern(inner, state, steps, depth + 1, complete)? {
                let Some(ranges) = condition(expr, &state, steps, depth + 1)? else {
                    *complete = false;
                    continue;
                };
                state.ranges = intersect(&state.ranges, &ranges, steps)?;
                out.push(state);
            }
            Ok(out)
        }
        _ => {
            *complete = false;
            Ok(vec![])
        }
    }
}
pub(crate) fn extract(pat: &Pattern, steps: &mut usize) -> Result<Vec<Interval>, ()> {
    // Ordinary positive evidence may omit unknown regions. Only callers taking
    // a complement need the exactness flag used by match conditions above.
    let mut complete = true;
    Ok(pattern(
        pat,
        State {
            ranges: whole(),
            bindings: BTreeMap::new(),
        },
        steps,
        0,
        &mut complete,
    )?
    .into_iter()
    .flat_map(|state| state.ranges)
    .collect())
}

pub(crate) fn domain(primitive: Primitive) -> Option<Vec<Interval>> {
    let finite = |start, end| Interval {
        start: Edge::Finite(Integer::from_u128(start)),
        end: Edge::Finite(Integer::from_u128(end)),
    };
    if primitive == Primitive::Integer {
        return Some(whole());
    }
    if primitive == Primitive::Char {
        return Some(vec![finite(0, 0xd800), finite(0xe000, 0x110000)]);
    }
    let (signed, width) = primitive.fixed_integer_layout()?;
    let mut budget = Budget::default();
    let (start, maximum) = if signed {
        (
            Integer::from_i128(if width == 128 {
                i128::MIN
            } else {
                -(1i128 << (width - 1))
            }),
            Integer::from_i128(if width == 128 {
                i128::MAX
            } else {
                (1i128 << (width - 1)) - 1
            }),
        )
    } else {
        (
            Integer::from_u128(0),
            Integer::from_u128(if width == 128 {
                u128::MAX
            } else {
                (1u128 << width) - 1
            }),
        )
    };
    Some(vec![Interval {
        start: Edge::Finite(start),
        end: Edge::Finite(maximum.add(&Integer::from_u128(1), &mut budget).ok()?),
    }])
}
pub(crate) fn covers(
    range: &Interval,
    start: &Edge,
    end: &Edge,
    steps: &mut usize,
) -> Result<bool, ()> {
    Ok(!compare(&range.start, start, steps)?.is_gt() && !compare(&range.end, end, steps)?.is_lt())
}
pub(crate) fn cuts<'a>(
    ranges: impl Iterator<Item = &'a Interval>,
    steps: &mut usize,
) -> Result<Vec<Edge>, ()> {
    let mut cuts: Vec<Edge> = Vec::new();
    for range in ranges {
        for edge in [&range.start, &range.end] {
            let mut position = 0;
            while position < cuts.len() && compare(&cuts[position], edge, steps)?.is_lt() {
                position += 1;
            }
            if position == cuts.len() || compare(&cuts[position], edge, steps)?.is_ne() {
                spend(steps, cuts.len().saturating_add(1))?;
                cuts.insert(position, copy_edge(edge, steps)?);
            }
        }
    }
    Ok(cuts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::{Coverage, check};
    use nir::Term;
    use nir::program::SourceRange;
    fn expr(ty: Primitive, kind: K) -> Expr {
        Expr {
            ty: Term::Primitive(ty),
            kind,
            source: SourceRange { start: 0, end: 0 },
        }
    }
    fn range(
        ty: Primitive,
        lower: Option<Literal>,
        upper: Option<Literal>,
        inclusive: bool,
    ) -> Pattern {
        let id = LocalId { owner: 1, index: 0 };
        let observed = || expr(ty, K::Local(id));
        let le = |a, b| {
            expr(
                Primitive::Bool,
                if ty == Primitive::Integer {
                    K::IntegerBinary {
                        op: IntegerOp::LessEqual,
                        left: Box::new(a),
                        right: Box::new(b),
                    }
                } else {
                    K::ScalarLessEqual(Box::new(a), Box::new(b))
                },
            )
        };
        let mut constraints = Vec::new();
        if let Some(lower) = lower {
            constraints.push(le(expr(ty, K::Literal(lower)), observed()));
        }
        if let Some(upper) = upper {
            let upper = expr(ty, K::Literal(upper));
            constraints.push(if inclusive {
                le(observed(), upper)
            } else {
                expr(Primitive::Bool, K::Not(Box::new(le(upper, observed()))))
            });
        }
        let condition = if constraints.len() == 2 {
            expr(
                Primitive::Bool,
                K::And(
                    Box::new(constraints.remove(0)),
                    Box::new(constraints.remove(0)),
                ),
            )
        } else {
            constraints.remove(0)
        };
        Pattern::Guard {
            pattern: Box::new(Pattern::Binding {
                binding: id,
                inner: Box::new(Pattern::Wild),
            }),
            condition: Box::new(condition),
        }
    }
    #[test]
    fn conditional_regions_match_integer_oracle_without_escaping_input() {
        let subject = LocalId {
            owner: 11,
            index: 0,
        };
        let edge = |n| Edge::Finite(Integer::from_i128(n));
        let state = State {
            ranges: vec![Interval {
                start: edge(-4),
                end: edge(5),
            }],
            bindings: BTreeMap::from([(subject, Value::Subject)]),
        };
        let less = |point| {
            expr(
                Primitive::Bool,
                K::IntegerBinary {
                    op: IntegerOp::Less,
                    left: Box::new(expr(Primitive::Integer, K::Local(subject))),
                    right: Box::new(expr(
                        Primitive::Integer,
                        K::Literal(Literal::SignedInteger(point)),
                    )),
                },
            )
        };
        for split in -5..=6 {
            for cut in -5..=6 {
                let check = expr(
                    Primitive::Bool,
                    K::If {
                        condition: Box::new(less(split)),
                        then_branch: Box::new(less(cut)),
                        else_branch: Some(Box::new(expr(
                            Primitive::Bool,
                            K::Not(Box::new(less(cut))),
                        ))),
                    },
                );
                assert!(condition(&check, &state, &mut 1, 0).is_err());
                let ranges = condition(&check, &state, &mut 100_000, 0)
                    .unwrap()
                    .expect("exact condition");
                for value in -6..=7 {
                    let point = edge(value);
                    let count = ranges
                        .iter()
                        .filter(|range| range.start <= point && point < range.end)
                        .count();
                    let expected = (-4..5).contains(&value)
                        && if value < split {
                            value < cut
                        } else {
                            value >= cut
                        };
                    assert_eq!(
                        count,
                        usize::from(expected),
                        "split={split} cut={cut} value={value}"
                    );
                }
            }
        }
    }

    #[test]
    fn outer_guard_observes_only_the_first_successful_alternative() {
        let source = LocalId { owner: 8, index: 0 };
        let observed = LocalId { owner: 8, index: 1 };
        let selected = LocalId { owner: 8, index: 2 };
        let alternative = |from| Pattern::Remap {
            pattern: Box::new(Pattern::Wild),
            bindings: vec![(from, selected)],
        };
        let candidate = Pattern::Guard {
            pattern: Box::new(Pattern::Or(vec![
                alternative(source),
                alternative(observed),
            ])),
            condition: Box::new(expr(
                Primitive::Bool,
                K::ScalarEqual(
                    Box::new(expr(Primitive::U8, K::Local(selected))),
                    Box::new(expr(Primitive::U8, K::Local(observed))),
                ),
            )),
        };
        let state = State {
            ranges: whole(),
            bindings: BTreeMap::from([
                (source, Value::Constant(Integer::from_u128(3))),
                (observed, Value::Subject),
            ]),
        };
        let mut complete = true;
        let states = pattern(&candidate, state, &mut 10000, 0, &mut complete).unwrap();
        assert!(complete);
        assert_eq!(states.len(), 1);
        let outside = complement(&states[0].ranges, &mut 10000).unwrap();
        let point = extract(&Pattern::Literal(Literal::UnsignedInteger(3)), &mut 10000).unwrap();
        assert!(intersect(&point, &outside, &mut 10000).unwrap().is_empty());
        assert!(!outside.is_empty());
    }

    #[test]
    fn exact_pattern_remaps_read_a_snapshot_and_remove_old_sources() {
        let a = LocalId { owner: 1, index: 0 };
        let b = LocalId { owner: 1, index: 1 };
        let c = LocalId { owner: 1, index: 2 };
        let state = State {
            ranges: whole(),
            bindings: BTreeMap::from([
                (a, Value::Subject),
                (b, Value::Constant(Integer::from_u128(3))),
            ]),
        };
        let remap = Pattern::Remap {
            pattern: Box::new(Pattern::Wild),
            bindings: vec![(a, b), (b, c)],
        };
        let mut complete = true;
        let states = pattern(&remap, state, &mut 1000, 0, &mut complete).unwrap();
        assert!(complete);
        assert!(!states[0].bindings.contains_key(&a));
        assert!(matches!(states[0].bindings.get(&b), Some(Value::Subject)));
        assert!(
            matches!(states[0].bindings.get(&c),Some(Value::Constant(value)) if *value==Integer::from_u128(3))
        );
    }

    #[test]
    fn complemented_matches_require_exact_patterns_and_respect_budget() {
        use nir::program::MatchArm;
        let id = LocalId { owner: 7, index: 0 };
        let make = |inner| Pattern::Guard {
            pattern: Box::new(Pattern::Binding {
                binding: id,
                inner: Box::new(Pattern::Wild),
            }),
            condition: Box::new(expr(
                Primitive::Bool,
                K::Match {
                    value: Box::new(expr(Primitive::U8, K::Local(id))),
                    arms: vec![
                        MatchArm {
                            pattern: inner,
                            guard: None,
                            body: expr(Primitive::Bool, K::Literal(Literal::Bool(false))),
                        },
                        MatchArm {
                            pattern: Pattern::Wild,
                            guard: None,
                            body: expr(Primitive::Bool, K::Literal(Literal::Bool(true))),
                        },
                    ],
                },
            )),
        };
        let outside = make(Pattern::Literal(Literal::UnsignedInteger(3)));
        let ranges = extract(&outside, &mut 10000).unwrap();
        let point = extract(&Pattern::Literal(Literal::UnsignedInteger(3)), &mut 10000).unwrap();
        assert!(intersect(&ranges, &point, &mut 10000).unwrap().is_empty());
        let mut union = ranges;
        union.extend(point);
        assert!(complement(&union, &mut 10000).unwrap().is_empty());
        assert!(extract(&outside, &mut 1).is_err());
        assert!(!extract(&outside, &mut 10000).unwrap().is_empty());
        let unknown = make(Pattern::Guard {
            pattern: Box::new(Pattern::Wild),
            condition: Box::new(expr(
                Primitive::Bool,
                K::Local(LocalId {
                    owner: 7,
                    index: 99,
                }),
            )),
        });
        assert!(extract(&unknown, &mut 10000).unwrap().is_empty());
    }

    #[test]
    fn every_byte_split_is_covered_but_removing_one_point_is_not() {
        for split in 1..=255 {
            let left = range(
                Primitive::U8,
                Some(Literal::UnsignedInteger(0)),
                Some(Literal::UnsignedInteger(split)),
                false,
            );
            let right = range(
                Primitive::U8,
                Some(Literal::UnsignedInteger(split)),
                Some(Literal::UnsignedInteger(255)),
                true,
            );
            assert_eq!(
                check(
                    &BTreeMap::new(),
                    &Term::Primitive(Primitive::U8),
                    &[&left, &right],
                    100_000
                ),
                Coverage::Exhaustive
            );
            let hole = range(
                Primitive::U8,
                Some(Literal::UnsignedInteger(0)),
                Some(Literal::UnsignedInteger(split - 1)),
                false,
            );
            assert_eq!(
                check(
                    &BTreeMap::new(),
                    &Term::Primitive(Primitive::U8),
                    &[&hole, &right],
                    100_000
                ),
                Coverage::Missing
            );
        }
    }
    #[test]
    fn character_domain_excludes_surrogates_but_not_valid_boundary_points() {
        let low = range(
            Primitive::Char,
            Some(Literal::Char('\0')),
            Some(Literal::Char('\u{d7ff}')),
            true,
        );
        let high = range(
            Primitive::Char,
            Some(Literal::Char('\u{e000}')),
            Some(Literal::Char('\u{10ffff}')),
            true,
        );
        assert_eq!(
            check(
                &BTreeMap::new(),
                &Term::Primitive(Primitive::Char),
                &[&low, &high],
                100_000
            ),
            Coverage::Exhaustive
        );
        assert_eq!(
            check(
                &BTreeMap::new(),
                &Term::Primitive(Primitive::Char),
                &[&low],
                100_000
            ),
            Coverage::Missing
        );
    }
    #[test]
    fn integer_cuts_are_unbounded_and_budget_exhaustion_is_unknown() {
        let point = Literal::Integer(
            "100000000000000000000000000000000000000000000000000000000000000000000".into(),
        );
        let left = range(Primitive::Integer, None, Some(point.clone()), false);
        let right = range(Primitive::Integer, Some(point), None, false);
        assert_eq!(
            check(
                &BTreeMap::new(),
                &Term::Primitive(Primitive::Integer),
                &[&left, &right],
                100_000
            ),
            Coverage::Exhaustive
        );
        assert_eq!(
            check(
                &BTreeMap::new(),
                &Term::Primitive(Primitive::Integer),
                &[&left, &right],
                2
            ),
            Coverage::Unknown
        );
    }
    #[test]
    fn maximum_u128_and_minimum_i128_are_not_host_arithmetic_boundaries() {
        let unsigned = range(
            Primitive::U128,
            Some(Literal::UnsignedInteger(0)),
            Some(Literal::UnsignedInteger(u128::MAX)),
            true,
        );
        let signed = range(
            Primitive::I128,
            Some(Literal::SignedInteger(i128::MIN)),
            Some(Literal::SignedInteger(i128::MAX)),
            true,
        );
        assert_eq!(
            check(
                &BTreeMap::new(),
                &Term::Primitive(Primitive::U128),
                &[&unsigned],
                100_000
            ),
            Coverage::Exhaustive
        );
        assert_eq!(
            check(
                &BTreeMap::new(),
                &Term::Primitive(Primitive::I128),
                &[&signed],
                100_000
            ),
            Coverage::Exhaustive
        );
    }
}
