//! Receiving-world callable signature queries. Lookup never invokes source
//! code, and resource exhaustion cannot become a negative membership result.
use super::*;
use nir::value_admission::Error as AdmissionError;
use std::borrow::Cow;

fn input_failure(error: ErrorKind) -> AdmissionError {
    match error {
        ErrorKind::StepsExhausted | ErrorKind::DepthExhausted => AdmissionError::Budget,
        ErrorKind::FunctionInputRejected(reason) => AdmissionError::Invalid(reason.to_string()),
        error => AdmissionError::Invalid(format!("{error:?}")),
    }
}

// Older observation consumers use an optional view and the exhausted-budget
// sentinel. Keep that boundary explicit while admission consumes typed results.
fn optional_view<T>(result: Result<Option<T>, AdmissionError>, fuel: &mut usize) -> Option<T> {
    match result {
        Ok(value) => value,
        Err(AdmissionError::Budget) => {
            *fuel = 0;
            None
        }
        Err(AdmissionError::Invalid(_)) => None,
    }
}

impl Interpreter<'_> {
    pub(super) fn normal_callable_signature(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
        frames: &[Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Option<Term> {
        self.normal_callable_signature_view(declaration, arguments, frames, fuel)
            .map(Cow::into_owned)
    }

    pub(super) fn normal_callable_signature_view(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
        frames: &[Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Option<Cow<'_, Term>> {
        optional_view(
            self.admitted_function_signature(declaration, arguments, frames, fuel),
            fuel,
        )
    }

    pub(super) fn normal_value_callable_signature_view(
        &self,
        value: nir::CallableValueRef<'_>,
        frames: &[Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Option<Cow<'_, Term>> {
        optional_view(self.admitted_callable_signature(value, frames, fuel), fuel)
    }

    pub(super) fn admitted_callable_signature(
        &self,
        value: nir::CallableValueRef<'_>,
        frames: &[Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Result<Option<Cow<'_, Term>>, AdmissionError> {
        let nir::CallableValueRef::Method(method) = value else {
            let nir::CallableValueRef::Function {
                declaration,
                arguments,
            } = value
            else {
                unreachable!()
            };
            return self.admitted_function_signature(declaration, arguments, frames, fuel);
        };
        *fuel = fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
        for term in method.terms() {
            nir::type_computations::charge_term(term, fuel)?;
        }
        if self.graph.is_some() {
            let Some(contract) = self
                .iir_method(
                    method.interface.declaration,
                    &method.interface.arguments,
                    &method.subject,
                    method.member,
                    &method.method_arguments,
                )
                .map_err(|error| input_failure(error.kind))?
            else {
                return Ok(None);
            };
            let signature = contract.signature.ordinary().map_err(|e| e.to_string())?;
            if signature.is_comptime_callable() {
                return Ok(None);
            }
            nir::type_computations::charge_term(signature, fuel)?;
            return Ok(Some(Cow::Borrowed(signature)));
        }
        let Some((parameters, contract)) = self
            .method_contracts
            .get(&(method.interface.declaration, method.member))
        else {
            return Ok(None);
        };
        if method.method_arguments.len() != contract.type_parameters.len()
            || method
                .interface
                .arguments
                .len()
                .checked_add(method.method_arguments.len())
                .and_then(|n| n.checked_add(1))
                != Some(parameters.len())
            || contract.signature.is_comptime_callable()
        {
            return Ok(None);
        }
        *fuel = fuel
            .checked_sub(parameters.len())
            .ok_or(AdmissionError::Budget)?;
        let inputs: Vec<_> = method.terms().cloned().collect();
        let parameters: Vec<_> = parameters.iter().map(|p| p.declaration).collect();
        self.validate_function_inputs_in_scope(&parameters, &inputs, frames, fuel)
            .map_err(input_failure)?;
        let signature = self
            .function_substitution_bounded(&parameters, &inputs, frames, fuel)?
            .instantiate_bounded(
                contract.signature.ordinary().map_err(|e| e.to_string())?,
                fuel,
            )?;
        Ok(Some(Cow::Owned(signature)))
    }

    fn admitted_function_signature(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
        frames: &[Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Result<Option<Cow<'_, Term>>, AdmissionError> {
        *fuel = fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
        if let Some(graph) = self.graph {
            for argument in arguments {
                nir::type_computations::charge_term(argument, fuel)?;
            }
            let Some(index) = self.graph_indexes.as_ref().and_then(|indexes| {
                indexes
                    .callables
                    .get(&declaration)
                    .and_then(|instances| instances.get(arguments))
            }) else {
                return Ok(None);
            };
            let signature = graph.callable_identities[*index]
                .closed_signature()
                .map_err(|e| e.to_string())?;
            nir::type_computations::charge_term(signature, fuel)?;
            return Ok(Some(Cow::Borrowed(signature)));
        }
        if let Some(head) = self.normal_forms.get(&declaration) {
            if !arguments.is_empty() {
                return Ok(None);
            }
            *fuel = fuel
                .checked_sub(head.parameters.len())
                .ok_or(AdmissionError::Budget)?;
            for parameter in &head.parameters {
                nir::type_computations::charge_term(&parameter.ty, fuel)?;
            }
            return Ok(Some(Cow::Owned(
                head.callable_type().map_err(|e| e.to_string())?,
            )));
        }
        let (parameters, signature, qualifiers) =
            if let Some(identity) = self.runtime_function_identities.get(&declaration) {
                (&identity.type_parameters, &identity.signature, None)
            } else {
                let Some(function) = self.functions.get(&declaration) else {
                    return Ok(None);
                };
                (
                    &function.type_parameters,
                    &function.signature,
                    Some((
                        function.phase == nir::program::FunctionPhase::Comptime,
                        function.is_unsafe,
                    )),
                )
            };
        self.validate_function_inputs_in_scope(parameters, arguments, frames, fuel)
            .map_err(input_failure)?;
        let signature = self
            .function_substitution_bounded(parameters, arguments, frames, fuel)?
            .instantiate_bounded(signature, fuel)?;
        Ok(Some(Cow::Owned(match qualifiers {
            Some((comptime, is_unsafe)) => signature
                .with_comptime_phase(comptime)
                .with_unsafe_call_requirement(is_unsafe),
            None => signature,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_limits_do_not_become_missing_declarations() {
        let id = |local| DeclarationId { package: 23, local };
        let package = Package {
            normal_forms: vec![nir::normal_forms::Head {
                declaration: id(1),
                role: nir::normal_forms::Role::Opaque,
                collector: None,
                requirements: vec![],
                parameters: (2..6)
                    .map(|local| nir::normal_forms::Parameter {
                        declaration: id(local),
                        binding: LocalId {
                            owner: 1,
                            index: local,
                        },
                        kind: nir::normal_forms::Kind::Type,
                        ty: Term::Primitive(Primitive::Type),
                    })
                    .collect(),
            }],
            ..Default::default()
        };
        let vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut fuel = 3;
        assert_eq!(
            vm.admitted_function_signature(id(1), &[], &[], &mut fuel),
            Err(AdmissionError::Budget),
        );
        assert!(
            fuel > 0,
            "insufficient work allowance need not be exactly zero"
        );
        let mut fuel = 3;
        assert!(
            vm.normal_callable_signature_view(id(1), &[], &[], &mut fuel)
                .is_none()
        );
        assert_eq!(fuel, 0, "legacy optional consumers need the limit sentinel");
        let mut ready = false;
        for allowance in 0..1000 {
            match vm.admitted_function_signature(id(1), &[], &[], &mut { allowance }) {
                Ok(Some(_)) => {
                    ready = true;
                    break;
                }
                Err(AdmissionError::Budget) => {}
                other => panic!("valid signature rejected at budget {allowance}: {other:?}"),
            }
        }
        assert!(ready);
        assert_eq!(
            vm.admitted_function_signature(id(99), &[], &[], &mut 1000),
            Ok(None)
        );
        assert!(vm.events().is_empty());
    }
}
