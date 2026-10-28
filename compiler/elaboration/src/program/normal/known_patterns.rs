//! Bounded inspection of immutable match inputs. Unknown is distinct from
//! mismatch: an undecidable earlier candidate cannot be skipped.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn known_normal_pattern(
        &self,
        pattern: &n::Pattern,
        value: &Term,
        depth: usize,
        budget: &mut comptime_runtime::Budget,
        bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Option<bool> {
        PatternInspector {
            tuple_head: self.protocols.tuple,
            evaluate:
                |expr: &n::Expr,
                 depth,
                 budget: &mut comptime_runtime::Budget,
                 bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>| {
                    self.known_normal_input_in(expr, depth, budget, bindings)
                },
        }
        .inspect(pattern, value, depth, budget, bindings)
    }
}

pub(in crate::program) struct PatternInspector<E> {
    pub(in crate::program) evaluate: E,
    pub(in crate::program) tuple_head: Option<nir::DeclarationId>,
}

impl<E> PatternInspector<E>
where
    E: FnMut(
        &n::Expr,
        usize,
        &mut comptime_runtime::Budget,
        &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Option<Term>,
{
    pub(in crate::program) fn inspect(
        &mut self,
        pattern: &n::Pattern,
        value: &Term,
        depth: usize,
        budget: &mut comptime_runtime::Budget,
        bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Option<bool> {
        if depth >= 128 || budget.consume(1).is_err() {
            return None;
        }
        use n::Pattern as Pat;
        match pattern {
            Pat::OptionalAbsent => match value {
                Term::OptionalValue { value, .. } => Some(value.is_none()),
                _ => None,
            },
            Pat::OptionalPresent(inner) => match value {
                Term::OptionalValue {
                    value: Some(value), ..
                } => self.inspect(inner, value, depth + 1, budget, bindings),
                Term::OptionalValue { value: None, .. } => Some(false),
                _ => None,
            },
            Pat::CodeEvidence { .. } => None,
            Pat::ErrorResult { label, inner } => {
                if value.has_pending_error_branch() {
                    return None;
                }
                let Term::ErrorValue {
                    label: actual,
                    value,
                    ..
                } = value
                else {
                    return None;
                };
                match (label.as_ref(), actual.as_deref()) {
                    (None, None) => self.inspect(inner, value, depth + 1, budget, bindings),
                    (Some(a), Some(b)) if a == b => {
                        self.inspect(inner, value, depth + 1, budget, bindings)
                    }
                    (None, Some(_)) | (Some(_), None) => Some(false),
                    _ => None,
                }
            }
            Pat::Observe { .. } | Pat::NominalType { .. } => None,
            Pat::Wild => Some(true),
            Pat::Unit => match value {
                Term::Unit => Some(true),
                _ => None,
            },
            Pat::Binding { binding, inner } => {
                if !self.inspect(inner, value, depth + 1, budget, bindings)? {
                    return Some(false);
                }
                bindings.insert(
                    *binding,
                    std::sync::Arc::new(clone_known_term(value, budget)?),
                );
                Some(true)
            }
            Pat::Or(alternatives) => {
                for alternative in alternatives {
                    budget.consume(bindings.len().saturating_add(1)).ok()?;
                    let mut candidate = bindings.clone();
                    if self.inspect(alternative, value, depth + 1, budget, &mut candidate)? {
                        *bindings = candidate;
                        return Some(true);
                    }
                }
                Some(false)
            }
            Pat::Guard { pattern, condition } => {
                if !self.inspect(pattern, value, depth + 1, budget, bindings)? {
                    return Some(false);
                }
                match (self.evaluate)(condition, depth + 1, budget, bindings)? {
                    Term::Bool(accepted) => Some(accepted),
                    _ => None,
                }
            }
            Pat::Then {
                first,
                value: next_value,
                next,
            } => {
                if !self.inspect(first, value, depth + 1, budget, bindings)? {
                    return Some(false);
                }
                let next_value = (self.evaluate)(next_value, depth + 1, budget, bindings)?;
                self.inspect(next, &next_value, depth + 1, budget, bindings)
            }
            Pat::Remap {
                pattern,
                bindings: remap,
            } => {
                budget
                    .consume(bindings.len().saturating_add(remap.len()))
                    .ok()?;
                let ambient: BTreeSet<_> = bindings.keys().copied().collect();
                if !self.inspect(pattern, value, depth + 1, budget, bindings)? {
                    return Some(false);
                }
                let mut sources = BTreeSet::new();
                let mut targets = BTreeSet::new();
                for (from, to) in remap {
                    if ambient.contains(from) || !sources.insert(*from) || !targets.insert(*to) {
                        return None;
                    }
                }
                let remapped = remap
                    .iter()
                    .map(|(from, to)| Some((*to, bindings.get(from)?.clone())))
                    .collect::<Option<Vec<_>>>()?;
                if targets.iter().any(|target| {
                    bindings.contains_key(target)
                        && !ambient.contains(target)
                        && !sources.contains(target)
                }) {
                    return None;
                }
                bindings.extend(remapped);
                for source in sources.difference(&targets) {
                    bindings.remove(source);
                }
                Some(true)
            }
            Pat::Tuple(patterns) | Pat::TypeTuple(patterns) => {
                let values = match (pattern, value) {
                    (Pat::Tuple(_), Term::TupleValue(values))
                    | (Pat::TypeTuple(_), Term::Tuple(values)) => values,
                    (
                        Pat::TypeTuple(_),
                        Term::NormalForm {
                            head,
                            sort: Sort::Type,
                            arguments,
                        },
                    ) if Some(*head) == self.tuple_head => {
                        let [Term::TypeListValue(values)] = arguments.as_slice() else {
                            return None;
                        };
                        values
                    }
                    (Pat::TypeTuple(_), Term::Primitive(P::Unit)) => return None,
                    (
                        Pat::TypeTuple(_),
                        Term::Primitive(_)
                        | Term::Pointer(_)
                        | Term::Function { .. }
                        | Term::ComptimeFunction(_)
                        | Term::UnsafeFunction(_),
                    ) => return Some(false),
                    _ => return None,
                };
                if patterns.len() != values.len() {
                    return Some(false);
                }
                for (pattern, value) in patterns.iter().zip(values) {
                    if !self.inspect(pattern, value, depth + 1, budget, bindings)? {
                        return Some(false);
                    }
                }
                Some(true)
            }
            Pat::Record {
                declaration,
                fields,
            }
            | Pat::Variant {
                declaration,
                fields,
                ..
            } => {
                let Term::NominalValue {
                    ty,
                    variant,
                    fields: values,
                } = value
                else {
                    return None;
                };
                let Term::Nominal {
                    declaration: actual,
                    ..
                } = ty.as_ref()
                else {
                    return None;
                };
                if actual != declaration {
                    return Some(false);
                }
                let expected_variant = match pattern {
                    Pat::Variant { index, .. } => Some(u32::try_from(*index).ok()?),
                    _ => None,
                };
                if *variant != expected_variant {
                    return Some(false);
                }
                if fields.len() != values.len() {
                    return None;
                }
                for (pattern, value) in fields.iter().zip(values) {
                    if !self.inspect(pattern, value, depth + 1, budget, bindings)? {
                        return Some(false);
                    }
                }
                Some(true)
            }
            Pat::AnonymousRecord { names, fields } => {
                let Term::RecordValue(values) = value else {
                    return None;
                };
                if names.len() != fields.len() || fields.len() != values.len() {
                    return None;
                }
                for ((name, pattern), (actual, value)) in names.iter().zip(fields).zip(values) {
                    budget
                        .consume(name.len().saturating_add(actual.len()).saturating_add(1))
                        .ok()?;
                    if name != actual {
                        return None;
                    }
                    if !self.inspect(pattern, value, depth + 1, budget, bindings)? {
                        return Some(false);
                    }
                }
                Some(true)
            }
            Pat::NormalForm { head, arguments } => {
                let (actual, values) = match value {
                    Term::NormalForm {
                        head,
                        sort: Sort::Type,
                        arguments,
                    } => (head, arguments),
                    // These constructors establish a different outer identity,
                    // even when a nested component is still symbolic.
                    Term::Primitive(_)
                    | Term::Nominal { .. }
                    | Term::Tuple(_)
                    | Term::Record(_)
                    | Term::Pointer(_)
                    | Term::Function { .. }
                    | Term::ComptimeFunction(_)
                    | Term::UnsafeFunction(_)
                    | Term::Closure { .. } => return Some(false),
                    // Bound/hole/rigid/associated subjects do not yet establish
                    // a head. Unknown must not allow a later arm to win.
                    _ => return None,
                };
                if head != actual {
                    return Some(false);
                }
                if arguments.len() != values.len() {
                    // A malformed instance is not a legitimate mismatch.
                    return None;
                }
                for (pattern, value) in arguments.iter().zip(values) {
                    if !self.inspect(pattern, value, depth + 1, budget, bindings)? {
                        return Some(false);
                    }
                }
                Some(true)
            }
            Pat::Literal(literal) => {
                let ty = match value {
                    Term::Bool(_) => p(P::Bool),
                    Term::Char(_) => p(P::Char),
                    Term::String(_) => p(P::Str),
                    Term::Symbol(_) => p(P::Symbol),
                    Term::Integer(_) => p(P::Integer),
                    Term::BitVec(bits) => p(P::BitVec(bits.width())),
                    Term::Float(_) => p(P::Float),
                    Term::Float16(_) => p(P::F16),
                    Term::BFloat16(_) => p(P::Bf16),
                    Term::Float32(_) => p(P::F32),
                    Term::Float64(_) => p(P::F64),
                    _ => return None,
                };
                let size = match literal {
                    n::Literal::String(value)
                    | n::Literal::Symbol(value)
                    | n::Literal::Integer(value)
                    | n::Literal::Float(value) => value.len(),
                    n::Literal::BitVec { words, .. } => words.len(),
                    _ => 1,
                };
                budget.consume(size).ok()?;
                let expression = n::Expr {
                    source: n::SourceRange { start: 0, end: 0 },
                    ty,
                    kind: K::Literal(literal.clone()),
                };
                let expected = (self.evaluate)(&expression, depth + 1, budget, bindings)?;
                known_scalars::compare(value, &expected, n::IntegerOp::Equal, budget)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(index: u32) -> n::LocalId {
        n::LocalId { owner: 0, index }
    }

    #[test]
    fn unknown_or_malformed_heads_cannot_select_a_later_candidate() {
        let head = nir::DeclarationId {
            package: 0,
            local: 1,
        };
        let pattern = n::Pattern::Or(vec![
            n::Pattern::NormalForm {
                head,
                arguments: vec![n::Pattern::Wild],
            },
            n::Pattern::Binding {
                binding: id(9),
                inner: Box::new(n::Pattern::Wild),
            },
        ]);
        for (subject, expected) in [
            (
                Term::Rigid {
                    declaration: head,
                    sort: Sort::Type,
                },
                None,
            ),
            (
                Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::Type,
                },
                None,
            ),
            (
                Term::Hole {
                    index: 0,
                    sort: Sort::Type,
                },
                None,
            ),
            (
                Term::NormalForm {
                    head,
                    sort: Sort::Type,
                    arguments: vec![],
                },
                None,
            ),
            (p(P::Bool), Some(true)),
        ] {
            let mut bindings = BTreeMap::new();
            let mut budget = comptime_runtime::Budget {
                steps: 1000,
                max_limbs: 128,
            };
            let result = PatternInspector {
                tuple_head: None,
                evaluate: |_: &n::Expr,
                           _: usize,
                           _: &mut comptime_runtime::Budget,
                           _: &BTreeMap<n::LocalId, std::sync::Arc<Term>>|
                 -> Option<Term> {
                    panic!("head discrimination must not evaluate expressions")
                },
            }
            .inspect(&pattern, &subject, 0, &mut budget, &mut bindings);
            assert_eq!(result, expected, "{subject}");
            assert_eq!(bindings.contains_key(&id(9)), expected == Some(true));
        }
    }

    #[test]
    fn guard_candidates_execute_once_in_order_and_rollback_bindings() {
        let guarded = |index| n::Pattern::Guard {
            pattern: Box::new(n::Pattern::Binding {
                binding: id(index),
                inner: Box::new(n::Pattern::Wild),
            }),
            condition: Box::new(n::Expr {
                source: n::SourceRange {
                    start: index,
                    end: index + 1,
                },
                ty: p(P::Bool),
                kind: K::Literal(n::Literal::Bool(index == 2)),
            }),
        };
        let pattern = n::Pattern::Or(vec![guarded(1), guarded(2), guarded(3)]);
        let mut calls = Vec::new();
        let mut values = BTreeMap::new();
        let mut budget = comptime_runtime::Budget {
            steps: 1000,
            max_limbs: 128,
        };
        let matched = PatternInspector {
            tuple_head: None,
            evaluate:
                |expr: &n::Expr,
                 _: usize,
                 budget: &mut comptime_runtime::Budget,
                 bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>| {
                    budget.consume(1).ok()?;
                    calls.push(expr.source.start);
                    assert_eq!(bindings.len(), 1);
                    assert!(bindings.contains_key(&id(expr.source.start)));
                    Some(Term::Bool(expr.source.start == 2))
                },
        }
        .inspect(&pattern, &Term::Bool(false), 0, &mut budget, &mut values);
        assert_eq!(matched, Some(true));
        assert_eq!(calls, vec![1, 2]);
        assert_eq!(values.keys().copied().collect::<Vec<_>>(), vec![id(2)]);
    }

    #[test]
    fn remap_transfers_exports_simultaneously_and_removes_private_sources() {
        let binding = |index| n::Pattern::Binding {
            binding: id(index),
            inner: Box::new(n::Pattern::Wild),
        };
        for (remap, expected) in [
            (
                vec![(id(1), id(3)), (id(2), id(4))],
                vec![(id(3), Term::Bool(false)), (id(4), Term::Bool(true))],
            ),
            (
                vec![(id(1), id(2)), (id(2), id(1))],
                vec![(id(1), Term::Bool(true)), (id(2), Term::Bool(false))],
            ),
        ] {
            let pattern = n::Pattern::Remap {
                pattern: Box::new(n::Pattern::Tuple(vec![binding(1), binding(2)])),
                bindings: remap,
            };
            let mut values = BTreeMap::new();
            let mut budget = comptime_runtime::Budget {
                steps: 1000,
                max_limbs: 128,
            };
            let matched = PatternInspector {
                tuple_head: None,
                evaluate:
                    |_: &n::Expr,
                     _: usize,
                     _: &mut comptime_runtime::Budget,
                     _: &BTreeMap<n::LocalId, std::sync::Arc<Term>>| None,
            }
            .inspect(
                &pattern,
                &Term::TupleValue(vec![Term::Bool(false), Term::Bool(true)]),
                0,
                &mut budget,
                &mut values,
            );
            assert_eq!(matched, Some(true));
            assert_eq!(
                values
                    .into_iter()
                    .map(|(id, value)| (id, value.as_ref().clone()))
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn unresolved_inner_error_result_does_not_select_a_known_success_pattern() {
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let inner = Term::error_result(vec![boolean.clone()], boolean.clone());
        let input = Term::error_value(
            vec![Term::Primitive(nir::Primitive::Char)],
            inner,
            None,
            Term::Rigid {
                declaration: nir::DeclarationId {
                    package: 1,
                    local: 100,
                },
                sort: nir::Sort::ComptimeValue,
            },
        )
        .unwrap();
        for label in [None, Some(boolean)] {
            let pattern = n::Pattern::ErrorResult {
                label,
                inner: Box::new(n::Pattern::Wild),
            };
            let result = PatternInspector {
                tuple_head: None,
                evaluate:
                    |_: &n::Expr,
                     _: usize,
                     _: &mut comptime_runtime::Budget,
                     _: &BTreeMap<n::LocalId, std::sync::Arc<Term>>| None,
            }
            .inspect(
                &pattern,
                &input,
                0,
                &mut Default::default(),
                &mut BTreeMap::new(),
            );
            assert_eq!(result, None);
        }
    }
}
