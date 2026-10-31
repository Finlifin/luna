//! Source pattern checking with candidate-local bindings and type evidence.
//! Resource delivery remains the responsibility of program verification.
use super::*;
mod evidence;
mod matches;
pub(super) use evidence::{
    PatternGuard, boolean_pattern_coverage, matched_type_pattern_value, pattern_binding_ids,
    pattern_bindings, pattern_guard_evidence, readonly_guard_comparison, readonly_guard_operand,
};

impl Checker<'_, '_> {
    pub(super) fn pattern(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        ty: &Term,
    ) -> Result<n::Pattern, Error> {
        self.pattern_observed(pattern, ty, None)
    }
    pub(super) fn pattern_observed(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        ty: &Term,
        observed: Option<&Term>,
    ) -> Result<n::Pattern, Error> {
        use hir::pattern::PatternKind as HP;
        match &pattern.kind {
            HP::CodeSplice(operand, outputs) => {
                self.record_quote_hole(pattern.span)?;
                self.connect_pattern_code(operand, outputs, ty, pattern.span)
            }
            HP::ErrorOk(inner) => {
                let Term::ErrorResult { result, .. } = ty else {
                    return Err(error(
                        pattern.span,
                        "success pattern requires an error-qualified input",
                    ));
                };
                Ok(n::Pattern::ErrorResult {
                    label: None,
                    inner: Box::new(self.pattern(inner, result)?),
                })
            }
            HP::ErrorErr(inner) => {
                let Term::ErrorResult { errors, .. } = ty else {
                    return Err(error(
                        pattern.span,
                        "error pattern requires an error-qualified input",
                    ));
                };
                if matches!(inner.kind, HP::Wild) {
                    self.required_steps = self
                        .required_steps
                        .checked_sub(errors.len() as u64)
                        .ok_or_else(|| error(pattern.span, "error pattern budget exhausted"))?;
                    return Ok(n::Pattern::Or(
                        errors
                            .iter()
                            .map(|label| n::Pattern::ErrorResult {
                                label: Some(label.clone()),
                                inner: Box::new(n::Pattern::Wild),
                            })
                            .collect(),
                    ));
                }
                let label = if errors.len() == 1 {
                    errors[0].clone()
                } else {
                    let mut selected = *inner;
                    loop {
                        match &selected.kind {
                            HP::Guard(first, _) | HP::Then(first, _, _) => selected = first,
                            HP::Binding(_, _, Some(first)) => selected = first,
                            _ => break,
                        }
                    }
                    let callee = match &selected.kind {
                        HP::Const(value) => *value,
                        HP::AppTuple(head, _) | HP::Struct(head, _, _) => {
                            let HP::Const(value) = &head.kind else {
                                return Err(error(
                                    pattern.span,
                                    "error payload requires a type-identifying pattern",
                                ));
                            };
                            *value
                        }
                        _ => {
                            return Err(error(
                                pattern.span,
                                "multiple error types require a type-identifying payload pattern",
                            ));
                        }
                    };
                    let label = self
                        .variant_target(callee, None)?
                        .ok_or_else(|| {
                            error(
                                pattern.span,
                                "multiple error types require a qualified variant pattern",
                            )
                        })?
                        .0;
                    errors
                        .iter()
                        .find(|allowed| self.require(&label, allowed, pattern.span).is_ok())
                        .cloned()
                        .ok_or_else(|| {
                            error(
                                pattern.span,
                                "error pattern type is absent from the input error row",
                            )
                        })?
                };
                Ok(n::Pattern::ErrorResult {
                    label: Some(label.clone()),
                    inner: Box::new(self.pattern(inner, &label)?),
                })
            }
            HP::OptionSome(inner) => {
                let payload = self.optional_payload(ty, pattern.span)?;
                Ok(n::Pattern::OptionalPresent(Box::new(
                    self.pattern(inner, &payload)?,
                )))
            }
            HP::OptionNull => {
                self.optional_payload(ty, pattern.span)?;
                Ok(n::Pattern::OptionalAbsent)
            }

            HP::Not(inner) => {
                let saved = self.locals.clone();
                let checked = self.pattern(inner, ty);
                // Bindings exist only while checking the failed candidate.
                // Success of its negation exports none of them.
                self.locals = saved;
                let checked = checked?;
                if let Some(mask) = boolean_pattern_coverage(&checked) {
                    return Ok(match mask {
                        0 => n::Pattern::Wild,
                        1 => n::Pattern::Literal(n::Literal::Bool(true)),
                        2 => n::Pattern::Literal(n::Literal::Bool(false)),
                        _ => n::Pattern::Or(Vec::new()),
                    });
                }
                if !self
                    .negated_pattern_domain(
                        ty,
                        &mut 100_000,
                        0,
                        &mut std::collections::HashMap::new(),
                    )
                    .map_err(|message| error(pattern.span, message))?
                {
                    return Err(error(
                        pattern.span,
                        "negated pattern elaboration for this domain is not implemented",
                    ));
                }
                // An executable candidate must run normally, including guard
                // effects and rollback. Only its final match result is inverted.
                let index = self.next_temporary_binding;
                self.next_temporary_binding = index.checked_add(1).ok_or_else(|| {
                    error(pattern.span, "pattern temporary binding space exhausted")
                })?;
                let binding = n::LocalId {
                    owner: u32::MAX,
                    index,
                };
                let boolean = |value| n::Expr {
                    ty: p(P::Bool),
                    // Distinct synthetic arm boundaries anchored in the `not`
                    // prefix. Nested negations have their own prefix anchors.
                    source: n::SourceRange {
                        start: pattern.span.lo().0,
                        end: pattern.span.lo().0 + u32::from(value),
                    },
                    kind: K::Literal(n::Literal::Bool(value)),
                };
                // This alias is needed only by the candidate's condition. It
                // must not become an exported binding of an `or` alternative.
                Ok(n::Pattern::Observe {
                    binding,
                    inner: Box::new(n::Pattern::Guard {
                        pattern: Box::new(n::Pattern::Wild),
                        condition: Box::new(n::Expr {
                            ty: p(P::Bool),
                            source: source(pattern.span),
                            kind: K::Match {
                                value: Box::new(n::Expr {
                                    ty: ty.clone(),
                                    source: n::SourceRange {
                                        start: pattern.span.lo().0 + 1,
                                        end: pattern.span.lo().0 + 1,
                                    },
                                    kind: K::Local(binding),
                                }),
                                arms: vec![
                                    n::MatchArm {
                                        pattern: checked,
                                        guard: None,
                                        body: boolean(false),
                                    },
                                    n::MatchArm {
                                        pattern: n::Pattern::Wild,
                                        guard: None,
                                        body: boolean(true),
                                    },
                                ],
                            },
                        }),
                    }),
                })
            }
            HP::Range(lower, upper, bound) => {
                if lower.is_none() && upper.is_none() {
                    return Ok(n::Pattern::Wild);
                }
                let shape = self.structural_type(ty, pattern.span)?;
                if !matches!(&shape, Term::Primitive(primitive) if matches!(primitive, P::Integer | P::Float | P::F32 | P::F64 | P::Char) || self.integer_layout(*primitive).is_some())
                {
                    return Err(error(
                        pattern.span,
                        "range pattern ordering protocol for this type is not implemented",
                    ));
                }
                let mut fresh = || {
                    let index = self.next_temporary_binding;
                    self.next_temporary_binding = index.checked_add(1).ok_or_else(|| {
                        error(pattern.span, "pattern temporary binding space exhausted")
                    })?;
                    Ok::<_, Error>(n::LocalId {
                        owner: u32::MAX,
                        index,
                    })
                };
                let observed = fresh()?;
                let lower_binding = fresh()?;
                let upper_binding = fresh()?;
                let local_expr = |binding| n::Expr {
                    ty: ty.clone(),
                    source: source(pattern.span),
                    kind: K::Local(binding),
                };
                let mut statements = Vec::new();
                let mut checks = Vec::new();
                let mut diverges = false;
                for (endpoint, binding, upper_end) in [
                    (*lower, lower_binding, false),
                    (*upper, upper_binding, true),
                ] {
                    let Some(endpoint) = endpoint else { continue };
                    let endpoint = self.expr(endpoint, Some(ty))?;
                    diverges |= is_never(&endpoint.ty);
                    statements.push(n::Expr {
                        ty: if is_never(&endpoint.ty) {
                            p(P::Never)
                        } else {
                            p(P::Void)
                        },
                        source: endpoint.source,
                        kind: K::Let {
                            binding,
                            value: Box::new(endpoint),
                        },
                    });
                    let exclusive =
                        upper_end && matches!(bound, hir::pattern::BoundType::Exclusive);
                    let (left, right) = if upper_end {
                        (local_expr(observed), local_expr(binding))
                    } else {
                        (local_expr(binding), local_expr(observed))
                    };
                    let kind = if shape == p(P::Integer) {
                        K::IntegerBinary {
                            op: if exclusive {
                                n::IntegerOp::Less
                            } else {
                                n::IntegerOp::LessEqual
                            },
                            left: Box::new(left),
                            right: Box::new(right),
                        }
                    } else if exclusive {
                        // NaN is unordered: `not (upper <= value)` is not
                        // equivalent to the required ordered `value < upper`.
                        K::ScalarLess(Box::new(left), Box::new(right))
                    } else {
                        K::ScalarLessEqual(Box::new(left), Box::new(right))
                    };
                    let check = n::Expr {
                        ty: p(P::Bool),
                        source: source(pattern.span),
                        kind,
                    };
                    checks.push(check);
                }
                let first = checks.remove(0);
                let result = if let Some(second) = checks.pop() {
                    n::Expr {
                        ty: p(P::Bool),
                        source: source(pattern.span),
                        kind: K::And(Box::new(first), Box::new(second)),
                    }
                } else {
                    first
                };
                Ok(n::Pattern::Observe {
                    binding: observed,
                    inner: Box::new(n::Pattern::Guard {
                        pattern: Box::new(n::Pattern::Wild),
                        condition: Box::new(n::Expr {
                            ty: p(if diverges { P::Never } else { P::Bool }),
                            source: source(pattern.span),
                            kind: K::Block {
                                statements,
                                result: Some(Box::new(result)),
                            },
                        }),
                    }),
                })
            }
            HP::BitVec { radix, segments } => {
                let shape = self.structural_type(ty, pattern.span)?;
                let Term::Primitive(P::BitVec(width)) = shape else {
                    return Err(error(
                        pattern.span,
                        "bit-vector patterns require a BitVec subject",
                    ));
                };
                let binding = local(pattern.hir_id);
                let mut remaining = width;
                let mut values = Vec::new();
                let mut patterns = Vec::new();
                for segment in segments {
                    let (segment_width, segment_ty, checked) = match segment {
                        hir::pattern::BitSegment::Literal { spelling, span } => {
                            let digits = spelling.bytes().filter(|c| *c != b'_').count();
                            let bits_per_digit = match radix {
                                2 => 1usize,
                                8 => 3,
                                16 => 4,
                                _ => return Err(error(*span, "invalid bit-vector radix")),
                            };
                            let segment_width = digits
                                .checked_mul(bits_per_digit)
                                .ok_or_else(|| error(*span, "bit-vector pattern width overflow"))?;
                            let prefix = match radix {
                                2 => "0b",
                                8 => "0o",
                                _ => "0x",
                            };
                            let mut budget = comptime_runtime::Budget::default();
                            let integer = comptime_runtime::Integer::parse_literal(
                                &format!("{prefix}{spelling}"),
                                &mut budget,
                            )
                            .map_err(|_| {
                                error(*span, "invalid digit in bit-vector literal segment")
                            })?;
                            let bits = integer
                                .to_bitvec(segment_width, &mut budget)
                                .map_err(|e| error(*span, e.to_string()))?;
                            (
                                segment_width,
                                p(P::BitVec(segment_width)),
                                n::Pattern::Literal(n::Literal::BitVec {
                                    width: segment_width,
                                    words: bits.words().to_vec(),
                                }),
                            )
                        }
                        hir::pattern::BitSegment::Typed { pattern: inner, ty } => {
                            let ty = self.type_expr(ty)?;
                            let shape = self.structural_type(&ty, inner.span)?;
                            let width=match shape {
                                Term::Primitive(P::BitVec(width))=>width,
                                Term::Primitive(primitive)=>primitive.unrestricted_scalar_width_in(self.target).map(|width|width as usize)
                                    .ok_or_else(||error(inner.span,"bit-vector segment type requires a supported unrestricted scalar or BitVec representation"))?,
                                _=>return Err(error(inner.span,"bit-vector segment type has no supported representation")),
                            };
                            let checked = self.pattern(inner, &ty)?;
                            (width, ty, checked)
                        }
                        hir::pattern::BitSegment::Computed(expr) => {
                            let value = self.required_expression(expr, None)?;
                            let comptime_interpretor::Value::BitVec(bits) = value else {
                                return Err(error(
                                    expr.span,
                                    "computed bit-vector segment requires a concrete BitVec value",
                                ));
                            };
                            let width = bits.width();
                            (
                                width,
                                p(P::BitVec(width)),
                                n::Pattern::Literal(n::Literal::BitVec {
                                    width,
                                    words: bits.words().to_vec(),
                                }),
                            )
                        }
                    };
                    let start = remaining.checked_sub(segment_width).ok_or_else(|| {
                        error(
                            pattern.span,
                            "bit-vector pattern width differs from its subject",
                        )
                    })?;
                    let slice = n::Expr {
                        ty: p(P::BitVec(segment_width)),
                        source: source(pattern.span),
                        kind: K::BitSlice {
                            value: Box::new(n::Expr {
                                ty: shape.clone(),
                                source: source(pattern.span),
                                kind: K::Local(binding),
                            }),
                            start,
                            end: remaining,
                        },
                    };
                    let value = if self.structural_type(&segment_ty, pattern.span)? == slice.ty {
                        slice
                    } else {
                        n::Expr {
                            ty: segment_ty,
                            source: source(pattern.span),
                            kind: K::BitConvert(Box::new(slice)),
                        }
                    };
                    remaining = start;
                    values.push(value);
                    patterns.push(checked);
                }
                if remaining != 0 {
                    return Err(error(
                        pattern.span,
                        "bit-vector pattern width differs from its subject",
                    ));
                }
                let mut checked = n::Pattern::Wild;
                for (value, next) in values.into_iter().zip(patterns) {
                    checked = n::Pattern::Then {
                        first: Box::new(checked),
                        value: Box::new(value),
                        next: Box::new(next),
                    };
                }
                Ok(n::Pattern::Observe {
                    binding,
                    inner: Box::new(checked),
                })
            }
            HP::Then(first, value, next) => {
                let first = self.pattern_observed(first, ty, observed)?;
                let (value, next) = self.under_observed_pattern_type_facts(
                    &first,
                    observed,
                    value.span,
                    |checker| {
                        let value = checker.expr(value, None)?;
                        let observed = checker.known_pattern_type_value(&value);
                        let next = checker.pattern_observed(next, &value.ty, observed.as_ref())?;
                        Ok((value, next))
                    },
                )?;
                self.lower_computed_pattern(first, value, next, pattern.span)
            }
            HP::Guard(pattern, condition) => {
                let pattern = self.pattern_observed(pattern, ty, observed)?;
                let condition = self.under_observed_pattern_type_facts(
                    &pattern,
                    observed,
                    condition.span,
                    |checker| checker.expr(condition, Some(&p(P::Bool))),
                )?;
                Ok(n::Pattern::Guard {
                    pattern: Box::new(pattern),
                    condition: Box::new(condition),
                })
            }
            HP::List(elements, rest) => self.list_pattern(pattern, elements, *rest, ty),
            HP::NormalForm(head, arguments) => {
                if self.structural_type(ty, pattern.span)? != p(P::Type) {
                    return Err(error(
                        pattern.span,
                        "normal-form patterns require a Type value",
                    ));
                }
                let H::ResolvedDef { def_id, .. } = head.kind else {
                    return Err(error(
                        head.span,
                        "normal-form pattern head requires a resolved declaration",
                    ));
                };
                let owner = self
                    .package
                    .definition_owner(def_id)
                    .ok_or_else(|| error(head.span, "missing normal-form head"))?;
                let Some(ItemKind::Fn(sig, _)) = self.package.item(owner).map(|item| &item.kind)
                else {
                    return Err(error(
                        head.span,
                        "pattern head is not a normal-form definition",
                    ));
                };
                if !sig.modifiers.is_normal_form_head {
                    return Err(error(
                        head.span,
                        "pattern head is not a normal-form definition",
                    ));
                }
                self.ensure_signature(owner, head.span)?;
                let parameters = self.normal_parameters[&owner].clone();
                if arguments.len() != parameters.len() {
                    return Err(error(
                        pattern.span,
                        "normal-form pattern argument count mismatch",
                    ));
                }
                // Route names first, then elaborate in declaration order: a
                // later parameter's type may depend on an earlier Type input
                // even when the named patterns were written in reverse order.
                let mut routed = vec![None; parameters.len()];
                let mut positional = 0;
                for argument in *arguments {
                    let slot = if let Some(name) = argument.name {
                        sig.clause_params
                            .iter()
                            .position(|parameter| parameter.name.name == name.name)
                            .ok_or_else(|| {
                                error(name.span, "unknown normal-form pattern argument")
                            })?
                    } else {
                        while positional < routed.len() && routed[positional].is_some() {
                            positional += 1;
                        }
                        let slot = positional;
                        positional += 1;
                        slot
                    };
                    if slot >= routed.len() || routed[slot].replace(&argument.pattern).is_some() {
                        return Err(error(
                            argument.pattern.span,
                            "duplicate normal-form pattern argument",
                        ));
                    }
                }
                let head_id = declaration(self.package, &self.package_identities, owner);
                let observed_arguments = match observed {
                    Some(Term::NormalForm {
                        head, arguments, ..
                    }) if *head == head_id && arguments.len() == parameters.len() => {
                        Some(arguments)
                    }
                    _ => None,
                };
                let mut fields = Vec::with_capacity(parameters.len());
                let mut known_parameters = Vec::new();
                let mut known_types = Vec::new();
                for (slot, (argument, (parameter, _, declared_ty))) in
                    routed.into_iter().zip(&parameters).enumerate()
                {
                    let argument = argument.expect("arity and routing cover every parameter");
                    let ty = declared_ty
                        .instantiate_parameters(&known_parameters, &known_types)
                        .map_err(|failure| error(argument.span, failure.to_string()))?;
                    let dependent = parameters
                        .iter()
                        .any(|(parameter, _, _)| ty.mentions_rigid(*parameter));
                    let observed_type = if ty == p(P::Type) {
                        observed_arguments
                            .and_then(|arguments| arguments.get(slot))
                            .filter(|value| value.sort() == Sort::Type)
                    } else {
                        None
                    };
                    let field = if dependent {
                        if matches!(argument.kind, HP::Wild) {
                            n::Pattern::Wild
                        } else {
                            return Err(error(
                                argument.span,
                                "dependent normal-form value patterns require contextual pattern elaboration not yet implemented",
                            ));
                        }
                    } else {
                        self.pattern_observed(argument, &ty, observed_type)?
                    };
                    if ty == p(P::Type) {
                        let mut evidence = matched_type_pattern_value(
                            &field,
                            &mut self.required_steps,
                            argument.span,
                            0,
                        )?
                        .cloned();
                        if evidence.is_none() {
                            evidence = self.known_type_equality_pattern(&field, argument.span)?;
                        }
                        if let Some(value) = evidence.or_else(|| observed_type.cloned()) {
                            // This evidence belongs only to the successful
                            // candidate; do not install a global solver fact.
                            known_parameters.push(nir::DeclarationParameter {
                                declaration: *parameter,
                                sort: Sort::Type,
                            });
                            known_types.push(value.clone());
                        }
                    }
                    if ty != p(P::Type) {
                        let evidence = self
                            .matched_value_pattern_input(&field, &ty, argument.span, 0)?
                            .or_else(|| {
                                observed_arguments
                                    .and_then(|args| args.get(slot))
                                    .filter(|value| value.sort() == Sort::ComptimeValue)
                                    .cloned()
                            });
                        if let Some(value) = evidence {
                            known_parameters.push(nir::DeclarationParameter {
                                declaration: *parameter,
                                sort: Sort::ComptimeValue,
                            });
                            known_types.push(value);
                        }
                    }
                    fields.push(field);
                }
                Ok(n::Pattern::NormalForm {
                    head: declaration(self.package, &self.package_identities, owner),
                    arguments: fields,
                })
            }
            HP::Tuple(fields) => {
                let shape = self.structural_type(ty, pattern.span)?;
                if shape == p(P::Type) {
                    let observed_fields = observed
                        .and_then(|value| match value {
                            Term::Tuple(elements) => Some(elements.as_slice()),
                            _ => self.bound_tuple_elements(value),
                        })
                        .filter(|elements| elements.len() == fields.len());
                    let fields = fields
                        .iter()
                        .enumerate()
                        .map(|(index, field)| {
                            self.pattern_observed(
                                field,
                                &p(P::Type),
                                observed_fields.map(|elements| &elements[index]),
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(n::Pattern::TypeTuple(fields));
                }
                let Term::Tuple(types) = &shape else {
                    return Err(error(
                        pattern.span,
                        "tuple pattern requires a tuple type; custom pattern view elaboration is not implemented",
                    ));
                };
                if fields.len() != types.len() {
                    return Err(error(pattern.span, "tuple pattern arity mismatch"));
                }
                let fields = fields
                    .iter()
                    .zip(types)
                    .map(|(pattern, ty)| self.pattern(pattern, ty))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(n::Pattern::Tuple(fields))
            }
            HP::Struct(callee, fields, rest) if matches!(callee.kind, HP::Wild) => {
                let shape = self.structural_type(ty, pattern.span)?;
                if let Term::Record(declared) = &shape {
                    if *rest {
                        return Err(error(
                            pattern.span,
                            "record rest binding is not implemented",
                        ));
                    }
                    let mut routed = vec![None; declared.len()];
                    let mut seen = BTreeSet::new();
                    for field in *fields {
                        let slot = declared
                            .iter()
                            .position(|(name, _)| name == field.ident.name.as_str())
                            .ok_or_else(|| error(field.span, "unknown struct pattern field"))?;
                        if !seen.insert(slot) {
                            return Err(error(field.span, "duplicate struct pattern field"));
                        }
                        routed[slot] = Some(field);
                    }
                    // Runtime field traversal follows declaration order, so candidate
                    // locals must become available to guards in that same order.
                    let mut checked = Vec::with_capacity(routed.len());
                    for (slot, field) in routed.into_iter().enumerate() {
                        checked.push(match field {
                            Some(field) => self.pattern(&field.pat, &declared[slot].1)?,
                            None => n::Pattern::Wild,
                        });
                    }
                    return Ok(n::Pattern::AnonymousRecord {
                        names: declared.iter().map(|(name, _)| name.clone()).collect(),
                        fields: checked,
                    });
                }
                let Term::Nominal {
                    declaration,
                    arguments,
                } = &shape
                else {
                    return Err(error(
                        pattern.span,
                        "record pattern requires a struct type; custom pattern view elaboration is not implemented",
                    ));
                };
                let definition = self
                    .structs
                    .get(declaration)
                    .ok_or_else(|| error(pattern.span, "record pattern requires a struct type"))?;
                if *rest {
                    return Err(error(
                        pattern.span,
                        "record rest binding is not implemented",
                    ));
                }
                let declared = definition
                    .instantiate_fields_in_scope(arguments, &self.call_input_frames())
                    .map_err(|e| error(pattern.span, e.to_string()))?;
                let mut routed = vec![None; declared.len()];
                let mut seen = BTreeSet::new();
                for field in *fields {
                    let slot = declared
                        .iter()
                        .position(|(name, _)| name == field.ident.name.as_str())
                        .ok_or_else(|| error(field.span, "unknown struct pattern field"))?;
                    if !seen.insert(slot) {
                        return Err(error(field.span, "duplicate struct pattern field"));
                    }
                    routed[slot] = Some(field);
                }
                // Runtime field traversal follows declaration order, so candidate
                // locals must become available to guards in that same order.
                let mut checked = Vec::with_capacity(routed.len());
                for (slot, field) in routed.into_iter().enumerate() {
                    checked.push(match field {
                        Some(field) => self.pattern(&field.pat, &declared[slot].1)?,
                        None => n::Pattern::Wild,
                    });
                }
                Ok(n::Pattern::Record {
                    declaration: *declaration,
                    fields: checked,
                })
            }
            HP::Struct(callee, fields, rest) => {
                let HP::Const(callee) = &callee.kind else {
                    return Err(error(
                        pattern.span,
                        "named enum pattern requires a variant name",
                    ));
                };
                let Some((variant_type, index)) = self.variant_target(callee, Some(ty))? else {
                    return Err(error(
                        pattern.span,
                        "named enum pattern requires an enum context",
                    ));
                };
                self.require(&variant_type, ty, pattern.span)?;
                let Term::Nominal { declaration, .. } = variant_type else {
                    unreachable!()
                };
                let variant =
                    self.instantiated_variants(&variant_type, pattern.span)?[index].clone();
                let n::VariantShape::Named(names) = &variant.shape else {
                    return Err(error(
                        pattern.span,
                        "enum variant does not have named fields",
                    ));
                };
                if *rest {
                    return Err(error(pattern.span, "enum rest binding is not implemented"));
                }
                let mut routed = vec![None; names.len()];
                let mut seen = BTreeSet::new();
                for field in *fields {
                    let slot = names
                        .iter()
                        .position(|n| n == field.ident.name.as_str())
                        .ok_or_else(|| error(field.span, "unknown enum payload field"))?;
                    if !seen.insert(slot) {
                        return Err(error(field.span, "duplicate enum payload pattern field"));
                    }
                    routed[slot] = Some(field);
                }
                // Runtime field traversal follows declaration order, so candidate
                // locals must become available to guards in that same order.
                let mut checked = Vec::with_capacity(routed.len());
                for (slot, field) in routed.into_iter().enumerate() {
                    checked.push(match field {
                        Some(field) => self.pattern(&field.pat, &variant.fields[slot])?,
                        None => n::Pattern::Wild,
                    });
                }
                Ok(n::Pattern::Variant {
                    declaration,
                    index,
                    fields: checked,
                })
            }
            HP::AppTuple(callee, fields) => {
                let HP::Const(callee) = &callee.kind else {
                    return Err(error(
                        pattern.span,
                        "enum payload pattern requires a variant name",
                    ));
                };
                let Some((variant_type, index)) = self.variant_target(callee, Some(ty))? else {
                    return Err(error(
                        pattern.span,
                        "enum payload pattern requires an enum context",
                    ));
                };
                self.require(&variant_type, ty, pattern.span)?;
                let Term::Nominal { declaration, .. } = variant_type else {
                    unreachable!()
                };
                let variant =
                    self.instantiated_variants(&variant_type, pattern.span)?[index].clone();
                if variant.shape != n::VariantShape::Tuple || fields.len() != variant.fields.len() {
                    return Err(error(pattern.span, "enum payload pattern arity mismatch"));
                }
                let fields = fields
                    .iter()
                    .zip(&variant.fields)
                    .map(|(pattern, ty)| self.pattern(pattern, ty))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(n::Pattern::Variant {
                    declaration,
                    index,
                    fields,
                })
            }
            HP::Wild => Ok(n::Pattern::Wild),
            HP::Binding(hir::common::BindingMode::ByValue, _, inner) => {
                let inner = inner
                    .map(|p| self.pattern_observed(p, ty, observed))
                    .transpose()?
                    .unwrap_or(n::Pattern::Wild);
                let binding = local(pattern.hir_id);
                self.locals.insert(binding, ty.clone());
                Ok(n::Pattern::Binding {
                    binding,
                    inner: Box::new(inner),
                })
            }
            HP::Const(value) => {
                // The parser represents the empty tuple pattern as a unit
                // constant. In a Type subject it observes the tuple view,
                // just as HP::Tuple does; it does not construct a Type value.
                if matches!(value.kind, H::Unit)
                    && self.structural_type(ty, pattern.span)? == p(P::Type)
                {
                    return Ok(n::Pattern::TypeTuple(Vec::new()));
                }
                if let Some(group) = self.enum_group_pattern(value, ty)? {
                    return Ok(group);
                }
                let value = self.expr(value, Some(ty))?;
                if self.structural_type(ty, pattern.span)? == p(P::Type) {
                    let index = self.next_temporary_binding;
                    self.next_temporary_binding = index.checked_add(1).ok_or_else(|| {
                        error(pattern.span, "pattern temporary binding space exhausted")
                    })?;
                    let binding = n::LocalId {
                        owner: u32::MAX,
                        index,
                    };
                    let condition = n::Expr {
                        source: source(pattern.span),
                        ty: if is_never(&value.ty) {
                            p(P::Never)
                        } else {
                            p(P::Bool)
                        },
                        kind: K::TypeEqual(
                            Box::new(n::Expr {
                                source: source(pattern.span),
                                ty: p(P::Type),
                                kind: K::Local(binding),
                            }),
                            Box::new(value),
                        ),
                    };
                    // Equality matching is an observation, not inversion of a
                    // normal-form head. A guard leaves coverage conservative
                    // and uses the existing candidate-local rollback protocol.
                    return Ok(n::Pattern::Guard {
                        pattern: Box::new(n::Pattern::Binding {
                            binding,
                            inner: Box::new(n::Pattern::Wild),
                        }),
                        condition: Box::new(condition),
                    });
                }
                match value.kind {
                    K::Literal(literal) => Ok(n::Pattern::Literal(literal)),
                    K::Unit => Ok(n::Pattern::Unit),
                    K::OptionalAbsent => Ok(n::Pattern::OptionalAbsent),
                    // `()` is parsed as a constant pattern, but with a bound
                    // Tuple<> context its checked value is an empty tuple.
                    // Match its layout directly, just like a nonempty tuple
                    // pattern; it does not require a tuple equality protocol.
                    K::Tuple(fields) if fields.is_empty() => Ok(n::Pattern::Tuple(Vec::new())),
                    K::Variant {
                        declaration,
                        index,
                        fields,
                    } if fields.is_empty() => Ok(n::Pattern::Variant {
                        declaration,
                        index,
                        fields: Vec::new(),
                    }),
                    _ => {
                        let shape = self.structural_type(ty, pattern.span)?;
                        let index = self.next_temporary_binding;
                        self.next_temporary_binding = index.checked_add(1).ok_or_else(|| {
                            error(pattern.span, "pattern temporary binding space exhausted")
                        })?;
                        let binding = n::LocalId {
                            owner: u32::MAX,
                            index,
                        };
                        let observed = n::Expr {
                            ty: ty.clone(),
                            source: source(pattern.span),
                            kind: K::Local(binding),
                        };
                        let condition =
                            self.equality_expression(observed, value, &shape, pattern.span)?;
                        Ok(n::Pattern::Guard {
                            pattern: Box::new(n::Pattern::Binding {
                                binding,
                                inner: Box::new(n::Pattern::Wild),
                            }),
                            condition: Box::new(condition),
                        })
                    }
                }
            }
            HP::Or(patterns) => {
                let saved = self.locals.clone();
                let mut alternatives = Vec::new();
                let mut canonical = None;
                let mut exported = saved.clone();
                for pattern in *patterns {
                    self.locals = saved.clone();
                    let mut checked = self.pattern_observed(pattern, ty, observed)?;
                    let bindings = pattern_bindings(pattern)?;
                    if let Some(first) = &canonical {
                        let first: &BTreeMap<String, n::LocalId> = first;
                        if !first.keys().eq(bindings.keys()) {
                            self.locals = saved;
                            return Err(error(
                                pattern.span,
                                "or-pattern branches must export the same binding names",
                            ));
                        }
                        let mut remap = BTreeMap::new();
                        for (name, binding) in &bindings {
                            let target = first[name];
                            self.require(&self.locals[binding], &exported[&target], pattern.span)?;
                            remap.insert(*binding, target);
                        }
                        checked = n::Pattern::Remap {
                            pattern: Box::new(checked),
                            bindings: remap.into_iter().collect(),
                        };
                    } else {
                        canonical = Some(bindings);
                        exported = self.locals.clone();
                    }
                    alternatives.push(checked);
                }
                self.locals = exported;
                Ok(n::Pattern::Or(alternatives))
            }
            _ => Err(error(
                pattern.span,
                "pattern view/protocol elaboration is not implemented for this pattern",
            )),
        }
    }
}
