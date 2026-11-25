//! Collect variadic positional tuples and named property records through
//! user-implemented `VariadicCollector` and `ExtendedVariadicCollector` traits.
use super::*;
use crate::ptc::Budget;

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
        let Some(protocol) = (if extended {
            self.protocols.extended_variadic_collector
        } else {
            self.protocols.variadic_collector
        }) else {
            return Err(error(
                span,
                if extended {
                    "property collection requires a record type"
                } else {
                    "default variadic collection requires a tuple type"
                },
            ));
        };
        let interface = n::InterfaceInstance {
            declaration: protocol,
            arguments: vec![],
        };
        let required_phase = if staged {
            n::TraitPhase::Comptime
        } else {
            n::TraitPhase::Runtime
        };
        let incomplete = match self.symbolic_interface_evidence(ty, &interface) {
            Ok(true) => return Ok(()),
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
                    return Err(error(
                        span,
                        format!("selected implementation lacks {required_phase} trait phase"),
                    ));
                }
                Ok(())
            }
            crate::implementation::Selection::NoApplicableCandidate => {
                if let Some(given_error) = incomplete {
                    return Err(error(span, given_error.to_string()));
                }
                Err(error(
                    span,
                    if extended {
                        "property collection requires a record type or ExtendedVariadicCollector implementation"
                    } else {
                        "variadic collection requires a tuple type or VariadicCollector implementation"
                    },
                ))
            }
            other => {
                if let Some(given_error) = incomplete {
                    return Err(error(span, given_error.to_string()));
                }
                Err(error(
                    span,
                    format!("variadic collector capability is not established: {other}"),
                ))
            }
        }
    }

    pub(super) fn resolve_collector_tmp_type(
        &self,
        protocol: DeclarationId,
        target_ty: &Term,
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
        let projected = Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: protocol,
                member: tmp_member.declaration,
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
        if !self.call_environment_inputs(&callee, span)?.is_empty() {
            return Err(error(
                span,
                format!("{method_name} cannot yet require an implicit runtime environment"),
            ));
        }
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
        let call_source = if matches!(method_name, "variadic_init" | "extended_variadic_init") {
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
        Ok(n::Expr {
            source: call_source,
            ty: result_ty,
            kind: K::Call {
                callee: Box::new(callee),
                arguments,
                parameter_bindings,
                env_arguments: vec![],
            },
        })
    }

    /// Lower a raw tuple or anonymous-record packet into `target_ty` via
    /// `VariadicCollector` or `ExtendedVariadicCollector`.
    ///
    /// Runtime calls allocate a mutable caller-frame `tmp` binding inside
    /// `statements` so `tmp` stays pinned for the duration of the enclosing
    /// `CallPreparation` and is dropped at `Boundary::ScopeEnd`.
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
        self.check_variadic_collection_domain(&target_ty, extended, staged, span)?;
        let protocol = (if extended {
            self.protocols.extended_variadic_collector
        } else {
            self.protocols.variadic_collector
        })
        .ok_or_else(|| error(span, "variadic collector protocol binding is unavailable"))?;

        let packet_ty = packet.ty.clone();

        if staged {
            let method_name = if extended {
                "comptime_extended_variadic_collect"
            } else {
                "comptime_variadic_collect"
            };
            let call = self.synthesize_collector_method_call(
                protocol,
                method_name,
                &target_ty,
                &packet_ty,
                vec![packet],
                target_ty.clone(),
                span,
            )?;
            let collector_binding = self.spread_binding(span)?;
            let anchor = arguments::preparation_anchor(span);
            statements.push(n::Expr {
                ty: if is_never(&call.ty) {
                    p(P::Never)
                } else {
                    p(P::Void)
                },
                source: anchor,
                kind: K::Let {
                    binding: collector_binding,
                    value: Box::new(call),
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
        } else {
            ("variadic_init", "variadic_collect")
        };
        let tmp_ty = self.resolve_collector_tmp_type(protocol, &target_ty, span)?;
        let init_call = self.synthesize_collector_method_call(
            protocol,
            init_name,
            &target_ty,
            &packet_ty,
            vec![],
            tmp_ty.clone(),
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
            target_ty.clone(),
            span,
        )?;
        let collector_binding = self.spread_binding(span)?;
        statements.push(n::Expr {
            ty: if is_never(&collect_call.ty) {
                p(P::Never)
            } else {
                p(P::Void)
            },
            source: anchor,
            kind: K::Let {
                binding: collector_binding,
                value: Box::new(collect_call),
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
        self.check_variadic_collection_domain(&target_ty, false, true, span)?;
        let protocol = self
            .protocols
            .variadic_collector
            .ok_or_else(|| error(span, "VariadicCollector protocol binding is unavailable"))?;
        let packet_ty = packet.ty.clone();
        self.synthesize_collector_method_call(
            protocol,
            "comptime_variadic_collect",
            &target_ty.clone(),
            &packet_ty,
            vec![packet],
            target_ty,
            span,
        )
    }

    pub(super) fn call_parameter_collection_target(
        &mut self,
        callee: &n::Expr,
        arguments: &[n::CallArgument],
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
        let mut inputs = vec![None; signature.parameters.len()];
        for argument in arguments {
            if argument.parameter < inputs.len() {
                inputs[argument.parameter] = match &argument.value.kind {
                    K::TypeValue(term) | K::ComptimeValue(term) => Some(term.clone()),
                    _ => None,
                };
            }
        }
        let target_ty = self.with_call_type_fuel(|checker, fuel| {
            let binding = checker.prepare_call_binding(callable, &inputs, span, fuel)?;
            binding
                .substitute_bounded(&param.ty, fuel)
                .map_err(|failure| {
                    error(
                        span,
                        format!("dependent collector parameter is not available: {failure}"),
                    )
                })
        })?;
        let target_ty = self.copy_alias_type(&target_ty, span)?;
        Ok((target_ty, staged))
    }
}
