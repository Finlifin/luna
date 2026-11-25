//! Collect variadic positional tuples and named property records through
//! user-implemented `VariadicCollector`, `TryVariadicCollector`, and
//! `ExtendedVariadicCollector` traits.
use super::*;
use crate::ptc::Budget;

#[derive(Clone, Copy)]
struct CollectorSelection {
    protocol: DeclarationId,
    fallible: bool,
}

impl Checker<'_, '_> {
    pub(super) fn is_default_collection_shape(
        &self,
        ty: &Term,
        extended: bool,
        span: Span,
    ) -> Result<bool, Error> {
        let shape = self.structural_type(ty, span)?;
        Ok(if extended {
            matches!(shape, Term::Record(_))
        } else {
            matches!(shape, Term::Tuple(_))
        })
    }

    fn probe_collector_protocol(
        &self,
        ty: &Term,
        protocol: DeclarationId,
        required_phase: n::TraitPhase,
        span: Span,
    ) -> Result<Option<Result<(), String>>, Error> {
        let interface = n::InterfaceInstance {
            declaration: protocol,
            arguments: vec![],
        };
        let incomplete = match self.symbolic_interface_evidence(ty, &interface) {
            Ok(true) => return Ok(Some(Ok(()))),
            Ok(false) => None,
            Err(e @ nir::NominalAdmissionError::Unresolved(_)) => Some(e),
            Err(e) => return Err(error(span, e.to_string())),
        };
        let resolution = self
            .resolve_trait_capability(&interface, ty, Budget::default())
            .map_err(|reason| {
                error(
                    span,
                    format!("variadic collector capability query failed: {reason:?}"),
                )
            })?;
        match resolution.selection {
            crate::implementation::Selection::Selected(impl_id) => {
                let implementation =
                    &self.checked_implementations[self.implementation_positions[&impl_id]];
                if !implementation.phases.contains(required_phase) {
                    return Ok(Some(Err(format!(
                        "selected implementation lacks {required_phase} trait phase"
                    ))));
                }
                Ok(Some(Ok(())))
            }
            crate::implementation::Selection::NoApplicableCandidate => {
                if let Some(given_error) = incomplete {
                    return Ok(Some(Err(given_error.to_string())));
                }
                Ok(None)
            }
            other => {
                if let Some(given_error) = incomplete {
                    return Ok(Some(Err(given_error.to_string())));
                }
                Ok(Some(Err(format!(
                    "variadic collector capability is not established: {other}"
                ))))
            }
        }
    }

    fn select_collector_protocol(
        &self,
        ty: &Term,
        extended: bool,
        staged: bool,
        allow_try: bool,
        span: Span,
    ) -> Result<CollectorSelection, Error> {
        let required_phase = if staged {
            n::TraitPhase::Comptime
        } else {
            n::TraitPhase::Runtime
        };
        if extended {
            let Some(protocol) = self.protocols.extended_variadic_collector else {
                return Err(error(span, "property collection requires a record type"));
            };
            return match self.probe_collector_protocol(ty, protocol, required_phase, span)? {
                Some(Ok(())) => Ok(CollectorSelection {
                    protocol,
                    fallible: false,
                }),
                Some(Err(msg)) => Err(error(span, msg)),
                None => Err(error(
                    span,
                    "property collection requires a record type or ExtendedVariadicCollector implementation",
                )),
            };
        }

        let infallible = self
            .protocols
            .variadic_collector
            .map(|id| {
                self.probe_collector_protocol(ty, id, required_phase, span)
                    .map(|res| (id, res))
            })
            .transpose()?;
        let fallible = if allow_try {
            self.protocols
                .try_variadic_collector
                .map(|id| {
                    self.probe_collector_protocol(ty, id, required_phase, span)
                        .map(|res| (id, res))
                })
                .transpose()?
        } else {
            None
        };
        match (infallible, fallible) {
            (Some((inf_id, Some(Ok(())))), Some((_, Some(Ok(()))))) => Err(error(
                span,
                format!(
                    "ambiguous variadic collector for `{ty}`: implements both VariadicCollector and TryVariadicCollector (matched {inf_id})"
                ),
            )),
            (Some((_, Some(Err(msg)))), _) | (_, Some((_, Some(Err(msg))))) => {
                Err(error(span, msg))
            }
            (Some((id, Some(Ok(())))), _) => Ok(CollectorSelection {
                protocol: id,
                fallible: false,
            }),
            (_, Some((id, Some(Ok(()))))) => Ok(CollectorSelection {
                protocol: id,
                fallible: true,
            }),
            (None, None) => Err(error(
                span,
                "default variadic collection requires a tuple type",
            )),
            _ => Err(error(
                span,
                if allow_try {
                    "variadic collection requires a tuple type, VariadicCollector, or TryVariadicCollector implementation"
                } else {
                    "variadic collection requires a tuple type or VariadicCollector implementation"
                },
            )),
        }
    }

