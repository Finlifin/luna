//! Select an interface member before choosing its execution/delivery mechanism.
use super::*;
use comptime_compiler::methods::View;

#[cfg(test)]
mod budget_tests;

pub(super) enum Selection {
    Value(Value),
    Function {
        declaration: DeclarationId,
        arguments: Vec<Term>,
        ty: Term,
        instantiate: bool,
    },
}
impl Interpreter<'_> {
    pub(super) fn admit_runtime_method_identity(
        &mut self,
        identity: &TraitMethodIdentity,
    ) -> Result<(), Error> {
        let source = SourceRange { start: 0, end: 0 };
        if self.graph.is_some() {
            // The immutable graph owns the selected interface/body evidence.
            // No source method lookup or execution is needed for transport.
            let contract = self
                .iir_method(
                    identity.interface.declaration,
                    &identity.interface.arguments,
                    &identity.subject,
                    identity.member,
                    &identity.method_arguments,
                )?
                .ok_or_else(|| self.failure(ErrorKind::UnmaterializedDispatch, Some(source)))?;
            for requirement in &contract.requirements {
                self.check_type_contract(requirement, source)?;
            }
            self.check_capability_parameters(&contract.capabilities, &[], &[], source)?;
            return Ok(());
        }
        let (parameters, contract) = self
            .method_contracts
            .get(&(identity.interface.declaration, identity.member))
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
        self.budget.steps = self
            .budget
            .steps
            .checked_sub(parameters.len() as u64 + 1)
            .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
        let parameters = parameters.clone();
        self.charge_copy_work(Some(source), |fuel| {
            for argument in std::iter::once(&identity.subject)
                .chain(identity.interface.arguments.iter())
                .chain(&identity.method_arguments)
            {
                nir::copy_budget::charge_type_substitution(argument, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        let arguments: Vec<_> = std::iter::once(identity.subject.clone())
            .chain(identity.interface.arguments.iter().cloned())
            .chain(identity.method_arguments.iter().cloned())
            .collect();
        let signature = contract
            .signature
            .ordinary()
            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
        self.charge_copy_work(Some(source), |fuel| {
            nir::copy_budget::charge_type_substitution(signature, &parameters, &arguments, fuel)
        })?;
        let output = signature
            .instantiate_parameters(&parameters, &arguments)
            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
        self.select_trait_method(
            View {
                interface: identity.interface.declaration,
                interface_arguments: &identity.interface.arguments,
                subject: &identity.subject,
                member: identity.member,
                method_arguments: &identity.method_arguments,
                output: &output,
                source,
            },
            false,
        )?;
        Ok(())
    }

    pub(super) fn select_trait_method(
        &mut self,
        method: View<'_>,
        direct_callee: bool,
    ) -> Result<Selection, Error> {
        let View {
            interface,
            member,
            subject,
            interface_arguments,
            method_arguments,
            output,
            source,
        } = method;
        let (parameters, contract) = self
            .method_contracts
            .get(&(interface, member))
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
        let parameters = parameters.clone();
        // Raw NIR can check an immediate interface call, but cannot
        // erase its program domain into an implementation function
        // value. IIR transports the identity and specializes the call.
        let retained_template = self.comptime_only
            && !contract.signature.is_comptime_callable()
            && matches!(contract.signature.callable_signature(), Ok(Term::Function { parameters, .. })
                        if parameters.iter().any(|p| p.sort != nir::Sort::Runtime));
        if !direct_callee && !contract.program.preconditions.is_empty() && !retained_template {
            return Err(self.failure(ErrorKind::Unsupported, Some(source)));
        }
        if method_arguments.len() != contract.type_parameters.len() {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
        let contract_arguments: Vec<_> = std::iter::once(subject.clone())
            .chain(interface_arguments.iter().cloned())
            .chain(method_arguments.iter().cloned())
            .collect();
        self.check_capability_parameters(
            &contract.capabilities,
            &parameters,
            &contract_arguments,
            source,
        )?;
        for requirement in &contract.requirements {
            let requirement = requirement
                .instantiate_parameters(&parameters, &contract_arguments)
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            self.check_type_contract(&requirement, source)?;
        }
        let selected_subject = self.selected_type_view(subject, Some(source))?;
        let selected_arguments = interface_arguments
            .iter()
            .map(|argument| self.selected_type_view(argument, Some(source)))
            .collect::<Result<Vec<_>, _>>()?;
        let key = (
            InterfaceInstance {
                declaration: interface,
                arguments: selected_arguments,
            },
            selected_subject,
            member,
        );
        let (binding, pending) = if let Some(binding) = self
            .method_bodies
            .get(&key)
            .or_else(|| self.resolved_method_bodies.get(&key))
            .cloned()
        {
            (binding, false)
        } else {
            let resolver = self
                .method_resolver
                .filter(|_| self.comptime_only && self.graph.is_none())
                .ok_or_else(|| self.failure(ErrorKind::UnmaterializedDispatch, Some(source)))?;
            self.charge_copy_work(Some(source), |fuel| {
                for term in std::iter::once(&key.1)
                    .chain(&key.0.arguments)
                    .chain(method_arguments)
                    .chain(std::iter::once(output))
                {
                    nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
                }
                Ok(())
            })?;
            let expression = Expr {
                source,
                ty: output.clone(),
                kind: K::TraitMethod {
                    interface,
                    interface_arguments: key.0.arguments.clone(),
                    subject: key.1.clone(),
                    member,
                    method_arguments: method_arguments.to_vec(),
                },
            };
            let selected =
                host_budget::checked(&mut self.budget.steps, Some(source), |remaining| {
                    resolver(&expression, remaining)
                })?;
            self.charge_copy_work(Some(source), |fuel| {
                for argument in selected.arguments.iter().chain(&selected.method_arguments) {
                    nir::copy_budget::charge_type_substitution(argument, &[], &[], fuel)?;
                }
                Ok(())
            })?;
            if selected.member != member
                || selected.method_arguments != method_arguments
                || !selected.arguments.ends_with(method_arguments)
            {
                return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
            }
            let prefix = selected.arguments.len() - method_arguments.len();
            ((selected.body, selected.arguments[..prefix].to_vec()), true)
        };
        let (body, mut arguments) = binding.clone();
        arguments.extend(method_arguments.iter().cloned());
        if let Some(identity) = self.runtime_function_identities.get(&body) {
            self.admit_function_inputs(&identity.type_parameters, &arguments, source)?;
            let signature = identity
                .signature
                .instantiate_parameters(&self.input_binders(&identity.type_parameters), &arguments)
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            if !self.observe_same_type(&signature, output, source)?
                || (!contract.program.preconditions.is_empty() && !retained_template)
            {
                return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
            }
            if pending {
                self.resolved_method_bodies
                    .insert(key.clone(), binding.clone());
            }
            return Ok(Selection::Value(Value::RuntimeTraitMethod(Box::new(
                TraitMethodIdentity {
                    interface: InterfaceInstance {
                        declaration: interface,
                        arguments: interface_arguments.to_vec(),
                    },
                    subject: subject.clone(),
                    member: member,
                    method_arguments: method_arguments.to_vec(),
                },
            ))));
        }
        let definition = self
            .functions
            .get(&body)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
        if pending {
            if definition.phase == nir::program::FunctionPhase::NormalForm {
                return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
            }
            self.admit_function_inputs(&definition.type_parameters, &arguments, source)?;
        }
        let ty = definition
            .signature
            .instantiate_parameters(&self.input_binders(&definition.type_parameters), &arguments)
            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?
            .with_comptime_phase(definition.phase == nir::program::FunctionPhase::Comptime)
            .with_unsafe_call_requirement(definition.is_unsafe);
        if !self.observe_same_type(&ty, output, source)? {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        if self.comptime_only && definition.phase == nir::program::FunctionPhase::Runtime {
            self.admit_function_inputs(&definition.type_parameters, &arguments, source)?;
            if !contract.program.preconditions.is_empty() && !retained_template {
                return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
            }
            // Transport the public interface identity even when this
            // snapshot also contains its implementation's runtime body.
            if pending {
                self.resolved_method_bodies
                    .insert(key.clone(), binding.clone());
            }
            return Ok(Selection::Value(Value::RuntimeTraitMethod(Box::new(
                TraitMethodIdentity {
                    interface: InterfaceInstance {
                        declaration: interface,
                        arguments: interface_arguments.to_vec(),
                    },
                    subject: subject.clone(),
                    member: member,
                    method_arguments: method_arguments.to_vec(),
                },
            ))));
        }
        if pending {
            self.resolved_method_bodies.insert(key, binding);
        }
        Ok(Selection::Function {
            declaration: body,
            instantiate: !arguments.is_empty() || !definition.requirements.is_empty(),
            arguments,
            ty,
        })
    }
    pub(super) fn bytecode_trait_method(
        &mut self,
        method: View<'_>,
        direct_callee: bool,
    ) -> Result<Value, Error> {
        if self.graph.is_some() {
            return self.iir_method_value(method);
        }
        match self.select_trait_method(method, direct_callee)? {
            Selection::Value(value) => Ok(value),
            Selection::Function {
                declaration,
                arguments,
                ty,
                instantiate,
            } => {
                // Account for the selected callable observation, without tree execution.
                self.tick(Some(method.source))?;
                self.with_native_depth(|vm| {
                    if instantiate {
                        vm.function_instance_value(declaration, &arguments, &ty, method.source)
                    } else if vm.runtime_function_identities.contains_key(&declaration) {
                        Ok(Value::FunctionInstance {
                            declaration,
                            arguments: vec![],
                        })
                    } else {
                        Ok(Value::Function(declaration))
                    }
                })
            }
        }
    }
}
