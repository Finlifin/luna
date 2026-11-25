//! Range pattern dispatch: native scalars, `RangePatternView` (runtime & comptime), and `Ord` fallback.
use super::*;

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

fn is_static_const_range_expr(expr: &n::Expr) -> bool {
    match &expr.kind {
        K::Unit | K::Literal(_) | K::TypeValue(_) | K::OptionalAbsent => true,
        K::IntegerNegate(inner)
        | K::FloatNegate(inner)
        | K::NewtypeConvert(inner)
        | K::OptionalPresent(inner) => is_static_const_range_expr(inner),
        K::Tuple(elems)
        | K::Record { fields: elems, .. }
        | K::AnonymousRecord { fields: elems }
        | K::Variant { fields: elems, .. } => elems.iter().all(is_static_const_range_expr),
        _ => false,
    }
}

impl Checker<'_, '_> {
    fn fresh_range_binding(&mut self, span: Span) -> Result<n::LocalId, Error> {
        let index = self.next_temporary_binding;
        self.next_temporary_binding = index
            .checked_add(1)
            .ok_or_else(|| error(span, "pattern temporary binding space exhausted"))?;
        Ok(n::LocalId {
            owner: u32::MAX,
            index,
        })
    }

    fn has_range_protocol_impl(
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

    fn has_stable_range_view(&self, ty: &Term, active_phase: n::TraitPhase) -> bool {
        let Some(stable_id) = self.protocols.stable_range_pattern else {
            return false;
        };
        let stable_iface = n::InterfaceInstance {
            declaration: stable_id,
            arguments: vec![],
        };
        if self.symbolic_interface_evidence(ty, &stable_iface).ok() == Some(true) {
            return true;
        }
        let assumptions = self.capability_assumptions();
        let world = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        };
        world
            .resolve_instance_metered(&stable_iface, ty, &mut crate::ptc::Budget::default())
            .ok()
            .is_some_and(|res| {
                matches!(
                    res.types.selection,
                    crate::implementation::Selection::Selected(impl_id)
                        if matches!(
                            res.qualifications.get(&impl_id),
                            Some(crate::capabilities::Qualification::Proved(_))
                        ) && self
                            .implementation_positions
                            .get(&impl_id)
                            .and_then(|pos| self.checked_implementations.get(*pos))
                            .is_some_and(|imp| imp.phases.contains(active_phase))
                )
            })
    }

    fn resolve_range_bound_type(
        &self,
        ty: &Term,
        protocol: DeclarationId,
        bound_name: &str,
        span: Span,
    ) -> Result<Term, Error> {
        let Some(interface) = self.checked_traits.by_id(protocol) else {
            return Err(error(span, "RangePatternView trait metadata is missing"));
        };
        let Some(member) = interface
            .associated
            .iter()
            .find(|item| item.name == bound_name)
        else {
            return Err(error(
                span,
                format!("RangePatternView is missing associated {bound_name}"),
            ));
        };
        let projected = Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: protocol,
                member: member.declaration,
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
        Ok(self.structural_type(&resolved, span).unwrap_or(resolved))
    }

