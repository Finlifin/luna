//! Dynamic trait object types (`dyn Trait`), encapsulation (`expr.dyn(Trait)`),
//! and dynamic method dispatch (`receiver.method(...)`) elaboration.
use super::*;
use nir::code_interfaces::MethodSignature;

pub(in crate::program) fn is_dyn_dispatchable_contract(method: &n::MethodContract) -> bool {
    method.phase == n::TraitPhase::Runtime && !method.is_quote && method.type_parameters.is_empty()
}

pub(in crate::program) struct DynReceiverInfo {
    pub(in crate::program) full_dyn_ty: Term,
    pub(in crate::program) interface: DeclarationId,
    pub(in crate::program) arguments: Vec<Term>,
}

impl Checker<'_, '_> {
    pub(in crate::program) fn validate_dyn_trait_contract(
        &self,
        instance: &n::InterfaceInstance,
        span: Span,
    ) -> Result<&n::Trait, Error> {
        let mut visited = std::collections::BTreeSet::new();
        let mut stack = vec![instance.declaration];
        while let Some(decl_id) = stack.pop() {
            if !visited.insert(decl_id) {
                continue;
            }
            let current = self
                .checked_traits
                .by_id(decl_id)
                .ok_or_else(|| error(span, "dyn Trait requires a checked trait declaration"))?;
            if current.comptime_only {
                return Err(error(
                    span,
                    "pure comptime trait cannot form a runtime dyn Trait interface",
                ));
            }
            if !current.associated.is_empty() {
                return Err(error(
                    span,
                    "dyn Trait cannot contain unfixed associated items",
                ));
            }
            for parent in &current.parents {
                stack.push(parent.declaration);
            }
        }
        self.checked_traits
            .by_id(instance.declaration)
            .ok_or_else(|| error(span, "dyn Trait requires a checked trait declaration"))
    }

    pub(in crate::program) fn dyn_trait_type_term(
        &mut self,
        trait_expr: &Expr<'_>,
        span: Span,
    ) -> Result<Term, Error> {
        let instance = self.interface_expr_optional_subject(trait_expr, None)?;
        self.validate_dyn_trait_contract(&instance, span)?;
        Ok(Term::TraitObject {
            interface: instance.declaration,
            arguments: instance.arguments,
        })
    }

    pub(in crate::program) fn dyn_trait_type_expr(
        &mut self,
        expr: &Expr<'_>,
        trait_expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<Option<n::Expr>, Error> {
        let value = self.dyn_trait_type_term(trait_expr, expr.span)?;
        let ty = p(P::Type);
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(Some(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::TypeValue(value),
        }))
    }

    pub(in crate::program) fn extract_dyn_trait_info(
        &self,
        ty: &Term,
        span: Span,
    ) -> Result<Option<DynReceiverInfo>, Error> {
        let mut current = ty.clone();
        for _ in 0..=128 {
            let shape = self.structural_type(&current, span)?;
            match &shape {
                Term::Pointer(inner) => {
                    current = (**inner).clone();
                }
                Term::TraitObject {
                    interface,
                    arguments,
                } => {
                    return Ok(Some(DynReceiverInfo {
                        full_dyn_ty: current,
                        interface: *interface,
                        arguments: arguments.clone(),
                    }));
                }
                Term::Closure { signature, .. } | Term::ContextualClosure { signature, .. } => {
                    if let Term::TraitObject {
                        interface,
                        arguments,
                    } = signature.as_ref()
                    {
                        return Ok(Some(DynReceiverInfo {
                            full_dyn_ty: current,
                            interface: *interface,
                            arguments: arguments.clone(),
                        }));
                    }
                    return Ok(None);
                }
                _ => return Ok(None),
            }
        }
        Err(error(span, "pointer dereference depth exhausted"))
    }

