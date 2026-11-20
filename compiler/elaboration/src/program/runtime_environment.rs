//! Acquire runtime clause inputs in the caller. Hidden argument positions use
//! the resulting values after acquisition, including forwarded defaults.
use super::*;
use arguments::RoutedArgument;

impl Checker<'_, '_> {
    fn owner_environment_inputs(
        &mut self,
        owner: OwnerId,
        args: &[Term],
        span: Span,
    ) -> Result<Vec<(Term, Term)>, Error> {
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            return Ok(vec![]);
        };
        if function_bodies::phase(&sig.modifiers) != n::FunctionPhase::Runtime
            || sig.modifiers.is_normal_form_head
        {
            return Ok(vec![]);
        }
        self.ensure_signature(owner, span)?;
        let params = self
            .type_parameters
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        if params.is_empty() {
            return Ok(vec![]);
        }
        let domains = self.trait_domains(owner)?;
        if params.len() != args.len() || domains.len() != params.len() {
            if domains.iter().any(|d| matches!(d, Term::Lift(_))) {
                return Err(error(span, "runtime environment argument arity mismatch"));
            }
            return Ok(vec![]);
        }
        let frames = self.call_input_frames();
        let mut env_args = Vec::new();
        for (slot, (domain, arg)) in domains.iter().zip(args).enumerate() {
            let Term::Lift(inner) = domain else {
                continue;
            };
            let instantiated_inner = self.instantiate_input_types_in_scope(
                inner,
                &params[..slot],
                &args[..slot],
                &frames,
                span,
            )?;
            let instantiated_inner = self.callable_input_view(&instantiated_inner, span)?;
            env_args.push((
                declaration_inputs::copy_input_domain(arg, &mut self.required_steps, span)?,
                instantiated_inner,
            ));
        }
        Ok(env_args)
    }

    pub(super) fn call_environment_inputs(
        &mut self,
        callee: &n::Expr,
        span: Span,
    ) -> Result<Vec<(Term, Term)>, Error> {
        match &callee.kind {
            K::Function(decl_id) => {
                let Some(owner) = self.source_declarations.owner(*decl_id) else {
                    return Ok(vec![]);
                };
                self.owner_environment_inputs(owner, &[], span)
            }
            K::FunctionInstance {
                declaration: decl_id,
                arguments,
            } => {
                let Some(owner) = self.source_declarations.owner(*decl_id) else {
                    return Ok(vec![]);
                };
                self.owner_environment_inputs(owner, arguments, span)
            }
            K::TraitMethod {
                interface,
                interface_arguments,
                subject,
                member,
                method_arguments,
            } => {
                let instance = n::InterfaceInstance {
                    declaration: *interface,
                    arguments: interface_arguments.clone(),
                };
                if let Ok(resolution) =
                    self.resolve_trait_capability(&instance, subject, crate::ptc::Budget::default())
                    && let crate::implementation::Selection::Selected(impl_id) =
                        resolution.selection
                    && let Some(crate::implementation::TypeAssessment::Applicable(evidence)) =
                        resolution.assessments.get(&impl_id)
                {
                    let implementation =
                        &self.checked_implementations[self.implementation_positions[&impl_id]];
                    let contract = self
                        .checked_traits
                        .by_id(*interface)
                        .and_then(|t| t.methods.iter().find(|m| m.declaration == *member))
                        .ok_or_else(|| error(span, "missing trait method contract"))?;
                    let body = implementation
                        .methods
                        .iter()
                        .find(|(method, _)| *method == contract.name)
                        .map(|(_, body)| *body)
                        .ok_or_else(|| error(span, "unknown trait method implementation"))?;
                    let Some(impl_owner) = self.source_declarations.owner(body) else {
                        return Err(error(span, "missing trait method owner"));
                    };
                    let args = if self.self_containers.contains_key(&impl_owner) {
                        evidence
                            .type_arguments
                            .iter()
                            .cloned()
                            .chain(method_arguments.iter().cloned())
                            .collect::<Vec<_>>()
                    } else {
                        std::iter::once(subject.clone())
                            .chain(interface_arguments.iter().cloned())
                            .chain(method_arguments.iter().cloned())
                            .collect::<Vec<_>>()
                    };
                    return self.owner_environment_inputs(impl_owner, &args, span);
                }
                let Some(contract_owner) = self.source_declarations.owner(*member) else {
                    return Ok(vec![]);
                };
                let args: Vec<_> = std::iter::once(subject.clone())
                    .chain(interface_arguments.iter().cloned())
                    .chain(method_arguments.iter().cloned())
                    .collect();
                self.owner_environment_inputs(contract_owner, &args, span)
            }
            _ => Ok(vec![]),
        }
    }
}

