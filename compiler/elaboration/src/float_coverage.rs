//! Exact point and open-interval partitions for finite comptime Float patterns.
//! Pure symbolic coverage only: no invocation and no program facts enter PTC.
use comptime_runtime::{Budget, decimal::DecimalValue};
use nir::program::{Expr, ExprKind as K, Literal, LocalId, Pattern};
use nir::{Primitive, Term};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) enum Predicate {
    Constant(bool),
    Compare(Comparison, DecimalValue),
    Not(Box<Self>),
    And(Box<Self>, Box<Self>),
    Or(Vec<Self>),
}

#[derive(Clone, Copy)]
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
    spend(fuel, 1)?;
    if depth >= 128 {
        return Err(());
    }
    Ok(Some(match pattern {
        Pattern::Wild => Predicate::Constant(true),
        Pattern::Literal(Literal::Float(text)) => {
            Predicate::Compare(Comparison::Equal, decimal(text, fuel)?)
        }
        Pattern::Remap {
            pattern,
            bindings: remapping,
        } => {
            let mut local = copy_bindings(bindings, fuel)?;
            let Some(inner) = pattern_predicate(pattern, &mut local, fuel, depth + 1)? else {
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
        Pattern::Binding { binding, inner } => {
            let Some(inner) = pattern_predicate(inner, bindings, fuel, depth + 1)? else {
                return Ok(None);
            };
            if bindings.insert(*binding, Operand::Subject).is_some() {
                return Ok(None);
            }
            inner
        }
        Pattern::Guard { pattern, condition } => {
            let Some(pattern) = pattern_predicate(pattern, bindings, fuel, depth + 1)? else {
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
            let mut common: Option<BTreeMap<LocalId, Operand>> = None;
            for pattern in patterns {
                let mut local = copy_bindings(bindings, fuel)?;
                let Some(pattern) = pattern_predicate(pattern, &mut local, fuel, depth + 1)? else {
                    return Ok(None);
                };
                if let Some(common) = common.as_mut() {
                    spend(fuel, common.len())?;
                    let keys = common.keys().copied().collect::<Vec<_>>();
                    for key in keys {
                        let same = if let Some(other) = local.get(&key) {
                            charge_operand(&common[&key], fuel)?;
                            charge_operand(other, fuel)?;
                            common[&key] == *other
                        } else {
                            false
                        };
                        if !same {
                            common.remove(&key);
                        }
                    }
                } else {
                    common = Some(local);
                }
                alternatives.push(pattern);
            }
            if let Some(common) = common {
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
}
fn charge_operand(value: &Operand, fuel: &mut usize) -> Result<(), ()> {
    match value {
        Operand::Subject => spend(fuel, 1),
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
        K::Block {
            statements,
            result: Some(result),
        } => {
            let mut locals = copy_bindings(bindings, fuel)?;
            for statement in statements {
                spend(fuel, 1)?;
                let K::Let { binding, value } = &statement.kind else {
                    return Ok(None);
                };
                if statement.ty != Term::Primitive(Primitive::Void) {
                    return Ok(None);
                }
                let Some(value) = operand(value, &locals, fuel)? else {
                    return Ok(None);
                };
                if locals.insert(*binding, value).is_some() {
                    return Ok(None);
                }
            }
            let Some(result) = condition_predicate(result, &locals, fuel, depth + 1)? else {
                return Ok(None);
            };
            result
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
            let Some(right) = condition_predicate(right, bindings, fuel, depth + 1)? else {
                return Ok(None);
            };
            if matches!(expr.kind, K::And(..)) {
                Predicate::And(Box::new(left), Box::new(right))
            } else {
                Predicate::Or(vec![left, right])
            }
        }
        // The negation elaborator emits a match on the bound subject. Only
        // this closed, pure Boolean decision is interpreted as a complement.
        K::Match { value, arms } if value.ty == Term::Primitive(Primitive::Float) => {
            let K::Local(subject) = value.kind else {
                return Ok(None);
            };
            if !matches!(bindings.get(&subject), Some(Operand::Subject)) || arms.len() != 2 {
                return Ok(None);
            }
            if arms
                .iter()
                .any(|a| a.guard.is_some() || a.body.ty != Term::Primitive(Primitive::Bool))
                || !matches!(arms[0].body.kind, K::Literal(Literal::Bool(false)))
                || !matches!(arms[1].body.kind, K::Literal(Literal::Bool(true)))
                || !matches!(arms[1].pattern, Pattern::Wild)
            {
                return Ok(None);
            }
            let mut local = copy_bindings(bindings, fuel)?;
            let Some(inner) = pattern_predicate(&arms[0].pattern, &mut local, fuel, depth + 1)?
            else {
                return Ok(None);
            };
            Predicate::Not(Box::new(inner))
        }
        _ => return Ok(None),
    }))
}

impl Predicate {
    pub(crate) fn atoms<'a>(
        &'a self,
        output: &mut Vec<&'a DecimalValue>,
        fuel: &mut usize,
    ) -> Result<(), ()> {
        spend(fuel, 1)?;
        match self {
            Self::Compare(_, value) => output.push(value),
            Self::Not(inner) => inner.atoms(output, fuel)?,
            Self::And(left, right) => {
                left.atoms(output, fuel)?;
                right.atoms(output, fuel)?;
            }
            Self::Or(items) => {
                for item in items {
                    item.atoms(output, fuel)?;
                }
            }
            Self::Constant(_) => {}
        }
        Ok(())
    }
    pub(crate) fn matches(&self, cell: Cell<'_>, fuel: &mut usize) -> Result<bool, ()> {
        spend(fuel, 1)?;
        Ok(match self {
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
            Self::Not(inner) => !inner.matches(cell, fuel)?,
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