    pub(in crate::program) fn dyn_cast_expression(
        &mut self,
        expr: &Expr<'_>,
        value_expr: &Expr<'_>,
        trait_expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let value = self.expr(value_expr, None)?;
        let value_shape = self.structural_type(&value.ty, expr.span)?;
        let (by_pointer, subject) = match value_shape {
            Term::Pointer(pointee) => (true, (*pointee).clone()),
            _ => (false, value.ty.clone()),
        };

        let instance = self.interface_expr_optional_subject(trait_expr, Some(&subject))?;
        let contract = self
            .validate_dyn_trait_contract(&instance, expr.span)?
            .clone();

        self.establish_type_requirement(
            &n::TypePredicate::Trait(n::TraitRequirement {
                subject: subject.clone(),
                interface: instance.declaration,
                interface_arguments: instance.arguments.clone(),
            }),
            expr.span,
        )?;

        if by_pointer
            && contract
                .methods
                .iter()
                .filter(|method| is_dyn_dispatchable_contract(method))
                .any(|method| method.by_value_receiver)
        {
            let copy_ok = self.copy_read_input(&subject, expr.span)
                || self.protocols.copy.is_some_and(|copy_id| {
                    self.establish_type_requirement(
                        &n::TypePredicate::Trait(n::TraitRequirement {
                            subject: subject.clone(),
                            interface: copy_id,
                            interface_arguments: Vec::new(),
                        }),
                        expr.span,
                    )
                    .is_ok()
                });
            if !copy_ok {
                return Err(error(
                    expr.span,
                    "pointer dynamic view requires Copy when the trait has by-value self methods; encapsulate the value instead",
                ));
            }
        }

        let (env_arguments, env_identities) = self
            .collect_dyn_cast_environment(&contract, &instance, &subject, by_pointer, expr.span)?;

        let trait_obj = Term::TraitObject {
            interface: instance.declaration,
            arguments: instance.arguments.clone(),
        };
        let ty = if env_arguments.is_empty() {
            trait_obj
        } else {
            Term::Closure {
                environment: env_arguments.iter().map(|arg| arg.ty.clone()).collect(),
                signature: Box::new(trait_obj),
            }
        };

        if let Some(expected) = expected {
            if !env_arguments.is_empty()
                && self
                    .structural_type(expected, expr.span)
                    .is_ok_and(|shape| matches!(shape, Term::TraitObject { .. }))
            {
                return Err(error(
                    expr.span,
                    "cannot silently drop non-empty runtime environment when converting %c dyn Trait to bare dyn Trait",
                ));
            }
            self.require_expression(&ty, expected, expr.span)?;
        }

        let result_ty = if is_never(&value.ty) { p(P::Never) } else { ty };

        Ok(n::Expr {
            source: source(expr.span),
            ty: result_ty,
            kind: K::DynCast(Box::new(n::DynCast {
                interface: instance,
                subject,
                by_pointer,
                value: Box::new(value),
                env_arguments,
                env_identities,
            })),
        })
    }

