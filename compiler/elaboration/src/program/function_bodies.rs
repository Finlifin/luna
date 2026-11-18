//! One source-body checking rule for package emission and required dependencies.
//! Callers establish the declaration scope; only a completed body is published
//! as a NIR Function. This does not execute the function or grant an invocation.
use super::*;

pub(super) enum Defaults {
    /// The final declaration carries its checked default descriptions.
    Definition,
    /// The invocation's call plan already carries the defaults actually used.
    CallPlan,
}

/// Retain the fields needed to form a runtime function value in a source
/// snapshot. Program contract admission remains the consumer's responsibility.
pub(super) fn retain_runtime_identity(
    interface: &n::FunctionDeclaration,
    steps: &mut u64,
    span: Span,
) -> Result<n::RuntimeFunctionIdentity, Error> {
    debug_assert_eq!(interface.phase, n::FunctionPhase::Runtime);
    nir::copy_budget::charge_callable_formation(interface.into(), steps)
        .map_err(|_| error(span, "runtime function identity snapshot budget exhausted"))?;
    Ok(n::RuntimeFunctionIdentity {
        source: interface.source,
        declaration: interface.declaration,
        type_parameters: interface.type_parameters.clone(),
        requirements: interface.requirements.clone(),
        capabilities: interface.capabilities.clone(),
        signature: interface.callable_type(),
    })
}

impl Checker<'_, '_> {
    // Isolate definition/instance checking from the caller's locals, return and
    // control-flow context. Global demanded declarations and budgets survive.
    pub(super) fn in_function_scope<T>(
        &mut self,
        owner: OwnerId,
        instance: Option<std::sync::Arc<generated_functions::InstanceKey>>,
        check: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let saved_refinements = std::mem::take(&mut self.definition_refinements);
        let saved_behavior_inputs = std::mem::take(&mut self.ordinary_behavior_inputs);
        let saved_behavior = std::mem::replace(&mut self.checking_open_behavior, false);
        let saved_call = std::mem::replace(&mut self.describing_open_call, false);
        let saved_spread = std::mem::replace(&mut self.describing_open_spread, false);
        let saved_owner = self.signature_owner.replace(owner);
        let saved_instance = std::mem::replace(&mut self.open_instance, instance);
        let saved_inline = self.inline_selection.take();
        let saved_match_input = self.inline_match_input.take();
        let saved_unselected = std::mem::replace(&mut self.in_unselected_inline, false);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_code = self.code_values.clone();
        let saved_patterns = self.pattern_codes.clone();
        let saved_staged_locals = std::mem::take(&mut self.open_staged_locals);
        let saved_staged_locals_members = std::mem::take(&mut self.open_member_locals);
        let saved_captures = std::mem::take(&mut self.captures);
        let saved_control = std::mem::take(&mut self.control);
        let saved_immutable = std::mem::take(&mut self.immutable);
        let saved_context = self.context.clone();
        let saved_contract = std::mem::replace(&mut self.checking_contract, false);
        let saved_numeric = std::mem::replace(&mut self.numeric_defaults, true);
        let saved_staged = std::mem::replace(&mut self.in_staged_input, false);
        let saved_required = std::mem::replace(&mut self.in_required_comptime, false);
        let inputs = self.input_facts.for_open_body();
        let saved_inputs = std::mem::replace(&mut self.input_facts, inputs);
        let saved_default_scope = self.default_input_scope.take();
        let saved_default_owner = self.default_input_owner.take();
        let saved_effects = self.effect_environment.take();
        let saved_resume = self.effect_resume_result.take();
        self.context = if self.open_instance.is_some() {
            self.definition_context_in_scope(owner, self.call_input_frames())
        } else {
            self.definition_context(owner)
        };
        let result = check(self);
        self.definition_refinements = saved_refinements;
        self.ordinary_behavior_inputs = saved_behavior_inputs;
        self.checking_open_behavior = saved_behavior;
        self.describing_open_call = saved_call;
        self.describing_open_spread = saved_spread;
        self.signature_owner = saved_owner;
        self.open_instance = saved_instance;
        self.inline_selection = saved_inline;
        self.inline_match_input = saved_match_input;
        self.in_unselected_inline = saved_unselected;
        self.locals = saved_locals;
        self.code_values = saved_code;
        self.pattern_codes = saved_patterns;
        self.open_staged_locals = saved_staged_locals;
        self.open_member_locals = saved_staged_locals_members;
        self.captures = saved_captures;
        self.control = saved_control;
        self.immutable = saved_immutable;
        self.context = saved_context;
        self.checking_contract = saved_contract;
        self.numeric_defaults = saved_numeric;
        self.in_staged_input = saved_staged;
        self.in_required_comptime = saved_required;
        self.input_facts = saved_inputs;
        self.default_input_scope = saved_default_scope;
        self.default_input_owner = saved_default_owner;
        self.effect_environment = saved_effects;
        self.effect_resume_result = saved_resume;
        result
    }

