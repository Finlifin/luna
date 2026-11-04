//! Bounded inspection of immutable match inputs. Unknown is distinct from
//! mismatch: an undecidable earlier candidate cannot be skipped.
use super::observation::Failure;
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
            tuple_head: self.bound_tuple_metadata(),
            evaluate:
                |expr: &n::Expr,
                 depth,
                 budget: &mut comptime_runtime::Budget,
                 bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>| {
                    self.known_normal_input_in(expr, depth, budget, bindings)
                        .ok_or(Failure::Unavailable)
                },
        }
        .inspect(pattern, value, depth, budget, bindings)
        .ok()
    }
}

pub(in crate::program) struct PatternInspector<E> {
    pub(in crate::program) evaluate: E,
    pub(in crate::program) tuple_head: Option<nir::normal_forms::Head>,
}

impl<E> PatternInspector<E>
where
    E: FnMut(
        &n::Expr,
        usize,
        &mut comptime_runtime::Budget,
        &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Result<Term, Failure>,
{
    pub(in crate::program) fn inspect(
        &mut self,
        pattern: &n::Pattern,
        value: &Term,
        depth: usize,
        budget: &mut comptime_runtime::Budget,
        bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Result<bool, Failure> {
        if depth >= 128 {
            return Err(Failure::Depth);
        }
        budget.consume(1)?;
        use n::Pattern as Pat;
        match pattern {
            Pat::OptionalAbsent => match value {
                Term::OptionalValue { value, .. } => Ok(value.is_none()),
                _ => Err(Failure::Unavailable),
            },
            Pat::OptionalPresent(inner) => match value {
                Term::OptionalValue {
                    value: Some(value), ..
                } => self.inspect(inner, value, depth + 1, budget, bindings),
                Term::OptionalValue { value: None, .. } => Ok(false),
                _ => Err(Failure::Unavailable),
            },
            Pat::CodeEvidence { .. } => Err(Failure::Unavailable),
            Pat::ErrorResult { label, inner } => {
                if value.has_pending_error_branch() {
                    return Err(Failure::Unavailable);
                }
                let Term::ErrorValue {
                    label: actual,
                    value,
                    ..
                } = value
                else {
                    return Err(Failure::Unavailable);
                };
                match (label.as_ref(), actual.as_deref()) {
                    (None, None) => self.inspect(inner, value, depth + 1, budget, bindings),
                    (Some(a), Some(b)) if a == b => {
                        self.inspect(inner, value, depth + 1, budget, bindings)
                    }
                    (None, Some(_)) | (Some(_), None) => Ok(false),
                    _ => Err(Failure::Unavailable),
                }
            }
            Pat::Observe { .. } | Pat::NominalType { .. } => Err(Failure::Unavailable),
            Pat::Wild => Ok(true),
            Pat::Unit => match value {
                Term::Unit => Ok(true),
                _ => Err(Failure::Unavailable),
            },
            Pat::Binding { binding, inner } => {
                if !self.inspect(inner, value, depth + 1, budget, bindings)? {
                    return Ok(false);
                }
                bindings.insert(
                    *binding,
                    std::sync::Arc::new(nir::copy_budget::clone_term(value, &mut budget.steps)?),
                );
                Ok(true)
            }
            Pat::Or(alternatives) => {
                for alternative in alternatives {
                    budget.consume(bindings.len().saturating_add(1))?;
                    let mut candidate = bindings.clone();
                    if self.inspect(alternative, value, depth + 1, budget, &mut candidate)? {
                        *bindings = candidate;
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Pat::Guard { pattern, condition } => {
                if !self.inspect(pattern, value, depth + 1, budget, bindings)? {
                    return Ok(false);
                }
                match (self.evaluate)(condition, depth + 1, budget, bindings)? {
                    Term::Bool(accepted) => Ok(accepted),
                    _ => Err(Failure::Unavailable),
                }
            }
            Pat::Then {
                first,
                value: next_value,
                next,
            } => {
                if !self.inspect(first, value, depth + 1, budget, bindings)? {
                    return Ok(false);
                }
                let next_value = (self.evaluate)(next_value, depth + 1, budget, bindings)?;
                self.inspect(next, &next_value, depth + 1, budget, bindings)
            }
            Pat::Remap {
                pattern,
                bindings: remap,
            } => {
                budget.consume(bindings.len().saturating_add(remap.len()))?;
                let ambient: BTreeSet<_> = bindings.keys().copied().collect();
                if !self.inspect(pattern, value, depth + 1, budget, bindings)? {
                    return Ok(false);
                }
                let mut sources = BTreeSet::new();
                let mut targets = BTreeSet::new();
                for (from, to) in remap {
                    if ambient.contains(from) || !sources.insert(*from) || !targets.insert(*to) {
                        return Err(Failure::Unavailable);
                    }
                }
                let remapped = remap
                    .iter()
                    .map(|(from, to)| {
                        Ok((
                            *to,
                            bindings
                                .get(from)
                                .ok_or_else(|| Failure::Invalid("missing remap source".into()))?
                                .clone(),
                        ))
                    })
                    .collect::<Result<Vec<_>, Failure>>()?;
                if targets.iter().any(|target| {
                    bindings.contains_key(target)
                        && !ambient.contains(target)
                        && !sources.contains(target)
                }) {
                    return Err(Failure::Unavailable);
                }
                bindings.extend(remapped);
                for source in sources.difference(&targets) {
                    bindings.remove(source);
                }
                Ok(true)
            }
            Pat::Tuple(patterns) | Pat::TypeTuple(patterns) => {
                let values = match (pattern, value) {
                    (Pat::Tuple(_), Term::TupleValue(values))
                    | (Pat::TypeTuple(_), Term::Tuple(values)) => values,
                    (Pat::TypeTuple(_), Term::NormalForm { .. }) => self
                        .tuple_head
                        .as_ref()
                        .ok_or(Failure::Unavailable)?
                        .tuple_elements(value)
                        .ok_or(Failure::Unavailable)?,
                    (Pat::TypeTuple(_), Term::Primitive(P::Unit)) => {
                        return Err(Failure::Unavailable);
                    }
                    (
                        Pat::TypeTuple(_),
                        Term::Primitive(_)
                        | Term::Pointer(_)
                        | Term::Function { .. }
                        | Term::ComptimeFunction(_)
                        | Term::NormalFormFunction(..)
                        | Term::UnsafeFunction(_),
                    ) => return Ok(false),
                    _ => return Err(Failure::Unavailable),
                };
                if patterns.len() != values.len() {
                    return Ok(false);
                }
                for (pattern, value) in patterns.iter().zip(values) {
                    if !self.inspect(pattern, value, depth + 1, budget, bindings)? {
                        return Ok(false);
                    }
                }
                Ok(true)
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
                    return Err(Failure::Unavailable);
                };
                let Term::Nominal {
                    declaration: actual,
                    ..
                } = ty.as_ref()
                else {
                    return Err(Failure::Unavailable);
                };
                if actual != declaration {
                    return Ok(false);
                }
                let expected_variant = match pattern {
                    Pat::Variant { index, .. } => Some(
                        u32::try_from(*index)
                            .map_err(|_| Failure::Invalid("variant index overflow".into()))?,
                    ),
                    _ => None,
                };
                if *variant != expected_variant {
                    return Ok(false);
                }
                if fields.len() != values.len() {
                    return Err(Failure::Unavailable);
                }
                for (pattern, value) in fields.iter().zip(values) {
                    if !self.inspect(pattern, value, depth + 1, budget, bindings)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Pat::AnonymousRecord { names, fields } => {
                let Term::RecordValue(values) = value else {
                    return Err(Failure::Unavailable);
                };
                if names.len() != fields.len() || fields.len() != values.len() {
                    return Err(Failure::Unavailable);
                }
                for ((name, pattern), (actual, value)) in names.iter().zip(fields).zip(values) {
                    budget.consume(name.len().saturating_add(actual.len()).saturating_add(1))?;
                    if name != actual {
                        return Err(Failure::Unavailable);
                    }
                    if !self.inspect(pattern, value, depth + 1, budget, bindings)? {
                        return Ok(false);
                    }
                }
                Ok(true)
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
                    | Term::NormalFormFunction(..)
                    | Term::UnsafeFunction(_)
                    | Term::Closure { .. } => return Ok(false),
                    // Bound/hole/rigid/associated subjects do not yet establish
                    // a head. Unknown must not allow a later arm to win.
                    _ => return Err(Failure::Unavailable),
                };
                if head != actual {
                    return Ok(false);
                }
                if arguments.len() != values.len() {
                    // A malformed instance is not a legitimate mismatch.
                    return Err(Failure::Invalid(
                        "normal-form pattern arity mismatch".into(),
                    ));
                }
                for (pattern, value) in arguments.iter().zip(values) {
                    if !self.inspect(pattern, value, depth + 1, budget, bindings)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Pat::Literal(literal) => {
                if !matches!(
                    value,
                    Term::Bool(_)
                        | Term::Char(_)
                        | Term::String(_)
                        | Term::Symbol(_)
                        | Term::Integer(_)
                        | Term::BitVec(_)
                        | Term::Float(_)
                        | Term::Float16(_)
                        | Term::BFloat16(_)
                        | Term::Float32(_)
                        | Term::Float64(_)
                ) {
                    return Err(Failure::Unavailable);
                }
                let expected = known_scalars::literal(literal, budget)?;
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
                Err(Failure::Unavailable),
            ),
            (
                Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::Type,
                },
                Err(Failure::Unavailable),
            ),
            (
                Term::Hole {
                    index: 0,
                    sort: Sort::Type,
                },
                Err(Failure::Unavailable),
            ),
            (
                Term::NormalForm {
                    head,
                    sort: Sort::Type,
                    arguments: vec![],
                },
                Err(Failure::Invalid(
                    "normal-form pattern arity mismatch".into(),
                )),
            ),
            (p(P::Bool), Ok(true)),
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
                 -> Result<Term, Failure> {
                    panic!("head discrimination must not evaluate expressions")
                },
            }
            .inspect(&pattern, &subject, 0, &mut budget, &mut bindings);
            assert_eq!(result, expected, "{subject}");
            assert_eq!(bindings.contains_key(&id(9)), expected == Ok(true));
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
                    budget.consume(1)?;
                    calls.push(expr.source.start);
                    assert_eq!(bindings.len(), 1);
                    assert!(bindings.contains_key(&id(expr.source.start)));
                    Ok(Term::Bool(expr.source.start == 2))
                },
        }
        .inspect(&pattern, &Term::Bool(false), 0, &mut budget, &mut values);
        assert_eq!(matched, Ok(true));
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
                     _: &BTreeMap<n::LocalId, std::sync::Arc<Term>>| {
                        Err(Failure::Unavailable)
                    },
            }
            .inspect(
                &pattern,
                &Term::TupleValue(vec![Term::Bool(false), Term::Bool(true)]),
                0,
                &mut budget,
                &mut values,
            );
            assert_eq!(matched, Ok(true));
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
                     _: &BTreeMap<n::LocalId, std::sync::Arc<Term>>| {
                        Err(Failure::Unavailable)
                    },
            }
            .inspect(
                &pattern,
                &input,
                0,
                &mut Default::default(),
                &mut BTreeMap::new(),
            );
            assert_eq!(result, Err(Failure::Unavailable));
        }
    }
    #[test]
    fn type_tuple_inspection_reads_declared_payloads_and_keeps_unknown_order() {
        use nir::normal_forms::{Head, Kind, Parameter, Role};
        let decl = |local| nir::DeclarationId { package: 81, local };
        let domain = Term::NormalForm {
            head: decl(1),
            sort: Sort::Type,
            arguments: vec![p(P::Type)],
        };
        let metadata = Head {
            declaration: decl(2),
            role: Role::Tuple,
            collector: Some(0),
            requirements: vec![],
            parameters: vec![Parameter {
                declaration: decl(3),
                binding: id(0),
                kind: Kind::Value,
                ty: domain.clone(),
            }],
        };
        let tuple = |value| Term::NormalForm {
            head: decl(2),
            sort: Sort::Type,
            arguments: vec![value],
        };
        let ordinary = tuple(Term::NominalValue {
            ty: Box::new(domain.clone()),
            variant: None,
            fields: vec![p(P::Bool)],
        });
        let pattern = n::Pattern::TypeTuple(vec![n::Pattern::Binding {
            binding: id(1),
            inner: Box::new(n::Pattern::Wild),
        }]);
        let inspect =
            |pattern: &n::Pattern,
             value: &Term,
             head,
             budget: &mut comptime_runtime::Budget,
             bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>| {
                PatternInspector {
                    tuple_head: head,
                    evaluate:
                        |_: &n::Expr,
                         _: usize,
                         _: &mut comptime_runtime::Budget,
                         _: &BTreeMap<n::LocalId, std::sync::Arc<Term>>| {
                            Err(Failure::Unavailable)
                        },
                }
                .inspect(pattern, value, 0, budget, bindings)
            };
        for value in [&ordinary] {
            let mut bindings = BTreeMap::new();
            assert_eq!(
                inspect(
                    &pattern,
                    value,
                    Some(metadata.clone()),
                    &mut Default::default(),
                    &mut bindings
                ),
                Ok(true)
            );
            assert_eq!(bindings.get(&id(1)).map(|t| t.as_ref()), Some(&p(P::Bool)));
        }
        let alternatives = n::Pattern::Or(vec![
            pattern.clone(),
            n::Pattern::Binding {
                binding: id(9),
                inner: Box::new(n::Pattern::Wild),
            },
        ]);
        let foreign = tuple(Term::NominalValue {
            ty: Box::new(Term::NormalForm {
                head: decl(8),
                sort: Sort::Type,
                arguments: vec![p(P::Type)],
            }),
            variant: None,
            fields: vec![p(P::Bool)],
        });
        for (value, head) in [
            (&ordinary, None),
            (
                &tuple(Term::TupleValue(vec![p(P::Bool)])),
                Some(metadata.clone()),
            ),
            (&foreign, Some(metadata.clone())),
            (
                &ordinary,
                Some(Head {
                    role: Role::Opaque,
                    ..metadata.clone()
                }),
            ),
        ] {
            let mut bindings = BTreeMap::new();
            assert_eq!(
                inspect(
                    &alternatives,
                    value,
                    head,
                    &mut Default::default(),
                    &mut bindings
                ),
                Err(Failure::Unavailable)
            );
            assert!(!bindings.contains_key(&id(9)));
        }
        let empty = tuple(Term::NominalValue {
            ty: Box::new(domain),
            variant: None,
            fields: vec![],
        });
        assert_eq!(
            inspect(
                &pattern,
                &empty,
                Some(metadata.clone()),
                &mut Default::default(),
                &mut BTreeMap::new()
            ),
            Ok(false)
        );
        let mut budget = comptime_runtime::Budget {
            steps: 0,
            max_limbs: 4096,
        };
        assert_eq!(
            inspect(
                &pattern,
                &ordinary,
                Some(metadata.clone()),
                &mut budget,
                &mut BTreeMap::new()
            ),
            Err(Failure::Steps)
        );
        assert_eq!(
            inspect(
                &pattern,
                &ordinary,
                Some(metadata),
                &mut Default::default(),
                &mut BTreeMap::new()
            ),
            Ok(true)
        );
    }
    fn no_evaluation(
        _: &n::Expr,
        _: usize,
        _: &mut comptime_runtime::Budget,
        _: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Result<Term, Failure> {
        panic!("structural and literal patterns must not invoke expressions")
    }

    #[test]
    fn literal_patterns_parse_directly_and_distinguish_invalid_from_mismatch() {
        let mut inspector = PatternInspector {
            tuple_head: None,
            evaluate: no_evaluation,
        };
        for (literal, expected) in [
            (n::Literal::Integer("1".into()), Ok(true)),
            (n::Literal::Integer("2".into()), Ok(false)),
            (
                n::Literal::PendingInteger {
                    spelling: "1".into(),
                    negative: false,
                },
                Err(Failure::Unavailable),
            ),
        ] {
            assert_eq!(
                inspector.inspect(
                    &n::Pattern::Literal(literal),
                    &Term::Integer(comptime_runtime::Integer::from_u128(1)),
                    0,
                    &mut Default::default(),
                    &mut BTreeMap::new()
                ),
                expected
            );
        }
        assert!(matches!(
            inspector.inspect(
                &n::Pattern::Literal(n::Literal::Integer("oops".into())),
                &Term::Integer(comptime_runtime::Integer::from_u128(1)),
                0,
                &mut Default::default(),
                &mut BTreeMap::new()
            ),
            Err(Failure::Invalid(_))
        ));
    }

    #[test]
    fn failed_guard_cannot_select_fallback_or_publish_candidate_bindings() {
        let guarded = n::Pattern::Guard {
            pattern: Box::new(n::Pattern::Binding {
                binding: id(1),
                inner: Box::new(n::Pattern::Wild),
            }),
            condition: Box::new(n::Expr {
                source: n::SourceRange { start: 0, end: 1 },
                ty: p(P::Bool),
                kind: K::Literal(n::Literal::Bool(true)),
            }),
        };
        let pattern = n::Pattern::Or(vec![
            guarded,
            n::Pattern::Binding {
                binding: id(2),
                inner: Box::new(n::Pattern::Wild),
            },
        ]);
        for failure in [
            Failure::Unavailable,
            Failure::Steps,
            Failure::Depth,
            Failure::Size,
            Failure::Invalid("bad evidence".into()),
        ] {
            let mut bindings = BTreeMap::from([(id(0), std::sync::Arc::new(Term::Bool(false)))]);
            let original = bindings.clone();
            let mut calls = 0;
            let result = PatternInspector {
                tuple_head: None,
                evaluate:
                    |_: &n::Expr,
                     _: usize,
                     _: &mut comptime_runtime::Budget,
                     scope: &BTreeMap<n::LocalId, std::sync::Arc<Term>>| {
                        calls += 1;
                        assert!(scope.contains_key(&id(1)));
                        Err(failure.clone())
                    },
            }
            .inspect(
                &pattern,
                &Term::Bool(true),
                0,
                &mut Default::default(),
                &mut bindings,
            );
            assert_eq!(result, Err(failure));
            assert_eq!(calls, 1);
            assert_eq!(bindings, original);
        }
        let mut bindings = BTreeMap::new();
        let result = PatternInspector {
            tuple_head: None,
            evaluate: |_: &n::Expr,
                       _: usize,
                       _: &mut comptime_runtime::Budget,
                       _: &BTreeMap<n::LocalId, std::sync::Arc<Term>>| {
                Ok(Term::Bool(true))
            },
        }
        .inspect(
            &pattern,
            &Term::Bool(true),
            0,
            &mut Default::default(),
            &mut bindings,
        );
        assert_eq!(result, Ok(true));
        assert!(bindings.contains_key(&id(1)));
        assert!(!bindings.contains_key(&id(2)));
    }

    #[test]
    fn copy_and_depth_exhaustion_remain_limits_with_unspent_steps() {
        let binding = |index| n::Pattern::Binding {
            binding: id(index),
            inner: Box::new(n::Pattern::Wild),
        };
        let pattern = n::Pattern::Or(vec![binding(1), binding(2)]);
        let value = Term::ClosureContextValue(vec![Term::String("x".repeat(128).into())]);
        let mut inspector = PatternInspector {
            tuple_head: None,
            evaluate: no_evaluation,
        };
        let mut short = comptime_runtime::Budget {
            steps: 64,
            ..Default::default()
        };
        let mut bindings = BTreeMap::new();
        assert_eq!(
            inspector.inspect(&pattern, &value, 0, &mut short, &mut bindings),
            Err(Failure::Steps)
        );
        assert!(short.steps > 0);
        assert!(bindings.is_empty());
        assert_eq!(
            inspector.inspect(&pattern, &value, 0, &mut Default::default(), &mut bindings),
            Ok(true)
        );
        assert!(bindings.contains_key(&id(1)));
        assert!(!bindings.contains_key(&id(2)));
        let deep = (0..128).fold(n::Pattern::Wild, |inner, index| n::Pattern::Binding {
            binding: id(index),
            inner: Box::new(inner),
        });
        let mut budget = comptime_runtime::Budget::default();
        assert_eq!(
            inspector.inspect(&deep, &Term::Unit, 0, &mut budget, &mut BTreeMap::new()),
            Err(Failure::Depth)
        );
        assert!(budget.steps > 0);
    }
}