    fn collect_dyn_cast_environment(
        &mut self,
        contract: &n::Trait,
        instance: &n::InterfaceInstance,
        subject: &Term,
        by_pointer: bool,
        span: Span,
    ) -> Result<(Vec<n::Expr>, Vec<Term>), Error> {
        let mut raw_inputs = Vec::new();
        for method in &contract.methods {
            if !is_dyn_dispatchable_contract(method) {
                continue;
            }
            let callee = n::Expr {
                source: source(span),
                ty: p(P::Void),
                kind: K::TraitMethod {
                    interface: instance.declaration,
                    interface_arguments: instance.arguments.clone(),
                    member: method.declaration,
                    subject: subject.clone(),
                    method_arguments: Vec::new(),
                },
            };
            raw_inputs.extend(self.call_environment_inputs(&callee, span)?);
        }

        let needs_owned_drop = !by_pointer
            || contract.methods.iter().any(|method| {
                if !is_dyn_dispatchable_contract(method) {
                    return false;
                }
                if let MethodSignature::Callable(sig) = &method.signature
                    && let Term::Function { result, .. } = sig.callable_signature()
                {
                    matches!(
                        result.as_ref(),
                        Term::Rigid { declaration, sort: Sort::Type }
                            if *declaration == contract.self_parameter
                    )
                } else {
                    false
                }
            });

        if needs_owned_drop && let Some(drop_id) = self.protocols.drop {
            if let Some(drop_trait) = self.checked_traits.by_id(drop_id)
                && let Some(drop_method) = drop_trait.methods.first()
            {
                let callee = n::Expr {
                    source: source(span),
                    ty: p(P::Void),
                    kind: K::TraitMethod {
                        interface: drop_id,
                        interface_arguments: Vec::new(),
                        member: drop_method.declaration,
                        subject: subject.clone(),
                        method_arguments: Vec::new(),
                    },
                };
                raw_inputs.extend(self.call_environment_inputs(&callee, span)?);
            }
            if self.runtime_domains.len() != self.global_runtime_deliveries.len() {
                let mut type_roots = Vec::with_capacity(1 + instance.arguments.len());
                type_roots.push(subject.clone());
                type_roots.extend(instance.arguments.iter().cloned());
                let dependencies = nominal_fields::metered(&mut self.required_steps, |fuel| {
                    nir::type_computations::runtime_dependencies_bounded(&type_roots, fuel)
                })
                .map_err(|reason| error(span, reason.to_string()))?;
                if !dependencies.may_change && !dependencies.ids.is_empty() {
                    let implicit = n::implicit_runtime_env_ids(
                        &self.runtime_domains,
                        &self.global_runtime_deliveries,
                        dependencies.ids,
                        std::iter::empty(),
                    )
                    .map_err(|reason| error(span, reason))?;
                    for id in implicit {
                        let term = Term::Runtime(id);
                        let domain = self.runtime_root_payload(&term, span)?;
                        raw_inputs.push((term, domain));
                    }
                }
            }
        }

        let mut seen: Vec<Term> = Vec::new();
        let mut env_arguments = Vec::new();
        let mut env_identities = Vec::new();
        for (identity, domain) in raw_inputs {
            if self.is_global_runtime_identity(&identity) {
                continue;
            }
            let copy =
                declaration_inputs::copy_input_domain(&identity, &mut self.required_steps, span)?;
            if seen.contains(&copy) {
                continue;
            }
            seen.push(copy.clone());
            let operand = if let Term::Runtime(id) = &copy
                && let Some(local) = self.default_result_local(*id, span)
            {
                self.require_expression(&local.ty, &domain, span)?;
                local
            } else {
                self.deliver_runtime_dependency(&copy, &domain, span)?
            };
            env_identities.push(copy);
            env_arguments.push(operand);
        }

        Ok((env_arguments, env_identities))
    }

    fn is_global_runtime_identity(&self, term: &Term) -> bool {
        let mut current = term;
        for _ in 0..=128 {
            match current {
                Term::Runtime(id) => return self.global_runtime_deliveries.contains_key(id),
                Term::RuntimeField { base, .. } => current = base,
                _ => return false,
            }
        }
        false
    }

