//! Exact point and open-interval partitions for finite comptime Float patterns.
//! Pure symbolic coverage only: no invocation and no program facts enter PTC.
use comptime_runtime::{Budget, decimal::DecimalValue};
use nir::program::{Expr, ExprKind as K, Literal, LocalId, Pattern};
use nir::{Primitive, Term};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Predicate {
    // An opaque condition is harmless only when ordered evaluation skips it.
    // Reaching it cannot establish either truth or falsity.
    Unknown,
    Constant(bool),
    Compare(Comparison, DecimalValue),
    Not(Box<Self>),
    And(Box<Self>, Box<Self>),
    Or(Vec<Self>),
    Equal(Box<Self>, Box<Self>),
    If(Box<Self>, Box<Self>, Box<Self>),
    Select(Vec<(Self, Self)>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Comparison {
    Equal,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}
impl Comparison {
    fn reverse(self) -> Self {
        match self {
            Self::Equal => Self::Equal,
            Self::Less => Self::Greater,
            Self::LessEqual => Self::GreaterEqual,
            Self::Greater => Self::Less,
            Self::GreaterEqual => Self::LessEqual,
        }
    }
    fn accepts(self, order: std::cmp::Ordering) -> bool {
        match self {
            Self::Equal => order.is_eq(),
            Self::Less => order.is_lt(),
            Self::LessEqual => !order.is_gt(),
            Self::Greater => order.is_gt(),
            Self::GreaterEqual => !order.is_lt(),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Cell<'a> {
    Point(&'a DecimalValue),
    // The open cell immediately below this endpoint, after its predecessor.
    Below(&'a DecimalValue),
    AboveAll,
}
fn compare(
    left: &DecimalValue,
    right: &DecimalValue,
    fuel: &mut usize,
) -> Result<std::cmp::Ordering, ()> {
    let mut budget = Budget {
        steps: *fuel,
        max_limbs: 4096,
    };
    let result = left.compare(right, &mut budget);
    *fuel = budget.steps;
    result.map_err(|_| ())
}
pub(crate) fn cells<'a>(
    atoms: Vec<&'a DecimalValue>,
    fuel: &mut usize,
) -> Result<Vec<Cell<'a>>, ()> {
    let mut sorted: Vec<&DecimalValue> = Vec::new();
    for atom in atoms {
        spend(fuel, 1)?;
        let (mut lo, mut hi) = (0, sorted.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if compare(sorted[mid], atom, fuel)?.is_lt() {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo < sorted.len() && compare(sorted[lo], atom, fuel)?.is_eq() {
            continue;
        }
        spend(fuel, sorted.len() - lo + 1)?;
        sorted.insert(lo, atom);
    }
    spend(fuel, sorted.len().saturating_mul(2).saturating_add(1))?;
    let mut cells = Vec::new();
    for point in sorted {
        cells.push(Cell::Below(point));
        cells.push(Cell::Point(point));
    }
    cells.push(Cell::AboveAll);
    Ok(cells)
}

fn spend(fuel: &mut usize, count: usize) -> Result<(), ()> {
    *fuel = fuel.checked_sub(count).ok_or(())?;
    Ok(())
}
fn decimal(text: &str, fuel: &mut usize) -> Result<DecimalValue, ()> {
    let mut budget = Budget {
        steps: *fuel,
        max_limbs: 4096,
    };
    let result = DecimalValue::parse_literal(text, &mut budget).and_then(|value| {
        if value.is_zero() && value.is_negative() {
            value.negate(&mut budget)
        } else {
            Ok(value)
        }
    });
    *fuel = budget.steps;
    result.map_err(|_| ())
}
fn copy_bindings(
    bindings: &BTreeMap<LocalId, Operand>,
    fuel: &mut usize,
) -> Result<BTreeMap<LocalId, Operand>, ()> {
    spend(fuel, bindings.len())?;
    for value in bindings.values() {
        charge_operand(value, fuel)?;
    }
    Ok(bindings.clone())
}

pub(crate) fn extract(pattern: &Pattern, fuel: &mut usize) -> Result<Option<Predicate>, ()> {
    pattern_predicate(pattern, &mut BTreeMap::new(), fuel, 0)
}
fn pattern_predicate(
    pattern: &Pattern,
    bindings: &mut BTreeMap<LocalId, Operand>,
    fuel: &mut usize,
    depth: usize,
) -> Result<Option<Predicate>, ()> {
    pattern_subject(pattern, &Operand::Subject, bindings, fuel, depth)
}
fn pattern_subject(
    pattern: &Pattern,
    subject: &Operand,
    bindings: &mut BTreeMap<LocalId, Operand>,
    fuel: &mut usize,
    depth: usize,
) -> Result<Option<Predicate>, ()> {
    spend(fuel, 1)?;
    if depth >= 128 {
        return Err(());
    }
    Ok(Some(match pattern {
        Pattern::Wild => Predicate::Constant(true),
        Pattern::Literal(Literal::Bool(selected)) => {
            let Operand::Predicate(value) = subject else {
                return Ok(None);
            };
            value.charge(fuel)?;
            if *selected {
                (**value).clone()
            } else {
                Predicate::Not(value.clone())
            }
        }
        Pattern::Literal(Literal::Float(text)) => {
            let literal = decimal(text, fuel)?;
            match subject {
                Operand::Subject => Predicate::Compare(Comparison::Equal, literal),
                Operand::Constant(actual) => {
                    Predicate::Constant(compare(actual, &literal, fuel)?.is_eq())
                }
                Operand::Predicate(_) => return Ok(None),
            }
        }
        Pattern::Remap {
            pattern,
            bindings: remapping,
        } => {
            let mut local = copy_bindings(bindings, fuel)?;
            let Some(inner) = pattern_subject(pattern, subject, &mut local, fuel, depth + 1)?
            else {
                return Ok(None);
            };
            spend(fuel, remapping.len())?;
            let mut sources = BTreeSet::new();
            let mut targets = BTreeSet::new();
            let mut values = Vec::new();
            for (from, to) in remapping {
                if bindings.contains_key(from) || !sources.insert(*from) || !targets.insert(*to) {
                    return Ok(None);
                }
                let Some(value) = local.get(from) else {
                    return Ok(None);
                };
                charge_operand(value, fuel)?;
                values.push((*to, value.clone()));
            }
            for to in &targets {
                spend(fuel, 1)?;
                if local.contains_key(to) && !sources.contains(to) {
                    return Ok(None);
                }
            }
            for from in sources {
                local.remove(&from);
            }
            local.extend(values);
            *bindings = local;
            inner
        }
        Pattern::Observe { binding, inner } => {
            charge_operand(subject, fuel)?;
            let previous = bindings.insert(*binding, subject.clone());
            let result = pattern_subject(inner, subject, bindings, fuel, depth + 1);
            if let Some(previous) = previous {
                bindings.insert(*binding, previous);
            } else {
                bindings.remove(binding);
            }
            return result;
        }
        Pattern::Binding { binding, inner } => {
            let Some(inner) = pattern_subject(inner, subject, bindings, fuel, depth + 1)? else {
                return Ok(None);
            };
            charge_operand(subject, fuel)?;
            if bindings.insert(*binding, subject.clone()).is_some() {
                return Ok(None);
            }
            inner
        }
        Pattern::Then { first, value, next } => {
            let Some(first) = pattern_subject(first, subject, bindings, fuel, depth + 1)? else {
                return Ok(None);
            };
            let computed = if value.ty == Term::Primitive(Primitive::Bool) {
                condition_predicate(value, bindings, fuel, depth + 1)?
                    .map(|predicate| Operand::Predicate(Box::new(predicate)))
            } else {
                operand(value, bindings, fuel)?
            };
            let Some(computed) = computed else {
                return Ok(None);
            };
            let Some(next) = pattern_subject(next, &computed, bindings, fuel, depth + 1)? else {
                return Ok(None);
            };
            // Evaluating a boolean computation remains observable to coverage
            // even when the suffix discards its result. Unknown must not vanish
            // behind a wildcard or a constant suffix predicate.
            let observed = operand_evaluation(&computed, fuel)?;
            Predicate::And(
                Box::new(first),
                Box::new(Predicate::And(Box::new(observed), Box::new(next))),
            )
        }
        Pattern::Guard { pattern, condition } => {
            let Some(pattern) = pattern_subject(pattern, subject, bindings, fuel, depth + 1)?
            else {
                return Ok(None);
            };
            let Some(condition) = condition_predicate(condition, bindings, fuel, depth + 1)? else {
                return Ok(None);
            };
            Predicate::And(Box::new(pattern), Box::new(condition))
        }
        Pattern::Or(patterns) => {
            spend(fuel, patterns.len())?;
            let mut alternatives = Vec::new();
            let mut environments = Vec::new();
            for pattern in patterns {
                let mut local = copy_bindings(bindings, fuel)?;
                let Some(pattern) = pattern_subject(pattern, subject, &mut local, fuel, depth + 1)?
                else {
                    return Ok(None);
                };
                environments.push(local);
                alternatives.push(pattern);
            }
            if let Some(first) = environments.first() {
                let mut common = BTreeMap::new();
                for (key, value) in first {
                    spend(fuel, 1)?;
                    let mut identical = true;
                    let mut available = true;
                    for environment in &environments {
                        spend(fuel, 1)?;
                        let Some(other) = environment.get(key) else {
                            available = false;
                            break;
                        };
                        charge_operand(value, fuel)?;
                        charge_operand(other, fuel)?;
                        identical &= value == other;
                    }
                    if !available {
                        continue;
                    }
                    if identical {
                        charge_operand(value, fuel)?;
                        common.insert(*key, value.clone());
                        continue;
                    }
                    // Boolean results retain the choice which exported them.
                    // Select tests candidates in order; an outer guard cannot
                    // retry a later candidate after an earlier one matched.
                    let mut choices = Vec::new();
                    for (selected, environment) in alternatives.iter().zip(&environments) {
                        spend(fuel, 1)?;
                        let Some(Operand::Predicate(value)) = environment.get(key) else {
                            choices.clear();
                            break;
                        };
                        selected.charge(fuel)?;
                        value.charge(fuel)?;
                        choices.push((selected.clone(), (**value).clone()));
                    }
                    if !choices.is_empty() {
                        common.insert(
                            *key,
                            Operand::Predicate(Box::new(Predicate::Select(choices))),
                        );
                    }
                }
                *bindings = common;
            }
            Predicate::Or(alternatives)
        }
        _ => return Ok(None),
    }))
}

#[derive(Clone, PartialEq, Eq)]
enum Operand {
    Subject,
    Constant(DecimalValue),
    Predicate(Box<Predicate>),
}
fn charge_operand(value: &Operand, fuel: &mut usize) -> Result<(), ()> {
    match value {
        Operand::Subject => spend(fuel, 1),
        Operand::Predicate(value) => value.charge(fuel),
        Operand::Constant(value) => spend(
            fuel,
            value
                .digits()
                .len()
                .saturating_add(value.exponent().bit_len().div_ceil(32))
                .saturating_add(1),
        ),
    }
}
// Retain evaluation even if an initialized binding is never read. A
// conditional predicate may contain an opaque branch which is only harmless
// when that branch is skipped for the current subject.
fn operand_evaluation(value: &Operand, fuel: &mut usize) -> Result<Predicate, ()> {
    Ok(match value {
        Operand::Predicate(value) => {
            value.charge(fuel)?;
            Predicate::If(
                value.clone(),
                Box::new(Predicate::Constant(true)),
                Box::new(Predicate::Constant(true)),
            )
        }
        _ => Predicate::Constant(true),
    })
}
fn operand(
    expr: &Expr,
    bindings: &BTreeMap<LocalId, Operand>,
    fuel: &mut usize,
) -> Result<Option<Operand>, ()> {
    spend(fuel, 1)?;
    if expr.ty != Term::Primitive(Primitive::Float) {
        return Ok(None);
    }
    Ok(match &expr.kind {
        K::Local(id) => {
            let Some(value) = bindings.get(id) else {
                return Ok(None);
            };
            charge_operand(value, fuel)?;
            Some(value.clone())
        }
        K::Literal(Literal::Float(text)) => Some(Operand::Constant(decimal(text, fuel)?)),
        _ => None,
    })
}

fn condition_predicate(
    expr: &Expr,
    bindings: &BTreeMap<LocalId, Operand>,
    fuel: &mut usize,
    depth: usize,
) -> Result<Option<Predicate>, ()> {
    spend(fuel, 1)?;
    if depth >= 128 {
        return Err(());
    }
    if expr.ty != Term::Primitive(Primitive::Bool) {
        return Ok(None);
    }
    Ok(Some(match &expr.kind {
        K::Literal(Literal::Bool(value)) => Predicate::Constant(*value),
        K::Local(id) => {
            let Some(Operand::Predicate(value)) = bindings.get(id) else {
                return Ok(None);
            };
            value.charge(fuel)?;
            (**value).clone()
        }
        K::If {
            condition,
            then_branch,
            else_branch: Some(else_branch),
        } => {
            let Some(condition) = condition_predicate(condition, bindings, fuel, depth + 1)? else {
                return Ok(None);
            };
            let then_branch = condition_predicate(then_branch, bindings, fuel, depth + 1)?
                .unwrap_or(Predicate::Unknown);
            let else_branch = condition_predicate(else_branch, bindings, fuel, depth + 1)?
                .unwrap_or(Predicate::Unknown);
            Predicate::If(
                Box::new(condition),
                Box::new(then_branch),
                Box::new(else_branch),
            )
        }
        K::ScalarEqual(left, right)
            if left.ty == Term::Primitive(Primitive::Bool) && right.ty == left.ty =>
        {
            let Some(left) = condition_predicate(left, bindings, fuel, depth + 1)? else {
                return Ok(None);
            };
            let Some(right) = condition_predicate(right, bindings, fuel, depth + 1)? else {
                return Ok(None);
            };
            Predicate::Equal(Box::new(left), Box::new(right))
        }
        K::Block {
            statements,
            result: Some(result),
        } => {
            let mut locals = copy_bindings(bindings, fuel)?;
            let mut owned = BTreeSet::new();
            let mut evaluations = Vec::new();
            for statement in statements {
                spend(fuel, 1)?;
                let statement = match &statement.kind {
                    K::Discard(inner) => inner.as_ref(),
                    _ => statement,
                };
                if statement.ty != Term::Primitive(Primitive::Void) {
                    return Ok(None);
                }
                let (binding, value, declaration) = match &statement.kind {
                    K::Let { binding, value } => (*binding, value, true),
                    K::Assign { place, value }
                        if place.fields.is_empty() && owned.contains(&place.root) =>
                    {
                        (place.root, value, false)
                    }
                    _ => return Ok(None),
                };
                let value = if value.ty == Term::Primitive(Primitive::Bool) {
                    condition_predicate(value, &locals, fuel, depth + 1)?
                        .map(|value| Operand::Predicate(Box::new(value)))
                } else {
                    operand(value, &locals, fuel)?
                };
                let Some(value) = value else {
                    return Ok(None);
                };
                evaluations.push(operand_evaluation(&value, fuel)?);
                if declaration && locals.contains_key(&binding) {
                    return Ok(None);
                }
                locals.insert(binding, value);
                if declaration {
                    owned.insert(binding);
                }
            }
            let Some(result) = condition_predicate(result, &locals, fuel, depth + 1)? else {
                return Ok(None);
            };
            evaluations
                .into_iter()
                .rev()
                .fold(result, |result, evaluation| {
                    Predicate::And(Box::new(evaluation), Box::new(result))
                })
        }

        K::ScalarEqual(left, right)
        | K::ScalarLess(left, right)
        | K::ScalarLessEqual(left, right)
        | K::ScalarGreater(left, right)
        | K::ScalarGreaterEqual(left, right) => {
            let op = match expr.kind {
                K::ScalarEqual(..) => Comparison::Equal,
                K::ScalarLess(..) => Comparison::Less,
                K::ScalarLessEqual(..) => Comparison::LessEqual,
                K::ScalarGreater(..) => Comparison::Greater,
                _ => Comparison::GreaterEqual,
            };
            let Some(left) = operand(left, bindings, fuel)? else {
                return Ok(None);
            };
            let Some(right) = operand(right, bindings, fuel)? else {
                return Ok(None);
            };
            match (left, right) {
                (Operand::Subject, Operand::Subject) => {
                    Predicate::Constant(op.accepts(std::cmp::Ordering::Equal))
                }
                (Operand::Subject, Operand::Constant(value)) => Predicate::Compare(op, value),
                (Operand::Constant(value), Operand::Subject) => {
                    Predicate::Compare(op.reverse(), value)
                }
                (Operand::Constant(left), Operand::Constant(right)) => {
                    Predicate::Constant(op.accepts(compare(&left, &right, fuel)?))
                }
                _ => return Ok(None),
            }
        }

        K::Not(inner) => {
            let Some(inner) = condition_predicate(inner, bindings, fuel, depth + 1)? else {
                return Ok(None);
            };
            Predicate::Not(Box::new(inner))
        }
        K::And(left, right) | K::Or(left, right) => {
            let Some(left) = condition_predicate(left, bindings, fuel, depth + 1)? else {
                return Ok(None);
            };
            let right = condition_predicate(right, bindings, fuel, depth + 1)?
                .unwrap_or(Predicate::Unknown);
            if matches!(expr.kind, K::And(..)) {
                Predicate::And(Box::new(left), Box::new(right))
            } else {
                Predicate::Or(vec![left, right])
            }
        }
        K::Match { value, arms } => {
            let subject = if value.ty == Term::Primitive(Primitive::Bool) {
                let Some(value) = condition_predicate(value, bindings, fuel, depth + 1)? else {
                    return Ok(None);
                };
                Operand::Predicate(Box::new(value))
            } else {
                let Some(subject @ (Operand::Subject | Operand::Constant(_))) =
                    operand(value, bindings, fuel)?
                else {
                    return Ok(None);
                };
                subject
            };
            spend(fuel, arms.len())?;
            let mut choices = Vec::new();
            for arm in arms {
                let mut local = copy_bindings(bindings, fuel)?;
                let Some(mut selected) =
                    pattern_subject(&arm.pattern, &subject, &mut local, fuel, depth + 1)?
                else {
                    return Ok(None);
                };
                if let Some(guard) = &arm.guard {
                    let Some(guard) = condition_predicate(guard, &local, fuel, depth + 1)? else {
                        return Ok(None);
                    };
                    selected = Predicate::And(Box::new(selected), Box::new(guard));
                }
                let body = condition_predicate(&arm.body, &local, fuel, depth + 1)?
                    .unwrap_or(Predicate::Unknown);
                choices.push((selected, body));
            }
            Predicate::And(
                Box::new(operand_evaluation(&subject, fuel)?),
                Box::new(Predicate::Select(choices)),
            )
        }
        _ => return Ok(None),
    }))
}

impl Predicate {
    fn charge(&self, fuel: &mut usize) -> Result<(), ()> {
        let mut pending = vec![(self, 0usize)];
        while let Some((value, depth)) = pending.pop() {
            spend(fuel, 1)?;
            if depth >= 128 {
                return Err(());
            }
            match value {
                Self::Constant(_) | Self::Unknown => (),
                Self::Compare(_, value) => spend(
                    fuel,
                    value
                        .digits()
                        .len()
                        .saturating_add(value.exponent().bit_len().div_ceil(32))
                        .saturating_add(1),
                )?,
                Self::Not(inner) => pending.push((inner, depth + 1)),
                Self::And(a, b) | Self::Equal(a, b) => {
                    pending.push((a, depth + 1));
                    pending.push((b, depth + 1));
                }
                Self::If(a, b, c) => {
                    pending.push((a, depth + 1));
                    pending.push((b, depth + 1));
                    pending.push((c, depth + 1));
                }
                Self::Select(items) => {
                    spend(fuel, items.len())?;
                    for (pattern, body) in items {
                        pending.push((pattern, depth + 1));
                        pending.push((body, depth + 1));
                    }
                }
                Self::Or(items) => {
                    spend(fuel, items.len())?;
                    pending.extend(items.iter().map(|item| (item, depth + 1)));
                }
            }
        }
        Ok(())
    }
    pub(crate) fn atoms<'a>(
        &'a self,
        output: &mut Vec<&'a DecimalValue>,
        fuel: &mut usize,
    ) -> Result<(), ()> {
        spend(fuel, 1)?;
        match self {
            Self::Compare(_, value) => output.push(value),
            Self::Not(inner) => inner.atoms(output, fuel)?,
            Self::If(condition, then_branch, else_branch) => {
                condition.atoms(output, fuel)?;
                then_branch.atoms(output, fuel)?;
                else_branch.atoms(output, fuel)?;
            }
            Self::And(left, right) | Self::Equal(left, right) => {
                left.atoms(output, fuel)?;
                right.atoms(output, fuel)?;
            }
            Self::Or(items) => {
                for item in items {
                    item.atoms(output, fuel)?;
                }
            }
            Self::Select(items) => {
                for (pattern, body) in items {
                    pattern.atoms(output, fuel)?;
                    body.atoms(output, fuel)?;
                }
            }
            Self::Constant(_) | Self::Unknown => {}
        }
        Ok(())
    }
    pub(crate) fn matches(&self, cell: Cell<'_>, fuel: &mut usize) -> Result<bool, ()> {
        spend(fuel, 1)?;
        Ok(match self {
            Self::Unknown => return Err(()),
            Self::Constant(value) => *value,
            Self::Compare(op, expected) => {
                let order = match cell {
                    Cell::Point(actual) => compare(actual, expected, fuel)?,
                    Cell::Below(upper) => {
                        if compare(upper, expected, fuel)?.is_gt() {
                            std::cmp::Ordering::Greater
                        } else {
                            std::cmp::Ordering::Less
                        }
                    }
                    Cell::AboveAll => std::cmp::Ordering::Greater,
                };
                op.accepts(order)
            }
            Self::Select(items) => {
                for (pattern, body) in items {
                    if pattern.matches(cell, fuel)? {
                        return body.matches(cell, fuel);
                    }
                }
                // An uncovered nested decision is unknown, not false: taking
                // its complement must not invent a successful coverage proof.
                return Err(());
            }
            Self::Not(inner) => !inner.matches(cell, fuel)?,
            Self::Equal(left, right) => left.matches(cell, fuel)? == right.matches(cell, fuel)?,
            Self::If(condition, then_branch, else_branch) => {
                if condition.matches(cell, fuel)? {
                    then_branch.matches(cell, fuel)?
                } else {
                    else_branch.matches(cell, fuel)?
                }
            }
            Self::And(left, right) => left.matches(cell, fuel)? && right.matches(cell, fuel)?,
            Self::Or(items) => {
                let mut result = false;
                for item in items {
                    if item.matches(cell, fuel)? {
                        result = true;
                        break;
                    }
                }
                result
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opaque_conditions_are_unknown_only_when_reached() {
        let cell = Cell::AboveAll;
        for (predicate, expected) in [
            (
                Predicate::Or(vec![Predicate::Constant(true), Predicate::Unknown]),
                true,
            ),
            (
                Predicate::And(
                    Box::new(Predicate::Constant(false)),
                    Box::new(Predicate::Unknown),
                ),
                false,
            ),
            (
                Predicate::If(
                    Box::new(Predicate::Constant(false)),
                    Box::new(Predicate::Unknown),
                    Box::new(Predicate::Constant(true)),
                ),
                true,
            ),
            (
                Predicate::Select(vec![
                    (Predicate::Constant(true), Predicate::Constant(false)),
                    (Predicate::Constant(true), Predicate::Unknown),
                ]),
                false,
            ),
        ] {
            assert!(predicate.charge(&mut 0).is_err());
            predicate.charge(&mut 1000).unwrap();
            let mut atoms = Vec::new();
            predicate.atoms(&mut atoms, &mut 1000).unwrap();
            assert!(atoms.is_empty());
            assert_eq!(predicate.matches(cell, &mut 1000).unwrap(), expected);
        }
        for predicate in [
            Predicate::Unknown,
            Predicate::Not(Box::new(Predicate::Unknown)),
            Predicate::Equal(Box::new(Predicate::Unknown), Box::new(Predicate::Unknown)),
            Predicate::Or(vec![Predicate::Unknown, Predicate::Constant(true)]),
            Predicate::And(
                Box::new(Predicate::Unknown),
                Box::new(Predicate::Constant(false)),
            ),
        ] {
            assert!(predicate.matches(cell, &mut 1000).is_err());
        }
    }
    #[test]
    fn ordered_decisions_preserve_first_match_and_uncovered_errors() {
        let zero = decimal("0.0", &mut 1000).unwrap();
        let negative = decimal("-1.0", &mut 1000).unwrap();
        let positive = decimal("1.0", &mut 1000).unwrap();
        let incomplete = Predicate::Select(vec![(
            Predicate::Compare(Comparison::Less, zero),
            Predicate::Constant(false),
        )]);
        assert!(
            !incomplete
                .matches(Cell::Point(&negative), &mut 1000)
                .unwrap()
        );
        assert!(
            incomplete
                .matches(Cell::Point(&positive), &mut 1000)
                .is_err()
        );
        let negated = Predicate::Not(Box::new(incomplete));
        assert!(negated.matches(Cell::Point(&positive), &mut 1000).is_err());
        assert!(negated.charge(&mut 1).is_err());
        negated.charge(&mut 1000).unwrap();
        assert!(negated.matches(Cell::Point(&negative), &mut 1000).unwrap());
        let overlapping = Predicate::Select(vec![
            (Predicate::Constant(true), Predicate::Constant(false)),
            (Predicate::Constant(true), Predicate::Constant(true)),
        ]);
        assert!(
            !overlapping
                .matches(Cell::Point(&positive), &mut 1000)
                .unwrap()
        );
    }
    #[test]
    fn ordered_cells_match_independent_endpoint_and_open_interval_ranks() {
        let value = |text| decimal(text, &mut 100_000).unwrap();
        let values = [value("-1.0"), value("-0.0"), value("1.0"), value("0.0")];
        let atoms = vec![&values[2], &values[1], &values[0], &values[3], &values[2]];
        assert!(cells(atoms.clone(), &mut 0).is_err());
        let cells = cells(atoms, &mut 100_000).unwrap();
        assert_eq!(cells.len(), 7);
        for (cell_index, cell) in cells.into_iter().enumerate() {
            for (point_index, point) in values[..3].iter().enumerate() {
                let point_rank = point_index * 2 + 1;
                for (op_index, op) in [
                    Comparison::Equal,
                    Comparison::Less,
                    Comparison::LessEqual,
                    Comparison::Greater,
                    Comparison::GreaterEqual,
                ]
                .into_iter()
                .enumerate()
                {
                    let expected = [
                        cell_index == point_rank,
                        cell_index < point_rank,
                        cell_index <= point_rank,
                        cell_index > point_rank,
                        cell_index >= point_rank,
                    ][op_index];
                    let predicate = Predicate::Compare(op, point.clone());
                    assert_eq!(
                        predicate.matches(cell, &mut 100_000).unwrap(),
                        expected,
                        "cell={cell_index} point={point_index} op={op_index}"
                    );
                }
            }
        }
    }
    #[test]
    fn range_endpoint_bindings_are_scoped_and_unsupported_statements_supply_no_proof() {
        let subject = LocalId { owner: 1, index: 0 };
        let endpoint = LocalId { owner: 1, index: 1 };
        let expr = |kind, ty| Expr {
            kind,
            ty: Term::Primitive(ty),
            source: nir::program::SourceRange { start: 0, end: 1 },
        };
        let local = |id| expr(K::Local(id), Primitive::Float);
        let init = expr(
            K::Let {
                binding: endpoint,
                value: Box::new(expr(
                    K::Literal(Literal::Float("1.0".into())),
                    Primitive::Float,
                )),
            },
            Primitive::Void,
        );
        let condition = expr(
            K::Block {
                statements: vec![init.clone()],
                result: Some(Box::new(expr(
                    K::ScalarLess(Box::new(local(subject)), Box::new(local(endpoint))),
                    Primitive::Bool,
                ))),
            },
            Primitive::Bool,
        );
        let pattern = |condition| Pattern::Guard {
            pattern: Box::new(Pattern::Binding {
                binding: subject,
                inner: Box::new(Pattern::Wild),
            }),
            condition: Box::new(condition),
        };
        assert!(extract(&pattern(condition.clone()), &mut 0).is_err());
        let predicate = extract(&pattern(condition.clone()), &mut 10000)
            .unwrap()
            .unwrap();
        for (text, expected) in [("0.5", true), ("1.0", false), ("2.0", false)] {
            let value = decimal(text, &mut 10000).unwrap();
            assert_eq!(
                predicate.matches(Cell::Point(&value), &mut 10000).unwrap(),
                expected
            );
        }
        let mut assigned = condition.clone();
        let K::Block { statements, .. } = &mut assigned.kind else {
            unreachable!()
        };
        statements.push(expr(
            K::Assign {
                place: nir::program::Place {
                    root: endpoint,
                    fields: vec![],
                },
                value: Box::new(expr(
                    K::Literal(Literal::Float("2.0".into())),
                    Primitive::Float,
                )),
            },
            Primitive::Void,
        ));
        let predicate = extract(&pattern(assigned.clone()), &mut 10_000)
            .unwrap()
            .unwrap();
        for (text, expected) in [("1.5", true), ("2.0", false), ("2.5", false)] {
            let value = decimal(text, &mut 10_000).unwrap();
            assert_eq!(
                predicate.matches(Cell::Point(&value), &mut 10_000).unwrap(),
                expected
            );
        }
        let K::Block { statements, .. } = &mut assigned.kind else {
            unreachable!()
        };
        let K::Assign { place, .. } = &mut statements.last_mut().unwrap().kind else {
            unreachable!()
        };
        place.root = subject;
        assert!(extract(&pattern(assigned), &mut 10_000).unwrap().is_none());
        let mut duplicate = condition.clone();
        let K::Block { statements, .. } = &mut duplicate.kind else {
            unreachable!()
        };
        statements.push(init);
        assert!(extract(&pattern(duplicate), &mut 10000).unwrap().is_none());
        let mut unsupported = condition;
        let K::Block { statements, .. } = &mut unsupported.kind else {
            unreachable!()
        };
        statements.push(expr(K::Discard(Box::new(local(subject))), Primitive::Void));
        assert!(
            extract(&pattern(unsupported), &mut 10000)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn remaps_are_simultaneous_and_cannot_capture_or_merge_ambient_bindings() {
        let id = |index| LocalId { owner: 1, index };
        let bind = |binding, inner| Pattern::Binding {
            binding,
            inner: Box::new(inner),
        };
        let pair = bind(id(1), bind(id(2), Pattern::Wild));
        let valid = Pattern::Remap {
            pattern: Box::new(pair.clone()),
            bindings: vec![(id(1), id(2)), (id(2), id(1))],
        };
        let mut ambient = BTreeMap::from([(id(0), Operand::Subject)]);
        assert!(pattern_predicate(&valid, &mut ambient, &mut 0, 0).is_err());
        assert!(
            pattern_predicate(&valid, &mut ambient, &mut 10000, 0)
                .unwrap()
                .is_some()
        );
        assert!(ambient.contains_key(&id(1)) && ambient.contains_key(&id(2)));
        for mapping in [
            vec![(id(0), id(3))],
            vec![(id(1), id(0))],
            vec![(id(1), id(3)), (id(2), id(3))],
            vec![(id(1), id(3)), (id(1), id(4))],
            vec![(id(9), id(3))],
            vec![(id(1), id(2))],
        ] {
            let pattern = Pattern::Remap {
                pattern: Box::new(pair.clone()),
                bindings: mapping,
            };
            let mut ambient = BTreeMap::from([(id(0), Operand::Subject)]);
            assert!(
                pattern_predicate(&pattern, &mut ambient, &mut 10000, 0)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(ambient.len(), 1);
        }
    }

    #[test]
    fn alternatives_export_only_bindings_present_in_every_candidate() {
        let id = LocalId { owner: 1, index: 0 };
        let bound = Pattern::Binding {
            binding: id,
            inner: Box::new(Pattern::Wild),
        };
        let pattern = Pattern::Or(vec![bound.clone(), Pattern::Wild]);
        let mut bindings = BTreeMap::new();
        assert!(
            pattern_predicate(&pattern, &mut bindings, &mut 10000, 0)
                .unwrap()
                .is_some()
        );
        assert!(!bindings.contains_key(&id));
        let pattern = Pattern::Or(vec![bound.clone(), bound]);
        assert!(
            pattern_predicate(&pattern, &mut bindings, &mut 10000, 0)
                .unwrap()
                .is_some()
        );
        assert!(matches!(bindings.get(&id), Some(Operand::Subject)));
    }
}

#[cfg(test)]
mod sequential_computation_tests {
    use super::*;
    use nir::program::SourceRange;
    fn boolean(kind: K) -> Expr {
        Expr {
            ty: Term::Primitive(Primitive::Bool),
            source: SourceRange { start: 0, end: 0 },
            kind,
        }
    }
    #[test]
    fn discarded_computation_keeps_unknown_and_short_circuit_behavior() {
        let unknown = || {
            boolean(K::Call {
                callee: Box::new(Expr {
                    ty: Term::Function {
                        parameters: vec![],
                        result: Box::new(Term::Primitive(Primitive::Bool)),
                    },
                    source: SourceRange { start: 0, end: 0 },
                    kind: K::Local(LocalId { owner: 9, index: 0 }),
                }),
                arguments: vec![],
                parameter_bindings: vec![],
            })
        };
        for (computed, known) in [
            (unknown(), false),
            (
                boolean(K::Or(
                    Box::new(boolean(K::Literal(Literal::Bool(false)))),
                    Box::new(unknown()),
                )),
                false,
            ),
            (
                boolean(K::Or(
                    Box::new(boolean(K::Literal(Literal::Bool(true)))),
                    Box::new(unknown()),
                )),
                true,
            ),
            (
                boolean(K::And(
                    Box::new(boolean(K::Literal(Literal::Bool(false)))),
                    Box::new(unknown()),
                )),
                true,
            ),
        ] {
            let pattern = Pattern::Then {
                first: Box::new(Pattern::Wild),
                value: Box::new(computed),
                next: Box::new(Pattern::Wild),
            };
            let actual = extract(&pattern, &mut 100_000)
                .unwrap()
                .and_then(|predicate| predicate.matches(Cell::AboveAll, &mut 100_000).ok());
            assert_eq!(actual, known.then_some(true));
            assert!(extract(&pattern, &mut 1).is_err());
        }
    }
}