    pub(super) fn check_variadic_collection_domain(
        &self,
        ty: &Term,
        extended: bool,
        staged: bool,
        span: Span,
    ) -> Result<(), Error> {
        if self.is_default_collection_shape(ty, extended, span)? {
            return Ok(());
        }
        self.select_collector_protocol(ty, extended, staged, !extended, span)?;
        Ok(())
    }

    pub(super) fn resolve_collector_tmp_type(
        &mut self,
        protocol: DeclarationId,
        target_ty: &Term,
        packet_ty: &Term,
        span: Span,
    ) -> Result<Term, Error> {
        let trait_contract = self
            .checked_traits
            .by_id(protocol)
            .ok_or_else(|| error(span, "variadic collector trait contract is unavailable"))?;
        let tmp_member = trait_contract
            .associated
            .iter()
            .find(|item| item.name == "Tmp")
            .ok_or_else(|| error(span, "variadic collector lacks associated Tmp"))?;
        let projected = Term::AssociatedTypeCall(Box::new(nir::AssociatedTypeCall {
            origin: nir::AssociatedTypeOrigin {
                interface: protocol,
                member: tmp_member.declaration,
            },
            arguments: vec![],
            subject: target_ty.clone(),
            call_arguments: vec![packet_ty.clone()],
        }));
        let dummy = Expr {
            hir_id: hir::HirId::new(
                self.signature_owner.unwrap_or(OwnerId::INVALID),
                hir::ItemLocalId::new(0),
            ),
            span,
            kind: H::Unit,
        };
        let resolved = self.observe_call_type(&projected, &dummy)?;
        Ok(self.structural_type(&resolved, span).unwrap_or(resolved))
    }