pub(super) struct Acquisition(pub(super) Vec<n::Expr>);

pub(super) struct CallEnvironment {
    before_arguments: Acquisition,
    argument_prefixes: BTreeMap<usize, Vec<n::Expr>>,
    after_explicit: Vec<n::Expr>,
}

impl CallEnvironment {
    pub(super) fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        call: n::Expr,
        routes: &[RoutedArgument<'_>],
        offset: usize,
        site: &Expr<'_>,
    ) -> Result<n::Expr, Error> {
        let call = checker.stage_collected_call(
            call,
            routes,
            offset,
            site,
            self.argument_prefixes,
            self.after_explicit,
        )?;
        if self.before_arguments.0.is_empty() {
            return Ok(call);
        }
        let region = self.before_arguments.finish(checker, call, site.span)?;
        checker
            .local_runtime_origins
            .regions
            .insert(region.source, runtime_origins::Region::Call);
        Ok(region)
    }
}

impl Acquisition {
    pub(super) fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        call: n::Expr,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if self.0.is_empty() {
            return Ok(call);
        }
        Ok(n::Expr {
            source: checker.generated_expression_source(call.source, span)?,
            ty: call.ty.clone(),
            kind: K::Block {
                statements: self.0,
                result: Some(Box::new(call)),
            },
        })
    }
}

/// The outer field operations keep their checked layout and payload domains.
struct EnvironmentSlot {
    operand: usize,
    fields: Vec<usize>,
}