    pub(super) fn range_pattern(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        lower: Option<&Expr<'_>>,
        upper: Option<&Expr<'_>>,
        bound: &hir::pattern::BoundType,
        ty: &Term,
    ) -> Result<n::Pattern, Error> {
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
        if native_ordered {
            return self
                .native_or_ord_range_pattern(pattern, lower, upper, bound, ty, &shape, None);
        }

        let comptime = !self.runtime_value_context() && !self.in_code_body;
        let active_phase = if comptime {
            n::TraitPhase::Comptime
        } else {
            n::TraitPhase::Runtime
        };
        if let Some(range_protocol) = self.protocols.range_pattern {
            let has_active = self.has_range_protocol_impl(range_protocol, ty, Some(active_phase));
            if has_active {
                return self.view_range_pattern(
                    pattern,
                    lower,
                    upper,
                    bound,
                    ty,
                    range_protocol,
                    comptime,
                    active_phase,
                );
            }
            if self.has_range_protocol_impl(range_protocol, ty, None) {
                return Err(error(
                    pattern.span,
                    if comptime {
                        "range patterns require the comptime RangePatternView phase"
                    } else {
                        "range patterns require the runtime RangePatternView phase"
                    },
                ));
            }
        }

        let ordering_protocol = self
            .selected_comparison_protocol(true, ty, pattern.span)
            .map_err(|_| {
                error(
                    pattern.span,
                    "range pattern ordering protocol for this type is not implemented",
                )
            })?;
        self.native_or_ord_range_pattern(
            pattern,
            lower,
            upper,
            bound,
            ty,
            &shape,
            Some(ordering_protocol),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn view_range_pattern(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        lower: Option<&Expr<'_>>,
        upper: Option<&Expr<'_>>,
        bound: &hir::pattern::BoundType,
        ty: &Term,
        range_protocol: DeclarationId,
        comptime: bool,
        active_phase: n::TraitPhase,
    ) -> Result<n::Pattern, Error> {
        let range_end_id = self.protocols.range_end.ok_or_else(|| {
            error(
                pattern.span,
                "range pattern view requires a bound RangeEnd type",
            )
        })?;
        let contract = crate::range_patterns::for_phase(
            &self.checked_traits,
            range_protocol,
            Some(range_end_id),
            comptime,
        )
        .map_err(|message| error(pattern.span, message))?;
        if contract.comptime != comptime {
            return Err(error(
                pattern.span,
                if comptime {
                    "range patterns require the comptime RangePatternView phase"
                } else {
                    "range patterns require the runtime RangePatternView phase"
                },
            ));
        }

        let bound_ty =
            self.resolve_range_bound_type(ty, range_protocol, contract.bound, pattern.span)?;
        let range_end_ty = Term::Nominal {
            declaration: range_end_id,
            arguments: vec![bound_ty.clone()],
        };
        self.ensure_nominal_type(&range_end_ty, pattern.span)?;
        let (unbounded_idx, inclusive_idx, exclusive_idx) =
            crate::range_patterns::validate_range_end(&self.enums, range_end_id)
                .map_err(|message| error(pattern.span, message))?;

        let lower_expr = lower.map(|ep| self.expr(ep, Some(&bound_ty))).transpose()?;
        let upper_expr = upper.map(|ep| self.expr(ep, Some(&bound_ty))).transpose()?;
        let mut diverges = lower_expr.as_ref().is_some_and(|e| is_never(&e.ty))
            || upper_expr.as_ref().is_some_and(|e| is_never(&e.ty));

        let const_endpoints_ok = !diverges
            && lower_expr.as_ref().is_none_or(is_static_const_range_expr)
            && upper_expr.as_ref().is_none_or(is_static_const_range_expr);
        let is_stable = self.has_stable_range_view(ty, active_phase);

        let observed = self.fresh_range_binding(pattern.span)?;
        let subject_expr = n::Expr {
            ty: ty.clone(),
            source: source(pattern.span),
            kind: K::Local(observed),
        };
        let receiver = if contract.by_value {
            subject_expr
        } else {
            n::Expr {
                ty: Term::Pointer(Box::new(ty.clone())),
                source: source(pattern.span),
                kind: K::AddressOf(Box::new(subject_expr)),
            }
        };

        let make_end = |index: usize, field: Option<n::Expr>, end_span: Span| n::Expr {
            ty: range_end_ty.clone(),
            source: source(end_span),
            kind: K::Variant {
                declaration: range_end_id,
                index,
                fields: field.into_iter().collect(),
            },
        };

        let callee = self
            .selected_method(
                ty.clone(),
                Some(n::InterfaceInstance {
                    declaration: range_protocol,
                    arguments: vec![],
                }),
                contract.contains,
                pattern.span,
            )?
            .ok_or_else(|| error(pattern.span, "RangePatternView has no selected method"))?;
        let site = Expr {
            hir_id: pattern.hir_id,
            span: pattern.span,
            kind: H::Unit,
        };

        if const_endpoints_ok && is_stable {
            let start_arg = match lower_expr {
                None => make_end(unbounded_idx, None, pattern.span),
                Some(val) => {
                    let span = lower.map_or(pattern.span, |ep| ep.span);
                    make_end(inclusive_idx, Some(val), span)
                }
            };
            let end_arg = match upper_expr {
                None => make_end(unbounded_idx, None, pattern.span),
                Some(val) => {
                    let idx = match bound {
                        hir::pattern::BoundType::Inclusive => inclusive_idx,
                        hir::pattern::BoundType::Exclusive => exclusive_idx,
                    };
                    let span = upper.map_or(pattern.span, |ep| ep.span);
                    make_end(idx, Some(val), span)
                }
            };
            self.locals.insert(observed, ty.clone());
            let call_res = self.finish_call(
                &site,
                callee.clone(),
                &[],
                vec![receiver, start_arg.clone(), end_arg.clone()],
                Some(&p(P::Bool)),
                None,
            );
            self.locals.remove(&observed);
            let call_expr = call_res?;
            if !is_never(&call_expr.ty) {
                self.case_pattern_infos.insert(
                    observed,
                    crate::automata::CasePatternInfo {
                        declaration: range_protocol,
                        binding: observed,
                        callee,
                        extra_args: vec![start_arg, end_arg],
                        is_predicate: true,
                        payload_type: Term::Tuple(Vec::new()),
                        next_pattern: n::Pattern::Tuple(Vec::new()),
                    },
                );
            }
            return Ok(n::Pattern::Observe {
                binding: observed,
                inner: Box::new(n::Pattern::Then {
                    first: Box::new(n::Pattern::Wild),
                    value: Box::new(call_expr),
                    next: Box::new(n::Pattern::Literal(n::Literal::Bool(true))),
                }),
            });
        }

        let mut statements = Vec::new();
        let start_arg = match lower_expr {
            None => make_end(unbounded_idx, None, pattern.span),
            Some(endpoint) => {
                let end_span = lower.map_or(pattern.span, |ep| ep.span);
                let binding = self.fresh_range_binding(end_span)?;
                let end_ty = endpoint.ty.clone();
                statements.push(n::Expr {
                    ty: if is_never(&end_ty) {
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
                let val_expr = n::Expr {
                    ty: end_ty,
                    source: source(end_span),
                    kind: K::Local(binding),
                };
                make_end(inclusive_idx, Some(val_expr), end_span)
            }
        };
        let end_arg = match upper_expr {
            None => make_end(unbounded_idx, None, pattern.span),
            Some(endpoint) => {
                let end_span = upper.map_or(pattern.span, |ep| ep.span);
                let binding = self.fresh_range_binding(end_span)?;
                let end_ty = endpoint.ty.clone();
                statements.push(n::Expr {
                    ty: if is_never(&end_ty) {
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
                let idx = match bound {
                    hir::pattern::BoundType::Inclusive => inclusive_idx,
                    hir::pattern::BoundType::Exclusive => exclusive_idx,
                };
                let val_expr = n::Expr {
                    ty: end_ty,
                    source: source(end_span),
                    kind: K::Local(binding),
                };
                make_end(idx, Some(val_expr), end_span)
            }
        };
        self.locals.insert(observed, ty.clone());
        let call_res = self.finish_call(
            &site,
            callee,
            &[],
            vec![receiver, start_arg, end_arg],
            Some(&p(P::Bool)),
            None,
        );
        self.locals.remove(&observed);
        let call_expr = call_res?;
        diverges |= is_never(&call_expr.ty);
        Ok(n::Pattern::Observe {
            binding: observed,
            inner: Box::new(n::Pattern::Guard {
                pattern: Box::new(n::Pattern::Wild),
                condition: Box::new(n::Expr {
                    ty: p(if diverges { P::Never } else { P::Bool }),
                    source: source(pattern.span),
                    kind: K::Block {
                        statements,
                        result: Some(Box::new(call_expr)),
                    },
                }),
            }),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn native_or_ord_range_pattern(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        lower: Option<&Expr<'_>>,
        upper: Option<&Expr<'_>>,
        bound: &hir::pattern::BoundType,
        ty: &Term,
        shape: &Term,
        ordering_protocol: Option<DeclarationId>,
    ) -> Result<n::Pattern, Error> {
        let observed = self.fresh_range_binding(pattern.span)?;
        let lower_binding = self.fresh_range_binding(pattern.span)?;
        let upper_binding = self.fresh_range_binding(pattern.span)?;
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
                shape,
                Term::Primitive(P::Float | P::F16 | P::Bf16 | P::F32 | P::F64 | P::Never)
            );
        for (endpoint, binding, upper_end) in
            [(lower, lower_binding, false), (upper, upper_binding, true)]
        {
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
            let exclusive = upper_end && matches!(bound, hir::pattern::BoundType::Exclusive);
            let (left, right) = if upper_end {
                (local_expr(observed), local_expr(binding))
            } else {
                (local_expr(binding), local_expr(observed))
            };
            let check = if let Some(protocol) = ordering_protocol {
                let method = if exclusive { "lt" } else { "le" };
                self.finish_comparison_protocol_call(
                    endpoint_hir,
                    protocol,
                    method,
                    left,
                    right,
                    Some(&p(P::Bool)),
                )?
            } else {
                let kind = if *shape == p(P::Integer) {
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
            if *shape == p(P::Char) {
                let start = match lower_const {
                    None => pattern_automata::RangeBound::Unbounded,
                    Some(RangeEndpointLit::Char(c)) => pattern_automata::RangeBound::Inclusive(c),
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
                    Some(RangeEndpointLit::Int(i)) => pattern_automata::RangeBound::Inclusive(i),
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
}
