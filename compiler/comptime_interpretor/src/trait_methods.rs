//! Select an interface member before choosing its execution/delivery mechanism.
use super::*;
use comptime_compiler::methods::{Method, View};

type Key = (InterfaceInstance, Term, DeclarationId);

type Binding = (DeclarationId, Vec<Term>);

pub(super) enum Preparation {
    Ready(Selection),
    Request(Request),
}

pub(super) struct Request {
    access: Expr,
    method: Method,
    key: Key,
    has_preconditions: bool,
    retained_template: bool,
}

impl Request {
    pub(super) fn access(&self) -> &Expr {
        &self.access
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(method-selection-request (original {}) (access {}) (preconditions {}) (retained-template {}))",
            self.method, self.access, self.has_preconditions, self.retained_template
        )
    }
}

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
            if !contract.program.preconditions.is_empty()
                && self
                    .query_admission(Some(source), |vm, fuel| {
                        vm.iir_template_method_type(identity, fuel)
                    })?
                    .is_none()
            {
                return Err(self.failure(ErrorKind::Unsupported, Some(source)));
            }
            for requirement in &contract.requirements {
                self.check_type_contract(requirement, source)?;
            }
            for capability in &contract.capabilities {
                self.check_capability_contract(capability, source)?;
            }
            return Ok(());
        }
        let output = self.query_admission(Some(source), |vm, fuel| {
            vm.admitted_callable_signature(nir::CallableValueRef::Method(identity), &[], fuel)?
                .map(std::borrow::Cow::into_owned)
                .ok_or_else(|| nir::NominalAdmissionError::from("missing runtime method signature"))
        })?;
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
        match self.prepare_trait_method(method, direct_callee, false)? {
            Preparation::Ready(selection) => Ok(selection),
            Preparation::Request(request) => {
                let selected = self.resolve_method_request(&request);
                self.accept_method_selection(request, selected)
            }
        }
    }

    pub(super) fn prepare_trait_method(
        &mut self,
        method: View<'_>,
        direct_callee: bool,
        external: bool,
    ) -> Result<Preparation, Error> {
        let View {
            interface,
            member,
            subject,
            interface_arguments,
            method_arguments,
            output,
            source,
        } = method;
        let (method_inputs, contract) = self.method_inputs(method)?;
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
        let frames = [method_inputs.frame];
        let inputs = self.admit_function_inputs(
            &method_inputs.parameters,
            &method_inputs.arguments,
            nir::DeclarationInputScope::ClosedInBody(&frames),
            source,
        )?;
        for capability in &contract.capabilities {
            let bound = self.query_admission(Some(source), |_, fuel| {
                Ok(inputs.instantiate_requirement(capability, fuel)?)
            })?;
            self.check_capability_contract(&bound, source)?;
        }
        for requirement in &contract.requirements {
            let predicate = self.query_admission(Some(source), |_, fuel| {
                Ok(inputs.instantiate_predicate(requirement, fuel)?)
            })?;
            self.check_type_contract(&predicate, source)?;
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
        let has_preconditions = !contract.program.preconditions.is_empty();
        if let Some(binding) = self
            .method_bodies
            .get(&key)
            .or_else(|| self.resolved_method_bodies.get(&key))
            .cloned()
        {
            return self
                .finish_trait_selection(
                    method,
                    key,
                    binding,
                    false,
                    has_preconditions,
                    retained_template,
                )
                .map(Preparation::Ready);
        }
        if !self.comptime_only
            || self.graph.is_some()
            || (!external && self.method_resolver.is_none())
        {
            return Err(self.failure(ErrorKind::UnmaterializedDispatch, Some(source)));
        }
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
        self.charge_copy_work(Some(source), |fuel| {
            for term in std::iter::once(subject)
                .chain(interface_arguments)
                .chain(method_arguments)
                .chain(std::iter::once(output))
            {
                nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        Ok(Preparation::Request(Request {
            access: expression,
            method: Method {
                interface,
                member,
                subject: subject.clone(),
                interface_arguments: interface_arguments.to_vec(),
                method_arguments: method_arguments.to_vec(),
                output: output.clone(),
                source,
            },
            key,
            has_preconditions,
            retained_template,
        }))
    }

    pub(super) fn resolve_method_request(
        &mut self,
        request: &Request,
    ) -> Result<nir::program::MethodInstance, Error> {
        let source = request.access.source;
        let resolver = self
            .method_resolver
            .ok_or_else(|| self.failure(ErrorKind::UnmaterializedDispatch, Some(source)))?;
        host_budget::checked(&mut self.budget.steps, Some(source), |fuel| {
            resolver(&request.access, fuel)
        })
    }

    pub(super) fn accept_method_selection(
        &mut self,
        request: Request,
        selected: Result<nir::program::MethodInstance, Error>,
    ) -> Result<Selection, Error> {
        let selected = selected?;
        let method = request.method.view();
        let source = method.source;
        self.charge_copy_work(Some(source), |fuel| {
            for argument in selected.arguments.iter().chain(&selected.method_arguments) {
                nir::copy_budget::charge_type_substitution(argument, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        if selected.member != method.member
            || selected.method_arguments != method.method_arguments
            || !selected.arguments.ends_with(method.method_arguments)
        {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
        let prefix = selected.arguments.len() - method.method_arguments.len();
        let binding = (selected.body, selected.arguments[..prefix].to_vec());
        self.finish_trait_selection(
            method,
            request.key,
            binding,
            true,
            request.has_preconditions,
            request.retained_template,
        )
    }

    fn finish_trait_selection(
        &mut self,
        method: View<'_>,
        key: Key,
        binding: Binding,
        pending: bool,
        has_preconditions: bool,
        retained_template: bool,
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
        let (body, mut arguments) = binding.clone();
        arguments.extend(method_arguments.iter().cloned());
        let formation = self
            .functions
            .formation(&body)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
        if pending && formation.phase == nir::program::FunctionPhase::NormalForm {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
        let transport = self.functions.runtime_identity(&body).is_some()
            || (self.comptime_only && formation.phase == nir::program::FunctionPhase::Runtime);
        let ty = if transport {
            self.formed_function_signature(formation, &arguments, source)?
        } else {
            let inputs = self.admit_function_inputs(
                formation.parameters,
                &arguments,
                nir::DeclarationInputScope::Caller(&[]),
                source,
            )?;
            self.signature_from_admission(formation, &inputs, source)?
        };
        let signature_for_check = match &ty {
            Term::PureFunction(inner)
                if !output.is_pure_callable() && !output.is_unsafe_callable() =>
            {
                inner.as_ref()
            }
            _ => &ty,
        };
        if !self.observe_same_type(signature_for_check, output, source)? {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        if transport {
            if has_preconditions && !retained_template {
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
            instantiate: !arguments.is_empty()
                || !formation.requirements.is_empty()
                || !formation.capabilities.is_empty(),
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
        let selection = self.select_trait_method(method, direct_callee)?;
        self.trait_selection_value(selection, method.source)
    }

    pub(super) fn trait_selection_value(
        &mut self,
        selection: Selection,
        source: SourceRange,
    ) -> Result<Value, Error> {
        match selection {
            Selection::Value(value) => Ok(value),
            Selection::Function {
                declaration,
                arguments,
                ty,
                instantiate,
            } => {
                // Account for the selected callable observation, without tree execution.
                self.tick(Some(source))?;
                self.with_native_depth(|vm| {
                    if instantiate {
                        vm.function_instance_value(declaration, &arguments, &ty, source)
                    } else if vm.functions.runtime_identity(&declaration).is_some() {
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