    pub(in crate::program) fn try_dyn_receiver_call(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<Option<n::Expr>, Error> {
        if !self.interfaces_ready || self.checked_traits.is_empty() {
            return Ok(None);
        }
        let (view, name, has_clause_args) = match callee.kind {
            H::NFApplication(head, clause_args) => match head.kind {
                H::Projection(view, name) => (view, name, !clause_args.is_empty()),
                _ => return Ok(None),
            },
            H::Projection(view, name) => (view, name, false),
            _ => return Ok(None),
        };
        if Self::is_computed_method_subject(view) && matches!(view.kind, H::NFApplication(..)) {
            return Ok(None);
        }
        if self
            .computed_type_receiver_domain(view, callee.hir_id)?
            .is_some()
            || self.is_static_method_subject(view)
            || matches!(view.kind, H::Cast(..))
        {
            return Ok(None);
        }
        if let H::ResolvedDef { def_id, .. } = view.kind {
            let is_const = self
                .package
                .definition_owner(def_id)
                .and_then(|owner| self.package.item(owner))
                .is_some_and(|item| matches!(item.kind, ItemKind::Const(..)));
            if !is_const {
                return Ok(None);
            }
        }

        // Fast O(1) rejection for non-dyn local/param receivers without touching state.
        if let H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) = view.kind
            && let Some(ty) = self.locals.get(&local(id))
            && self.extract_dyn_trait_info(ty, view.span)?.is_none()
        {
            return Ok(None);
        }

        let probe_attempt = checking_attempts::Attempt::begin(
            self,
            view.span,
            self.numeric_defaults,
            checking_attempts::Disposition::Probe,
        )?;
        let probe =
            (|| -> Result<Option<(DynReceiverInfo, n::Trait, n::MethodContract)>, Error> {
                let operand = if let Some(pointer) = self.dereferenced_place(view)? {
                    let Term::Pointer(ty) = &pointer.ty else {
                        unreachable!()
                    };
                    splicing::Operand::Value(n::Expr {
                        ty: (**ty).clone(),
                        source: source(view.span),
                        kind: K::ReadPointer(Box::new(pointer)),
                    })
                } else {
                    self.field_operand(view)?
                };
                let receiver = self.deliver_operand(view, operand, None)?;
                let Some(dyn_info) = self.extract_dyn_trait_info(&receiver.ty, view.span)? else {
                    return Ok(None);
                };
                let Some(trait_contract) = self.checked_traits.by_id(dyn_info.interface).cloned()
                else {
                    return Ok(None);
                };
                let Some(method_contract) = trait_contract
                    .methods
                    .iter()
                    .find(|m| m.name == name.name.as_str())
                    .cloned()
                else {
                    return Ok(None);
                };
                Ok(Some((dyn_info, trait_contract, method_contract)))
            })();
        let Some((dyn_info, trait_contract, method_contract)) =
            (match probe_attempt.finish_checked(self, probe) {
                Ok(Some(found)) => Some(found),
                Ok(None) | Err(_) => return Ok(None),
            })
        else {
            unreachable!()
        };

        let receiver_operand = if let Some(pointer) = self.dereferenced_place(view)? {
            let Term::Pointer(ty) = &pointer.ty else {
                unreachable!()
            };
            splicing::Operand::Value(n::Expr {
                ty: (**ty).clone(),
                source: source(view.span),
                kind: K::ReadPointer(Box::new(pointer)),
            })
        } else {
            self.field_operand(view)?
        };
        let mut receiver = self.deliver_operand(view, receiver_operand, None)?;

        if has_clause_args || !method_contract.type_parameters.is_empty() {
            return Err(error(
                expr.span,
                "cannot dynamically dispatch trait method with uninstantiated clause parameters on dyn Trait",
            ));
        }
        if method_contract.phase != n::TraitPhase::Runtime || method_contract.is_quote {
            return Err(error(
                expr.span,
                "cannot dynamically dispatch comptime-only or quote trait method on dyn Trait",
            ));
        }
        if !method_contract.by_value_receiver && !method_contract.by_reference_receiver {
            return Err(error(
                expr.span,
                "dynamic dispatch requires a self or *self receiver method",
            ));
        }

        let sig = method_contract
            .signature
            .ordinary()
            .map_err(|e| error(expr.span, e.to_string()))?;
        let instantiated_sig = if trait_contract.type_parameters.is_empty() {
            sig.clone()
        } else {
            self.instantiate_input_types(
                sig,
                &trait_contract.type_parameters,
                &dyn_info.arguments,
                expr.span,
            )?
        };
        let Term::Function { parameters, result } = instantiated_sig.callable_signature() else {
            return Err(error(
                expr.span,
                "trait method must have a function signature",
            ));
        };
        if parameters.is_empty() {
            return Err(error(
                expr.span,
                "dynamic trait method requires receiver parameter",
            ));
        }

        let self_id = trait_contract.self_parameter;
        for param in parameters.iter().skip(1) {
            let mentions_self = nominal_fields::metered(&mut self.required_steps, |fuel| {
                nir::type_computations::mentions_rigid_bounded(&param.ty, fuel, |id| id == self_id)
            })
            .map_err(|e| error(expr.span, e.to_string()))?;
            if mentions_self {
                return Err(error(
                    expr.span,
                    "cannot call dyn Trait method whose non-receiver parameter mentions Self",
                ));
            }
        }

        let mentions_self_result = nominal_fields::metered(&mut self.required_steps, |fuel| {
            nir::type_computations::mentions_rigid_bounded(result, fuel, |id| id == self_id)
        })
        .map_err(|e| error(expr.span, e.to_string()))?;
        let call_result_ty = if mentions_self_result {
            if matches!(
                result.as_ref(),
                Term::Rigid { declaration, sort: Sort::Type } if *declaration == self_id
            ) {
                dyn_info.full_dyn_ty.clone()
            } else {
                return Err(error(
                    expr.span,
                    "cannot dynamically dispatch method returning compound Self type",
                ));
            }
        } else {
            (**result).clone()
        };

        // Elaborate explicit arguments (parameters[1..]).
        let explicit_params = &parameters[1..];
        let mut call_arguments = Vec::with_capacity(args.len());
        if args.len() > explicit_params.len() {
            return Err(error(
                expr.span,
                format!(
                    "expected at most {} arguments, found {}",
                    explicit_params.len(),
                    args.len()
                ),
            ));
        }
        let mut provided = vec![false; explicit_params.len()];
        let mut next_positional = 0usize;
        for arg in args {
            let (slot, arg_expr) = match arg {
                Arg::Positional(e) => {
                    while next_positional < provided.len() && provided[next_positional] {
                        next_positional += 1;
                    }
                    if next_positional >= explicit_params.len() {
                        return Err(error(e.span, "too many positional arguments"));
                    }
                    let slot = next_positional;
                    provided[slot] = true;
                    next_positional += 1;
                    (slot, *e)
                }
                Arg::Named(ident, e) => {
                    let Some(owner) = self.source_declarations.owner(method_contract.declaration)
                    else {
                        return Err(error(ident.span, "missing trait method parameter names"));
                    };
                    let item = self
                        .package
                        .item(owner)
                        .ok_or_else(|| error(ident.span, "missing trait method item"))?;
                    let (ItemKind::Fn(fn_sig, _) | ItemKind::FnDeclaration(fn_sig)) = &item.kind
                    else {
                        return Err(error(ident.span, "missing trait method signature"));
                    };
                    let Some(idx) = fn_sig
                        .params
                        .iter()
                        .skip(1)
                        .position(|p| p.name.name == ident.name)
                    else {
                        return Err(error(ident.span, "unknown named argument"));
                    };
                    if provided[idx] {
                        return Err(error(ident.span, "duplicate named argument"));
                    }
                    provided[idx] = true;
                    (idx, *e)
                }
                Arg::Expand(_) | Arg::Implicit(_) | Arg::PostLambda(_) => {
                    return Err(error(
                        expr.span,
                        "spread or implicit arguments are not supported on dyn Trait calls",
                    ));
                }
            };
            let param_ty = &explicit_params[slot].ty;
            let value = self.expr(arg_expr, Some(param_ty))?;
            call_arguments.push(n::CallArgument {
                passing: Default::default(),
                parameter: 1 + slot,
                value,
                is_default: false,
            });
        }
        if provided.iter().any(|&p| !p) {
            return Err(error(
                expr.span,
                "missing required arguments for dyn Trait method call",
            ));
        }

        // Normalize receiver for `*self` vs `self`.
        let mut temp_prefix: Option<Vec<n::Expr>> = None;
        if method_contract.by_value_receiver && !method_contract.by_reference_receiver {
            if let K::ReadPointer(_) = receiver.kind {
                return Err(error(
                    view.span,
                    "cannot move dyn Trait value out of pointer for by-value self method",
                ));
            }
            if matches!(
                self.structural_type(&receiver.ty, view.span)?,
                Term::Pointer(_)
            ) {
                return Err(error(
                    view.span,
                    "cannot call by-value self method on a pointer to non-Copy dyn Trait",
                ));
            }
        } else {
            // Method takes `*self`: supply `*dyn_ty` so affine `dyn Trait` is borrowed, not moved!
            loop {
                match &receiver.kind {
                    K::ReadPointer(ptr)
                        if matches!(
                            self.structural_type(&ptr.ty, view.span)?,
                            Term::Pointer(inner) if !matches!(self.structural_type(&inner, view.span)?, Term::Pointer(_))
                        ) =>
                    {
                        receiver = (**ptr).clone();
                        break;
                    }
                    _ => {}
                }
                if let Term::Pointer(inner) = self.structural_type(&receiver.ty, view.span)? {
                    if !matches!(self.structural_type(&inner, view.span)?, Term::Pointer(_)) {
                        break;
                    }
                    receiver = n::Expr {
                        ty: (*inner).clone(),
                        source: source(view.span),
                        kind: K::ReadPointer(Box::new(receiver)),
                    };
                    continue;
                }
                let ptr_ty = Term::Pointer(Box::new(receiver.ty.clone()));
                let addr = n::Expr {
                    ty: ptr_ty.clone(),
                    source: source(view.span),
                    kind: K::AddressOf(Box::new(receiver.clone())),
                };
                if n::pointer_operation_error_in(&addr, self.target).is_none() {
                    receiver = addr;
                } else {
                    let binding = self.spread_binding(view.span)?;
                    let dyn_ty = receiver.ty.clone();
                    self.locals.insert(binding, dyn_ty.clone());
                    let let_stmt = n::Expr {
                        source: receiver.source,
                        ty: if is_never(&dyn_ty) {
                            p(P::Never)
                        } else {
                            p(P::Void)
                        },
                        kind: K::Let {
                            binding,
                            value: Box::new(receiver),
                        },
                    };
                    temp_prefix = Some(vec![let_stmt]);
                    receiver = n::Expr {
                        ty: ptr_ty,
                        source: source(view.span),
                        kind: K::AddressOf(Box::new(n::Expr {
                            source: source(view.span),
                            ty: dyn_ty,
                            kind: K::Local(binding),
                        })),
                    };
                }
                break;
            }
        }

        let dyn_call = n::Expr {
            source: source(expr.span),
            ty: call_result_ty.clone(),
            kind: K::DynCall(Box::new(n::DynCall {
                interface: n::InterfaceInstance {
                    declaration: dyn_info.interface,
                    arguments: dyn_info.arguments,
                },
                member: method_contract.declaration,
                receiver: Box::new(receiver),
                by_value_receiver: method_contract.by_value_receiver
                    && !method_contract.by_reference_receiver,
                arguments: call_arguments,
                parameter_bindings: method_contract.parameter_bindings,
            })),
        };

        let result_expr = if let Some(statements) = temp_prefix {
            n::Expr {
                source: source(expr.span),
                ty: dyn_call.ty.clone(),
                kind: K::Block {
                    statements,
                    result: Some(Box::new(dyn_call)),
                },
            }
        } else {
            dyn_call
        };

        if let Some(expected) = expected {
            self.require_expression(&result_expr.ty, expected, expr.span)?;
        }

        Ok(Some(result_expr))
    }
}