    pub(super) fn runtime_function_identity(
        &self,
        owner: OwnerId,
        signature: Term,
    ) -> n::RuntimeFunctionIdentity {
        n::RuntimeFunctionIdentity {
            source: source(self.package.item(owner).unwrap().span),
            declaration: declaration(self.package, &self.package_identities, owner),
            signature: self.callable_type(owner, signature),
            type_parameters: self.type_parameters[&owner].clone(),
            requirements: self.definition_requirements(owner),
            capabilities: self.capabilities.get(&owner).cloned().unwrap_or_default(),
        }
    }

    pub(super) fn check_source_function(
        &mut self,
        owner: OwnerId,
        signature: Term,
        defaults: Defaults,
        span: Span,
    ) -> Result<n::Function, Error> {
        self.with_source_declaration(owner, signature, defaults, span, |this, interface| {
            let item = this.package.item(owner).unwrap();
            let ItemKind::Fn(sig, body_id) = &item.kind else {
                return Err(error(span, "source function needs a body"));
            };
            let body = this
                .package
                .body(*body_id)
                .ok_or_else(|| error(span, "missing function body"))?;
            let Term::Function { result, .. } = &interface.signature else {
                unreachable!("formed declaration has a function signature")
            };
            let saved_code = std::mem::replace(&mut this.in_code_body, sig.modifiers.is_quote);
            let saved_holes = std::mem::take(&mut this.quote_holes);
            let checked = this.function_body(owner, body.value, result);
            this.in_code_body = saved_code;
            this.quote_holes = saved_holes;
            Ok(n::Function {
                interface,
                body: checked?,
            })
        })
    }

    pub(super) fn with_source_declaration<R>(
        &mut self,
        owner: OwnerId,
        signature: Term,
        defaults: Defaults,
        span: Span,
        use_declaration: impl FnOnce(&mut Self, n::FunctionDeclaration) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let item = self
            .package
            .item(owner)
            .ok_or_else(|| error(span, "missing source function declaration"))?;
        let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) = &item.kind else {
            return Err(error(span, "source declaration needs a function signature"));
        };
        let Term::Function { parameters, result } = &signature else {
            return Err(error(span, "source function needs a callable signature"));
        };
        let saved_types = self.input_facts.types.clone();
        for (index, (parameter, ty)) in sig.params.iter().zip(parameters).enumerate() {
            let binding = local(parameter.hir_id);
            self.locals.insert(binding, ty.ty.clone());
            if ty.sort == Sort::Type {
                self.input_facts.types.insert(
                    binding,
                    Term::Bound {
                        depth: 0,
                        index,
                        sort: Sort::Type,
                    },
                );
            }
        }
        if let Some(parameters) = self.normal_parameters.get(&owner) {
            for (_, binding, ty) in parameters.iter().filter(|(_, _, ty)| *ty != p(P::Type)) {
                self.locals.insert(*binding, ty.clone());
            }
        }
        self.control.result = Some((**result).clone());
        let checked = (|| {
            let Term::Function { result, .. } = &signature else {
                unreachable!("source function signature was checked above")
            };
            let preconditions = self.program_preconditions(owner)?;
            let result_binding = sig.return_binding.map(local);
            let (postconditions, entry_snapshots) =
                self.program_postconditions(owner, result_binding, result)?;
            let interface = n::FunctionDeclaration {
                source: source(item.span),
                declaration: declaration(self.package, &self.package_identities, owner),
                name: item.ident.name.as_str().into(),
                phase: if sig.modifiers.is_normal_form_head {
                    n::FunctionPhase::NormalForm
                } else if sig.modifiers.is_comptime {
                    n::FunctionPhase::Comptime
                } else {
                    n::FunctionPhase::Runtime
                },
                inline: sig.modifiers.is_inline,
                pure: sig.modifiers.is_pure,
                is_unsafe: sig.modifiers.is_unsafe,
                preconditions,
                postconditions,
                entry_snapshots,
                result_binding,
                requirements: self.definition_requirements(owner),
                capabilities: self.capabilities.get(&owner).cloned().unwrap_or_default(),
                type_parameters: self.type_parameters[&owner].clone(),
                parameters: if sig.modifiers.is_normal_form_head {
                    self.normal_parameters[&owner]
                        .iter()
                        .filter(|(_, _, ty)| *ty != p(P::Type))
                        .map(|(_, binding, _)| *binding)
                        .collect()
                } else {
                    sig.params
                        .iter()
                        .map(|parameter| local(parameter.hir_id))
                        .collect()
                },
                defaults: match defaults {
                    Defaults::Definition => self.defaults[&owner]
                        .iter()
                        .enumerate()
                        .filter_map(|(index, value)| value.clone().map(|value| (index, value)))
                        .collect(),
                    // Forcing unused defaults here introduces dependencies
                    // even when the actual call supplies every input.
                    Defaults::CallPlan => vec![],
                },
                signature,
            };
            use_declaration(self, interface)
        })();
        self.input_facts.types = saved_types;
        checked
    }
}
