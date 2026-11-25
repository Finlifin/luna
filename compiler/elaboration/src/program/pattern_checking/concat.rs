//! Concat pattern (`++`) elaboration for `BitVec` bit algebra and `ConcatPatternView`.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn concat_pattern(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        lhs: &hir::pattern::Pattern<'_>,
        rhs: &hir::pattern::Pattern<'_>,
        ty: &Term,
    ) -> Result<n::Pattern, Error> {
        let shape = self.structural_type(ty, pattern.span)?;
        if let Term::Primitive(P::BitVec(width)) = shape {
            return self.bitvec_concat_pattern(pattern, lhs, rhs, width);
        }
        if let Term::Nominal { declaration, .. } = &shape
            && let Some(def) = self.enums.get(declaration)
            && let Some(encoding) = &def.encoding
        {
            let width = encoding.layout.width();
            let binding = self.alloc_pattern_temporary(pattern.span)?;
            self.locals.insert(binding, ty.clone());
            let inner = self.bitvec_concat_pattern(pattern, lhs, rhs, width)?;
            let subject = n::Expr {
                ty: ty.clone(),
                source: source(pattern.span),
                kind: K::Local(binding),
            };
            let bits = n::Expr {
                ty: p(P::BitVec(width)),
                source: source(pattern.span),
                kind: K::TakeBits(Box::new(subject)),
            };
            return Ok(n::Pattern::Observe {
                binding,
                inner: Box::new(n::Pattern::Then {
                    first: Box::new(n::Pattern::Wild),
                    value: Box::new(bits),
                    next: Box::new(inner),
                }),
            });
        }
        self.sequence_concat_pattern(pattern, lhs, rhs, ty)
    }

    fn alloc_pattern_temporary(&mut self, span: Span) -> Result<n::LocalId, Error> {
        let binding = n::LocalId {
            owner: u32::MAX,
            index: self.next_temporary_binding,
        };
        self.next_temporary_binding = self
            .next_temporary_binding
            .checked_add(1)
            .ok_or_else(|| error(span, "pattern temporary binding space exhausted"))?;
        Ok(binding)
    }

    fn static_bitvec_pattern_width(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
    ) -> Result<Option<usize>, Error> {
        use hir::pattern::PatternKind as HP;
        match &pattern.kind {
            HP::BitVec { radix, segments } => {
                let mut total = 0usize;
                for segment in segments {
                    let seg_w = match segment {
                        hir::pattern::BitSegment::Literal { spelling, span } => {
                            let digits = spelling.bytes().filter(|c| *c != b'_').count();
                            let bits_per_digit = match radix {
                                2 => 1usize,
                                8 => 3,
                                16 => 4,
                                _ => return Err(error(*span, "invalid bit-vector radix")),
                            };
                            digits
                                .checked_mul(bits_per_digit)
                                .ok_or_else(|| error(*span, "bit-vector pattern width overflow"))?
                        }
                        hir::pattern::BitSegment::Typed { pattern: inner, ty } => {
                            let ty = self.type_expr(ty)?;
                            let shape = self.structural_type(&ty, inner.span)?;
                            self.bitvec_type_width(&shape, inner.span)?
                        }
                        hir::pattern::BitSegment::Computed(expr) => {
                            let value = self.required_expression(expr, None)?;
                            let comptime_interpretor::Value::BitVec(bits) = value else {
                                return Err(error(
                                    expr.span,
                                    "computed bit-vector segment requires a concrete BitVec value",
                                ));
                            };
                            bits.width()
                        }
                    };
                    total = total
                        .checked_add(seg_w)
                        .ok_or_else(|| error(pattern.span, "bit-vector pattern width overflow"))?;
                }
                Ok(Some(total))
            }
            HP::Typed(_, ty_expr) => {
                let ty = self.type_expr(ty_expr)?;
                let shape = self.structural_type(&ty, pattern.span)?;
                self.bitvec_type_width(&shape, pattern.span).map(Some)
            }
            HP::Comptime(expr) => match self.required_expression(expr, None) {
                Ok(comptime_interpretor::Value::BitVec(bits)) => Ok(Some(bits.width())),
                _ => Ok(None),
            },
            HP::Concat(left, right) => match (
                self.static_bitvec_pattern_width(left)?,
                self.static_bitvec_pattern_width(right)?,
            ) {
                (Some(wl), Some(wr)) => wl
                    .checked_add(wr)
                    .map(Some)
                    .ok_or_else(|| error(pattern.span, "bit-vector pattern width overflow")),
                _ => Ok(None),
            },
            HP::Binding(_, _, Some(inner))
            | HP::Guard(inner, _)
            | HP::OptionSome(inner)
            | HP::ErrorOk(inner)
            | HP::ErrorErr(inner) => self.static_bitvec_pattern_width(inner),
            _ => Ok(None),
        }
    }

    fn bitvec_type_width(&self, shape: &Term, span: Span) -> Result<usize, Error> {
        match shape {
            Term::Primitive(P::BitVec(width)) => Ok(*width),
            Term::Primitive(primitive) => primitive
                .unrestricted_scalar_width_in(self.target)
                .map(|width| width as usize)
                .ok_or_else(|| {
                    error(
                        span,
                        "bit-vector segment type requires a supported unrestricted scalar or BitVec representation",
                    )
                }),
            _ => Err(error(
                span,
                "bit-vector segment type has no supported representation",
            )),
        }
    }

    fn bitvec_concat_pattern(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        lhs: &hir::pattern::Pattern<'_>,
        rhs: &hir::pattern::Pattern<'_>,
        total_width: usize,
    ) -> Result<n::Pattern, Error> {
        let wl_opt = self.static_bitvec_pattern_width(lhs)?;
        let wr_opt = self.static_bitvec_pattern_width(rhs)?;
        let (w_left, w_right) = match (wl_opt, wr_opt) {
            (None, None) => {
                return Err(error(
                    pattern.span,
                    "ambiguous concat pattern: cannot split BitVec between two variable-width patterns without a BitVec<M> width annotation",
                ));
            }
            (Some(wl), Some(wr)) => {
                if wl.checked_add(wr) != Some(total_width) {
                    return Err(error(
                        pattern.span,
                        "bit-vector pattern width differs from its subject",
                    ));
                }
                (wl, wr)
            }
            (Some(wl), None) => {
                let wr = total_width.checked_sub(wl).ok_or_else(|| {
                    error(
                        pattern.span,
                        "bit-vector pattern width differs from its subject",
                    )
                })?;
                (wl, wr)
            }
            (None, Some(wr)) => {
                let wl = total_width.checked_sub(wr).ok_or_else(|| {
                    error(
                        pattern.span,
                        "bit-vector pattern width differs from its subject",
                    )
                })?;
                (wl, wr)
            }
        };

        let left_ty = p(P::BitVec(w_left));
        let right_ty = p(P::BitVec(w_right));
        let left_checked = self.pattern(lhs, &left_ty)?;
        let right_checked = self.pattern(rhs, &right_ty)?;

        let binding = self.alloc_pattern_temporary(pattern.span)?;
        let subject_ty = p(P::BitVec(total_width));
        self.locals.insert(binding, subject_ty.clone());
        let subject = n::Expr {
            ty: subject_ty.clone(),
            source: source(pattern.span),
            kind: K::Local(binding),
        };
        let high_slice = n::Expr {
            ty: left_ty.clone(),
            source: source(lhs.span),
            kind: K::BitSlice {
                value: Box::new(subject.clone()),
                start: w_right,
                end: total_width,
            },
        };
        let low_slice = n::Expr {
            ty: right_ty.clone(),
            source: source(rhs.span),
            kind: K::BitSlice {
                value: Box::new(subject),
                start: 0,
                end: w_right,
            },
        };

        if let (Ok(total_w), Ok(lw), Ok(rw)) = (
            u32::try_from(total_width),
            u32::try_from(w_left),
            u32::try_from(w_right),
        ) {
            self.bitvec_pattern_infos.insert(
                binding,
                crate::automata::BitVecPatternInfo {
                    total_width: total_w,
                    segments: vec![
                        crate::automata::BitSegmentInfo::Typed {
                            width: lw,
                            ty: left_ty,
                            pattern: left_checked.clone(),
                        },
                        crate::automata::BitSegmentInfo::Typed {
                            width: rw,
                            ty: right_ty,
                            pattern: right_checked.clone(),
                        },
                    ],
                },
            );
        }

        let inner = n::Pattern::Then {
            first: Box::new(n::Pattern::Then {
                first: Box::new(n::Pattern::Wild),
                value: Box::new(high_slice),
                next: Box::new(left_checked),
            }),
            value: Box::new(low_slice),
            next: Box::new(right_checked),
        };
        Ok(n::Pattern::Observe {
            binding,
            inner: Box::new(inner),
        })
    }

    fn has_concat_protocol_impl(
        &self,
        protocol: DeclarationId,
        ty: &Term,
        required_phase: Option<n::TraitPhase>,
    ) -> bool {
        let iface = n::InterfaceInstance {
            declaration: protocol,
            arguments: vec![],
        };
        if self.symbolic_interface_evidence(ty, &iface).ok() == Some(true) {
            return true;
        }
        self.resolve_trait_capability(&iface, ty, crate::ptc::Budget::default())
            .ok()
            .is_some_and(|res| {
                matches!(
                    res.selection,
                    crate::implementation::Selection::Selected(impl_id)
                        if self
                            .implementation_positions
                            .get(&impl_id)
                            .and_then(|pos| self.checked_implementations.get(*pos))
                            .is_some_and(|imp| required_phase.is_none_or(|p| imp.phases.contains(p)))
                )
            })
    }

    fn resolve_concat_assoc_type(
        &self,
        ty: &Term,
        protocol: DeclarationId,
        assoc_id: DeclarationId,
        span: Span,
    ) -> Term {
        let projected = Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: protocol,
                member: assoc_id,
            }),
            arguments: vec![],
            subject: Box::new(ty.clone()),
        };
        let assumptions = self.capability_assumptions();
        let resolved = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        }
        .associated_type_view_metered(&projected, &mut crate::ptc::Budget::default())
        .map(|value| value.value)
        .unwrap_or(projected);
        self.structural_type(&resolved, span).unwrap_or(resolved)
    }

    fn sequence_concat_pattern(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        lhs: &hir::pattern::Pattern<'_>,
        rhs: &hir::pattern::Pattern<'_>,
        ty: &Term,
    ) -> Result<n::Pattern, Error> {
        let mut pieces = Vec::new();
        flatten_concat_pieces(lhs, &mut pieces);
        flatten_concat_pieces(rhs, &mut pieces);
        self.sequence_concat_pieces(&pieces, ty, pattern.span)
    }

    fn sequence_concat_pieces(
        &mut self,
        pieces: &[&hir::pattern::Pattern<'_>],
        ty: &Term,
        span: Span,
    ) -> Result<n::Pattern, Error> {
        let [first, rest @ ..] = pieces else {
            return Err(error(span, "empty concat pattern"));
        };
        if rest.is_empty() {
            return self.pattern(first, ty);
        }
        let last = pieces[pieces.len() - 1];
        let strip_prefix = is_part_pattern(first);
        let strip_suffix = !strip_prefix && is_part_pattern(last);
        if !strip_prefix && !strip_suffix {
            return Err(error(
                span,
                "ambiguous concat pattern: cannot split sequence between two variable-length patterns without a fixed-width BitVec annotation",
            ));
        }

        let comptime = !self.runtime_value_context() && !self.in_code_body;
        let active_phase = if comptime {
            n::TraitPhase::Comptime
        } else {
            n::TraitPhase::Runtime
        };
        let protocol = self.protocols.concat_pattern.ok_or_else(|| {
            error(
                span,
                "concat patterns require a bound ConcatPatternView protocol",
            )
        })?;
        let contract = crate::concat_patterns::for_phase(&self.checked_traits, protocol, comptime)
            .map_err(|message| error(span, message))?;
        if contract.comptime != comptime {
            return Err(error(
                span,
                if comptime {
                    "concat patterns require the comptime ConcatPatternView phase"
                } else {
                    "concat patterns require the runtime ConcatPatternView phase"
                },
            ));
        }
        let (view_ty, deref_subject) =
            if self.has_concat_protocol_impl(protocol, ty, Some(active_phase)) {
                (ty.clone(), false)
            } else if !comptime
                && let Ok(Term::Pointer(pointee)) = self.structural_type(ty, span)
                && !contract.by_value
                && self.has_concat_protocol_impl(protocol, &pointee, Some(active_phase))
            {
                ((*pointee).clone(), true)
            } else {
                return Err(error(
                    span,
                    if comptime {
                        "concat pattern subject does not implement comptime ConcatPatternView"
                    } else {
                        "concat pattern subject does not implement ConcatPatternView"
                    },
                ));
            };

        let part_ty = self.resolve_concat_assoc_type(&view_ty, protocol, contract.part_assoc, span);
        let rest_ty = self.resolve_concat_assoc_type(&view_ty, protocol, contract.rest_assoc, span);

        let (part_pat, remaining, method_name) = if strip_prefix {
            (*first, rest, contract.strip_prefix)
        } else {
            (last, &pieces[..pieces.len() - 1], contract.strip_suffix)
        };

        let binding = self.alloc_pattern_temporary(span)?;
        self.locals.insert(binding, ty.clone());
        let subject = n::Expr {
            source: source(span),
            ty: ty.clone(),
            kind: K::Local(binding),
        };
        let receiver = if deref_subject {
            subject
        } else if contract.by_value {
            subject
        } else if comptime {
            return Err(error(
                span,
                "comptime concat patterns require value-reference self; replace *self with self",
            ));
        } else {
            n::Expr {
                source: source(span),
                ty: Term::Pointer(Box::new(ty.clone())),
                kind: K::AddressOf(Box::new(subject)),
            }
        };

        let part_expr = self.elaborate_concat_part(part_pat, &part_ty)?;
        let site = Expr {
            hir_id: part_pat.hir_id,
            span: part_pat.span,
            kind: H::Unit,
        };
        let method = self
            .selected_method(
                view_ty,
                Some(n::InterfaceInstance {
                    declaration: protocol,
                    arguments: vec![],
                }),
                method_name,
                part_pat.span,
            )?
            .ok_or_else(|| error(part_pat.span, "ConcatPatternView has no selected method"))?;
        let call_expr =
            self.finish_call(&site, method, &[], vec![receiver, part_expr], None, None)?;
        let rest_pattern = self.sequence_concat_pieces(remaining, &rest_ty, span)?;

        Ok(n::Pattern::Observe {
            binding,
            inner: Box::new(n::Pattern::Then {
                first: Box::new(n::Pattern::Wild),
                value: Box::new(call_expr),
                next: Box::new(n::Pattern::OptionalPresent(Box::new(rest_pattern))),
            }),
        })
    }

    fn elaborate_concat_part(
        &mut self,
        part: &hir::pattern::Pattern<'_>,
        expected: &Term,
    ) -> Result<n::Expr, Error> {
        use hir::pattern::PatternKind as HP;
        match &part.kind {
            HP::Const(expr) | HP::Comptime(expr) => self.expr(expr, Some(expected)),
            HP::List(elements, None) => {
                let Some(elem_ty) = self.list_element_type(expected).cloned() else {
                    return Err(error(
                        part.span,
                        "list literal in concat pattern requires a meta.List part type",
                    ));
                };
                let mut items = Vec::with_capacity(elements.len());
                for element in *elements {
                    items.push(self.elaborate_concat_part(element, &elem_ty)?);
                }
                let ty = if items.iter().any(|item| is_never(&item.ty)) {
                    p(P::Never)
                } else {
                    expected.clone()
                };
                Ok(n::Expr {
                    source: source(part.span),
                    ty,
                    kind: K::TypeList(items),
                })
            }
            HP::Tuple(elements) => {
                let shape = self.structural_type(expected, part.span)?;
                let Term::Tuple(elem_tys) = shape else {
                    return Err(error(
                        part.span,
                        "tuple literal in concat pattern requires a tuple part type",
                    ));
                };
                if elements.len() != elem_tys.len() {
                    return Err(error(part.span, "concat pattern tuple part arity mismatch"));
                }
                let mut items = Vec::with_capacity(elements.len());
                for (element, elem_ty) in elements.iter().zip(elem_tys) {
                    items.push(self.elaborate_concat_part(element, &elem_ty)?);
                }
                Ok(n::Expr {
                    source: source(part.span),
                    ty: expected.clone(),
                    kind: K::Tuple(items),
                })
            }
            HP::OptionNull => {
                let _ = self.optional_payload(expected, part.span)?;
                Ok(n::Expr {
                    source: source(part.span),
                    ty: expected.clone(),
                    kind: K::OptionalAbsent,
                })
            }
            _ => Err(error(
                part.span,
                "concat pattern part must be a fixed expression",
            )),
        }
    }
}

fn flatten_concat_pieces<'hir>(
    pat: &'hir hir::pattern::Pattern<'hir>,
    out: &mut Vec<&'hir hir::pattern::Pattern<'hir>>,
) {
    if let hir::pattern::PatternKind::Concat(left, right) = &pat.kind {
        flatten_concat_pieces(left, out);
        flatten_concat_pieces(right, out);
    } else {
        out.push(pat);
    }
}

fn is_part_pattern(pat: &hir::pattern::Pattern<'_>) -> bool {
    use hir::pattern::PatternKind as HP;
    match &pat.kind {
        HP::Const(_) | HP::Comptime(_) | HP::OptionNull => true,
        HP::List(elements, None) => elements.iter().all(is_part_pattern),
        HP::Tuple(elements) => elements.iter().all(is_part_pattern),
        _ => false,
    }
}
