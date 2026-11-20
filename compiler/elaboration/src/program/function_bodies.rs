//! One source-body checking rule for package emission and required dependencies.
//! Source interfaces bind parameters and contracts in the declaration's own
//! scope. Only checking the body produces a NIR Function.
use super::*;

pub(super) enum SourceBody {
    Package,
    Required,
}

pub(super) fn phase(modifiers: &hir::item::FnModifiers) -> n::FunctionPhase {
    if modifiers.is_normal_form_head {
        n::FunctionPhase::NormalForm
    } else if modifiers.is_comptime {
        n::FunctionPhase::Comptime
    } else {
        n::FunctionPhase::Runtime
    }
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

    pub(super) fn check_source_interface(
        &mut self,
        owner: OwnerId,
        signature: Term,
        span: Span,
    ) -> Result<n::FunctionDeclaration, Error> {
        self.in_declaration_scope(owner, |this| {
            this.bind_source_declaration(owner, signature, span)
        })
    }

    pub(super) fn check_source_function(
        &mut self,
        owner: OwnerId,
        signature: Term,
        request: SourceBody,
        span: Span,
    ) -> Result<n::Function, Error> {
        let required = matches!(request, SourceBody::Required) && self.in_required_comptime;
        self.in_declaration_scope(owner, |this| {
            this.in_required_comptime = required;
            let mut interface = this.bind_source_declaration(owner, signature, span)?;
            if matches!(request, SourceBody::Package) {
                // Required calls already carry the defaults selected by their call plan.
                interface.defaults = this.defaults[&owner]
                    .iter()
                    .enumerate()
                    .filter_map(|(index, value)| value.clone().map(|value| (index, value)))
                    .collect();
            }
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
            this.in_code_body = sig.modifiers.is_quote;
            let body = this.function_body(owner, body.value, result)?;
            this.verify_returned_type_guarantees(&body, span)?;
            Ok(n::Function { interface, body })
        })
    }

    fn bind_source_declaration(
        &mut self,
        owner: OwnerId,
        signature: Term,
        span: Span,
    ) -> Result<n::FunctionDeclaration, Error> {
        self.context = self.definition_context(owner);
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
        for (index, (parameter, ty)) in sig.params.iter().zip(parameters).enumerate() {
            let binding = local(parameter.hir_id);
            self.locals.insert(binding, ty.ty.clone());
            self.input_facts
                .parameters
                .bind(binding, index, ty.sort, &ty.ty);
        }
        if let Some(parameters) = self.normal_parameters.get(&owner) {
            for (_, binding, ty) in parameters.iter().filter(|(_, _, ty)| *ty != p(P::Type)) {
                self.locals.insert(*binding, ty.clone());
            }
        }
        self.control.result = Some((**result).clone());
        let preconditions = self.program_preconditions(owner)?;
        let result_binding = sig.return_binding.map(local);
        let (postconditions, entry_snapshots) =
            self.program_postconditions(owner, result_binding, result)?;
        let (result_type_binder, result_guarantees) = self
            .result_guarantees
            .get(&owner)
            .map_or((None, Vec::new()), |(binder, guarantees)| {
                (Some(*binder), guarantees.clone())
            });
        self.control.result_guarantees = result_type_binder
            .filter(|_| !result_guarantees.is_empty())
            .map(|binder| (binder, result_guarantees.clone()));
        Ok(n::FunctionDeclaration {
            source: source(item.span),
            declaration: declaration(self.package, &self.package_identities, owner),
            name: item.ident.name.as_str().into(),
            phase: phase(&sig.modifiers),
            inline: sig.modifiers.is_inline,
            pure: sig.modifiers.is_pure,
            is_unsafe: sig.modifiers.is_unsafe,
            preconditions,
            postconditions,
            entry_snapshots,
            result_binding,
            result_type_binder,
            result_guarantees,
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
            env_parameters: vec![],
            defaults: vec![],
            signature,
        })
    }
}
