//! Source pattern checking with candidate-local bindings and type evidence.
//! Resource delivery remains the responsibility of program verification.
use super::*;

mod errors;
mod evidence;
mod matches;
mod records;

pub(super) use evidence::{
    PatternGuard, boolean_pattern_coverage, matched_type_pattern_value, pattern_binding_ids,
    pattern_bindings, pattern_guard_evidence, readonly_guard_comparison, readonly_guard_operand,
};
use records::RecordHead;

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
        self.pattern_with_record_head(pattern, ty, observed, None)
    }

    // A heterogeneous error row resolves a named payload constructor before
    // checking its fields. Carry that selection through its enclosing guards
    // and bindings so formation never evaluates the constructor inputs twice.
    fn pattern_with_record_head(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        ty: &Term,
        observed: Option<&Term>,
        record: Option<RecordHead>,
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
            HP::ErrorErr(inner) => self.error_payload_pattern(inner, ty, pattern.span),
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
                if !self.can_negate_pattern_domain(ty, pattern.span)? {
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
                let native_ordered = matches!(
                    &shape,
                    Term::Primitive(primitive)
                        if matches!(
                            primitive,
                            P::Integer
                                | P::Char
                                | P::Float
                                | P::F16
                                | P::Bf16
                                | P::F32
                                | P::F64
                                | P::Usize
                                | P::Isize
                                | P::Never
                        ) || self.integer_layout(*primitive).is_some()
                ) || self.proved_ordered_scalar_family(&shape);
                let ordering_protocol = if native_ordered {
                    None
                } else {
                    Some(self.protocols.ordering.ok_or_else(|| {
                        error(
                            pattern.span,
                            "range pattern ordering protocol for this type is not implemented",
                        )
                    })?)
                };
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
                let mut lower_const = None;
                let mut upper_const = None;
                let mut const_endpoints_ok = ordering_protocol.is_none()
                    && !matches!(
                        &shape,
                        Term::Primitive(P::Float | P::F16 | P::Bf16 | P::F32 | P::F64 | P::Never)
                    );
                for (endpoint, binding, upper_end) in [
                    (*lower, lower_binding, false),
                    (*upper, upper_binding, true),
                ] {
                    let Some(endpoint_hir) = endpoint else {
                        continue;
                    };
                    let endpoint = self.expr(endpoint_hir, Some(ty))?;
                    if const_endpoints_ok {
                        match const_range_endpoint(&endpoint) {
                            Some(lit) => {
                                if upper_end {
                                    upper_const = Some(lit);
                                } else {
                                    lower_const = Some(lit);
                                }
                            }
                            None => const_endpoints_ok = false,
                        }
                    }
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
                    let check = if let Some(protocol) = ordering_protocol {
                        let method = if exclusive { "lt" } else { "le" };
                        let callee = self
                            .selected_method(
                                left.ty.clone(),
                                Some(n::InterfaceInstance {
                                    declaration: protocol,
                                    arguments: vec![],
                                }),
                                method,
                                endpoint_hir.span,
                            )?
                            .ok_or_else(|| {
                                error(
                                    endpoint_hir.span,
                                    "Ord protocol has no selected comparison method",
                                )
                            })?;
                        self.finish_call(
                            endpoint_hir,
                            callee,
                            &[],
                            vec![left, right],
                            Some(&p(P::Bool)),
                            None,
                        )?
                    } else {
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
                        n::Expr {
                            ty: p(P::Bool),
                            source: source(pattern.span),
                            kind,
                        }
                    };
                    diverges |= is_never(&check.ty);
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
                if const_endpoints_ok {
                    if shape == p(P::Char) {
                        let start = match lower_const {
                            None => pattern_automata::RangeBound::Unbounded,
                            Some(RangeEndpointLit::Char(c)) => {
                                pattern_automata::RangeBound::Inclusive(c)
                            }
                            _ => {
                                const_endpoints_ok = false;
                                pattern_automata::RangeBound::Unbounded
                            }
                        };
                        let end = match upper_const {
                            None => pattern_automata::RangeBound::Unbounded,
                            Some(RangeEndpointLit::Char(c)) => match bound {
                                hir::pattern::BoundType::Inclusive => {
                                    pattern_automata::RangeBound::Inclusive(c)
                                }
                                hir::pattern::BoundType::Exclusive => {
                                    pattern_automata::RangeBound::Exclusive(c)
                                }
                            },
                            _ => {
                                const_endpoints_ok = false;
                                pattern_automata::RangeBound::Unbounded
                            }
                        };
                        if const_endpoints_ok {
                            self.range_pattern_infos.insert(
                                observed,
                                crate::automata::RangePatternInfo::Char { start, end },
                            );
                        }
                    } else {
                        let start = match lower_const {
                            None => pattern_automata::RangeBound::Unbounded,
                            Some(RangeEndpointLit::Int(i)) => {
                                pattern_automata::RangeBound::Inclusive(i)
                            }
                            _ => {
                                const_endpoints_ok = false;
                                pattern_automata::RangeBound::Unbounded
                            }
                        };
                        let end = match upper_const {
                            None => pattern_automata::RangeBound::Unbounded,
                            Some(RangeEndpointLit::Int(i)) => match bound {
                                hir::pattern::BoundType::Inclusive => {
                                    pattern_automata::RangeBound::Inclusive(i)
                                }
                                hir::pattern::BoundType::Exclusive => {
                                    pattern_automata::RangeBound::Exclusive(i)
                                }
                            },
                            _ => {
                                const_endpoints_ok = false;
                                pattern_automata::RangeBound::Unbounded
                            }
                        };
                        if const_endpoints_ok {
                            self.range_pattern_infos.insert(
                                observed,
                                crate::automata::RangePatternInfo::Int { start, end },
                            );
                        }
                    }
                }
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
                let mut auto_segments = Vec::with_capacity(segments.len());
                let mut auto_ok = u32::try_from(width).is_ok();
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
                            if auto_ok {
                                match bitvec_from_u32_words(segment_width, bits.words()) {
                                    Some(bv) => auto_segments
                                        .push(crate::automata::BitSegmentInfo::Fixed(bv)),
                                    None => auto_ok = false,
                                }
                            }
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
                            if auto_ok {
                                match u32::try_from(width) {
                                    Ok(seg_w) => {
                                        auto_segments.push(
                                            crate::automata::BitSegmentInfo::Typed {
                                                width: seg_w,
                                                ty: ty.clone(),
                                                pattern: checked.clone(),
                                            },
                                        );
                                    }
                                    Err(_) => auto_ok = false,
                                }
                            }
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
                            if auto_ok {
                                match bitvec_from_u32_words(width, bits.words()) {
                                    Some(bv) => auto_segments
                                        .push(crate::automata::BitSegmentInfo::Fixed(bv)),
                                    None => auto_ok = false,
                                }
                            }
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
                if auto_ok && let Ok(total_width) = u32::try_from(width) {
                    self.bitvec_pattern_infos.insert(
                        binding,
                        crate::automata::BitVecPatternInfo {
                            total_width,
                            segments: auto_segments,
                        },
                    );
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
                let first = self.pattern_with_record_head(first, ty, observed, record)?;
                let (value, next) = self.under_observed_pattern_type_facts(
                    &first,
                    observed,
                    value.span,
                    |checker| {
                        let value = checker.expr(value, None)?;
                        let observed = checker.known_pattern_type_value(&value, next.span)?;
                        let next = checker.pattern_observed(next, &value.ty, observed.as_ref())?;
                        Ok((value, next))
                    },
                )?;
                self.lower_computed_pattern(first, value, next, pattern.span)
            }
            HP::Guard(pattern, condition) => {
                let pattern = self.pattern_with_record_head(pattern, ty, observed, record)?;
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
                let owner = self.resolve_pattern_head_owner(head)?;
                if let Some(application) = self.static_normal_application(owner, head.span)? {
                    let target = application.head;
                    if matches!(
                        self.package.item(target).map(|item| &item.kind),
                        Some(ItemKind::Fn(..))
                    ) {
                        self.ensure_signature(target, head.span)?;
                    }
                    let parameters = self.normal_parameters[&target].clone();
                    let head_id = declaration(self.package, &self.package_identities, target);
                    let observed_arguments = match observed {
                        Some(Term::NormalForm {
                            head, arguments, ..
                        }) if *head == head_id && arguments.len() == parameters.len() => {
                            Some(arguments.as_slice())
                        }
                        _ => None,
                    };
                    let inputs = parameters
                        .iter()
                        .map(|(declaration, _, declared_ty)| {
                            (*declaration, declared_ty.clone(), false)
                        })
                        .collect::<Vec<_>>();
                    let fields = self.elaborate_type_application_fields(
                        pattern,
                        arguments,
                        &application.interface,
                        &inputs,
                        observed_arguments,
                    )?;
                    return Ok(n::Pattern::NormalForm {
                        head: head_id,
                        arguments: fields,
                    });
                }
                if matches!(
                    self.package.item(owner).map(|item| &item.kind),
                    Some(ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..))
                ) {
                    self.ensure_nominal(owner, head.span)?;
                    let parameter_ids = self.type_parameters[&owner].clone();
                    let clauses = self.type_clause_params(owner);
                    let mut domains = self.trait_domains(owner)?;
                    if domains.is_empty() {
                        domains.resize(parameter_ids.len(), p(P::Type));
                    }
                    let head_id = declaration(self.package, &self.package_identities, owner);
                    let observed_arguments = match observed {
                        Some(Term::Nominal {
                            declaration,
                            arguments,
                        }) if *declaration == head_id && arguments.len() >= parameter_ids.len() => {
                            Some(arguments.as_slice())
                        }
                        _ => None,
                    };
                    let inputs = parameter_ids
                        .iter()
                        .zip(&domains)
                        .enumerate()
                        .map(|(slot, (id, domain))| {
                            let is_quote = clauses
                                .get(slot)
                                .is_some_and(|clause| clause.kind.is_quote());
                            (*id, domain.clone(), is_quote)
                        })
                        .collect::<Vec<_>>();
                    let mut fields = self.elaborate_type_application_fields(
                        pattern,
                        arguments,
                        &normal::Supply::Declaration(owner),
                        &inputs,
                        observed_arguments,
                    )?;
                    for parameter in self.nominal_captures(owner, head.span)? {
                        let capture_expr = self.captured_input_expression(parameter, head.span)?;
                        let capture_ty = capture_expr.ty.clone();
                        fields.push(self.equality_guard_pattern(
                            &capture_ty,
                            capture_expr,
                            head.hir_id,
                            head.span,
                        )?);
                    }
                    return Ok(n::Pattern::NominalType {
                        declaration: head_id,
                        fields,
                    });
                }
                Err(error(
                    head.span,
                    "pattern head is not a normal-form definition",
                ))
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
            HP::Struct(callee, fields, rest) => {
                if matches!(callee.kind, HP::Wild) {
                    return self.record_pattern(ty, fields, *rest, pattern.span);
                }
                let HP::Const(callee) = &callee.kind else {
                    return Err(error(
                        pattern.span,
                        "named record pattern requires a constructor",
                    ));
                };
                let head = match record {
                    Some(head) => head,
                    None => {
                        let head = self.named_record_head(callee, Some(ty))?;
                        self.require_type_identity(&head.ty, ty, pattern.span)?;
                        head
                    }
                };
                let variant_type = head.ty;
                let Some(index) = head.variant else {
                    return self.record_pattern(&variant_type, fields, *rest, pattern.span);
                };
                let Term::Nominal { declaration, .. } = variant_type else {
                    unreachable!()
                };
                let variant = self
                    .instantiated_variants(&variant_type, pattern.span)?
                    .into_iter()
                    .nth(index)
                    .ok_or_else(|| error(pattern.span, "enum variant index out of bounds"))?;
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
                let variant = self
                    .instantiated_variants(&variant_type, pattern.span)?
                    .into_iter()
                    .nth(index)
                    .ok_or_else(|| error(pattern.span, "enum variant index out of bounds"))?;
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
                    .map(|p| self.pattern_with_record_head(p, ty, observed, record))
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
                let site_hir_id = value.hir_id;
                let value = self.expr(value, Some(ty))?;
                if self.structural_type(ty, pattern.span)? == p(P::Type) {
                    return self.equality_guard_pattern(ty, value, site_hir_id, pattern.span);
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
                    _ => self.equality_guard_pattern(ty, value, site_hir_id, pattern.span),
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

    fn resolve_pattern_head_owner(&mut self, head: &Expr<'_>) -> Result<OwnerId, Error> {
        let def_id = match head.kind {
            H::ResolvedDef { def_id, .. } => def_id,
            H::Projection(container, _) => {
                if let Some(def_id) = self.package.member_projections.get(&head.hir_id).copied() {
                    def_id
                } else if let Some(candidates) = self
                    .package
                    .deferred_member_projections
                    .get(&head.hir_id)
                    .cloned()
                {
                    let invalid = || {
                        error(
                            head.span,
                            "normal-form pattern head requires a resolved declaration",
                        )
                    };
                    if !self.type_member_base(container)? {
                        return Err(invalid());
                    }
                    let Term::Nominal {
                        declaration: parent,
                        ..
                    } = self.type_expr(container)?
                    else {
                        return Err(invalid());
                    };
                    candidates
                        .iter()
                        .find_map(|(container, member)| {
                            let owner = self.package.definition_owner(*container)?;
                            (declaration(self.package, &self.package_identities, owner) == parent)
                                .then_some(*member)
                        })
                        .ok_or_else(invalid)?
                } else {
                    return Err(error(
                        head.span,
                        "normal-form pattern head requires a resolved declaration",
                    ));
                }
            }
            _ => {
                return Err(error(
                    head.span,
                    "normal-form pattern head requires a resolved declaration",
                ));
            }
        };
        self.package
            .definition_owner(def_id)
            .ok_or_else(|| error(head.span, "missing normal-form head"))
    }

    fn elaborate_type_application_fields(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        arguments: &[hir::pattern::TypePatternArg<'_>],
        interface: &normal::Supply,
        parameters: &[(DeclarationId, Term, bool)],
        observed_arguments: Option<&[Term]>,
    ) -> Result<Vec<n::Pattern>, Error> {
        let entries = match interface {
            normal::Supply::Declaration(decl_owner) => self
                .type_clause_params(*decl_owner)
                .iter()
                .map(|clause| input_routing::Entry {
                    name: (!clause.name.name.as_str().is_empty()).then_some(clause.name.name),
                    optional: false,
                    variadic: matches!(
                        clause.kind.unquoted(),
                        hir::clause::ClauseParamKind::Varadic(..)
                    ),
                    post_target: false,
                    ..input_routing::Entry::anchored()
                })
                .collect::<Vec<_>>(),
            normal::Supply::Callable { collector } => {
                let mut entries = vec![
                    input_routing::Entry {
                        post_target: false,
                        ..input_routing::Entry::anchored()
                    };
                    parameters.len()
                ];
                if let Some(collector) = collector
                    && let Some(entry) = entries.get_mut(collector.slot)
                {
                    entry.variadic = true;
                    entry.name = collector.name.as_deref().map(hir::Symbol::intern);
                }
                entries
            }
        };
        if entries.len() != parameters.len() {
            return Err(error(
                pattern.span,
                "normal-form pattern argument count mismatch",
            ));
        }
        let inputs = arguments
            .iter()
            .map(|argument| match argument.name {
                Some(name) => input_routing::Input::Named(name.name),
                None => input_routing::Input::Positional,
            })
            .collect::<Vec<_>>();
        let plan = input_routing::route(&entries, &inputs).map_err(|failure| {
            let failure_span = failure
                .input
                .and_then(|pos| arguments.get(pos))
                .map(|arg| {
                    if matches!(failure.kind, input_routing::FailureKind::UnknownName) {
                        arg.name.map_or(arg.pattern.span, |name| name.span)
                    } else {
                        arg.pattern.span
                    }
                })
                .unwrap_or(pattern.span);
            let message = match failure.kind {
                input_routing::FailureKind::UnknownName => "unknown normal-form pattern argument",
                input_routing::FailureKind::Duplicate
                | input_routing::FailureKind::AppendToComplete => {
                    "duplicate normal-form pattern argument"
                }
                _ => "normal-form pattern argument count mismatch",
            };
            error(failure_span, message)
        })?;
        if plan.assigned.iter().any(|assigned| !*assigned) {
            return Err(error(
                pattern.span,
                "normal-form pattern argument count mismatch",
            ));
        }
        let mut by_slot: Vec<Option<&input_routing::Route>> = vec![None; parameters.len()];
        for route in &plan.routes {
            by_slot[route.parameter] = Some(route);
        }

        // Elaborate fields in declaration order: a later parameter's type may
        // depend on an earlier input even when named patterns appear out of order.
        let mut fields = Vec::with_capacity(parameters.len());
        let mut known_parameters = Vec::new();
        let mut known_types = Vec::new();
        for (slot, (parameter, declared_ty, is_quote)) in parameters.iter().enumerate() {
            let route = by_slot[slot].expect("assigned check guarantees every slot is routed");
            let ty = declared_ty
                .instantiate_parameters(&known_parameters, &known_types)
                .map_err(|failure| error(pattern.span, failure.to_string()))?;
            let err_span = route
                .sources
                .first()
                .and_then(|pos| arguments.get(*pos))
                .map_or(pattern.span, |arg| arg.pattern.span);
            if *is_quote || matches!(ty, Term::Lift(_)) {
                return Err(error(
                    err_span,
                    "nominal type patterns cannot match runtime dependency clauses",
                ));
            }
            let dependent = parameters
                .iter()
                .any(|(parameter, _, _)| ty.mentions_rigid(*parameter));
            let field = if route.collected {
                if dependent {
                    return Err(error(
                        err_span,
                        "dependent normal-form value patterns require contextual pattern elaboration not yet implemented",
                    ));
                }
                let elements = route
                    .sources
                    .iter()
                    .map(|pos| arguments[*pos].pattern.clone())
                    .collect::<Vec<_>>();
                self.list_pattern(pattern, &elements, None, &ty)?
            } else {
                let argument = &arguments[route.sources[0]].pattern;
                let observed_type = if ty == p(P::Type) {
                    observed_arguments
                        .and_then(|arguments| arguments.get(slot))
                        .filter(|value| value.sort() == Sort::Type)
                } else {
                    None
                };
                if dependent {
                    if matches!(argument.kind, hir::pattern::PatternKind::Wild) {
                        n::Pattern::Wild
                    } else {
                        return Err(error(
                            argument.span,
                            "dependent normal-form value patterns require contextual pattern elaboration not yet implemented",
                        ));
                    }
                } else {
                    self.pattern_observed(argument, &ty, observed_type)?
                }
            };
            if ty == p(P::Type) {
                let mut evidence =
                    matched_type_pattern_value(&field, &mut self.required_steps, err_span, 0)?
                        .cloned();
                if evidence.is_none() {
                    evidence = self.known_type_equality_pattern(&field, err_span)?;
                }
                let observed_type = observed_arguments
                    .and_then(|arguments| arguments.get(slot))
                    .filter(|value| value.sort() == Sort::Type);
                if let Some(value) = evidence.or_else(|| observed_type.cloned()) {
                    // This evidence belongs only to the successful
                    // candidate; do not install a global solver fact.
                    known_parameters.push(nir::DeclarationParameter {
                        declaration: *parameter,
                        sort: Sort::Type,
                    });
                    known_types.push(value);
                }
            } else {
                let evidence = self
                    .matched_value_pattern_input(&field, &ty, err_span, 0)?
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
        Ok(fields)
    }

    fn equality_guard_pattern(
        &mut self,
        observed_ty: &Term,
        value: n::Expr,
        site_hir_id: hir::hir_id::HirId,
        span: Span,
    ) -> Result<n::Pattern, Error> {
        let shape = self.structural_type(observed_ty, span)?;
        let index = self.next_temporary_binding;
        self.next_temporary_binding = index
            .checked_add(1)
            .ok_or_else(|| error(span, "pattern temporary binding space exhausted"))?;
        let binding = n::LocalId {
            owner: u32::MAX,
            index,
        };
        let observed = n::Expr {
            source: source(span),
            ty: observed_ty.clone(),
            kind: K::Local(binding),
        };
        let condition = if shape == p(P::Type) {
            n::Expr {
                source: source(span),
                ty: if is_never(&value.ty) {
                    p(P::Never)
                } else {
                    p(P::Bool)
                },
                kind: K::TypeEqual(Box::new(observed), Box::new(value)),
            }
        } else if self.native_equality_shape(&shape) {
            self.equality_expression(observed, value, &shape, span)?
        } else {
            let protocol = self.protocols.equality.ok_or_else(|| {
                error(
                    span,
                    "equality protocol for this type is not implemented without an Eq binding",
                )
            })?;
            let callee = self
                .selected_method(
                    observed.ty.clone(),
                    Some(n::InterfaceInstance {
                        declaration: protocol,
                        arguments: vec![],
                    }),
                    "eq",
                    span,
                )?
                .ok_or_else(|| error(span, "Eq protocol has no selected comparison method"))?;
            let site = Expr {
                hir_id: site_hir_id,
                span,
                kind: H::Unit,
            };
            self.finish_call(
                &site,
                callee,
                &[],
                vec![observed, value],
                Some(&p(P::Bool)),
                None,
            )?
        };
        // Equality matching is an observation, not inversion of a constructor
        // head. A guard leaves coverage conservative and uses rollback.
        Ok(n::Pattern::Guard {
            pattern: Box::new(n::Pattern::Binding {
                binding,
                inner: Box::new(n::Pattern::Wild),
            }),
            condition: Box::new(condition),
        })
    }

    fn can_negate_pattern_domain(&mut self, ty: &Term, span: Span) -> Result<bool, Error> {
        let mut visited = Vec::new();
        self.can_negate_pattern_domain_at(ty, span, 0, &mut visited)
    }

    fn can_negate_pattern_domain_at(
        &mut self,
        ty: &Term,
        span: Span,
        depth: usize,
        visited: &mut Vec<Term>,
    ) -> Result<bool, Error> {
        if depth >= 64 {
            return Err(error(span, "negated pattern domain depth limit exceeded"));
        }
        let shape = self.structural_type(ty, span)?;
        match &shape {
            Term::Primitive(primitive) => Ok(matches!(
                primitive,
                P::Void
                    | P::Bool
                    | P::Char
                    | P::Str
                    | P::Symbol
                    | P::Integer
                    | P::Float
                    | P::F16
                    | P::Bf16
                    | P::F32
                    | P::F64
                    | P::Type
                    | P::Object
                    | P::BitVec(_)
            ) || self.integer_layout(*primitive).is_some()),
            Term::Optional(inner) => {
                self.can_negate_pattern_domain_at(inner, span, depth + 1, visited)
            }
            Term::NormalForm { head, .. } if Some(*head) == self.protocols.meta_list => Ok(true),
            Term::Nominal {
                declaration,
                arguments,
            } => {
                if visited.iter().any(|seen| *seen == shape) {
                    return Ok(true);
                }
                self.ensure_nominal_type(&shape, span)?;
                let variants = self.enums.get(declaration).map(|item| item.variants.len());
                let mut fuel = 100_000usize;
                let mut collected = Some(Vec::new());
                for index in 0..variants.unwrap_or(1) {
                    let Some(fields) = nir::normal_forms::nominal_value_fields_with_collections_in(
                        &shape,
                        variants.map(|_| index as u32),
                        &mut fuel,
                        &|id, fuel| {
                            *fuel = fuel.checked_sub(1)?;
                            self.structs
                                .get(&id)
                                .map(nir::normal_forms::NominalValueDeclaration::Struct)
                                .or_else(|| {
                                    self.enums
                                        .get(&id)
                                        .map(nir::normal_forms::NominalValueDeclaration::Enum)
                                })
                                .or_else(|| {
                                    self.newtypes
                                        .get(&id)
                                        .map(nir::normal_forms::NominalValueDeclaration::Newtype)
                                })
                        },
                        &|domain, fuel| self.collection_domain_budgeted(domain, fuel),
                        self.target,
                    ) else {
                        collected = None;
                        break;
                    };
                    if let Some(list) = collected.as_mut() {
                        list.extend(fields);
                    }
                }
                let fields_to_check = if let Some(fields) = collected {
                    fields
                } else {
                    let mut fallback = Vec::new();
                    if let Some(def) = self.newtypes.get(declaration) {
                        let arity = def.type_parameters.len().min(arguments.len());
                        let field_ty = self.instantiate_input_types(
                            &def.representation,
                            &def.type_parameters[..arity],
                            &arguments[..arity],
                            span,
                        )?;
                        fallback.push(field_ty);
                    } else if let Some(def) = self.structs.get(declaration).cloned() {
                        let arity = def.type_parameters.len().min(arguments.len());
                        for (_, field_ty) in &def.fields {
                            let field_ty = self.instantiate_input_types(
                                field_ty,
                                &def.type_parameters[..arity],
                                &arguments[..arity],
                                span,
                            )?;
                            fallback.push(field_ty);
                        }
                    } else if let Some(def) = self.enums.get(declaration).cloned() {
                        let arity = def.type_parameters.len().min(arguments.len());
                        for variant in &def.variants {
                            for field_ty in &variant.fields {
                                let field_ty = self.instantiate_input_types(
                                    field_ty,
                                    &def.type_parameters[..arity],
                                    &arguments[..arity],
                                    span,
                                )?;
                                fallback.push(field_ty);
                            }
                        }
                    } else {
                        return Ok(false);
                    }
                    fallback
                };
                visited.push(shape);
                let result = (|| {
                    for field_ty in fields_to_check {
                        if !self.can_negate_pattern_domain_at(
                            &field_ty,
                            span,
                            depth + 1,
                            visited,
                        )? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                })();
                visited.pop();
                result
            }
            Term::Tuple(elements) => {
                for element in elements {
                    if !self.can_negate_pattern_domain_at(element, span, depth + 1, visited)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Term::Record(fields) => {
                for (_, field_ty) in fields {
                    if !self.can_negate_pattern_domain_at(field_ty, span, depth + 1, visited)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Term::ErrorResult { errors, result } => {
                for err_ty in errors {
                    if !self.can_negate_pattern_domain_at(err_ty, span, depth + 1, visited)? {
                        return Ok(false);
                    }
                }
                self.can_negate_pattern_domain_at(result, span, depth + 1, visited)
            }
            _ => self
                .negated_pattern_domain(
                    &shape,
                    &mut 100_000,
                    0,
                    &mut std::collections::HashMap::new(),
                )
                .map_err(|message| error(span, message)),
        }
    }
}

#[derive(Clone, Copy)]
enum RangeEndpointLit {
    Int(pattern_automata::IntLit),
    Char(char),
}

fn const_range_endpoint(expr: &n::Expr) -> Option<RangeEndpointLit> {
    match &expr.kind {
        K::Literal(n::Literal::Char(c)) => Some(RangeEndpointLit::Char(*c)),
        K::Literal(n::Literal::SignedInteger(v)) => {
            Some(RangeEndpointLit::Int(pattern_automata::IntLit::Signed(*v)))
        }
        K::Literal(n::Literal::UnsignedInteger(v)) => Some(RangeEndpointLit::Int(
            pattern_automata::IntLit::Unsigned(*v),
        )),
        K::Literal(n::Literal::Integer(s)) => {
            if let Ok(v) = s.parse::<i128>() {
                Some(RangeEndpointLit::Int(pattern_automata::IntLit::Signed(v)))
            } else if let Ok(u) = s.parse::<u128>() {
                Some(RangeEndpointLit::Int(pattern_automata::IntLit::Unsigned(u)))
            } else {
                None
            }
        }
        K::IntegerNegate(inner) => match const_range_endpoint(inner) {
            Some(RangeEndpointLit::Int(pattern_automata::IntLit::Signed(v))) => v
                .checked_neg()
                .map(|neg| RangeEndpointLit::Int(pattern_automata::IntLit::Signed(neg))),
            Some(RangeEndpointLit::Int(pattern_automata::IntLit::Unsigned(u))) => {
                if u <= i128::MAX as u128 {
                    Some(RangeEndpointLit::Int(pattern_automata::IntLit::Signed(
                        -(u as i128),
                    )))
                } else if u == (i128::MAX as u128) + 1 {
                    Some(RangeEndpointLit::Int(pattern_automata::IntLit::Signed(
                        i128::MIN,
                    )))
                } else {
                    None
                }
            }
            _ => None,
        },
        _ => None,
    }
}

fn bitvec_from_u32_words(
    width: usize,
    words: &[u32],
) -> Option<pattern_automata::bitvec::BitValue> {
    let w = u32::try_from(width).ok()?;
    let mut u64_words = Vec::with_capacity(words.len().div_ceil(2));
    for pair in words.chunks(2) {
        let lo = u64::from(pair[0]);
        let hi = pair.get(1).copied().map(u64::from).unwrap_or(0);
        u64_words.push(lo | (hi << 32));
    }
    pattern_automata::bitvec::BitValue::from_words(w, u64_words).ok()
}
