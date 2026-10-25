//! Exact, bounded constructor complements for coverage, never execution.
//! Unknown guards cannot be inverted as if they matched no values.
use super::*;
use nir::program::{Expr, ExprKind as K, LocalId, MatchArm, SourceRange};

// Coverage-only predicates have no free value dependencies. Their local may
// shadow a bound source local inside a nested match, but cannot capture one.
const SCALAR_SUBJECT: LocalId = LocalId {
    owner: u32::MAX,
    index: u32::MAX,
};

pub(super) fn inner(pattern: &Pattern) -> Option<&Pattern> {
    let (binding, condition) = match pattern {
        Pattern::Observe { binding, inner } => {
            let Pattern::Guard { pattern, condition } = inner.as_ref() else {
                return None;
            };
            if !matches!(pattern.as_ref(), Pattern::Wild) {
                return None;
            }
            (binding, condition)
        }
        Pattern::Guard { pattern, condition } => {
            let Pattern::Binding { binding, inner } = pattern.as_ref() else {
                return None;
            };
            if !matches!(inner.as_ref(), Pattern::Wild) {
                return None;
            }
            (binding, condition)
        }
        _ => return None,
    };
    let K::Match { value, arms } = &condition.kind else {
        return None;
    };
    if !matches!(value.kind, K::Local(id) if id == *binding) {
        return None;
    }
    let [yes, no] = arms.as_slice() else {
        return None;
    };
    if yes.guard.is_some()
        || no.guard.is_some()
        || !matches!(no.pattern, Pattern::Wild)
        || !matches!(yes.body.kind, K::Literal(Literal::Bool(false)))
        || !matches!(no.body.kind, K::Literal(Literal::Bool(true)))
    {
        return None;
    }
    Some(&yes.pattern)
}

