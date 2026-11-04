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
            nir::value_admission::validate(
                argument,
                &domain,
                frames,
                fuel,
                &mut nir::value_admission::DomainQueries {
                    layout: &mut |ty, variant, fuel| {
                        self.admitted_nominal_layout(ty, variant, fuel)
                    },
                    declared: &mut |value, domain, _| {
                        matches!(value,
                        Term::Rigid { declaration, sort: nir::Sort::ComptimeValue }
                        if self.abstract_value_parameters.get(declaration) == Some(domain))
                    },
                    collection: &mut |domain, fuel| self.admitted_collection_domain(domain, fuel),
                    callable: &mut |value, frames, fuel| {
                        self.admitted_callable_signature(value, frames, fuel)
                    },
                    target: self.target,
                },
            )
            .map_err(|failure| {
                if matches!(failure, nir::value_admission::Error::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    rejection(
                        index,
                        binder.declaration,
                        InputAdmissionStage::Value,
                        failure,
                    )
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
