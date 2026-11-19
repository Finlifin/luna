//! Receiving-world callable signature queries. Lookup never invokes source
//! code, and resource exhaustion cannot become a negative membership result.
use super::*;
use nir::value_admission::Error as AdmissionError;
use std::borrow::Cow;

fn input_failure(error: ErrorKind) -> AdmissionError {
    match error {
        ErrorKind::StepsExhausted | ErrorKind::DepthExhausted => AdmissionError::Budget,
        ErrorKind::TypeContractUnresolved => {
            AdmissionError::Unresolved("callable input contract".into())
        }
        ErrorKind::FunctionInputRejected(reason) => AdmissionError::Invalid(reason.to_string()),
        error => AdmissionError::Invalid(format!("{error:?}")),
    }
}

impl Interpreter<'_> {
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
        if self.graph.is_some() {
            for term in method.terms() {
                nir::type_computations::charge_term(term, fuel)?;
            }
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
        let inputs: Vec<_> = method.terms().collect();
        let signature = self
            .admitted_function_inputs(
                parameters,
                &inputs,
                nir::DeclarationInputScope::Caller(frames),
                fuel,
            )
            .map_err(input_failure)?
            .instantiate(
                contract.signature.ordinary().map_err(|e| e.to_string())?,
                fuel,
            )?;
        Ok(Some(Cow::Owned(signature)))
    }

    pub(super) fn admitted_function_signature(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
        frames: &[Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Result<Option<Cow<'_, Term>>, AdmissionError> {
        *fuel = fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
        if let Some(graph) = self.graph {
            return self.iir_callable_signature(graph, declaration, arguments, fuel);
        }
        if let Some(head) = self.normal_forms.get(&declaration) {
            if !arguments.is_empty() {
                return Ok(None);
            }
            *fuel = fuel
                .checked_sub(head.parameters.len())
                .ok_or(AdmissionError::Budget)?;
            return Ok(Some(Cow::Owned(head.callable_type_in(fuel)?)));
        }
        let Some(formation) = self.functions.formation(&declaration) else {
            return Ok(None);
        };
        let signature = self
            .admitted_function_inputs(
                formation.parameters,
                arguments,
                nir::DeclarationInputScope::Caller(frames),
                fuel,
            )
            .map_err(input_failure)?
            .instantiate(formation.signature, fuel)?;
        Ok(Some(Cow::Owned(formation.qualify(signature))))
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
