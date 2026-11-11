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

impl Checker<'_, '_> {
    pub(super) fn check_source_function(
        &mut self,
        owner: OwnerId,
        signature: Term,
        defaults: Defaults,
        span: Span,
    ) -> Result<n::Function, Error> {
        let item = self
            .package
            .item(owner)
            .ok_or_else(|| error(span, "missing source function declaration"))?;
        let ItemKind::Fn(sig, body_id) = &item.kind else {
            return Err(error(span, "source function needs a body"));
        };
        let Term::Function { parameters, result } = &signature else {
            return Err(error(span, "source function needs a callable signature"));
        };
        let body = self
            .package
            .body(*body_id)
            .ok_or_else(|| error(span, "missing function body"))?;
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
            let preconditions = self.program_preconditions(owner)?;
            let result_binding = sig.return_binding.map(local);
            let (postconditions, entry_snapshots) =
                self.program_postconditions(owner, result_binding, result)?;
            let saved_code = std::mem::replace(&mut self.in_code_body, sig.modifiers.is_quote);
            let saved_holes = std::mem::take(&mut self.quote_holes);
            let checked = self.function_body(owner, body.value, result);
            self.in_code_body = saved_code;
            self.quote_holes = saved_holes;
            let body = checked?;
            Ok((
                preconditions,
                postconditions,
                entry_snapshots,
                result_binding,
                body,
            ))
        })();
        self.input_facts.types = saved_types;
        let (preconditions, postconditions, entry_snapshots, result_binding, body) = checked?;
        Ok(n::Function {
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
            body,
        })
    }
}
