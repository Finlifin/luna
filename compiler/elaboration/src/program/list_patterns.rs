//! List views elaborate to ordinary selected calls and transactional patterns.
//! No extractor is invoked to prove coverage or inspect a runtime subject.
use super::*;

impl Checker<'_, '_> {
    fn has_list_protocol_impl(
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

    pub(super) fn list_pattern(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        elements: &[hir::pattern::Pattern<'_>],
        rest: Option<&hir::pattern::Pattern<'_>>,
        ty: &Term,
    ) -> Result<n::Pattern, Error> {
        let comptime = !self.runtime_value_context() && !self.in_code_body;
        let active_phase = if comptime {
            n::TraitPhase::Comptime
        } else {
            n::TraitPhase::Runtime
        };
        let finite_any = self
            .protocols
            .list_pattern
            .is_some_and(|id| self.has_list_protocol_impl(id, ty, None))
            || (comptime
                && self
                    .protocols
                    .comptime_list_pattern
                    .is_some_and(|id| self.has_list_protocol_impl(id, ty, None)));
        let endless_any = self
            .protocols
            .list_pattern_endless
            .is_some_and(|id| self.has_list_protocol_impl(id, ty, None));
        if finite_any && endless_any {
            return Err(error(
                pattern.span,
                "ambiguous list pattern view: type implements both ListPatternView and ListPatternViewEndless",
            ));
        }

        let (selected, endless) = if let Some(endless_id) = self
            .protocols
            .list_pattern_endless
            .filter(|&id| self.has_list_protocol_impl(id, ty, Some(active_phase)))
        {
            (Some(endless_id), true)
        } else if comptime {
            let finite_staged_active = self.protocols.list_pattern.filter(|&id| {
                crate::list_patterns::has_staged_contract(&self.checked_traits, id)
                    && self.has_list_protocol_impl(id, ty, Some(n::TraitPhase::Comptime))
            });
            let comptime_active = self
                .protocols
                .comptime_list_pattern
                .filter(|&id| self.has_list_protocol_impl(id, ty, Some(n::TraitPhase::Comptime)));
            if let Some(id) = finite_staged_active {
                if comptime_active.is_some() {
                    return Err(error(
                        pattern.span,
                        "ambiguous list pattern view: type implements both ListPatternView and ComptimeListPatternView in comptime phase",
                    ));
                }
                (Some(id), false)
            } else if let Some(id) = comptime_active {
                (Some(id), false)
            } else if let Some(endless_id) = self
                .protocols
                .list_pattern_endless
                .filter(|&id| self.has_list_protocol_impl(id, ty, None))
            {
                (Some(endless_id), true)
            } else if let Some(lp) = self.protocols.list_pattern
                && !crate::list_patterns::has_staged_contract(&self.checked_traits, lp)
                && self.protocols.comptime_list_pattern.is_none()
            {
                (Some(lp), false)
            } else if let Some(lp) = self
                .protocols
                .list_pattern
                .filter(|&id| self.has_list_protocol_impl(id, ty, None))
            {
                (Some(lp), false)
            } else {
                (
                    self.protocols
                        .comptime_list_pattern
                        .or(self.protocols.list_pattern),
                    false,
                )
            }
        } else if let Some(endless_id) = self
            .protocols
            .list_pattern_endless
            .filter(|&id| self.has_list_protocol_impl(id, ty, None))
        {
            (Some(endless_id), true)
        } else if let Some(lp) = self.protocols.list_pattern {
            (Some(lp), false)
        } else {
            (self.protocols.list_pattern_endless, true)
        };

        let protocol = selected.ok_or_else(|| {
            error(
                pattern.span,
                "list patterns require a bound ListPatternView protocol",
            )
        })?;
        let contract = if endless {
            crate::list_patterns::endless_for_phase(&self.checked_traits, protocol, comptime)
        } else {
            crate::list_patterns::for_phase(&self.checked_traits, protocol, comptime)
        }
        .map_err(|message| error(pattern.span, message))?;
        if contract.comptime != comptime {
            return Err(error(
                pattern.span,
                if endless {
                    if comptime {
                        "list patterns require the comptime ListPatternViewEndless phase"
                    } else {
                        "list patterns require the runtime ListPatternViewEndless phase"
                    }
                } else if comptime {
                    "list patterns require the comptime ListPatternView phase"
                } else {
                    "list patterns require the runtime ListPatternView phase"
                },
            ));
        }
        if endless && rest.is_none() {
            return Err(error(
                pattern.span,
                "endless list pattern requires a trailing '..._' or '...rest'",
            ));
        }
        let size = contract.size.clone();
        let binding = n::LocalId {
            owner: u32::MAX,
            index: self.next_temporary_binding,
        };
        self.next_temporary_binding = self
            .next_temporary_binding
            .checked_add(1)
            .ok_or_else(|| error(pattern.span, "pattern temporary binding space exhausted"))?;
        self.locals.insert(binding, ty.clone());
        let subject = n::Expr {
            source: source(pattern.span),
            ty: ty.clone(),
            kind: K::Local(binding),
        };
        let receiver = if contract.by_value {
            subject
        } else if comptime {
            return Err(error(
                pattern.span,
                "comptime list patterns require value-reference self; replace *self with self",
            ));
        } else {
            n::Expr {
                source: source(pattern.span),
                ty: Term::Pointer(Box::new(ty.clone())),
                kind: K::AddressOf(Box::new(subject)),
            }
        };
        let call = |this: &mut Self,
                    location: &hir::pattern::Pattern<'_>,
                    call_span: Span,
                    name: &str,
                    index: Option<usize>| {
            let site = Expr {
                hir_id: location.hir_id,
                span: call_span,
                kind: H::Unit,
            };
            let method = this
                .selected_method(
                    ty.clone(),
                    Some(n::InterfaceInstance {
                        declaration: protocol,
                        arguments: vec![],
                    }),
                    name,
                    pattern.span,
                )?
                .ok_or_else(|| error(pattern.span, "ListPatternView has no selected method"))?;
            let mut inputs = vec![receiver.clone()];
            if let Some(index) = index {
                inputs.push(this.integer_proxy(
                    &index.to_string(),
                    false,
                    Some(&size),
                    call_span,
                )?);
            }
            this.finish_call(&site, method, &[], inputs, None, None)
        };
        let bridge_span = pattern.span.shrink_to_hi();
        let mut steps = if let Some(length_name) = contract.length {
            let len_call_span = pattern.span.shrink_to_lo();
            let len_cmp_span = if pattern.span.hi().0 > pattern.span.lo().0 + 1 {
                pattern
                    .span
                    .with_hi(rustc_span::BytePos(pattern.span.lo().0 + 1))
            } else {
                pattern.span
            };
            let arity = self.integer_proxy(
                &elements.len().to_string(),
                false,
                Some(&size),
                len_cmp_span,
            )?;
            let length = call(self, pattern, len_call_span, length_name, None)?;
            let (value, next) = if rest.is_some() {
                (
                    n::Expr {
                        source: source(len_cmp_span),
                        ty: p(P::Bool),
                        kind: if size == p(P::Integer) {
                            K::IntegerBinary {
                                op: n::IntegerOp::LessEqual,
                                left: Box::new(arity),
                                right: Box::new(length),
                            }
                        } else {
                            K::ScalarLessEqual(Box::new(arity), Box::new(length))
                        },
                    },
                    n::Pattern::Literal(n::Literal::Bool(true)),
                )
            } else {
                let K::Literal(arity) = arity.kind else {
                    unreachable!("validated list size literal")
                };
                (length, n::Pattern::Literal(arity))
            };
            vec![n::Pattern::Then {
                first: Box::new(n::Pattern::Wild),
                value: Box::new(value),
                next: Box::new(next),
            }]
        } else {
            Vec::new()
        };
        let mut prefix_patterns = Vec::with_capacity(elements.len());
        let mut first_element_ty = None;
        for (index, element) in elements.iter().enumerate() {
            let value = call(self, element, element.span, contract.element, Some(index))?;
            if first_element_ty.is_none() {
                first_element_ty = Some(value.ty.clone());
            }
            let next = self.pattern(element, &value.ty)?;
            prefix_patterns.push(next.clone());
            steps.push(n::Pattern::Then {
                first: Box::new(n::Pattern::Wild),
                value: Box::new(value),
                next: Box::new(next),
            });
        }
        let mut rest_pattern = None;
        let mut rest_type = None;
        let mut named_rest_used = false;
        if let Some(rest_pat) = rest {
            if matches!(rest_pat.kind, hir::pattern::PatternKind::Wild) {
                rest_pattern = Some(n::Pattern::Wild);
            } else {
                named_rest_used = true;
                let rest_name = contract.rest.ok_or_else(|| {
                    error(
                        rest_pat.span,
                        "list rest patterns require a slicing protocol",
                    )
                })?;
                if let Some(default_body) = contract.rest_default {
                    let resolution = self
                        .resolve_trait_capability(
                            &n::InterfaceInstance {
                                declaration: protocol,
                                arguments: vec![],
                            },
                            ty,
                            crate::ptc::Budget::default(),
                        )
                        .map_err(|e| {
                            error(rest_pat.span, format!("trait resolution failed: {e:?}"))
                        })?;
                    let overridden = matches!(
                        resolution.selection,
                        crate::implementation::Selection::Selected(selected_id)
                            if self
                                .implementation_positions
                                .get(&selected_id)
                                .and_then(|pos| self.checked_implementations.get(*pos))
                                .is_some_and(|implementation| {
                                    implementation
                                        .methods
                                        .iter()
                                        .any(|(method, body)| {
                                            method == rest_name && *body != default_body
                                        })
                                })
                    );
                    if !overridden {
                        return Err(error(
                            rest_pat.span,
                            if endless {
                                "type implements ListPatternViewEndless without overriding list_pattern_view_rest required for '...rest' binding"
                            } else {
                                "type implements ListPatternView without overriding list_pattern_view_rest required for '...rest' binding"
                            },
                        ));
                    }
                }
                let rest_value = call(
                    self,
                    rest_pat,
                    rest_pat.span,
                    rest_name,
                    Some(elements.len()),
                )?;
                rest_type = Some(rest_value.ty.clone());
                let rest_next = self.pattern(rest_pat, &rest_value.ty)?;
                rest_pattern = Some(rest_next.clone());
                steps.push(n::Pattern::Then {
                    first: Box::new(n::Pattern::Wild),
                    value: Box::new(rest_value),
                    next: Box::new(rest_next),
                });
            }
        }
        let element_type = self.resolve_list_element_type(
            ty,
            protocol,
            contract.element,
            first_element_ty,
            pattern.span,
        );
        let (mode, length_method, element_method, rest_method) =
            self.classify_list_observation(ty, protocol, &contract, comptime, named_rest_used);
        self.list_pattern_infos.insert(
            binding,
            crate::automata::ListPatternInfo {
                protocol,
                mode,
                endless,
                by_value: contract.by_value,
                size_type: size,
                element_type,
                rest_type,
                length_method,
                element_method,
                rest_method,
                prefix_patterns,
                rest_pattern,
            },
        );
        // Each generated step observes the alias, not its incoming subject.
        // A unit bridge can therefore balance the sequence without copying the
        // subject or changing the left-to-right short-circuiting evaluation.
        // User element patterns remain intact and retain their own input.
        while steps.len() > 1 {
            let mut balanced = Vec::with_capacity(steps.len().div_ceil(2));
            let mut pending = steps.into_iter();
            while let Some(first) = pending.next() {
                balanced.push(match pending.next() {
                    Some(next) => n::Pattern::Then {
                        first: Box::new(first),
                        value: Box::new(n::Expr {
                            source: source(bridge_span),
                            ty: p(P::Void),
                            kind: K::Unit,
                        }),
                        next: Box::new(next),
                    },
                    None => first,
                });
            }
            steps = balanced;
        }
        let result = steps.pop().unwrap_or(n::Pattern::Wild);
        Ok(n::Pattern::Observe {
            binding,
            inner: Box::new(result),
        })
    }

    fn resolve_list_element_type(
        &self,
        ty: &Term,
        protocol: DeclarationId,
        element_method_name: &str,
        first_element_ty: Option<Term>,
        span: Span,
    ) -> Term {
        if let Some(elem) = first_element_ty {
            return elem;
        }
        if let Some(elem) = self.list_element_type(ty) {
            return elem.clone();
        }
        let assoc_name = if element_method_name.ends_with("_comptime") {
            "ElementComptime"
        } else {
            "Element"
        };
        if let Some(interface) = self.checked_traits.by_id(protocol)
            && let Some(member) = interface
                .associated
                .iter()
                .find(|item| item.name == assoc_name)
        {
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
            return self.structural_type(&resolved, span).unwrap_or(resolved);
        }
        Term::Primitive(P::Void)
    }

    fn classify_list_observation(
        &self,
        ty: &Term,
        protocol: DeclarationId,
        contract: &crate::list_patterns::Protocol,
        comptime: bool,
        named_rest_used: bool,
    ) -> (
        Option<n::ListObservationMode>,
        Option<DeclarationId>,
        Option<DeclarationId>,
        Option<DeclarationId>,
    ) {
        let assumptions = self.capability_assumptions();
        let world = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        };
        let list_iface = n::InterfaceInstance {
            declaration: protocol,
            arguments: vec![],
        };
        let selected_impl = world
            .resolve_instance_metered(&list_iface, ty, &mut crate::ptc::Budget::default())
            .ok()
            .and_then(|res| {
                let crate::implementation::Selection::Selected(impl_id) = res.types.selection
                else {
                    return None;
                };
                if !matches!(
                    res.qualifications.get(&impl_id),
                    Some(crate::capabilities::Qualification::Proved(_))
                ) {
                    return None;
                }
                let pos = *self.implementation_positions.get(&impl_id)?;
                self.checked_implementations.get(pos)
            });

        let method_decl = |name: &str| -> Option<DeclarationId> {
            selected_impl.and_then(|imp| {
                imp.methods
                    .iter()
                    .find(|(method, _)| method == name)
                    .map(|(_, body)| *body)
            })
        };
        let length_method = contract.length.and_then(&method_decl);
        let element_method = method_decl(contract.element);
        let rest_method = contract.rest.and_then(|name| {
            let decl = method_decl(name)?;
            if contract.rest_default == Some(decl) {
                None
            } else {
                Some(decl)
            }
        });

        if comptime {
            return (
                Some(n::ListObservationMode::Comptime),
                length_method,
                element_method,
                rest_method,
            );
        }

        let stable_id = if contract.endless {
            self.protocols.stable_list_pattern_endless
        } else {
            self.protocols.stable_list_pattern
        };
        let stable = if let Some(stable_id) = stable_id {
            let stable_iface = n::InterfaceInstance {
                declaration: stable_id,
                arguments: vec![],
            };
            self.symbolic_interface_evidence(ty, &stable_iface).ok() == Some(true)
                || world
                    .resolve_instance_metered(&stable_iface, ty, &mut crate::ptc::Budget::default())
                    .ok()
                    .is_some_and(|res| {
                        matches!(
                            res.types.selection,
                            crate::implementation::Selection::Selected(impl_id)
                                if matches!(
                                    res.qualifications.get(&impl_id),
                                    Some(crate::capabilities::Qualification::Proved(_))
                                )
                        )
                    })
        } else {
            false
        };
        if stable {
            return (
                Some(n::ListObservationMode::StableProtocol),
                length_method,
                element_method,
                rest_method,
            );
        }

        let trait_contract = self.checked_traits.by_id(protocol);
        let method_pure = |name: &str, impl_decl: Option<DeclarationId>| -> bool {
            if trait_contract.is_some_and(|t| t.methods.iter().any(|m| m.name == name && m.pure)) {
                return true;
            }
            let Some(body_id) = impl_decl else {
                return false;
            };
            self.source_declarations
                .owner(body_id)
                .and_then(|owner| self.package.item(owner))
                .is_some_and(
                    |item| matches!(&item.kind, ItemKind::Fn(sig, _) if sig.modifiers.is_pure),
                )
        };
        let length_pure = contract
            .length
            .is_none_or(|length_name| method_pure(length_name, length_method));
        let rest_pure = if named_rest_used {
            contract
                .rest
                .is_some_and(|rest_name| method_pure(rest_name, rest_method))
        } else {
            true
        };
        let mode = (length_pure && method_pure(contract.element, element_method) && rest_pure)
            .then_some(n::ListObservationMode::PureMethods);

        (mode, length_method, element_method, rest_method)
    }
}