impl Engine<'_> {
    // Only transparent wrappers share the current subject domain. Constructor
    // fields are normalized later, after matrix specialization selects their
    // types. Unknown alternatives remain present and cannot authorize a proof.
    pub(super) fn expand_negations(
        &mut self,
        pattern: &Pattern,
        ty: &Term,
        depth: usize,
    ) -> Result<Option<Pattern>, ()> {
        self.spend(1)?;
        if depth >= 128 {
            return Err(());
        }
        if inner(pattern).is_some() {
            return self.constructor_pattern(pattern, ty, false, depth);
        }
        match pattern {
            Pattern::Binding { inner, .. } | Pattern::Remap { pattern: inner, .. } => {
                let Some(expanded) = self.expand_negations(inner, ty, depth + 1)? else {
                    return Ok(None);
                };
                let mut result = self.copy_pattern(pattern)?;
                match &mut result {
                    Pattern::Binding { inner, .. } | Pattern::Remap { pattern: inner, .. } => {
                        **inner = expanded;
                    }
                    _ => unreachable!(),
                }
                Ok(Some(result))
            }
            Pattern::Or(alternatives) => {
                self.spend(alternatives.len())?;
                let expanded = alternatives
                    .iter()
                    .map(|alternative| self.expand_negations(alternative, ty, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?;
                if !expanded.iter().any(Option::is_some) {
                    return Ok(None);
                }
                let mut result = Vec::with_capacity(alternatives.len());
                for (original, expanded) in alternatives.iter().zip(expanded) {
                    result.push(match expanded {
                        Some(expanded) => expanded,
                        None => self.copy_pattern(original)?,
                    });
                }
                Ok(Some(Pattern::Or(result)))
            }
            _ => Ok(None),
        }
    }

    fn copy_pattern(&mut self, pattern: &Pattern) -> Result<Pattern, ()> {
        let mut fuel = self.steps as u64;
        let result = nir::copy_budget::charge_pattern_rewrite(pattern, &mut fuel);
        self.steps = fuel as usize;
        result.map_err(|_| ())?;
        Ok(pattern.clone())
    }

    pub(super) fn constructor_pattern(
        &mut self,
        pattern: &Pattern,
        ty: &Term,
        invert: bool,
        depth: usize,
    ) -> Result<Option<Pattern>, ()> {
        self.spend(1)?;
        if depth >= 128 {
            return Err(());
        }
        if let Some(inner) = inner(pattern) {
            return self.constructor_pattern(inner, ty, !invert, depth + 1);
        }
        match pattern {
            Pattern::Binding { inner, .. } | Pattern::Remap { pattern: inner, .. } => {
                return self.constructor_pattern(inner, ty, invert, depth + 1);
            }
            Pattern::Wild => {
                return Ok(Some(if invert {
                    Pattern::Or(vec![])
                } else {
                    Pattern::Wild
                }));
            }
            Pattern::Or(alternatives) => {
                let mut result = if invert {
                    Pattern::Wild
                } else {
                    Pattern::Or(vec![])
                };
                for alternative in alternatives {
                    let Some(next) =
                        self.constructor_pattern(alternative, ty, invert, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    if invert {
                        result = self.intersect(&result, &next, depth + 1)?;
                    } else if let Pattern::Or(items) = &mut result {
                        self.spend(1)?;
                        items.push(next);
                    }
                }
                return Ok(Some(result));
            }
            _ => {}
        }
        crate::associated::charge_projection_copy(ty, &mut self.steps).map_err(|_| ())?;
        let domain = self.domain(ty)?;
        if domain == Term::Primitive(Primitive::Bool) {
            let mut alternatives = Vec::new();
            for value in [false, true] {
                let Some(matched) =
                    crate::interval_coverage::boolean_case(pattern, value, &mut self.steps)?
                else {
                    return Ok(None);
                };
                if matched != invert {
                    alternatives.push(Pattern::Literal(Literal::Bool(value)));
                }
            }
            self.spend(alternatives.len())?;
            return Ok(Some(Pattern::Or(alternatives)));
        }
        let scalar_exact = match &domain {
            Term::Primitive(Primitive::Symbol) => {
                Some(crate::symbol_coverage::exact(pattern, &mut self.steps).unwrap_or(false))
            }
            // The BitVec engine checks completeness while evaluating the
            // ordered Match below, before it can take the inverse arm. Avoid
            // running that same partition analysis once more as a preflight.
            Term::Primitive(Primitive::BitVec(_)) => Some(true),
            Term::Primitive(primitive)
                if crate::interval_coverage::domain_in(*primitive, self.target).is_some() =>
            {
                Some(crate::interval_coverage::extract_exact(pattern, &mut self.steps).is_ok())
            }
            Term::Primitive(primitive)
                if crate::machine_float_coverage::Domain::new(*primitive).is_some() =>
            {
                Some(
                    crate::machine_float_coverage::Domain::new(*primitive)
                        .unwrap()
                        .extract(pattern, &mut self.steps)
                        .is_ok(),
                )
            }
            Term::Primitive(Primitive::Float) => Some(
                crate::float_coverage::extract(pattern, &mut self.steps)
                    .is_ok_and(|predicate| predicate.is_some()),
            ),
            _ => None,
        };
        if let Some(exact) = scalar_exact {
            // The ordinary extractor may under-approximate unknown guards;
            // only exact analysis authorizes complementing a scalar leaf.
            if !exact
                || !nir::free_bindings::collect_pattern(pattern, &mut self.steps)
                    .map_err(|_| ())?
                    .is_empty()
            {
                return Ok(None);
            }
            let pattern = self.copy_pattern(pattern)?;
            self.spend(9)?;
            let source = SourceRange { start: 0, end: 0 };
            let boolean = |value| Expr {
                source,
                ty: Term::Primitive(Primitive::Bool),
                kind: K::Literal(Literal::Bool(value)),
            };
            return Ok(Some(Pattern::Guard {
                pattern: Box::new(Pattern::Binding {
                    binding: SCALAR_SUBJECT,
                    inner: Box::new(Pattern::Wild),
                }),
                condition: Box::new(Expr {
                    source,
                    ty: Term::Primitive(Primitive::Bool),
                    kind: K::Match {
                        value: Box::new(Expr {
                            source,
                            ty: domain,
                            kind: K::Local(SCALAR_SUBJECT),
                        }),
                        arms: vec![
                            MatchArm {
                                pattern,
                                guard: None,
                                body: boolean(!invert),
                            },
                            MatchArm {
                                pattern: Pattern::Wild,
                                guard: None,
                                body: boolean(invert),
                            },
                        ],
                    },
                }),
            }));
        }
        match (pattern, &domain) {
            (Pattern::ErrorResult { label, inner }, Term::ErrorResult { errors, result }) => {
                let payload = match label {
                    None => result.as_ref(),
                    Some(label) => {
                        let mut payload = None;
                        for error in errors {
                            crate::associated::charge_projection_copy(error, &mut self.steps)
                                .map_err(|_| ())?;
                            if error == label {
                                payload = Some(error);
                                break;
                            }
                        }
                        let Some(payload) = payload else {
                            return Ok(None);
                        };
                        payload
                    }
                };
                let Some(selected) = self.constructor_pattern(inner, payload, invert, depth + 1)?
                else {
                    return Ok(None);
                };
                if let Some(label) = label {
                    crate::associated::charge_projection_copy(label, &mut self.steps)
                        .map_err(|_| ())?;
                }
                let selected = Pattern::ErrorResult {
                    label: label.clone(),
                    inner: Box::new(selected),
                };
                if !invert {
                    return Ok(Some(selected));
                }
                self.spend(errors.len().saturating_add(2))?;
                let mut alternatives = vec![selected];
                if label.is_some() {
                    alternatives.push(Pattern::ErrorResult {
                        label: None,
                        inner: Box::new(Pattern::Wild),
                    });
                }
                for error in errors {
                    crate::associated::charge_projection_copy(error, &mut self.steps)
                        .map_err(|_| ())?;
                    if label.as_ref() == Some(error) {
                        continue;
                    }
                    crate::associated::charge_projection_copy(error, &mut self.steps)
                        .map_err(|_| ())?;
                    alternatives.push(Pattern::ErrorResult {
                        label: Some(error.clone()),
                        inner: Box::new(Pattern::Wild),
                    });
                }
                Ok(Some(Pattern::Or(alternatives)))
            }
            (Pattern::Literal(Literal::Bool(value)), Term::Primitive(Primitive::Bool)) => {
                Ok(Some(Pattern::Literal(Literal::Bool(*value != invert))))
            }
            (Pattern::Unit, Term::Primitive(Primitive::Void)) => Ok(Some(if invert {
                Pattern::Or(vec![])
            } else {
                Pattern::Unit
            })),
            (Pattern::Tuple(fields), Term::Tuple(types)) => {
                self.product(fields, types, invert, depth, |fields| {
                    Pattern::Tuple(fields)
                })
            }
            (
                Pattern::Record {
                    declaration,
                    fields,
                },
                Term::Nominal {
                    declaration: actual,
                    ..
                },
            ) if declaration == actual => {
                let Some(types) = self.complement_fields(&domain, None)? else {
                    return Ok(None);
                };
                self.product(fields, &types, invert, depth, |fields| Pattern::Record {
                    declaration: *declaration,
                    fields,
                })
            }
            (Pattern::AnonymousRecord { names, fields }, Term::Record(types))
                if names.iter().eq(types.iter().map(|(name, _)| name)) =>
            {
                let copies = if invert { fields.len() } else { 1 };
                self.spend(
                    names
                        .iter()
                        .map(String::len)
                        .sum::<usize>()
                        .saturating_mul(copies)
                        .saturating_add(types.len()),
                )?;
                for (_, ty) in types {
                    crate::associated::charge_projection_copy(ty, &mut self.steps)
                        .map_err(|_| ())?;
                }
                let types = types.iter().map(|(_, ty)| ty.clone()).collect::<Vec<_>>();
                self.product(fields, &types, invert, depth, |fields| {
                    Pattern::AnonymousRecord {
                        names: names.clone(),
                        fields,
                    }
                })
            }
            (
                Pattern::Variant {
                    declaration,
                    index,
                    fields,
                },
                Term::Nominal {
                    declaration: actual,
                    ..
                },
            ) if declaration == actual => {
                let Some(definition) = self.enums.get(declaration) else {
                    return Ok(None);
                };
                let count = definition.variants.len();
                if *index >= count {
                    return Ok(None);
                }
                let Some(types) = self.complement_fields(&domain, Some(*index as u32))? else {
                    return Ok(None);
                };
                let Some(selected) =
                    self.product(fields, &types, invert, depth, |fields| Pattern::Variant {
                        declaration: *declaration,
                        index: *index,
                        fields,
                    })?
                else {
                    return Ok(None);
                };
                if !invert {
                    return Ok(Some(selected));
                }
                self.spend(count)?;
                let mut alternatives = vec![selected];
                for other in 0..count {
                    if other == *index {
                        continue;
                    }
                    let Some(types) = self.complement_fields(&domain, Some(other as u32))? else {
                        return Ok(None);
                    };
                    self.spend(types.len())?;
                    alternatives.push(Pattern::Variant {
                        declaration: *declaration,
                        index: other,
                        fields: vec![Pattern::Wild; types.len()],
                    });
                }
                Ok(Some(Pattern::Or(alternatives)))
            }
            // In particular, a nonconstant guard or extractor is unknown,
            // not false. Its complement cannot grant any coverage.
            _ => Ok(None),
        }
    }

    fn complement_fields(
        &mut self,
        ty: &Term,
        variant: Option<u32>,
    ) -> Result<Option<Vec<Term>>, ()> {
        let structs = self.structs;
        let enums = self.enums;
        Ok(nir::normal_forms::nominal_value_fields_with_lookup_in(
            ty,
            variant,
            &mut self.steps,
            &|id, fuel| {
                *fuel = fuel.checked_sub(1)?;
                structs
                    .get(&id)
                    .map(nir::normal_forms::NominalValueDeclaration::Struct)
                    .or_else(|| {
                        enums
                            .get(&id)
                            .map(nir::normal_forms::NominalValueDeclaration::Enum)
                    })
            },
            self.target,
        ))
    }

    fn product(
        &mut self,
        fields: &[Pattern],
        types: &[Term],
        invert: bool,
        depth: usize,
        build: impl Fn(Vec<Pattern>) -> Pattern,
    ) -> Result<Option<Pattern>, ()> {
        if fields.len() != types.len() {
            return Ok(None);
        }
        self.spend(fields.len())?;
        let mut result = Vec::new();
        for (index, (field, ty)) in fields.iter().zip(types).enumerate() {
            let Some(checked) = self.constructor_pattern(field, ty, invert, depth + 1)? else {
                return Ok(None);
            };
            if invert {
                self.spend(fields.len())?;
                let mut row = vec![Pattern::Wild; fields.len()];
                row[index] = checked;
                result.push(build(row));
            } else {
                result.push(checked);
            }
        }
        Ok(Some(if invert {
            Pattern::Or(result)
        } else {
            build(result)
        }))
    }

    fn intersect(&mut self, a: &Pattern, b: &Pattern, depth: usize) -> Result<Pattern, ()> {
        self.spend(1)?;
        if depth >= 128 {
            return Err(());
        }
        if matches!(a, Pattern::Wild) {
            return self.copy_pattern(b);
        }
        if matches!(b, Pattern::Wild) {
            return self.copy_pattern(a);
        }
        if let Pattern::Or(items) = a {
            self.spend(items.len())?;
            return Ok(Pattern::Or(
                items
                    .iter()
                    .map(|item| self.intersect(item, b, depth + 1))
                    .collect::<Result<_, _>>()?,
            ));
        }
        if matches!(b, Pattern::Or(_)) {
            return self.intersect(b, a, depth + 1);
        }
        let fields = |a: &[Pattern], b: &[Pattern], this: &mut Self| -> Result<Vec<Pattern>, ()> {
            if a.len() != b.len() {
                return Err(());
            }
            this.spend(a.len())?;
            a.iter()
                .zip(b)
                .map(|(a, b)| this.intersect(a, b, depth + 1))
                .collect()
        };
        Ok(match (a, b) {
            (
                Pattern::ErrorResult {
                    label: a,
                    inner: ai,
                },
                Pattern::ErrorResult {
                    label: b,
                    inner: bi,
                },
            ) => {
                for label in a.iter().chain(b) {
                    crate::associated::charge_projection_copy(label, &mut self.steps)
                        .map_err(|_| ())?;
                }
                if a == b {
                    Pattern::ErrorResult {
                        label: a.clone(),
                        inner: Box::new(self.intersect(ai, bi, depth + 1)?),
                    }
                } else {
                    Pattern::Or(vec![])
                }
            }
            (
                Pattern::Guard {
                    pattern: a,
                    condition: ac,
                },
                Pattern::Guard {
                    pattern: b,
                    condition: bc,
                },
            ) if matches!(a.as_ref(), Pattern::Binding {binding,inner} if *binding==SCALAR_SUBJECT && matches!(inner.as_ref(), Pattern::Wild))
                && matches!(b.as_ref(), Pattern::Binding {binding,inner} if *binding==SCALAR_SUBJECT && matches!(inner.as_ref(), Pattern::Wild)) =>
            {
                let mut fuel = self.steps as u64;
                let result = nir::copy_budget::charge_expression(ac, &mut fuel)
                    .and_then(|_| nir::copy_budget::charge_expression(bc, &mut fuel));
                self.steps = fuel as usize;
                result.map_err(|_| ())?;
                self.spend(4)?;
                Pattern::Guard {
                    pattern: Box::new(Pattern::Binding {
                        binding: SCALAR_SUBJECT,
                        inner: Box::new(Pattern::Wild),
                    }),
                    condition: Box::new(Expr {
                        source: ac.source,
                        ty: Term::Primitive(Primitive::Bool),
                        kind: K::And(ac.clone(), bc.clone()),
                    }),
                }
            }
            (Pattern::Literal(Literal::Bool(a)), Pattern::Literal(Literal::Bool(b))) if a == b => {
                Pattern::Literal(Literal::Bool(*a))
            }
            (Pattern::Unit, Pattern::Unit) => Pattern::Unit,
            (Pattern::Tuple(a), Pattern::Tuple(b)) => Pattern::Tuple(fields(a, b, self)?),
            (
                Pattern::Record {
                    declaration: a,
                    fields: af,
                },
                Pattern::Record {
                    declaration: b,
                    fields: bf,
                },
            ) if a == b => Pattern::Record {
                declaration: *a,
                fields: fields(af, bf, self)?,
            },
            (
                Pattern::Variant {
                    declaration: a,
                    index: ai,
                    fields: af,
                },
                Pattern::Variant {
                    declaration: b,
                    index: bi,
                    fields: bf,
                },
            ) if a == b && ai == bi => Pattern::Variant {
                declaration: *a,
                index: *ai,
                fields: fields(af, bf, self)?,
            },
            (
                Pattern::AnonymousRecord {
                    names: a,
                    fields: af,
                },
                Pattern::AnonymousRecord {
                    names: b,
                    fields: bf,
                },
            ) if a == b => {
                self.spend(a.iter().map(String::len).sum())?;
                Pattern::AnonymousRecord {
                    names: a.clone(),
                    fields: fields(af, bf, self)?,
                }
            }
            _ => Pattern::Or(vec![]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::{Expr, LocalId, MatchArm, SourceRange, Variant, VariantShape};

    fn setup() -> (BTreeMap<DeclarationId, Enum>, Term) {
        let id = DeclarationId {
            package: 0,
            local: 42,
        };
        let definition = Enum {
            declaration: id,
            name: "Pair".into(),
            parameter_types: vec![],
            type_parameters: vec![],
            capabilities: vec![],
            requirements: vec![],
            variants: vec![Variant {
                name: "pair".into(),
                shape: VariantShape::Tuple,
                fields: vec![Term::Primitive(Primitive::Bool); 2],
            }],
            groups: vec![],
            encoding: None,
            is_intrinsic: false,
            symbol_encoding: None,
        };
        (
            BTreeMap::from([(id, definition)]),
            Term::Nominal {
                declaration: id,
                arguments: vec![],
            },
        )
    }
    fn selected(mask: u8, ty: &Term) -> Pattern {
        let Term::Nominal { declaration, .. } = ty else {
            unreachable!()
        };
        Pattern::Or(
            (0..4)
                .filter(|index| mask & (1 << index) != 0)
                .map(|index| Pattern::Variant {
                    declaration: *declaration,
                    index: 0,
                    fields: vec![
                        Pattern::Literal(Literal::Bool(index & 1 != 0)),
                        Pattern::Literal(Literal::Bool(index & 2 != 0)),
                    ],
                })
                .collect(),
        )
    }
    fn negate(pattern: Pattern, ty: &Term) -> Pattern {
        let binding = LocalId { owner: 0, index: 9 };
        let source = SourceRange { start: 0, end: 1 };
        let boolean = |value| Expr {
            source,
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(Literal::Bool(value)),
        };
        Pattern::Guard {
            pattern: Box::new(Pattern::Binding {
                binding,
                inner: Box::new(Pattern::Wild),
            }),
            condition: Box::new(Expr {
                source,
                ty: Term::Primitive(Primitive::Bool),
                kind: K::Match {
                    value: Box::new(Expr {
                        source,
                        ty: ty.clone(),
                        kind: K::Local(binding),
                    }),
                    arms: vec![
                        MatchArm {
                            pattern,
                            guard: None,
                            body: boolean(false),
                        },
                        MatchArm {
                            pattern: Pattern::Wild,
                            guard: None,
                            body: boolean(true),
                        },
                    ],
                },
            }),
        }
    }
    #[test]
    fn nominal_complements_agree_with_all_boolean_pair_subsets() {
        let (enums, ty) = setup();
        for excluded in 0..16u8 {
            for included in 0..16u8 {
                let patterns = [
                    negate(selected(excluded, &ty), &ty),
                    selected(included, &ty),
                ];
                let expected = if ((!excluded | included) & 15) == 15 {
                    Coverage::Exhaustive
                } else {
                    Coverage::Missing
                };
                assert_eq!(
                    check(&enums, &ty, &patterns.iter().collect::<Vec<_>>(), 100_000),
                    expected,
                    "excluded={excluded}, included={included}"
                );
                let alternative = Pattern::Or(patterns.to_vec());
                assert_eq!(
                    check(&enums, &ty, &[&alternative], 100_000),
                    expected,
                    "or: excluded={excluded}, included={included}"
                );
            }
        }
    }

    #[test]
    fn wrapper_depth_is_independent_of_specialized_columns() {
        let ty = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 140]);
        let patterns = [false, true].map(|last| {
            let mut fields = vec![Pattern::Wild; 139];
            fields.push(Pattern::Literal(Literal::Bool(last)));
            Pattern::Tuple(fields)
        });
        assert_eq!(
            check(
                &BTreeMap::new(),
                &ty,
                &[&patterns[0], &patterns[1]],
                100_000
            ),
            Coverage::Exhaustive
        );
    }

    #[test]
    fn numeric_complements_agree_with_all_small_subset_pairs() {
        for primitive in [Primitive::U8, Primitive::BitVec(2), Primitive::Symbol] {
            let (mut enums, ty) = setup();
            let Term::Nominal { declaration, .. } = &ty else {
                unreachable!()
            };
            enums.get_mut(declaration).unwrap().variants[0].fields =
                vec![Term::Primitive(primitive)];
            let selected = |mask: u8| {
                Pattern::Or(
                    (0..4u8)
                        .filter(|index| mask & (1 << index) != 0)
                        .map(|value| Pattern::Variant {
                            declaration: *declaration,
                            index: 0,
                            fields: vec![Pattern::Literal(if primitive == Primitive::U8 {
                                Literal::UnsignedInteger(value as u128)
                            } else if primitive == Primitive::Symbol {
                                Literal::Symbol(format!("name{value}"))
                            } else {
                                Literal::BitVec {
                                    width: 2,
                                    words: vec![value as u32],
                                }
                            })],
                        })
                        .collect(),
                )
            };
            for excluded in 0..16u8 {
                for included in 0..16u8 {
                    let patterns = [negate(selected(excluded), &ty), selected(included)];
                    let expected = if ((!excluded | included) & 15) == 15 {
                        Coverage::Exhaustive
                    } else {
                        Coverage::Missing
                    };
                    assert_eq!(
                        check(&enums, &ty, &patterns.iter().collect::<Vec<_>>(), 100_000),
                        expected,
                        "excluded={excluded}, included={included}"
                    );
                }
            }
        }
    }

    #[test]
    fn float_complements_preserve_nan_and_zero_equivalence_in_all_subset_pairs() {
        for primitive in [Primitive::F32, Primitive::F64] {
            let (mut enums, ty) = setup();
            let Term::Nominal { declaration, .. } = &ty else {
                unreachable!()
            };
            enums.get_mut(declaration).unwrap().variants[0].fields =
                vec![Term::Primitive(primitive)];
            let literals = if primitive == Primitive::F32 {
                vec![
                    Literal::Float32(0),
                    Literal::Float32(0x80000000),
                    Literal::Float32(0x3f800000),
                    Literal::Float32(0x7fc00042),
                ]
            } else {
                vec![
                    Literal::Float64(0),
                    Literal::Float64(0x8000000000000000),
                    Literal::Float64(0x3ff0000000000000),
                    Literal::Float64(0x7ff8000000000042),
                ]
            };
            let selected = |mask: u8| {
                Pattern::Or(
                    literals
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| mask & (1 << index) != 0)
                        .map(|(_, literal)| Pattern::Variant {
                            declaration: *declaration,
                            index: 0,
                            fields: vec![Pattern::Literal(literal.clone())],
                        })
                        .collect(),
                )
            };
            // Both zeros select one equivalence class; NaN literals select none.
            let classes = |mask: u8| u8::from(mask & 3 != 0) | ((mask & 4) >> 1);
            for excluded in 0..16u8 {
                for included in 0..16u8 {
                    let patterns = [negate(selected(excluded), &ty), selected(included)];
                    let expected = if classes(excluded) & !classes(included) == 0 {
                        Coverage::Exhaustive
                    } else {
                        Coverage::Missing
                    };
                    assert_eq!(
                        check(&enums, &ty, &patterns.iter().collect::<Vec<_>>(), 100_000),
                        expected,
                        "{primitive:?}: excluded={excluded}, included={included}"
                    );
                }
            }
        }
    }

    #[test]
    fn error_row_complements_preserve_all_success_and_error_cases() {
        let (mut enums, _) = setup();
        let first = *enums.keys().next().unwrap();
        enums.get_mut(&first).unwrap().variants[0].fields = vec![Term::Primitive(Primitive::Bool)];
        let second = DeclarationId {
            package: 0,
            local: first.local + 1,
        };
        let mut definition = enums[&first].clone();
        definition.declaration = second;
        enums.insert(second, definition);
        let errors = vec![
            Term::Nominal {
                declaration: first,
                arguments: vec![],
            },
            Term::Nominal {
                declaration: second,
                arguments: vec![],
            },
        ];
        let ty = Term::error_result(errors.clone(), Term::Primitive(Primitive::Bool));
        let selected = |mask: u8| {
            Pattern::Or(
                (0..6u8)
                    .filter(|index| mask & (1 << index) != 0)
                    .map(|index| {
                        let value = Pattern::Literal(Literal::Bool(index & 1 != 0));
                        if index < 2 {
                            Pattern::ErrorResult {
                                label: None,
                                inner: Box::new(value),
                            }
                        } else {
                            let slot = usize::from(index / 2 - 1);
                            Pattern::ErrorResult {
                                label: Some(errors[slot].clone()),
                                inner: Box::new(Pattern::Variant {
                                    declaration: if slot == 0 { first } else { second },
                                    index: 0,
                                    fields: vec![value],
                                }),
                            }
                        }
                    })
                    .collect(),
            )
        };
        for excluded in 0..64u8 {
            for included in 0..64u8 {
                let patterns = [negate(selected(excluded), &ty), selected(included)];
                let expected = if ((!excluded | included) & 63) == 63 {
                    Coverage::Exhaustive
                } else {
                    Coverage::Missing
                };
                assert_eq!(
                    check(&enums, &ty, &patterns.iter().collect::<Vec<_>>(), 100_000),
                    expected,
                    "excluded={excluded}, included={included}"
                );
            }
        }
    }
    #[test]
    fn complement_budget_exhaustion_and_wrong_subject_do_not_create_evidence() {
        let (enums, ty) = setup();
        let positive = selected(7, &ty);
        let negative = negate(positive.clone(), &ty);
        assert_eq!(
            check(&enums, &ty, &[&negative, &positive], 4),
            Coverage::Unknown
        );
        assert_eq!(
            check(&enums, &ty, &[&negative, &positive], 100_000),
            Coverage::Exhaustive
        );
        let mut wrong = negative;
        let Pattern::Guard { condition, .. } = &mut wrong else {
            unreachable!()
        };
        let K::Match { value, .. } = &mut condition.kind else {
            unreachable!()
        };
        value.kind = K::Local(LocalId {
            owner: 0,
            index: 10,
        });
        assert_eq!(
            check(&enums, &ty, &[&wrong, &positive], 100_000),
            Coverage::Missing
        );
    }
}