impl Checker<'_, '_> {
    pub(super) fn prepare_call_environment(
        &mut self,
        call: &mut n::Expr,
        inputs: Vec<(Term, Term)>,
        receiver_count: usize,
        routes: &[RoutedArgument<'_>],
        site: &Expr<'_>,
    ) -> Result<CallEnvironment, Error> {
        let outer = self.locals.clone();
        let result = self.acquire_call_inputs(call, inputs, receiver_count, routes, site);
        self.locals = outer;
        result
    }

    fn acquire_call_inputs(
        &mut self,
        call: &mut n::Expr,
        inputs: Vec<(Term, Term)>,
        receiver_count: usize,
        routes: &[RoutedArgument<'_>],
        site: &Expr<'_>,
    ) -> Result<CallEnvironment, Error> {
        let mut statements = Vec::new();
        if inputs.is_empty() && self.local_runtime_origins.acquired_bindings.is_empty() {
            return Ok(CallEnvironment {
                before_arguments: Acquisition(statements),
                argument_prefixes: BTreeMap::new(),
                after_explicit: Vec::new(),
            });
        }
        let K::Call {
            callee,
            arguments,
            env_arguments,
            ..
        } = &mut call.kind
        else {
            return Err(error(site.span, "runtime environment requires a call"));
        };
        let mut receiver_prefixes = Vec::with_capacity(receiver_count);
        for arg in arguments.iter_mut().take(receiver_count) {
            let value = arguments::reference_base(&mut arg.value);
            receiver_prefixes.push(self.take_value_environment(value, site.span)?);
        }
        let mut argument_prefixes = BTreeMap::new();
        let mut argument_bindings = BTreeSet::new();
        if !self.local_runtime_origins.acquired_bindings.is_empty() {
            self.visit_routed_arguments(
                arguments,
                routes,
                receiver_count,
                site.span,
                false,
                |checker, value, position, _| {
                    let prefix = checker.take_value_environment(value, site.span)?;
                    if !prefix.is_empty() {
                        argument_bindings.extend(prefix.iter().filter_map(
                            |stmt| match stmt.kind {
                                K::Let { binding, .. } => Some(binding),
                                _ => None,
                            },
                        ));
                        argument_prefixes.insert(position, prefix);
                    }
                    Ok(())
                },
            )?;
        }
        let mut identities = std::collections::HashMap::<Term, usize>::new();
        let mut defaults = BTreeMap::new();
        let mut slots = Vec::new();
        let mut operands: Vec<n::Expr> = Vec::new();
        let mut bind = receiver_prefixes.iter().any(|prefix| !prefix.is_empty());
        for (identity, mut domain) in inputs {
            let (root, fields) = self.runtime_field_path(&identity, site.span)?;
            if !fields.is_empty() {
                domain = self.runtime_root_payload(root, site.span)?;
                bind = true;
            }
            let identity =
                declaration_inputs::copy_input_domain(root, &mut self.required_steps, site.span)?;
            if let Some(&slot) = identities.get(&identity) {
                self.require_expression(&operands[slot].ty, &domain, site.span)?;
                slots.push(EnvironmentSlot {
                    operand: slot,
                    fields,
                });
                continue;
            }
            let value = if let Term::Runtime(id) = &identity
                && let Some(local) = self.default_result_local(*id, site.span)
            {
                self.require_expression(&local.ty, &domain, site.span)?;
                defaults.insert(operands.len(), *id);
                bind = true;
                local
            } else {
                self.deliver_runtime_dependency(&identity, &domain, site.span)?
            };
            bind |= match self.runtime_place(&value, site.span)? {
                Some(runtime_origins::Place::Local(LocalRuntimeOrigin::LocalValue {
                    binding,
                    ..
                })) => !self.immutable.contains(&binding),
                Some(_) => false,
                None => true,
            };
            let slot = operands.len();
            slots.push(EnvironmentSlot {
                operand: slot,
                fields,
            });
            identities.insert(identity, slot);
            operands.push(value);
        }
        if bind {
            if let Some(selected) = self.prepare_call_callee(callee, site.span)? {
                statements.push(selected);
            }
            for (arg, prefix) in arguments.iter_mut().zip(receiver_prefixes) {
                statements.extend(prefix);
                let value = if arg.passing == n::ArgumentPassing::Value {
                    &mut arg.value
                } else {
                    arguments::reference_base(&mut arg.value)
                };
                if arg.passing != n::ArgumentPassing::Value && matches!(value.kind, K::Local(_)) {
                    continue;
                }
                statements.push(self.prepare_call_operand(value, site.span)?);
            }
            // Acquire the written inputs first, then any remaining default or
            // inherited dependencies in telescope order. A reused origin can
            // have a different source occurrence at each call.
            let mut producers = BTreeMap::new();
            let mut order = Vec::with_capacity(operands.len());
            let mut acquired = vec![false; operands.len()];
            if let H::Application(head, _) | H::ExtendedApplication(head, _) = &site.kind
                && let H::NFApplication(_, args) = &head.kind
            {
                for arg in *args {
                    let (Arg::Positional(value)
                    | Arg::Named(_, value)
                    | Arg::Expand(value)
                    | Arg::Implicit(value)
                    | Arg::PostLambda(value)) = arg;
                    if let Some(identity) = self.local_runtime_origins.sources.get(&value.hir_id)
                        && let Some(&slot) = identities.get(identity)
                        && !std::mem::replace(&mut acquired[slot], true)
                    {
                        order.push(slot);
                        if let Term::Runtime(id) = identity
                            && self.local_runtime_deliveries.get(id).is_some_and(|entry| {
                                entry.producer_site == Some(local(value.hir_id))
                            })
                        {
                            producers.insert(slot, *id);
                        }
                    }
                }
            }
            order.extend((0..operands.len()).filter(|&slot| !acquired[slot]));

            if !producers.is_empty() {
                let result_ids = self.result_dependencies(&call.ty, site.span)?;
                producers.retain(|_, id| result_ids.contains(id));
            }
            for slot in order {
                // The enclosing source argument establishes this binding at
                // its own position, after explicit quote suppliers have run.
                if defaults.contains_key(&slot)
                    || matches!(&operands[slot].kind, K::Local(binding) if argument_bindings.contains(binding))
                {
                    continue;
                }
                statements.push(self.prepare_call_operand(&mut operands[slot], site.span)?);
                if let Some(&id) = producers.get(&slot) {
                    let operand = &operands[slot];
                    let K::Local(binding) = operand.kind else {
                        unreachable!("prepared quote operand")
                    };
                    constants::charge_copy(operand, &mut self.required_steps, site.span)?;
                    self.local_runtime_deliveries.get_mut(&id).unwrap().expr = operand.clone();
                    self.local_runtime_origins
                        .acquired_bindings
                        .insert(binding, id);
                    self.immutable.insert(binding);
                }
            }
        }
        let mut after_explicit = Vec::with_capacity(defaults.len());
        for (_, id) in defaults {
            let prepared = |root: &Term| identities.get(root).map(|&slot| &operands[slot]);
            after_explicit.push(self.acquire_runtime_default(id, prepared, site.span)?);
        }
        for slot in slots {
            let base = &operands[slot.operand];
            constants::charge_copy(base, &mut self.required_steps, site.span)?;
            let value = self.apply_runtime_fields(base.clone(), slot.fields, site.span)?;
            env_arguments.push(value);
        }
        Ok(CallEnvironment {
            before_arguments: Acquisition(statements),
            argument_prefixes,
            after_explicit,
        })
    }
}
