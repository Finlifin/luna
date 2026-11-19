//! Ordinary callable input admission, including dependent domains and evidence.
//! Reject before body preparation or cache lookup; exhaustion is not rejection.
use super::*;

pub use nir::InputAdmissionStage;
use std::borrow::Borrow;

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

pub(super) use nir::function_parameters::ordered_domains as input_domains;

impl Interpreter<'_> {
    pub(super) fn admitted_function_inputs<'v, T: Borrow<Term>>(
        &self,
        parameters: &[DeclarationId],
        arguments: &'v [T],
        scope: nir::DeclarationInputScope<'v>,
        fuel: &mut usize,
    ) -> Result<nir::AdmittedArguments<'v, T>, ErrorKind> {
        let domains = input_domains(&self.declaration_parameter_domains, parameters, fuel)
            .map_err(|_| ErrorKind::StepsExhausted)?;
        self.with_admission_queries(|queries| {
            nir::admit_declaration_inputs(parameters, &domains, arguments, scope, 0, fuel, queries)
        })
        .map_err(|failure| match (failure.reason, failure.slot) {
            (nir::NominalAdmissionError::Budget, _) => ErrorKind::StepsExhausted,
            (nir::NominalAdmissionError::Unresolved(_), _) => ErrorKind::TypeContractUnresolved,
            (nir::NominalAdmissionError::Invalid(reason), Some((slot, stage))) => {
                rejection(slot, parameters[slot], stage, reason)
            }
            (nir::NominalAdmissionError::Invalid(_), None) => ErrorKind::Type,
        })
    }

    /// The receiving world reads the signature using the same checked inputs.
    /// No executable body or dispatch authorization is acquired by this query.
    pub(super) fn signature_from_inputs(
        &mut self,
        parameters: &[DeclarationId],
        arguments: &[Term],
        signature: &Term,
        source: SourceRange,
    ) -> Result<Term, Error> {
        let admitted = self.admit_function_inputs(
            parameters,
            arguments,
            nir::DeclarationInputScope::Caller(&[]),
            source,
        )?;
        self.query_admission(Some(source), |_, fuel| {
            Ok(admitted.instantiate(signature, fuel)?)
        })
    }

    pub(super) fn admit_function_inputs<'v, T: Borrow<Term>>(
        &mut self,
        parameters: &[DeclarationId],
        arguments: &'v [T],
        scope: nir::DeclarationInputScope<'v>,
        source: SourceRange,
    ) -> Result<nir::AdmittedArguments<'v, T>, Error> {
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let checked = self.admitted_function_inputs(parameters, arguments, scope, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        checked.map_err(|kind| self.failure(kind, Some(source)))
    }
}