    fn resolve_collector_err_type(
        &self,
        protocol: DeclarationId,
        target_ty: &Term,
        span: Span,
    ) -> Result<Term, Error> {
        let trait_contract = self
            .checked_traits
            .by_id(protocol)
            .ok_or_else(|| error(span, "try variadic collector trait contract is unavailable"))?;
        let err_member = trait_contract
            .associated
            .iter()
            .find(|item| item.name == "Err")
            .ok_or_else(|| error(span, "try variadic collector lacks associated Err"))?;
        let projected = Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: protocol,
                member: err_member.declaration,
            }),
            arguments: vec![],
            subject: Box::new(target_ty.clone()),
        };
        let assumptions = self.capability_assumptions();
        let resolved = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        }
        .associated_type_view_metered(&projected, &mut Budget::default())
        .map(|view| view.value)
        .unwrap_or(projected);
        Ok(self.structural_type(&resolved, span).unwrap_or(resolved))
    }

    fn synthesize_collector_method_call(
        &mut self,
        protocol: DeclarationId,
        method_name: &str,
        target_ty: &Term,
        packet_ty: &Term,
        arguments: Vec<n::Expr>,
        result_ty: Term,
        statements: Option<&mut Vec<n::Expr>>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let interface = n::InterfaceInstance {
            declaration: protocol,
            arguments: vec![],
        };
        let packet_arg = self.copy_alias_type(packet_ty, span)?;
        let (callee, parameter_bindings) = if let Some((body_owner, mut impl_args)) =
            self.concrete_trait_method_body(&interface, target_ty, method_name, span)?
            && self.open_functions.contains_key(&body_owner)
        {
            impl_args.push(packet_arg);
            self.specialize_open_trait_method_value(body_owner, impl_args, span)?
        } else {
            let mut callee = self
                .selected_method(target_ty.clone(), Some(interface), method_name, span)?
                .ok_or_else(|| error(span, format!("{method_name} method is unavailable")))?;
            let method_contract = match &callee.kind {
                K::TraitMethod { member, .. } => {
                    self.checked_traits.by_id(protocol).and_then(|interface| {
                        interface
                            .methods
                            .iter()
                            .find(|method| method.declaration == *member)
                    })
                }
                _ => None,
            };
            let parameter_bindings = method_contract
                .map(|method| method.parameter_bindings.clone())
                .unwrap_or_default();
            if let Some(method_param) =
                method_contract.and_then(|m| m.type_parameters.first().copied())
            {
                match &mut callee.kind {
                    K::TraitMethod {
                        method_arguments, ..
                    } => {
                        callee.ty = self.instantiate_input_types(
                            &callee.ty,
                            &[method_param],
                            std::slice::from_ref(&packet_arg),
                            span,
                        )?;
                        method_arguments.push(packet_arg);
                    }
                    K::FunctionInstance { arguments, .. } => {
                        callee.ty = self.instantiate_input_types(
                            &callee.ty,
                            &[method_param],
                            std::slice::from_ref(&packet_arg),
                            span,
                        )?;
                        arguments.push(packet_arg);
                    }
                    _ => {}
                }
            }
            (callee, parameter_bindings)
        };
        let arguments = arguments
            .into_iter()
            .enumerate()
            .map(|(parameter, value)| n::CallArgument {
                passing: n::ArgumentPassing::Value,
                parameter,
                value,
                is_default: false,
            })
            .collect();
        let call_source = if matches!(
            method_name,
            "variadic_init" | "extended_variadic_init" | "try_variadic_init"
        ) {
            n::SourceRange {
                start: span.lo().0,
                end: span.lo().0,
            }
        } else {
            n::SourceRange {
                start: span.hi().0,
                end: span.hi().0,
            }
        };
        let mut call = n::Expr {
            source: call_source,
            ty: result_ty,
            kind: K::Call {
                callee: Box::new(callee),
                arguments,
                parameter_bindings,
                env_arguments: vec![],
            },
        };
        if let Some(statements) = statements {
            self.attach_call_environment(statements, &mut call, span)?;
        } else if let K::Call { ref callee, .. } = call.kind
            && !self.call_environment_inputs(callee, span)?.is_empty()
        {
            return Err(error(
                span,
                format!("{method_name} cannot yet require an implicit runtime environment"),
            ));
        }
        Ok(call)
    }

    /// Lower a raw tuple or anonymous-record packet into `target_ty` via
    /// `VariadicCollector`, `TryVariadicCollector`, or `ExtendedVariadicCollector`.
    ///
    /// Runtime calls allocate a mutable caller-frame `tmp` binding inside
    /// `statements` so `tmp` stays pinned for the duration of the enclosing
    /// `CallPreparation` and is dropped at `Boundary::ScopeEnd` (including
    /// reverse-order cleanup on `TryVariadicCollector` error propagation).
    pub(super) fn lower_variadic_collector_packet(
        &mut self,
        statements: &mut Vec<n::Expr>,
        packet: n::Expr,
        target_ty: &Term,
        extended: bool,
        staged: bool,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let target_ty = self.copy_alias_type(target_ty, span)?;
        if self.is_default_collection_shape(&target_ty, extended, span)? {
            return Ok(packet);
        }
        let staged = staged || self.in_required_comptime || self.in_staged_input;
        let selection =
            self.select_collector_protocol(&target_ty, extended, staged, !extended, span)?;
        let protocol = selection.protocol;
        let packet_ty = packet.ty.clone();
        let collect_result_ty = if selection.fallible {
            let err_ty = self.resolve_collector_err_type(protocol, &target_ty, span)?;
            Term::error_result(vec![err_ty], target_ty.clone())
        } else {
            target_ty.clone()
        };

        if staged {
            let method_name = if extended {
                "comptime_extended_variadic_collect"
            } else if selection.fallible {
                "comptime_try_variadic_collect"
            } else {
                "comptime_variadic_collect"
            };
            let call = self.synthesize_collector_method_call(
                protocol,
                method_name,
                &target_ty,
                &packet_ty,
                vec![packet],
                collect_result_ty,
                None,
                span,
            )?;
            let collected = if selection.fallible {
                self.propagate_error_expr(call, Some(&target_ty), span)?
            } else {
                call
            };
            let collector_binding = self.spread_binding(span)?;
            let anchor = arguments::preparation_anchor(span);
            statements.push(n::Expr {
                ty: if is_never(&collected.ty) {
                    p(P::Never)
                } else {
                    p(P::Void)
                },
                source: anchor,
                kind: K::Let {
                    binding: collector_binding,
                    value: Box::new(collected),
                },
            });
            return Ok(n::Expr {
                ty: target_ty,
                source: anchor,
                kind: K::Local(collector_binding),
            });
        }

        let anchor = arguments::preparation_anchor(span);
        let packet_local = match packet.kind {
            K::Local(binding) => n::Expr {
                ty: packet_ty.clone(),
                source: anchor,
                kind: K::Local(binding),
            },
            _ => {
                let packet_binding = self.spread_binding(span)?;
                statements.push(n::Expr {
                    ty: if is_never(&packet.ty) {
                        p(P::Never)
                    } else {
                        p(P::Void)
                    },
                    source: anchor,
                    kind: K::Let {
                        binding: packet_binding,
                        value: Box::new(packet),
                    },
                });
                n::Expr {
                    ty: packet_ty.clone(),
                    source: anchor,
                    kind: K::Local(packet_binding),
                }
            }
        };

        let (init_name, collect_name) = if extended {
            ("extended_variadic_init", "extended_variadic_collect")
        } else if selection.fallible {
            ("try_variadic_init", "try_variadic_collect")
        } else {
            ("variadic_init", "variadic_collect")
        };
        let tmp_ty = self.resolve_collector_tmp_type(protocol, &target_ty, &packet_ty, span)?;
        let init_call = self.synthesize_collector_method_call(
            protocol,
            init_name,
            &target_ty,
            &packet_ty,
            vec![],
            tmp_ty.clone(),
            Some(statements),
            span,
        )?;
        let tmp_binding = self.spread_binding(span)?;
        self.locals.insert(tmp_binding, tmp_ty.clone());
        statements.push(n::Expr {
            ty: if is_never(&init_call.ty) {
                p(P::Never)
            } else {
                p(P::Void)
            },
            source: anchor,
            kind: K::Let {
                binding: tmp_binding,
                value: Box::new(init_call),
            },
        });

        let tmp_ref = n::Expr {
            ty: Term::Pointer(Box::new(tmp_ty)),
            source: anchor,
            kind: K::AddressOf(Box::new(n::Expr {
                ty: self.locals[&tmp_binding].clone(),
                source: anchor,
                kind: K::Local(tmp_binding),
            })),
        };
        let collect_call = self.synthesize_collector_method_call(
            protocol,
            collect_name,
            &target_ty,
            &packet_ty,
            vec![tmp_ref, packet_local],
            collect_result_ty,
            Some(statements),
            span,
        )?;
        let collected = if selection.fallible {
            let mut propagated = self.propagate_error_expr(collect_call, Some(&target_ty), span)?;
            // Error-propagation match inside call preparation must keep a
            // distinct cleanup source from the enclosing call expression.
            propagated.source = self.error_generated_source(span)?;
            propagated
        } else {
            collect_call
        };
        let collector_binding = self.spread_binding(span)?;
        statements.push(n::Expr {
            ty: if is_never(&collected.ty) {
                p(P::Never)
            } else {
                p(P::Void)
            },
            source: anchor,
            kind: K::Let {
                binding: collector_binding,
                value: Box::new(collected),
            },
        });
        Ok(n::Expr {
            ty: target_ty,
            source: anchor,
            kind: K::Local(collector_binding),
        })
    }

    /// Directly invoke `comptime_variadic_collect` without a `CallPreparation`
    /// wrapper for normal-form/clause inputs (`...Xs: Collector`) in comptime positions.
    pub(super) fn lower_comptime_variadic_nf(
        &mut self,
        packet: n::Expr,
        target_ty: &Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let target_ty = self.copy_alias_type(target_ty, span)?;
        let selection = self.select_collector_protocol(&target_ty, false, true, false, span)?;
        let packet_ty = packet.ty.clone();
        self.synthesize_collector_method_call(
            selection.protocol,
            "comptime_variadic_collect",
            &target_ty.clone(),
            &packet_ty,
            vec![packet],
            target_ty,
            None,
            span,
        )
    }

    pub(super) fn call_parameter_collection_target(
        &mut self,
        callee: &n::Expr,
        arguments: &[n::CallArgument],
        statements: &[n::Expr],
        parameter: usize,
        span: Span,
    ) -> Result<(Term, bool), Error> {
        let callable = callee.ty.callable_signature();
        let Some(signature) = dependent_inputs::CallSignature::of(callable) else {
            return Err(error(
                span,
                "variadic collection requires a callable signature",
            ));
        };
        let Some(param) = signature.parameters.get(parameter) else {
            return Err(error(
                span,
                "variadic collector parameter index out of range",
            ));
        };
        let staged = callee.ty.is_comptime_callable() || param.sort != Sort::Runtime;
        if param.ty.check_scope(&[]).is_ok() {
            let target_ty = self.copy_alias_type(&param.ty, span)?;
            return Ok((target_ty, staged));
        }
        let mut inputs = vec![None; signature.parameters.len()];
        for argument in arguments {
            if argument.parameter < inputs.len() {
                inputs[argument.parameter] =
                    resolve_prepared_comptime_term(&argument.value, statements);
            }
        }
        let target_ty = if callee.ty.is_comptime_callable() {
            self.with_call_type_fuel(|_, fuel| {
                param
                    .ty
                    .substitute_partial_comptime_bounded(&inputs, fuel)
                    .map_err(|failure| {
                        error(
                            span,
                            format!("dependent collector parameter is not available: {failure}"),
                        )
                    })
            })?
        } else {
            self.with_call_type_fuel(|checker, fuel| {
                let binding = checker.prepare_call_binding(callable, &inputs, span, fuel)?;
                binding
                    .substitute_bounded(&param.ty, fuel)
                    .map_err(|failure| {
                        error(
                            span,
                            format!("dependent collector parameter is not available: {failure}"),
                        )
                    })
            })?
        };
        let target_ty = self.copy_alias_type(&target_ty, span)?;
        Ok((target_ty, staged))
    }
}

fn resolve_prepared_comptime_term<'a>(
    mut expr: &'a n::Expr,
    statements: &'a [n::Expr],
) -> Option<Term> {
    for _ in 0..32 {
        match &expr.kind {
            K::TypeValue(term) | K::ComptimeValue(term) => return Some(term.clone()),
            K::Local(binding) => {
                let bound = statements.iter().rev().find_map(|stmt| match &stmt.kind {
                    K::Let { binding: b, value } if b == binding => Some(value.as_ref()),
                    _ => None,
                })?;
                expr = bound;
            }
            _ => return None,
        }
    }
    None
}
