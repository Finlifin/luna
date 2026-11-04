//! Ordinary callable input admission, including dependent domains and evidence.
//! Reject before body preparation or cache lookup; exhaustion is not rejection.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputAdmissionStage {
    Domain,
    Value,
}
impl fmt::Display for InputAdmissionStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Domain => "domain",
            Self::Value => "value",
        })
    }
}

/// One rejected slot in the complete declaration telescope, including implicit
/// lexical inputs. The identity is retained for frontend source-name lookup.
/// The underlying cause is retained without copying the argument or its domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionInputRejection {
    pub index: usize,
    pub parameter: DeclarationId,
    pub stage: InputAdmissionStage,
    pub reason: String,
}
impl fmt::Display for FunctionInputRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(function-input-rejected (index {}) (parameter {}) (stage {}) (reason {:?}))",
            self.index, self.parameter, self.stage, self.reason
        )
    }
}
fn rejection(
    index: usize,
    parameter: DeclarationId,
    stage: InputAdmissionStage,
    reason: impl fmt::Display,
) -> ErrorKind {
    ErrorKind::FunctionInputRejected(Box::new(FunctionInputRejection {
        index,
        parameter,
        stage,
        reason: reason.to_string(),
    }))
}

impl Interpreter<'_> {
    pub(super) fn validate_function_inputs(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<(), ErrorKind> {
        self.validate_function_inputs_in_scope(parameters, arguments, &[], fuel)
    }

    pub(super) fn validate_function_inputs_in_scope(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
        frames: &[Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Result<(), ErrorKind> {
        *fuel = fuel
            .checked_sub(parameters.len())
            .ok_or(ErrorKind::StepsExhausted)?;
        if parameters.len() != arguments.len() {
            return Err(ErrorKind::Type);
        }
        let substitution = self
            .function_substitution_bounded(parameters, arguments, frames, fuel)
            .map_err(|error| {
                if matches!(error, nir::type_computations::Error::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                }
            })?;
        let binders = self.input_binders(parameters);
        for (index, (binder, argument)) in binders.iter().zip(arguments).enumerate() {
            let raw = self
                .declaration_parameter_domains
                .get(&binder.declaration)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            let domain = substitution
                .instantiate_bounded(raw, fuel)
                .map_err(|error| {
                    if matches!(error, nir::type_computations::Error::Budget) {
                        ErrorKind::StepsExhausted
                    } else {
                        rejection(
                            index,
                            binder.declaration,
                            InputAdmissionStage::Domain,
                            error,
                        )
                    }
                })?;
            if domain == Term::Primitive(Primitive::Type) {
                continue;
            }
            if domain == Term::Primitive(Primitive::Object)
                || self.list_element(&domain).is_some()
                || matches!(domain.callable_signature(), Term::Function { .. })
            {
                nir::type_computations::charge_term(&domain, fuel)
                    .map_err(|_| ErrorKind::StepsExhausted)?;
                // Ordinary function clauses have the same value-domain contract
                // as normal-form inputs; this descriptor is only a checker,
                // never a new declaration or a type identity.
                let contract = nir::normal_forms::Head {
                    collector: None,
                    declaration: binder.declaration,
                    role: nir::normal_forms::Role::Opaque,
                    requirements: vec![],
                    parameters: vec![nir::normal_forms::Parameter {
                        declaration: binder.declaration,
                        binding: LocalId {
                            owner: binder.declaration.local,
                            index: 0,
                        },
                        kind: nir::normal_forms::Kind::Value,
                        ty: domain.clone(),
                    }],
                };
                contract.check_arguments_in_scope_with_callable_values_budgeted_in(
                    std::slice::from_ref(argument),
                    |id| self.normal_forms.get(&id).is_some_and(|head| head.role == nir::normal_forms::Role::Tuple),
                    |id| self.normal_forms.get(&id).is_some_and(|head| head.role == nir::normal_forms::Role::TypeList),
                    |ty, variant, fuel| self.normal_value_fields(ty, variant, fuel),
                    |value, domain| matches!(value, Term::Rigid { declaration, sort: nir::Sort::ComptimeValue }
                        if self.abstract_value_parameters.get(declaration) == Some(domain)),
                    |value, frames, fuel| self.normal_value_callable_signature_view(value, frames, fuel),
                    frames, fuel, self.target,
                ).map_err(|_| if *fuel == 0 {
                    ErrorKind::StepsExhausted
                } else {
                    // This legacy checker conflates some limits and type
                    // failures. Do not label an ambiguous result as a proven
                    // per-input rejection until it supplies a typed cause.
                    ErrorKind::Type
                })?;
                continue;
            }
            nir::validate_declaration_arguments_with_collections_in(
                &[binder.declaration],
                &[domain],
                std::slice::from_ref(argument),
                fuel,
                |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)), |domain, fuel| self.admitted_collection_domain(domain, fuel), self.target)
            .map_err(|failure| {
                if matches!(failure, nir::NominalAdmissionError::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    rejection(index, binder.declaration, InputAdmissionStage::Value, failure)
                }
            })?;
        }
        Ok(())
    }

    pub(super) fn admit_function_inputs(
        &mut self,
        parameters: &[DeclarationId],
        arguments: &[Term],
        source: SourceRange,
    ) -> Result<(), Error> {
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let checked = self.validate_function_inputs(parameters, arguments, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        checked.map_err(|kind| self.failure(kind, Some(source)))
    }
}
