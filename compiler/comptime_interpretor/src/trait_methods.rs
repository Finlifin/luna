//! Select an interface member before choosing its execution/delivery mechanism.
use super::*;
use comptime_compiler::methods::View;

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
        let contract = *contract;
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
        if !self.capabilities_hold_parameters(
            &contract.capabilities,
            &parameters,
            &contract_arguments,
        ) {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
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
        let (body, mut arguments) = self
            .method_bodies
            .get(&(
                InterfaceInstance {
                    declaration: interface,
                    arguments: selected_arguments,
                },
                selected_subject,
                member,
            ))
            .cloned()
            .ok_or_else(|| self.failure(ErrorKind::UnmaterializedDispatch, Some(source)))?;
        arguments.extend(method_arguments.iter().cloned());
        if let Some(identity) = self.runtime_function_identities.get(&body).copied() {
            self.admit_function_inputs(&identity.type_parameters, &arguments, source)?;
            let signature = identity
                .signature
                .instantiate_parameters(&self.input_binders(&identity.type_parameters), &arguments)
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            if !selected_types::same(&signature, output, &self.associated_values)
                || (!contract.program.preconditions.is_empty() && !retained_template)
            {
                return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
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
        let ty = definition
            .signature
            .instantiate_parameters(&self.input_binders(&definition.type_parameters), &arguments)
            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?
            .with_comptime_phase(definition.phase == nir::program::FunctionPhase::Comptime)
            .with_unsafe_call_requirement(definition.is_unsafe);
        if !selected_types::same(&ty, output, &self.associated_values) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        if self.comptime_only && definition.phase == nir::program::FunctionPhase::Runtime {
            self.admit_function_inputs(&definition.type_parameters, &arguments, source)?;
            if !contract.program.preconditions.is_empty() && !retained_template {
                return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
            }
            // Transport the public interface identity even when this
            // snapshot also contains its implementation's runtime body.
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
                self.depth += 1;
                let result = if instantiate {
                    self.function_instance_value(declaration, &arguments, &ty, method.source)
                } else if self.runtime_function_identities.contains_key(&declaration) {
                    Ok(Value::FunctionInstance {
                        declaration,
                        arguments: vec![],
                    })
                } else {
                    Ok(Value::Function(declaration))
                };
                self.depth -= 1;
                result
            }
        }
    }
}
